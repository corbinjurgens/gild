use crate::cache::{Cache, CommitFiles};
use anyhow::{Context, Result};
use git2::{DiffLineType, DiffOptions, Repository, Sort};
use rayon::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

pub struct RepoInfo {
    pub name: String,
    pub branch: String,
}

/// Per-commit numeric stats kept resident in memory.
///
/// File-path lists live only in the SQLite `commit_files` table and are
/// fetched on demand by the detail view and add-ons; at ~80 bytes per record
/// this keeps a 1M-commit repo under ~80 MB resident.
#[derive(Clone)]
pub struct Commit {
    pub author_name: String,
    pub author_email: String,
    pub group_id: usize,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub files_changed: usize,
    pub timestamp: i64,
    pub whitespace_added: usize,
    pub whitespace_removed: usize,
    pub files_added: usize,
    pub files_deleted: usize,
    pub is_merge: bool,
}

// Only the OID and its slot index — everything else computed in the parallel phase.
struct PendingCommit {
    idx: usize,
    hash: String,
    oid: git2::Oid,
}

thread_local! {
    static THREAD_REPO: RefCell<Option<Repository>> = RefCell::new(None);
}

pub fn load_commits(
    path: &Path,
    branch: Option<&str>,
    max_commits: Option<usize>,
    cache: &mut Cache,
    on_total: impl Fn(usize),
    on_progress: impl Fn(usize, usize),
) -> Result<(RepoInfo, Repository, Vec<Commit>)> {
    let repo = Repository::open(path).context("Failed to open git repository")?;

    let (branch_name, head_target) = {
        let head = match branch {
            Some(b) => repo
                .find_branch(b, git2::BranchType::Local)
                .or_else(|_| repo.find_branch(b, git2::BranchType::Remote))
                .with_context(|| format!("Branch '{}' not found", b))?
                .into_reference(),
            None => repo.head().context("Failed to resolve HEAD")?,
        };
        let name = head.shorthand().unwrap_or("HEAD").to_string();
        let target = head.target().context("HEAD has no target")?;
        (name, target)
    };

    let repo_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let info = RepoInfo { name: repo_name, branch: branch_name };

    // Fast pre-count: OID-only walk so on_total fires before any diff work begins,
    // giving the TUI an immediate total to display.
    {
        let mut count_walk = repo.revwalk()?;
        count_walk.push(head_target)?;
        count_walk.set_sorting(Sort::TIME)?;
        let full = count_walk.count();
        on_total(max_commits.map_or(full, |m| m.min(full)));
    }

    // Phase 1: sequential walk — cache hits resolve immediately, misses collect
    // only the OID (no find_commit, no diff). This keeps phase 1 fast even for
    // large repos with cold caches.
    let mut revwalk = repo.revwalk()?;
    revwalk.push(head_target)?;
    revwalk.set_sorting(Sort::TIME)?;

    let mut ordered: Vec<Option<Commit>> = Vec::new();
    let mut pending: Vec<PendingCommit> = Vec::new();
    let mut cached_count = 0usize;

    for oid_result in revwalk {
        let oid = oid_result?;
        let hash = oid.to_string();
        let idx = ordered.len();

        if let Some(cached) = cache.get(&hash) {
            ordered.push(Some(cached));
            cached_count += 1;
            // Throttled progress for warm-cache runs.
            if cached_count % 200 == 0 {
                on_progress(cached_count, 0);
            }
        } else {
            pending.push(PendingCommit { idx, hash, oid });
            ordered.push(None);
        }

        if let Some(max) = max_commits {
            if ordered.len() >= max {
                break;
            }
        }
    }

    // Flush any remaining cached-commit progress.
    if cached_count > 0 {
        on_progress(cached_count, 0);
    }

    // Phase 2: parallel diff computation.
    // Each rayon OS thread opens its own Repository once (thread-local) and reuses it.
    // Small chunks (≈1% of work) keep the progress bar fluid.
    let new_total = pending.len();

    if new_total > 0 {
        let chunk_size = (new_total / 100).max(8);
        let mut new_done = 0usize;

        for chunk in pending.chunks(chunk_size) {
            let results = chunk
                .par_iter()
                .map(|p| -> Result<(usize, String, Commit, CommitFiles)> {
                    THREAD_REPO.with(|cell| {
                        let mut opt = cell.borrow_mut();
                        if opt.is_none() {
                            *opt = Some(
                                Repository::open(path)
                                    .context("Failed to open repo in worker thread")?,
                            );
                        }
                        let repo = opt.as_ref().unwrap();
                        build_commit(repo, p)
                    })
                })
                .collect::<Result<Vec<_>>>()?;

            for (idx, hash, commit, files) in results {
                ordered[idx] = Some(commit.clone());
                cache.insert(hash, commit, files);
                new_done += 1;
            }
            on_progress(cached_count + new_done, new_done);
        }
    }

    // Phase 3: flatten ordered slots into a time-sorted Vec<Commit>.
    let commits: Vec<Commit> = ordered
        .into_iter()
        .map(|opt| opt.expect("unfilled commit slot"))
        .collect();

    Ok((info, repo, commits))
}

