use anyhow::{Context, Result};
use git2::build::RepoBuilder;
use git2::{Cred, CredentialType, FetchOptions, RemoteCallbacks, Repository};
use std::cell::Cell;
use std::path::{Path, PathBuf};

pub fn is_remote_url(input: &str) -> bool {
    input.starts_with("https://")
        || input.starts_with("http://")
        || input.starts_with("git://")
        || input.starts_with("ssh://")
        || input.starts_with("git@")
}

pub fn ensure_repo(url: &str) -> Result<PathBuf> {
    let local_path = clone_dir_for_url(url);

    if local_path.join(".git").exists() || local_path.join("HEAD").exists() {
        if Repository::open(&local_path).is_ok() {
            fetch_repo(&local_path, url)?;
            return Ok(local_path);
        }
    }

    clone_repo(url, &local_path)?;
    Ok(local_path)
}

fn clone_dir_for_url(url: &str) -> PathBuf {
    let normalized = normalize_url(url);
    crate::storage::clone_dir(&crate::storage::RepoKey::Remote {
        normalized_url: normalized,
    })
}

pub fn normalize_url(url: &str) -> String {
    let s = if let Some(rest) = url.strip_prefix("git@") {
        rest.replacen(':', "/", 1)
    } else if let Some(idx) = url.find("://") {
        url[idx + 3..].to_string()
    } else {
        url.to_string()
    };

    let s = s.strip_suffix(".git").unwrap_or(&s);
    let s = s.strip_suffix('/').unwrap_or(s);
    // Strip userinfo (user@ or user:pass@)
    let s = if let Some(at_pos) = s.find('@') {
        if let Some(slash_pos) = s.find('/') {
            if at_pos < slash_pos {
                &s[at_pos + 1..]
            } else {
                s
            }
        } else {
            s
        }
    } else {
        s
    };
    s.to_lowercase()
}

fn clone_repo(url: &str, dest: &Path) -> Result<()> {
    eprintln!("  Cloning {}...", url);

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create cache directory: {}", parent.display()))?;
    }

    let fetch_opts = make_fetch_options();
    RepoBuilder::new()
        .fetch_options(fetch_opts)
        .clone(url, dest)
        .with_context(|| auth_error_hint(url))?;

    eprintln!("  Clone complete.");
    Ok(())
}

fn fetch_repo(local_path: &Path, url: &str) -> Result<()> {
    eprintln!("  Fetching updates...");

    let repo = Repository::open(local_path)
        .context("Failed to open cached repository")?;

    let mut remote = repo
        .find_remote("origin")
        .or_else(|_| repo.remote_anonymous(url))
        .context("No remote 'origin' found")?;

    let mut fetch_opts = make_fetch_options();
    remote
        .fetch(&[] as &[&str], Some(&mut fetch_opts), None)
        .with_context(|| auth_error_hint(url))?;

    // Update HEAD to track remote default branch
    if let Ok(head) = remote.default_branch() {
        if let Some(refname) = head.as_str() {
            let _ = repo.set_head(refname);
            // Fast-forward working tree to match
            if let Ok(reference) = repo.find_reference(refname) {
                if let Ok(commit) = reference.peel_to_commit() {
                    let _ = repo.checkout_tree(
                        commit.as_object(),
                        Some(git2::build::CheckoutBuilder::new().force()),
                    );
                }
            }
        }
    }

    Ok(())
}

fn make_fetch_options<'a>() -> FetchOptions<'a> {
    let mut callbacks = RemoteCallbacks::new();
    let attempts = Cell::new(0usize);

    callbacks.credentials(move |url, username_from_url, allowed_types| {
        let attempt = attempts.get();
        if attempt >= 4 {
            return Err(git2::Error::from_str("authentication failed after multiple attempts"));
        }
        attempts.set(attempt + 1);

        let username = username_from_url.unwrap_or("git");

        if allowed_types.contains(CredentialType::SSH_KEY) {
            if let Ok(cred) = Cred::ssh_key_from_agent(username) {
                return Ok(cred);
            }

            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            let ssh_dir = home.join(".ssh");
            for key_name in &["id_ed25519", "id_rsa", "id_ecdsa"] {
                let key_path = ssh_dir.join(key_name);
                if key_path.exists() {
                    if let Ok(cred) = Cred::ssh_key(username, None, &key_path, None) {
                        return Ok(cred);
                    }
                }
            }
        }

        if allowed_types.contains(CredentialType::USER_PASS_PLAINTEXT) {
            if let Ok(config) = git2::Config::open_default() {
                if let Ok(cred) = Cred::credential_helper(&config, url, username_from_url) {
                    return Ok(cred);
                }
            }

            for var in &["GILD_TOKEN", "GIT_TOKEN", "GITHUB_TOKEN"] {
                if let Ok(token) = std::env::var(var) {
                    return Cred::userpass_plaintext(username, &token);
                }
            }
        }

        if allowed_types.contains(CredentialType::USERNAME) {
            return Cred::username(username);
        }

        Err(git2::Error::from_str("no authentication method available"))
    });

    callbacks.transfer_progress(|stats| {
        if stats.total_objects() > 0 {
            eprint!(
                "\r  Cloning... {}/{} objects",
                stats.received_objects(),
                stats.total_objects()
            );
        }
        true
    });

    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);
    fetch_opts
}

fn auth_error_hint(url: &str) -> String {
    if url.starts_with("git@") || url.starts_with("ssh://") {
        format!(
            "Failed to access {}. For SSH repos, ensure ssh-agent is running \
             and your key is added (ssh-add).",
            url
        )
    } else {
        format!(
            "Failed to access {}. For private repos, configure a git credential \
             helper or set GILD_TOKEN / GITHUB_TOKEN.",
            url
        )
    }
}
