use crate::cache::Cache;
use anyhow::{Context, Result};
use git2::{DiffLineType, DiffOptions, Repository, Sort};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct RepoInfo {
    pub name: String,
    pub branch: String,
    pub git_dir: PathBuf,
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
        git_dir: repo.path().to_path_buf(),
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
            cached.clone()
        } else {
            let tree = commit.tree()?;
            let parent_tree = if commit.parent_count() > 0 {
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

    let mut removed: HashMap<String, usize> = HashMap::new();
    let mut added: HashMap<String, usize> = HashMap::new();
    let file_idx = Cell::new(0usize);

    let _ = diff.foreach(
        &mut |_delta, _progress| {
            file_idx.set(file_idx.get() + 1);
            true
        },
        None,
        None,
        Some(&mut |_delta, _hunk, line| {
            let content = String::from_utf8_lossy(line.content());
            let trimmed = content.trim();
            let original = content.trim_end_matches('\n').trim_end_matches('\r');

            if trimmed == original.trim() && trimmed != original {
                let key = format!("{}:{}", file_idx.get(), trimmed);
                match line.origin_value() {
                    DiffLineType::Addition => *added.entry(key).or_default() += 1,
                    DiffLineType::Deletion => *removed.entry(key).or_default() += 1,
                    _ => {}
                }
            }
            true
        }),
    );

    let mut ws_add = 0usize;
    let mut ws_rm = 0usize;

    for (key, del_count) in &removed {
        if let Some(add_count) = added.get(key) {
            let paired = (*del_count).min(*add_count);
            ws_rm += paired;
            ws_add += paired;
        }
    }

    (ws_add, ws_rm)
}