fn build_commit(
    repo: &Repository,
    p: &PendingCommit,
) -> Result<(usize, String, Commit, CommitFiles)> {
    let commit = repo.find_commit(p.oid)?;
    let tree = commit.tree()?;
    let is_merge = commit.parent_count() > 1;
    let author_name = commit.author().name().unwrap_or("Unknown").to_string();
    let author_email = commit
        .author()
        .email()
        .unwrap_or("unknown@unknown")
        .to_string();
    let timestamp = commit.time().seconds();

    let parent_tree = if is_merge {
        let pa = commit.parent_id(0)?;
        let pb = commit.parent_id(1)?;
        repo.merge_base(pa, pb)
            .ok()
            .and_then(|b| repo.find_commit(b).ok())
            .and_then(|c| c.tree().ok())
    } else if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };

    let mut opts = DiffOptions::new();
    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;
    let stats = diff.stats()?;

    let mut files = Vec::new();
    let mut added = Vec::new();
    let mut deleted = Vec::new();
    for d in diff.deltas() {
        if let Some(fp) = d.new_file().path().or_else(|| d.old_file().path()) {
            let name = fp.to_string_lossy().to_string();
            match d.status() {
                git2::Delta::Added => added.push(name.clone()),
                git2::Delta::Deleted => deleted.push(name.clone()),
                _ => {}
            }
            files.push(name);
        }
    }
    let files_added = added.len();
    let files_deleted = deleted.len();

    let total_changed = stats.insertions() + stats.deletions();
    let (ws_add, ws_rm) = if total_changed <= 2000 {
        count_whitespace_lines(&diff)
    } else {
        (0, 0)
    };

    Ok((
        p.idx,
        p.hash.clone(),
        Commit {
            author_name,
            author_email,
            group_id: 0,
            lines_added: stats.insertions(),
            lines_removed: stats.deletions(),
            files_changed: stats.files_changed(),
            timestamp,
            whitespace_added: ws_add,
            whitespace_removed: ws_rm,
            files_added,
            files_deleted,
            is_merge,
        },
        CommitFiles { files, added, deleted },
    ))
}

fn count_whitespace_lines(diff: &git2::Diff<'_>) -> (usize, usize) {
    use std::cell::Cell;

    // (file_idx, trimmed bytes) -> [added_count, removed_count]
    let mut counts: HashMap<(u32, Vec<u8>), [u32; 2]> = HashMap::new();
    let file_idx = Cell::new(0u32);

    let _ = diff.foreach(
        &mut |_delta, _progress| {
            file_idx.set(file_idx.get() + 1);
            true
        },
        None,
        None,
        Some(&mut |_delta, _hunk, line| {
            let slot = match line.origin_value() {
                DiffLineType::Addition => 0,
                DiffLineType::Deletion => 1,
                _ => return true,
            };
            let original = strip_trailing_newline(line.content());
            let trimmed = trim_ascii_ws(original);
            if !trimmed.is_empty() && trimmed.len() != original.len() {
                let key = (file_idx.get(), trimmed.to_vec());
                counts.entry(key).or_insert([0, 0])[slot] += 1;
            }
            true
        }),
    );

    let paired: u32 = counts.values().map(|[a, r]| (*a).min(*r)).sum();
    (paired as usize, paired as usize)
}

fn strip_trailing_newline(mut bytes: &[u8]) -> &[u8] {
    if let Some(b'\n') = bytes.last().copied() {
        bytes = &bytes[..bytes.len() - 1];
    }
    if let Some(b'\r') = bytes.last().copied() {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn trim_ascii_ws(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &bytes[start..end]
}
