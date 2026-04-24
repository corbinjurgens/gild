use crate::cache::Cache;
use anyhow::{Context, Result};
use git2::{DiffLineType, DiffOptions, Repository, Sort};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub struct RepoInfo {
    pub name: String,
    pub branch: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Commit {
    pub author_name: String,
    pub author_email: String,
    #[serde(skip)]
    pub group_id: usize,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub files_changed: usize,
    pub timestamp: i64,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub whitespace_added: usize,
    #[serde(default)]
    pub whitespace_removed: usize,
    #[serde(default)]
    pub files_added: usize,
    #[serde(default)]
    pub files_deleted: usize,
    #[serde(default)]
    pub added_file_names: Vec<String>,
    #[serde(default)]
    pub deleted_file_names: Vec<String>,
    #[serde(default)]
    pub is_merge: bool,
}

pub fn load_commits(
    path: &Path,
    branch: Option<&str>,
    max_commits: Option<usize>,
    cache: &mut Cache,
    on_progress: impl Fn(usize, usize),
) -> Result<(RepoInfo, Repository, Vec<Commit>)> {
    let repo = Repository::open(path).context("Failed to open git repository")?;

    let (branch_name, head_target) = {
        let head = match branch {
            Some(b) => repo
                .find_branch(b, git2::BranchType::Local)
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

    let info = RepoInfo {
        name: repo_name,
        branch: branch_name,
    };

    let mut revwalk = repo.revwalk()?;
    revwalk.push(head_target)?;
    revwalk.set_sorting(Sort::TIME)?;

    let mut commits = Vec::new();
    let mut new_count = 0;

    for oid_result in revwalk {
        let oid = oid_result?;
        let hash = oid.to_string();
        let commit = repo.find_commit(oid)?;

        let raw = if let Some(cached) = cache.get(&hash) {
            cached
        } else {
            let tree = commit.tree()?;
            let is_merge = commit.parent_count() > 1;

            let parent_tree = if is_merge {
                let parent_a = commit.parent_id(0)?;
                let parent_b = commit.parent_id(1)?;
                repo.merge_base(parent_a, parent_b)
                    .ok()
                    .and_then(|base_oid| repo.find_commit(base_oid).ok())
                    .and_then(|base_commit| base_commit.tree().ok())
            } else if commit.parent_count() > 0 {
                Some(commit.parent(0)?.tree()?)
            } else {
                None
            };

            let mut opts = DiffOptions::new();
            let diff =
                repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;
            let stats = diff.stats()?;

            let mut files = Vec::new();
            let mut files_added = 0usize;
            let mut files_deleted = 0usize;
            let mut added_file_names = Vec::new();
            let mut deleted_file_names = Vec::new();
            for d in diff.deltas() {
                if let Some(p) = d.new_file().path().or_else(|| d.old_file().path()) {
                    let name = p.to_string_lossy().to_string();
                    match d.status() {
                        git2::Delta::Added => {
                            files_added += 1;
                            added_file_names.push(name.clone());
                        }
                        git2::Delta::Deleted => {
                            files_deleted += 1;
                            deleted_file_names.push(name.clone());
                        }
                        _ => {}
                    }
                    files.push(name);
                }
            }

            let total_changed = stats.insertions() + stats.deletions();
            let (ws_add, ws_rm) = if total_changed <= 2000 {
                count_whitespace_lines(&diff)
            } else {
                (0, 0)
            };

            let raw = Commit {
                author_name: commit.author().name().unwrap_or("Unknown").to_string(),
                author_email: commit
                    .author()
                    .email()
                    .unwrap_or("unknown@unknown")
                    .to_string(),
                group_id: 0,
                lines_added: stats.insertions(),
                lines_removed: stats.deletions(),
                files_changed: stats.files_changed(),
                timestamp: commit.time().seconds(),
                files,
                whitespace_added: ws_add,
                whitespace_removed: ws_rm,
                files_added,
                files_deleted,
                added_file_names,
                deleted_file_names,
                is_merge,
            };

            cache.insert(hash, raw.clone());
            new_count += 1;

            raw
        };

        commits.push(raw);
        on_progress(commits.len(), new_count);

        if let Some(max) = max_commits {
            if commits.len() >= max {
                break;
            }
        }
    }

    Ok((info, repo, commits))
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
