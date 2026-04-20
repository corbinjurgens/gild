use anyhow::Result;
use std::fs;
use std::path::Path;
use std::process;

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

/// Read `path` and parse with `parse`. Returns `Default::default()` on missing file or parse error,
/// logging a warning to stderr when a file exists but fails to parse.
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
