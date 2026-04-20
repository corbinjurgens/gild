use crate::git::Commit;
use crate::util::{load_or_default, write_atomic};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const CACHE_SCHEMA_VERSION: u8 = 3;

#[derive(Serialize, Deserialize, Default)]
pub struct Cache {
    #[serde(default)]
    pub schema_version: u8,
    pub commits: HashMap<String, Commit>,
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
        let loaded: Self =
            load_or_default(&cache_path(repo_git_dir), |s| serde_json::from_str::<Self>(s));
        if loaded.schema_version != CACHE_SCHEMA_VERSION {
            return Self {
                schema_version: CACHE_SCHEMA_VERSION,
                ..Default::default()
            };
        }
        loaded
    }

    pub fn save(&self, repo_git_dir: &Path) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        fs::create_dir_all(storage_dir(repo_git_dir))?;
        let json = serde_json::to_string(&self)?;
        write_atomic(&cache_path(repo_git_dir), json.as_bytes())
    }

    pub fn get(&self, hash: &str) -> Option<&Commit> {
        self.commits.get(hash)
    }

    pub fn insert(&mut self, hash: String, commit: Commit) {
        self.commits.insert(hash, commit);
        self.dirty = true;
        self.schema_version = CACHE_SCHEMA_VERSION;
    }
}
