use anyhow::Result;
use rusqlite::params;
use std::collections::HashSet;
use std::path::Path;

use crate::db::Database;

pub fn run(repo_path: &Path, db: &Database) -> Result<()> {
    let repo = match gix::open(repo_path) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    let reachable = reachable_commits(&repo)?;

    let tx = db.transaction()?;
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

fn reachable_commits(repo: &gix::Repository) -> Result<HashSet<String>> {
    let mut tips: Vec<gix::ObjectId> = Vec::new();
    if let Ok(refs) = repo.references() {
        if let Ok(all) = refs.all() {
            for reference in all.flatten() {
                if let Ok(id) = reference.into_fully_peeled_id() {
                    tips.push(id.detach());
                }
            }
        }
    }
    if let Ok(head) = repo.head() {
        if let Ok(id) = head.into_peeled_id() {
            tips.push(id.detach());
        }
    }

    let mut reachable = HashSet::new();
    if !tips.is_empty() {
        let walk = repo.rev_walk(tips).all()?;
        for info in walk.flatten() {
            reachable.insert(info.id.to_string());
        }
    }
    Ok(reachable)
}
