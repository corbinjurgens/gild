use crate::cache::{Cache, CachedCommit};
use anyhow::{Context, Result};
use git2::{DiffOptions, Repository, Sort};
use std::path::{Path, PathBuf};

pub struct RepoInfo {
    pub name: String,
    pub branch: String,
    pub git_dir: PathBuf,
}

pub struct RawCommit {
    pub author_name: String,
    pub author_email: String,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub files_changed: usize,
    pub timestamp: i64,
    pub files: Vec<String>,
}

pub fn load_commits(
    path: &Path,
    branch: Option<&str>,
    max_commits: Option<usize>,
    cache: &mut Cache,
    on_progress: impl Fn(usize, usize),
) -> Result<(RepoInfo, Vec<RawCommit>)> {
    let repo = Repository::open(path).context("Failed to open git repository")?;

    let head = match branch {
        Some(b) => repo
            .find_branch(b, git2::BranchType::Local)
            .with_context(|| format!("Branch '{}' not found", b))?
            .into_reference(),
        None => repo.head().context("Failed to resolve HEAD")?,
    };

    let branch_name = head.shorthand().unwrap_or("HEAD").to_string();
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
    revwalk.push(head.target().context("HEAD has no target")?)?;
    revwalk.set_sorting(Sort::TIME)?;

    let mut commits = Vec::new();
    let mut new_count = 0;

    for oid_result in revwalk {
        let oid = oid_result?;
        let hash = oid.to_string();
        let commit = repo.find_commit(oid)?;

        let cached = cache
            .get(&hash)
            .filter(|c| c.files_changed == 0 || !c.files.is_empty());
        let raw = if let Some(cached) = cached {
            RawCommit {
                author_name: cached.author_name.clone(),
                author_email: cached.author_email.clone(),
                lines_added: cached.lines_added,
                lines_removed: cached.lines_removed,
                files_changed: cached.files_changed,
                timestamp: cached.timestamp,
                files: cached.files.clone(),
            }
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

            let files: Vec<String> = diff
                .deltas()
                .filter_map(|d| {
                    d.new_file()
                        .path()
                        .or_else(|| d.old_file().path())
                        .map(|p| p.to_string_lossy().to_string())
                })
                .collect();

            let raw = RawCommit {
                author_name: commit.author().name().unwrap_or("Unknown").to_string(),
                author_email: commit
                    .author()
                    .email()
                    .unwrap_or("unknown@unknown")
                    .to_string(),
                lines_added: stats.insertions(),
                lines_removed: stats.deletions(),
                files_changed: stats.files_changed(),
                timestamp: commit.time().seconds(),
                files,
            };

            cache.insert(
                hash,
                CachedCommit {
                    author_name: raw.author_name.clone(),
                    author_email: raw.author_email.clone(),
                    lines_added: raw.lines_added,
                    lines_removed: raw.lines_removed,
                    files_changed: raw.files_changed,
                    timestamp: raw.timestamp,
                    files: raw.files.clone(),
                },
            );
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

    Ok((info, commits))
}
