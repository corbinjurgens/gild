use crate::cache::{Cache, CommitFiles};
use anyhow::{Context, Result};
use gix::bstr::ByteSlice;
use gix::object::tree::diff::{Action, Change};
use rayon::prelude::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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
    pub files_renamed: usize,
    pub is_merge: bool,
}

struct PendingCommit {
    idx: usize,
    oid: gix::ObjectId,
}

thread_local! {
    static THREAD_REPO: RefCell<Option<gix::Repository>> = const { RefCell::new(None) };
}

const OBJECT_CACHE_BYTES: usize = 64 * 1024 * 1024;

pub fn load_commits(
    path: &Path,
    branch: Option<&str>,
    max_commits: Option<usize>,
    cache: &mut Cache,
    on_total: impl Fn(usize),
    on_progress: impl Fn(usize, usize),
) -> Result<(RepoInfo, Vec<Commit>)> {
    let mut repo = gix::open(path).context("Failed to open git repository")?;
    repo.object_cache_size_if_unset(OBJECT_CACHE_BYTES);

    let (branch_name, head_target) = resolve_head(&repo, branch)?;

    let repo_name = name_from_origin(&repo)
        .or_else(|| path.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    let info = RepoInfo {
        name: repo_name,
        branch: branch_name,
    };

    let walk = repo
        .rev_walk([head_target])
        .sorting(gix::revision::walk::Sorting::ByCommitTime(
            gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
        ))
        .all()?;

    let mut ordered: Vec<Option<Commit>> = Vec::new();
    let mut pending: Vec<PendingCommit> = Vec::new();
    let mut cached_count = 0usize;

    for info_result in walk {
        let walk_info = info_result?;
        let oid = walk_info.id;
        let hash = oid.to_string();
        let idx = ordered.len();

        if let Some(cached) = cache.get(&hash) {
            ordered.push(Some(cached));
            cached_count += 1;
            if cached_count % 200 == 0 {
                on_progress(cached_count, 0);
            }
        } else {
            pending.push(PendingCommit { idx, oid });
            ordered.push(None);
        }

        if let Some(max) = max_commits {
            if ordered.len() >= max {
                break;
            }
        }
    }

    on_total(ordered.len());
    if cached_count > 0 {
        on_progress(cached_count, 0);
    }

    let new_total = pending.len();

    if new_total > 0 {
        let safe_repo = repo.into_sync();
        let chunk_size = (new_total / 100).max(8);
        let mut new_done = 0usize;

        for chunk in pending.chunks(chunk_size) {
            let results = chunk
                .par_iter()
                .map(|p| -> Result<(usize, String, Commit, CommitFiles)> {
                    THREAD_REPO.with(|cell| {
                        let mut opt = cell.borrow_mut();
                        if opt.is_none() {
                            let mut r = safe_repo.to_thread_local();
                            r.object_cache_size_if_unset(OBJECT_CACHE_BYTES);
                            *opt = Some(r);
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

    let commits: Vec<Commit> = ordered
        .into_iter()
        .enumerate()
        .map(|(i, opt)| {
            opt.ok_or_else(|| {
                anyhow::anyhow!("commit slot {i} was never filled — possible gap in walk")
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok((info, commits))
}

fn name_from_origin(repo: &gix::Repository) -> Option<String> {
    let remote = repo.find_remote("origin").ok()?;
    let url = remote.url(gix::remote::Direction::Fetch)?;
    let path = url.path.to_str().ok()?;
    let path = path.strip_prefix('/').unwrap_or(path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let path = path.strip_suffix('/').unwrap_or(path);
    if path.is_empty() { None } else { Some(path.to_string()) }
}

fn resolve_head(repo: &gix::Repository, branch: Option<&str>) -> Result<(String, gix::ObjectId)> {
    match branch {
        Some(b) => {
            let full = format!("refs/heads/{b}");
            let r = repo
                .find_reference(&full)
                .or_else(|_| repo.find_reference(&format!("refs/remotes/{b}")))
                .with_context(|| format!("Branch '{b}' not found"))?;
            let name = r.name().shorten().to_str().unwrap_or(b).to_string();
            let oid = r.into_fully_peeled_id()?.detach();
            Ok((name, oid))
        }
        None => {
            let head = repo.head().context("Failed to resolve HEAD")?;
            let name = head
                .referent_name()
                .map(|n| n.shorten().to_str().unwrap_or("HEAD").to_string())
                .unwrap_or_else(|| "HEAD".to_string());
            let oid = head
                .into_peeled_id()
                .context("HEAD has no target")?
                .detach();
            Ok((name, oid))
        }
    }
}

fn build_commit(
    repo: &gix::Repository,
    p: &PendingCommit,
) -> Result<(usize, String, Commit, CommitFiles)> {
    let commit = repo.find_commit(p.oid)?;
    let decoded = commit.decode()?;
    let author = decoded.author()?;
    let author_name = author.name.to_str().unwrap_or("Unknown").to_string();
    let author_email = author
        .email
        .to_str()
        .unwrap_or("unknown@unknown")
        .to_string();
    let timestamp = decoded.committer()?.seconds();

    let parent_ids: Vec<gix::ObjectId> = decoded.parents().collect();
    let is_merge = parent_ids.len() > 1;

    let tree = commit.tree()?;

    let parent_tree = if is_merge {
        parent_ids
            .first()
            .and_then(|&pid| repo.find_commit(pid).ok().and_then(|c| c.tree().ok()))
    } else if let Some(&pid) = parent_ids.first() {
        Some(repo.find_commit(pid)?.tree()?)
    } else {
        None
    };

    let from = parent_tree.unwrap_or_else(|| repo.empty_tree());

    let mut files = Vec::new();
    let mut added_files = Vec::new();
    let mut deleted_files = Vec::new();
    let mut renamed_count = 0usize;
    let mut lines_added = 0usize;
    let mut lines_removed = 0usize;
    let mut total_changed = 0usize;

    let mut whitespace_paired = 0usize;

    if is_merge {
        let mut changed_vs_p1: HashSet<String> = HashSet::new();
        from.changes()?
            .for_each_to_obtain_tree(&tree, |change| {
                let entry_mode = match &change {
                    Change::Addition { entry_mode, .. }
                    | Change::Deletion { entry_mode, .. }
                    | Change::Modification { entry_mode, .. } => *entry_mode,
                    Change::Rewrite { entry_mode, .. } => *entry_mode,
                };
                if !entry_mode.is_blob() {
                    return Ok(Action::Continue(()));
                }
                let path = change.location().to_str_lossy().into_owned();
                if !path.is_empty() {
                    match &change {
                        Change::Addition { .. } => added_files.push(path.clone()),
                        Change::Deletion { .. } => deleted_files.push(path.clone()),
                        Change::Rewrite { .. } => renamed_count += 1,
                        _ => {}
                    }
                    files.push(path.clone());
                    changed_vs_p1.insert(path);
                }
                Ok::<_, anyhow::Error>(Action::Continue(()))
            })
            .context("tree diff failed")?;

        if let Some(&p2) = parent_ids.get(1) {
            if let Some(p2_tree) = repo.find_commit(p2).ok().and_then(|c| c.tree().ok()) {
                let mut conflict_files: HashSet<String> = HashSet::new();
                let _ = p2_tree.changes().map(|mut changes| {
                    let _ = changes.for_each_to_obtain_tree(&tree, |change| {
                        if !match &change {
                            Change::Addition { entry_mode, .. }
                            | Change::Deletion { entry_mode, .. }
                            | Change::Modification { entry_mode, .. }
                            | Change::Rewrite { entry_mode, .. } => entry_mode.is_blob(),
                        } {
                            return Ok(Action::Continue(()));
                        }
                        let path = change.location().to_str_lossy().into_owned();
                        if !path.is_empty() && changed_vs_p1.contains(&path) {
                            conflict_files.insert(path);
                        }
                        Ok::<_, anyhow::Error>(Action::Continue(()))
                    });
                });

                if !conflict_files.is_empty() {
                    let mut resource_cache = repo.diff_resource_cache_for_tree_diff()?;
                    from.changes()?
                        .for_each_to_obtain_tree(&tree, |change| {
                            if !match &change {
                                Change::Addition { entry_mode, .. }
                                | Change::Deletion { entry_mode, .. }
                                | Change::Modification { entry_mode, .. }
                                | Change::Rewrite { entry_mode, .. } => entry_mode.is_blob(),
                            } {
                                return Ok(Action::Continue(()));
                            }
                            let path = change.location().to_str_lossy();
                            if !conflict_files.contains(path.as_ref()) {
                                return Ok(Action::Continue(()));
                            }
                            if let Ok(mut platform) = change.diff(&mut resource_cache) {
                                if let Some(c) = platform.line_counts().ok().flatten() {
                                    lines_added += c.insertions as usize;
                                    lines_removed += c.removals as usize;
                                }
                            }
                            Ok::<_, anyhow::Error>(Action::Continue(()))
                        })
                        .context("merge conflict diff failed")?;
                }
            }
        }
    } else {
        let mut resource_cache = repo.diff_resource_cache_for_tree_diff()?;
        let mut ws_file_idx = 0u32;
        let mut ws_counts: HashMap<(u32, u64), [u32; 2]> = HashMap::new();

        from.changes()?
            .for_each_to_obtain_tree(&tree, |change| {
                let entry_mode = match &change {
                    Change::Addition { entry_mode, .. }
                    | Change::Deletion { entry_mode, .. }
                    | Change::Modification { entry_mode, .. } => *entry_mode,
                    Change::Rewrite { entry_mode, .. } => *entry_mode,
                };
                if !entry_mode.is_blob() {
                    return Ok(Action::Continue(()));
                }
                let path = change.location().to_str_lossy().into_owned();
                if !path.is_empty() {
                    match &change {
                        Change::Addition { .. } => added_files.push(path.clone()),
                        Change::Deletion { .. } => deleted_files.push(path.clone()),
                        Change::Rewrite { .. } => renamed_count += 1,
                        _ => {}
                    }
                    files.push(path);
                }
                ws_file_idx += 1;

                if let Ok(mut platform) = change.diff(&mut resource_cache) {
                    if total_changed <= 2000 {
                        let mut file_added = 0usize;
                        let mut file_removed = 0usize;
                        let fidx = ws_file_idx;
                        let ws = &mut ws_counts;
                        let _ = platform.lines(|hunk| {
                            use gix::object::blob::diff::lines;
                            match hunk {
                                lines::Change::Addition { lines } => {
                                    file_added += lines.len();
                                    for line in lines {
                                        check_ws_line(ws, fidx, line, 0);
                                    }
                                }
                                lines::Change::Deletion { lines } => {
                                    file_removed += lines.len();
                                    for line in lines {
                                        check_ws_line(ws, fidx, line, 1);
                                    }
                                }
                                lines::Change::Modification {
                                    lines_before,
                                    lines_after,
                                } => {
                                    file_removed += lines_before.len();
                                    file_added += lines_after.len();
                                    for line in lines_before {
                                        check_ws_line(ws, fidx, line, 1);
                                    }
                                    for line in lines_after {
                                        check_ws_line(ws, fidx, line, 0);
                                    }
                                }
                            }
                            Ok::<_, std::convert::Infallible>(())
                        });
                        lines_added += file_added;
                        lines_removed += file_removed;
                        total_changed += file_added + file_removed;
                    } else {
                        if let Some(c) = platform.line_counts().ok().flatten() {
                            lines_added += c.insertions as usize;
                            lines_removed += c.removals as usize;
                            total_changed += c.insertions as usize + c.removals as usize;
                        }
                    }
                }

                Ok::<_, anyhow::Error>(Action::Continue(()))
            })
            .context("tree diff failed")?;

        let ws_paired: u32 = ws_counts.values().map(|[a, r]| (*a).min(*r)).sum();
        whitespace_paired = ws_paired as usize;
    }

    let files_changed = files.len();

    Ok((
        p.idx,
        p.oid.to_string(),
        Commit {
            author_name,
            author_email,
            group_id: 0,
            lines_added,
            lines_removed,
            files_changed,
            timestamp,
            whitespace_added: whitespace_paired,
            whitespace_removed: whitespace_paired,
            files_added: added_files.len(),
            files_deleted: deleted_files.len(),
            files_renamed: renamed_count,
            is_merge,
        },
        CommitFiles {
            files,
            added: added_files,
            deleted: deleted_files,
        },
    ))
}

fn hash_trimmed(bytes: &[u8]) -> u64 {
    xxhash_rust::xxh3::xxh3_64(bytes)
}

/// Whitespace detection using a hash of trimmed content instead of storing
/// the full line bytes. Trades a negligible collision risk for zero heap
/// allocation per line.
fn check_ws_line(
    counts: &mut HashMap<(u32, u64), [u32; 2]>,
    file_idx: u32,
    line: &gix::bstr::BStr,
    slot: usize,
) {
    let bytes = line.as_bytes();
    let stripped = match bytes.last() {
        Some(b'\r') => &bytes[..bytes.len() - 1],
        _ => bytes,
    };
    let trimmed = trim_ascii_ws(stripped);
    if !trimmed.is_empty() && trimmed.len() != stripped.len() {
        let key = (file_idx, hash_trimmed(trimmed));
        counts.entry(key).or_insert([0, 0])[slot] += 1;
    }
}

// Manual impl to avoid requiring Rust 1.80+ for <[u8]>::trim_ascii()
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
