use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone)]
pub struct CachedCommit {
    pub author_name: String,
    pub author_email: String,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub files_changed: usize,
    pub timestamp: i64,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Cache {
    pub commits: HashMap<String, CachedCommit>,
    #[serde(skip)]
    dirty: bool,
}

pub fn storage_dir(repo_git_dir: &Path) -> PathBuf {
    repo_git_dir.join("gild")
}

pub fn cache_path(repo_git_dir: &Path) -> PathBuf {
    storage_dir(repo_git_dir).join("cache.json")
}

pub fn identities_path(repo_git_dir: &Path) -> PathBuf {
    storage_dir(repo_git_dir).join("identities.toml")
}

impl Cache {
    pub fn load(repo_git_dir: &Path) -> Self {
        let path = cache_path(repo_git_dir);
        if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self, repo_git_dir: &Path) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let dir = storage_dir(repo_git_dir);
        fs::create_dir_all(&dir)?;
        let final_path = cache_path(repo_git_dir);
        let tmp_path = final_path.with_extension("json.tmp");
        let json = serde_json::to_string(&self)?;
        fs::write(&tmp_path, json)?;
        fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    pub fn get(&self, hash: &str) -> Option<&CachedCommit> {
        self.commits.get(hash)
    }

    pub fn insert(&mut self, hash: String, commit: CachedCommit) {
        self.commits.insert(hash, commit);
        self.dirty = true;
    }
}
