use anyhow::Result;
use gix::bstr::ByteSlice;
use std::path::{Path, PathBuf};

pub enum RepoKey {
    Remote { normalized_url: String },
    Local { folder_label: String },
}

pub fn data_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("GILD_DATA_DIR") {
        return PathBuf::from(custom);
    }
    dirs::data_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gild")
}

pub fn resolve_repo_key(repo_path: &Path) -> Result<RepoKey> {
    let repo = gix::open(repo_path)?;
    if let Some(Ok(remote)) = repo.find_default_remote(gix::remote::Direction::Fetch) {
        if let Some(url) = remote.url(gix::remote::Direction::Fetch) {
            let url_str = url.to_bstring().to_str_lossy().into_owned();
            return Ok(RepoKey::Remote {
                normalized_url: crate::remote::normalize_url(&url_str),
            });
        }
    }
    let canonical = repo_path
        .canonicalize()
        .unwrap_or_else(|_| repo_path.to_path_buf());
    let folder = canonical
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let hash = fnv1a_hex8(canonical.to_string_lossy().as_bytes());
    Ok(RepoKey::Local {
        folder_label: format!("{folder}-{hash}"),
    })
}

pub fn repo_data_dir(key: &RepoKey) -> PathBuf {
    let base = data_dir().join("repos");
    match key {
        RepoKey::Remote { normalized_url } => base.join(normalized_url),
        RepoKey::Local { folder_label } => base.join("local").join(folder_label),
    }
}

pub fn db_path(key: &RepoKey) -> PathBuf {
    repo_data_dir(key).join("gild.db")
}

pub fn identities_path(key: &RepoKey) -> PathBuf {
    repo_data_dir(key).join("identities.toml")
}

pub fn clone_dir(key: &RepoKey) -> PathBuf {
    repo_data_dir(key).join("repo")
}

fn fnv1a_hex8(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let hex = format!("{h:016x}");
    hex.chars().take(8).collect()
}
