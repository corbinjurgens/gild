use anyhow::Result;
use gix::bstr::ByteSlice;
use std::path::{Path, PathBuf};
use std::{fs, process};

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
    let joined = match key {
        RepoKey::Remote { normalized_url } => base.join(normalized_url),
        RepoKey::Local { folder_label } => base.join("local").join(folder_label),
    };
    assert!(
        safe_under(&joined, &base),
        "repo key escapes data directory: {}",
        joined.display()
    );
    joined
}

fn safe_under(path: &Path, base: &Path) -> bool {
    use std::path::Component;
    let mut depth: isize = 0;
    for comp in path.strip_prefix(base).unwrap_or(path).components() {
        match comp {
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            _ => {}
        }
        if depth < 0 {
            return false;
        }
    }
    path.starts_with(base) && depth >= 0
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

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!("{}.tmp.{}", s, process::id()))
        .unwrap_or_else(|| format!("tmp.{}", process::id()));
    let tmp = path.with_extension(ext);
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load_or_default<T, F, E>(path: &Path, parse: F) -> T
where
    T: Default,
    F: FnOnce(&str) -> std::result::Result<T, E>,
    E: std::fmt::Display,
{
    if !path.exists() {
        return T::default();
    }
    let content = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  Warning: failed to read {}: {}", path.display(), e);
            return T::default();
        }
    };
    match parse(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  Warning: failed to parse {}: {}", path.display(), e);
            T::default()
        }
    }
}
