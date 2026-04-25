use anyhow::Result;
use git2::Repository;
use rusqlite::params;
use std::collections::HashSet;
use std::path::Path;

use crate::db::Database;

/// Prune DB rows keyed to commits or HEADs that are no longer reachable from
/// any ref in the working repository. Runs at startup; revwalk + a handful
/// of DELETEs. Keeps the caches scoped to the repo's current live history so
/// no add-on ever aggregates over dead/force-pushed commits.
pub fn run(repo_path: &Path, db: &Database) -> Result<()> {
    let repo = match Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return Ok(()), // caller handles repo-open failures later
    };
    let reachable = reachable_commits(&repo)?;

    let tx = db.conn.unchecked_transaction()?;
    tx.execute(
        "CREATE TEMP TABLE IF NOT EXISTS _gc_reachable (hash TEXT PRIMARY KEY)",
        [],
    )?;
    tx.execute("DELETE FROM _gc_reachable", [])?;
    {
        let mut stmt = tx.prepare("INSERT INTO _gc_reachable (hash) VALUES (?1)")?;
        for hash in &reachable {
            stmt.execute(params![hash])?;
        }
    }

    // Raw commit-keyed tables.
    tx.execute(
        "DELETE FROM commit_files
         WHERE commit_hash NOT IN (SELECT hash FROM _gc_reachable)",
        [],
    )?;
    tx.execute(
        "DELETE FROM commits
         WHERE hash NOT IN (SELECT hash FROM _gc_reachable)",
        [],
    )?;

    // Blame-prefixed HEAD keys: head_hash column stores 'blame:<oid>',
    // so strip the 6-char prefix before matching.
    tx.execute(
        "DELETE FROM ownership
         WHERE SUBSTR(head_hash, 7) NOT IN (SELECT hash FROM _gc_reachable)",
        [],
    )?;
    tx.execute(
        "DELETE FROM ownership_meta
         WHERE SUBSTR(head_hash, 7) NOT IN (SELECT hash FROM _gc_reachable)",
        [],
    )?;
    tx.execute(
        "DELETE FROM blame_file
         WHERE SUBSTR(head_hash, 7) NOT IN (SELECT hash FROM _gc_reachable)",
        [],
    )?;

    // Raw HEAD hash (no prefix).
    tx.execute(
        "DELETE FROM file_bus_factor
         WHERE head_hash NOT IN (SELECT hash FROM _gc_reachable)",
        [],
    )?;

    tx.commit()?;
    Ok(())
}

fn reachable_commits(repo: &Repository) -> Result<HashSet<String>> {
    let mut walk = repo.revwalk()?;

    // Push every ref (branches + remotes + tags). `revwalk` de-duplicates
    // commits reached via multiple refs, so one walk covers all of them.
    if let Ok(refs) = repo.references() {
        for reference in refs.flatten() {
            if let Ok(resolved) = reference.resolve() {
                if let Some(oid) = resolved.target() {
                    let _ = walk.push(oid);
                }
            }
        }
    }
    // Detached HEAD isn't in references(); push it explicitly.
    if let Ok(head) = repo.head() {
        if let Some(oid) = head.target() {
            let _ = walk.push(oid);
        }
    }

    let mut reachable = HashSet::new();
    for oid_res in walk {
        if let Ok(oid) = oid_res {
            reachable.insert(oid.to_string());
        }
    }
    Ok(reachable)
}
