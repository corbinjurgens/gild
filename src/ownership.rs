use anyhow::Result;
use gix::bstr::ByteSlice;
use rayon::prelude::*;
use rusqlite::params;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::db::Database;
use crate::identity::IdentityGroup;

pub struct OwnershipData {
    pub by_email: Vec<(String, String, usize)>,
}

// Cache key prefix distinguishes blame-based data from legacy last-touch data.
const CACHE_PREFIX: &str = "blame:";

// Files blamed per DB-commit batch. Large enough to amortize transaction
// overhead; small enough that a Ctrl-C loses at most a few minutes of work.
const BLAME_CHUNK: usize = 500;

// Per-rayon-worker gix Repository. Each thread converts the shared
// ThreadSafeRepository once and reuses it.
thread_local! {
    static THREAD_REPO: RefCell<Option<gix::Repository>> = const { RefCell::new(None) };
}

pub fn compute(
    repo: &gix::Repository,
    db: &Database,
    groups: &[IdentityGroup],
    on_progress: impl Fn(usize, usize) + Send,
) -> Result<OwnershipData> {
    let head = repo.head()?.into_peeled_id()?;
    let cache_key = format!("{CACHE_PREFIX}{head}");

    // Fast path: a prior run finished and wrote the aggregate.
    // Return whatever is in `ownership` even if empty — an empty result is
    // still a valid cache entry (e.g. repo has no matchable authors), and
    // falling through here would pointlessly re-blame every time.
    if db
        .query_row(
            "SELECT 1 FROM ownership_meta WHERE head_hash = ?1",
            params![&cache_key],
            |_| Ok(()),
        )
        .is_ok()
    {
        return load_aggregate(db, &cache_key);
    }

    let file_sizes = walk_tree_sizes(repo);
    let total_files = file_sizes.len();

    // Preload every known commit's author email. A typical blame hunk resolves
    // its final_commit_id via `repo.find_commit(oid)` — doing that per hunk
    // across ~30k files dominates CPU once parallelism is saturated. One
    // SQLite read gives us an O(1) process-wide lookup instead.
    let oid_to_email: HashMap<String, String> = {
        let mut stmt = db.prepare("SELECT hash, LOWER(author_email) FROM commits")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    // Is the commits table a superset of HEAD's history? The fast path below
    // is safe as long as every commit reachable from HEAD is in the table.
    // Extras (stale rows from previous HEADs or other branches mapped to the
    // same origin URL) only make per-file author counts over-estimate, which
    // is conservative — we fall back to blame, never mis-attribute.
    // If HEAD itself is missing or the table has fewer rows than the revwalk,
    // the cache is truncated (e.g. --max-commits) and the heuristic is unsafe.
    let commits_complete = {
        let head_str = head.to_string();
        let head_in_table: bool = db
            .query_row(
                "SELECT 1 FROM commits WHERE hash = ?1",
                params![&head_str],
                |_| Ok(()),
            )
            .is_ok();
        let reachable = repo
            .rev_walk([head.detach()])
            .all()?
            .filter_map(|r| r.ok())
            .count() as i64;
        let in_table: i64 = db.query_row("SELECT COUNT(*) FROM commits", [], |r| r.get(0))?;
        head_in_table && in_table >= reachable
    };

    // Fast path: files whose entire commit history has exactly one unique
    // author must be 100% that author's code at HEAD — blame would attribute
    // every line to them anyway. Skip the blame and write
    // straight to blame_file.
    let solo_authored: HashMap<String, String> = if commits_complete {
        let mut stmt = db.prepare(
            "SELECT cf.file_path, LOWER(MIN(c.author_email))
             FROM commit_files cf
             JOIN commits c ON c.hash = cf.commit_hash
             WHERE cf.kind = 0
             GROUP BY cf.file_path
             HAVING COUNT(DISTINCT LOWER(c.author_email)) = 1",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.filter_map(|r| r.ok()).collect()
    } else {
        HashMap::new()
    };

    // Resume: a file has rows in blame_file → already processed in a prior
    // run. Empty-blame files write a sentinel row (author_email='') so they
    // also count as done.
    let already_done: HashSet<String> = {
        let mut stmt =
            db.prepare("SELECT DISTINCT file_path FROM blame_file WHERE head_hash = ?1")?;
        let rows = stmt.query_map(params![&cache_key], |r| r.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    // Partition remaining work: solo-authored files bypass blame, the rest
    // go through the chunked parallel blame loop below.
    let mut solo_rows: Vec<(String, String, usize)> = Vec::new();
    let mut multi_files: Vec<String> = Vec::new();
    for (path, size) in &file_sizes {
        if already_done.contains(path) {
            continue;
        }
        if let Some(email) = solo_authored.get(path) {
            solo_rows.push((path.clone(), email.clone(), *size));
        } else {
            multi_files.push(path.clone());
        }
    }

    if !solo_rows.is_empty() {
        let tx = db.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO blame_file
                    (head_hash, file_path, author_email, lines)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (file, email, lines) in &solo_rows {
                stmt.execute(params![&cache_key, file, email, *lines as i64])?;
            }
        }
        tx.commit()?;
    }

    let initial_done = already_done.len() + solo_rows.len();
    on_progress(initial_done, total_files);

    let safe_repo = repo.clone().into_sync();
    let gix_head_oid = head.detach();
    let done = AtomicUsize::new(initial_done);
    let cb = Mutex::new(on_progress);
    let oid_map = &oid_to_email;

    for chunk in multi_files.chunks(BLAME_CHUNK) {
        let results: Vec<(String, HashMap<String, usize>)> = chunk
            .par_iter()
            .map(|file_path| {
                let local = THREAD_REPO.with(|cell| {
                    let mut opt = cell.borrow_mut();
                    if opt.is_none() {
                        let mut r = safe_repo.to_thread_local();
                        r.object_cache_size_if_unset(4 * 1024 * 1024);
                        *opt = Some(r);
                    }
                    match opt.as_ref() {
                        Some(r) => blame_file_lines(r, file_path, gix_head_oid, oid_map),
                        None => HashMap::new(),
                    }
                });
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                if let Ok(guard) = cb.lock() {
                    (*guard)(d, total_files);
                }
                (file_path.clone(), local)
            })
            .collect();

        let tx = db.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO blame_file
                    (head_hash, file_path, author_email, lines)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (file_path, emails) in &results {
                if emails.is_empty() {
                    stmt.execute(params![&cache_key, file_path, "", 0i64])?;
                } else {
                    for (email, lines) in emails {
                        stmt.execute(params![&cache_key, file_path, email, *lines as i64])?;
                    }
                }
            }
        }
        tx.commit()?;
    }

    // Aggregate per-file rows into per-group totals. Email comparison is
    // lowercase to match the identity merge normalization.
    let mut email_to_gid: HashMap<String, usize> = HashMap::new();
    for (gid, group) in groups.iter().enumerate() {
        for (_name, email) in &group.aliases {
            email_to_gid.insert(email.to_lowercase(), gid);
        }
    }

    let mut group_lines: HashMap<usize, usize> = HashMap::new();
    let mut total_lines = 0usize;
    {
        let mut stmt = db.prepare(
            "SELECT author_email, SUM(lines) FROM blame_file
             WHERE head_hash = ?1 AND author_email != ''
             GROUP BY author_email",
        )?;
        let rows = stmt.query_map(params![&cache_key], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
        })?;
        for row in rows {
            let (email, lines) = row?;
            if let Some(&gid) = email_to_gid.get(&email) {
                *group_lines.entry(gid).or_insert(0) += lines;
                total_lines += lines;
            }
        }
    }

    let mut by_email: Vec<(String, String, usize)> = Vec::new();
    for (&gid, &lines) in &group_lines {
        if let Some(group) = groups.get(gid) {
            if let Some((name, email)) = group.aliases.first() {
                by_email.push((email.clone(), name.clone(), lines));
            }
        }
    }

    // Persist the aggregate and drop the per-file detail — we don't need it
    // anymore once ownership_meta marks this HEAD as complete.
    let tx = db.transaction()?;
    tx.execute(
        "DELETE FROM ownership WHERE head_hash = ?1",
        params![&cache_key],
    )?;
    tx.execute(
        "DELETE FROM ownership_meta WHERE head_hash = ?1",
        params![&cache_key],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO ownership (head_hash, author_email, author_name, lines)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (email, name, lines) in &by_email {
            stmt.execute(params![&cache_key, email, name, *lines as i64])?;
        }
    }
    tx.execute(
        "INSERT INTO ownership_meta (head_hash, total_lines) VALUES (?1, ?2)",
        params![&cache_key, total_lines as i64],
    )?;
    tx.execute(
        "DELETE FROM blame_file WHERE head_hash = ?1",
        params![&cache_key],
    )?;
    tx.commit()?;

    Ok(OwnershipData { by_email })
}

fn load_aggregate(db: &Database, cache_key: &str) -> Result<OwnershipData> {
    let mut stmt =
        db.prepare("SELECT author_email, author_name, lines FROM ownership WHERE head_hash = ?1")?;
    let rows = stmt.query_map(params![cache_key], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)? as usize,
        ))
    })?;
    let by_email = rows.collect::<Result<Vec<_>, _>>()?;
    Ok(OwnershipData { by_email })
}

