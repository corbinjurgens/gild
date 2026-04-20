use anyhow::Result;
use git2::{ObjectType, Repository, TreeWalkMode, TreeWalkResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::cache::storage_dir;
use crate::git::Commit;
use crate::util::{load_or_default, write_atomic};

#[derive(Serialize, Deserialize, Default)]
pub struct OwnershipData {
    pub head: String,
    pub total_lines: usize,
    pub authors: HashMap<usize, usize>,
}

pub fn compute(
    repo: &Repository,
    git_dir: &Path,
    commits: &[Commit],
    on_progress: impl Fn(usize, usize),
) -> Result<OwnershipData> {
    let head = repo.head()?.peel_to_commit()?;
    let head_hash = head.id().to_string();

    let cache_path = storage_dir(git_dir).join("ownership.json");
    let cached: OwnershipData = load_or_default(&cache_path, |s| serde_json::from_str(s));
    if cached.head == head_hash && !cached.authors.is_empty() {
        return Ok(cached);
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

    on_progress(total_files, total_files);

    let result = OwnershipData {
        head: head_hash,
        total_lines,
        authors: group_lines,
    };

    fs::create_dir_all(storage_dir(git_dir))?;
    write_atomic(&cache_path, serde_json::to_string(&result)?.as_bytes())?;

    Ok(result)
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
    if content.is_empty() {
        return 0;
    }
    let nl = bytecount(content, b'\n');
    if *content.last().unwrap() == b'\n' { nl } else { nl + 1 }
}

fn bytecount(haystack: &[u8], needle: u8) -> usize {
    haystack.iter().filter(|&&b| b == needle).count()
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
    num_groups: usize,
) -> (Vec<usize>, usize) {
    let mut group_lines = vec![0usize; num_groups];
    let mut mapped_total = 0usize;

    for (&gid, &lines) in &data.authors {
        if gid < num_groups {
            group_lines[gid] += lines;
            mapped_total += lines;
        }
    }

    (group_lines, mapped_total)
}
