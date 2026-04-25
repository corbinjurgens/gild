use anyhow::{Context, Result};
use gix::bstr::ByteSlice;
use gix::credentials::{helper, protocol};
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

    if (local_path.join(".git").exists() || local_path.join("HEAD").exists())
        && gix::open(&local_path).is_ok()
    {
        fetch_repo(&local_path, url)?;
        return Ok(local_path);
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
    eprintln!("  Cloning {url}...");

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create cache directory: {}", parent.display()))?;
    }

    let mut clone = gix::prepare_clone(url, dest)?;
    if let Some(credentials) = env_credentials() {
        let mut credentials = Some(credentials);
        clone = clone.configure_connection(move |conn| {
            if let Some(c) = credentials.take() {
                conn.set_credentials(c);
            }
            Ok(())
        });
    }
    let (mut checkout, _) = clone
        .fetch_then_checkout(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
        .with_context(|| auth_error_hint(url))?;

    checkout
        .main_worktree(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
        .context("Failed to checkout working tree")?;

    eprintln!("  Clone complete.");
    Ok(())
}

fn fetch_repo(local_path: &Path, url: &str) -> Result<()> {
    eprintln!("  Fetching updates...");

    let repo = gix::open(local_path).context("Failed to open cached repository")?;

    let remote = repo
        .find_remote("origin")
        .context("No remote 'origin' found")?;

    let mut conn = remote.connect(gix::remote::Direction::Fetch)?;
    if let Some(credentials) = env_credentials() {
        conn.set_credentials(credentials);
    }
    conn.prepare_fetch(gix::progress::Discard, Default::default())?
        .receive(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
        .with_context(|| auth_error_hint(url))?;

    // Fast-forward local branch to match its remote tracking counterpart.
    if let Ok(Some(head_ref)) = repo.head_ref() {
        let name = head_ref.name().as_bstr().to_str_lossy().into_owned();
        if let Some(branch) = name.strip_prefix("refs/heads/") {
            let tracking = format!("refs/remotes/origin/{branch}");
            if let Ok(remote_ref) = repo.find_reference(&tracking) {
                if let Ok(target) = remote_ref.into_fully_peeled_id() {
                    let _ = repo.reference(
                        name.as_str(),
                        target.detach(),
                        gix::refs::transaction::PreviousValue::Any,
                        "gild: fast-forward after fetch",
                    );
                }
            }
        }
    }

    Ok(())
}

fn env_token() -> Option<String> {
    ["GILD_TOKEN", "GIT_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .find_map(|var| std::env::var(var).ok())
}

fn env_credentials() -> Option<impl FnMut(helper::Action) -> protocol::Result> {
    let token = env_token()?;
    Some(move |action| match action {
        helper::Action::Get(_) => {
            let identity = gix::sec::identity::Account {
                username: "x-access-token".into(),
                password: token.clone(),
                oauth_refresh_token: None,
            };
            Ok(Some(protocol::Outcome {
                identity,
                next: helper::NextAction::from(protocol::Context::default()),
            }))
        }
        helper::Action::Store(_) | helper::Action::Erase(_) => Ok(None),
    })
}

fn auth_error_hint(url: &str) -> String {
    if url.starts_with("git@") || url.starts_with("ssh://") {
        format!(
            "Failed to access {url}. For SSH repos, ensure ssh-agent is running \
             and your key is added (ssh-add)."
        )
    } else {
        format!(
            "Failed to access {url}. For private repos, configure a git credential \
             helper or set GILD_TOKEN / GITHUB_TOKEN."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scp_style() {
        assert_eq!(
            normalize_url("git@github.com:user/repo.git"),
            "github.com/user/repo"
        );
    }

    #[test]
    fn scp_no_suffix() {
        assert_eq!(
            normalize_url("git@github.com:user/repo"),
            "github.com/user/repo"
        );
    }

    #[test]
    fn https_dotgit() {
        assert_eq!(
            normalize_url("https://github.com/user/repo.git"),
            "github.com/user/repo"
        );
    }

    #[test]
    fn https_clean() {
        assert_eq!(
            normalize_url("https://github.com/user/repo"),
            "github.com/user/repo"
        );
    }

    #[test]
    fn trailing_slash() {
        assert_eq!(
            normalize_url("https://github.com/user/repo/"),
            "github.com/user/repo"
        );
    }

    #[test]
    fn uppercase() {
        assert_eq!(
            normalize_url("HTTPS://GITHUB.COM/USER/REPO"),
            "github.com/user/repo"
        );
    }

    #[test]
    fn ssh_scheme() {
        assert_eq!(
            normalize_url("ssh://git@github.com/user/repo.git"),
            "github.com/user/repo"
        );
    }

    #[test]
    fn userinfo_token() {
        assert_eq!(
            normalize_url("https://token@github.com/user/repo"),
            "github.com/user/repo"
        );
    }

    #[test]
    fn userinfo_user_pass() {
        assert_eq!(
            normalize_url("https://u:p@github.com/user/repo"),
            "github.com/user/repo"
        );
    }

    #[test]
    fn no_scheme() {
        assert_eq!(
            normalize_url("github.com/user/repo"),
            "github.com/user/repo"
        );
    }
}