fn blame_file_lines(
    repo: &gix::Repository,
    file_path: &str,
    head_oid: gix::ObjectId,
    oid_to_email: &HashMap<String, String>,
) -> HashMap<String, usize> {
    let outcome =
        match repo.blame_file(file_path.as_bytes().as_bstr(), head_oid, Default::default()) {
            Ok(o) => o,
            Err(_) => return HashMap::new(),
        };

    let mut local: HashMap<String, usize> = HashMap::new();
    for entry in &outcome.entries {
        let oid_str = entry.commit_id.to_string();
        if let Some(email) = oid_to_email.get(&oid_str) {
            *local.entry(email.clone()).or_insert(0) += entry.len.get() as usize;
        } else if let Some(email) = resolve_commit_email(repo, entry.commit_id) {
            *local.entry(email).or_insert(0) += entry.len.get() as usize;
        }
    }
    local
}

fn resolve_commit_email(repo: &gix::Repository, oid: gix::ObjectId) -> Option<String> {
    let commit = repo.find_object(oid).ok()?.try_into_commit().ok()?;
    let decoded = commit.decode().ok()?;
    let sig = decoded.author().ok()?;
    Some(sig.email.to_str().ok()?.to_lowercase())
}

pub fn walk_tree_sizes(repo: &gix::Repository) -> HashMap<String, usize> {
    let tree_id = match repo.head_tree_id() {
        Ok(id) => id,
        Err(_) => return HashMap::new(),
    };
    let tree = match tree_id.object().map(|o| o.into_tree()) {
        Ok(t) => t,
        Err(_) => return HashMap::new(),
    };

    let mut files = HashMap::new();
    let mut recorder = gix::traverse::tree::Recorder::default();
    if tree.traverse().breadthfirst(&mut recorder).is_err() {
        return files;
    }
    for entry in recorder.records {
        if !entry.mode.is_blob() {
            continue;
        }
        let path = entry.filepath.to_str_lossy().into_owned();
        if is_likely_binary(&path) {
            continue;
        }
        if let Ok(obj) = repo.find_object(entry.oid) {
            let data: &[u8] = obj.data.as_ref();
            if data.contains(&0) {
                continue;
            }
            let line_count = count_lines(data);
            if line_count > 0 {
                files.insert(path, line_count);
            }
        }
    }
    files
}

