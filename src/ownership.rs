use anyhow::Result;
use git2::{ObjectType, Repository, TreeWalkMode, TreeWalkResult};
use rusqlite::params;
use std::collections::HashMap;

use crate::db::Database;
use crate::git::Commit;
use crate::identity::IdentityGroup;

pub struct OwnershipData {
    pub by_email: Vec<(String, String, usize)>,
}

pub fn compute(
    repo: &Repository,
    db: &Database,
    commits: &[Commit],
    groups: &[IdentityGroup],
    on_progress: impl Fn(usize, usize),
) -> Result<OwnershipData> {
    let head = repo.head()?.peel_to_commit()?;
    let head_hash = head.id().to_string();

    if db
        .conn
        .query_row(
            "SELECT 1 FROM ownership_meta WHERE head_hash = ?1",
            params![&head_hash],
            |_| Ok(()),
        )
        .is_ok()
    {
        let mut stmt = db.conn.prepare(
            "SELECT author_email, author_name, lines FROM ownership WHERE head_hash = ?1",
        )?;
        let rows = stmt.query_map(params![&head_hash], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? as usize,
            ))
        })?;
        let by_email: Vec<(String, String, usize)> = rows.collect::<Result<_, _>>()?;
        if !by_email.is_empty() {
            return Ok(OwnershipData { by_email });
        }
    }

    let tree = head.tree()?;
    let file_sizes = walk_tree_sizes(repo, &tree);
    let total_files = file_sizes.len();

    on_progress(0, total_files);

    let mut file_owner: HashMap<&str, usize> = HashMap::new();
    for commit in commits {
        for file in &commit.files {
            if !file_sizes.contains_key(file.as_str()) {
                continue;
            }
            file_owner.entry(file.as_str()).or_insert(commit.group_id);
        }
    }

    on_progress(total_files / 2, total_files);

    let mut group_lines: HashMap<usize, usize> = HashMap::new();
    let mut total_lines = 0usize;

    for (file, &gid) in &file_owner {
        if let Some(&size) = file_sizes.get(*file) {
            *group_lines.entry(gid).or_insert(0) += size;
            total_lines += size;
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

    on_progress(total_files, total_files);

    let tx = db.conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM ownership WHERE head_hash = ?1",
        params![&head_hash],
    )?;
    tx.execute(
        "DELETE FROM ownership_meta WHERE head_hash = ?1",
        params![&head_hash],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO ownership (head_hash, author_email, author_name, lines)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (email, name, lines) in &by_email {
            stmt.execute(params![&head_hash, email, name, *lines as i64])?;
        }
    }
    tx.execute(
        "INSERT INTO ownership_meta (head_hash, total_lines) VALUES (?1, ?2)",
        params![&head_hash, total_lines as i64],
    )?;
    tx.commit()?;

    Ok(OwnershipData { by_email })
}

fn walk_tree_sizes(repo: &Repository, tree: &git2::Tree) -> HashMap<String, usize> {
    let mut files = HashMap::new();
    tree.walk(TreeWalkMode::PreOrder, |dir, entry| {
        if entry.kind() == Some(ObjectType::Blob) {
            if let Some(name) = entry.name() {
                let path = format!("{}{}", dir, name);
                if !is_likely_binary(&path) {
                    let line_count = repo
                        .find_blob(entry.id())
                        .ok()
                        .filter(|b| !b.is_binary())
                        .map(|b| count_lines(b.content()))
                        .unwrap_or(0);
                    if line_count > 0 {
                        files.insert(path, line_count);
                    }
                }
            }
        }
        TreeWalkResult::Ok
    })
    .ok();
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
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".ico", ".webp", ".svg",
    ".woff", ".woff2", ".ttf", ".eot", ".otf",
    ".zip", ".gz", ".tar", ".bz2", ".xz", ".7z", ".rar",
    ".exe", ".dll", ".so", ".dylib", ".a", ".o", ".obj",
    ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt",
    ".mp3", ".mp4", ".avi", ".mov", ".wav", ".flac",
    ".pyc", ".class", ".wasm",
    ".lock", ".min.js", ".min.css",
    ".map", ".snap",
    ".db", ".sqlite", ".sqlite3",
    ".jar", ".war", ".ear",
    ".dmg", ".iso", ".img",
    ".tgz", ".gem", ".deb", ".rpm",
    ".psd", ".ai", ".sketch",
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

pub fn map_to_groups(
    data: &OwnershipData,
    groups: &[IdentityGroup],
) -> (Vec<usize>, usize) {
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