fn count_lines(content: &[u8]) -> usize {
    let nl = content.iter().filter(|&&b| b == b'\n').count();
    match content.last() {
        None => 0,
        Some(&b'\n') => nl,
        Some(_) => nl + 1,
    }
}

const BINARY_EXTS: &[&str] = &[
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".bmp",
    ".ico",
    ".webp",
    ".svg",
    ".woff",
    ".woff2",
    ".ttf",
    ".eot",
    ".otf",
    ".zip",
    ".gz",
    ".tar",
    ".bz2",
    ".xz",
    ".7z",
    ".rar",
    ".exe",
    ".dll",
    ".so",
    ".dylib",
    ".a",
    ".o",
    ".obj",
    ".pdf",
    ".doc",
    ".docx",
    ".xls",
    ".xlsx",
    ".ppt",
    ".mp3",
    ".mp4",
    ".avi",
    ".mov",
    ".wav",
    ".flac",
    ".pyc",
    ".class",
    ".wasm",
    ".lock",
    ".min.js",
    ".min.css",
    ".map",
    ".snap",
    ".db",
    ".sqlite",
    ".sqlite3",
    ".jar",
    ".war",
    ".ear",
    ".dmg",
    ".iso",
    ".img",
    ".tgz",
    ".gem",
    ".deb",
    ".rpm",
    ".psd",
    ".ai",
    ".sketch",
    ".DS_Store",
];

fn is_likely_binary(path: &str) -> bool {
    let bytes = path.as_bytes();
    BINARY_EXTS.iter().any(|ext| {
        let ext_bytes = ext.as_bytes();
        bytes.len() >= ext_bytes.len()
            && bytes[bytes.len() - ext_bytes.len()..].eq_ignore_ascii_case(ext_bytes)
    })
}

pub fn map_to_groups(data: &OwnershipData, groups: &[IdentityGroup]) -> (Vec<usize>, usize) {
    let mut email_to_gid: HashMap<String, usize> = HashMap::new();
    for (gid, group) in groups.iter().enumerate() {
        for (_name, email) in &group.aliases {
            email_to_gid.insert(email.clone(), gid);
        }
    }

    let mut per_group = vec![0usize; groups.len()];
    let mut mapped_total = 0usize;
    for (email, _name, lines) in &data.by_email {
        if let Some(&gid) = email_to_gid.get(email) {
            per_group[gid] += lines;
            mapped_total += lines;
        }
    }
    (per_group, mapped_total)
}
