use anyhow::Result;
use rusqlite::params;
use std::collections::HashMap;

use crate::db::{Database, FILE_KIND_TOUCHED};

pub struct ChurnRow {
    pub file: String,
    pub commit_count: u32,
    pub churn_score: f64,
}

fn cache_key(db: &Database, head_hash: &str) -> Result<String> {
    let (count, max_ts): (i64, i64) = db.query_row(
        "SELECT COUNT(*), COALESCE(MAX(timestamp), 0) FROM commits",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    // Matches coupling's key: head_hash ties `current_lines` (computed from
    // HEAD's tree) to the specific HEAD it was measured at.
    Ok(format!("n={count},ts={max_ts},head={head_hash}"))
}

pub fn compute(
    repo: &gix::Repository,
    db: &Database,
    head_hash: &str,
    on_progress: impl Fn(usize, usize),
) -> Result<Vec<ChurnRow>> {
    let key = cache_key(db, head_hash)?;

    let has_churn: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM file_stats WHERE cache_key = ?1 AND churn_score > 0",
            params![&key],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has_churn > 0 {
        return load_from_db(db, &key);
    }

    let file_sizes = crate::ownership::walk_tree_sizes(repo);
    let total = file_sizes.len();
    on_progress(0, total);

    // Prefer commit counts already populated by `coupling` (they share the cache key).
    let coupling_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM file_stats WHERE cache_key = ?1 AND commit_count > 0",
            params![&key],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let file_commit_counts: HashMap<String, u32> = if coupling_count > 0 {
        let mut stmt =
            db.prepare("SELECT file, commit_count FROM file_stats WHERE cache_key = ?1")?;
        let rows = stmt.query_map(params![&key], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
        })?;
        rows.filter_map(|r| r.ok()).collect()
    } else {
        let mut stmt = db.prepare(
            "SELECT cf.file_path, COUNT(*)
             FROM commit_files cf
             JOIN commits c ON c.hash = cf.commit_hash
             WHERE cf.kind = ?1 AND c.is_merge = 0
             GROUP BY cf.file_path",
        )?;
        let rows = stmt.query_map(params![FILE_KIND_TOUCHED], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
        })?;
        rows.filter_map(|r| r.ok())
            .filter(|(file, _)| file_sizes.contains_key(file))
            .collect()
    };

    on_progress(total, total);

    let tx = db.transaction()?;
    if coupling_count > 0 {
        let mut stmt = tx.prepare(
            "UPDATE file_stats SET current_lines = ?1, churn_score = ?2
             WHERE file = ?3 AND cache_key = ?4",
        )?;
        for (file, &current_lines) in &file_sizes {
            let commit_count = file_commit_counts.get(file).copied().unwrap_or(0);
            let churn_score = commit_count as f64 / current_lines.max(1) as f64;
            stmt.execute(params![current_lines as i64, churn_score, file, &key])?;
        }
    } else {
        tx.execute(
            "DELETE FROM file_stats WHERE cache_key != ?1",
            params![&key],
        )?;
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO file_stats (file, commit_count, current_lines, churn_score, cache_key)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (file, &current_lines) in &file_sizes {
            let commit_count = file_commit_counts.get(file).copied().unwrap_or(0);
            let churn_score = commit_count as f64 / current_lines.max(1) as f64;
            stmt.execute(params![
                file,
                commit_count as i64,
                current_lines as i64,
                churn_score,
                &key
            ])?;
        }
    }
    tx.commit()?;

    load_from_db(db, &key)
}

fn load_from_db(db: &Database, key: &str) -> Result<Vec<ChurnRow>> {
    let mut stmt = db.prepare(
        "SELECT file, commit_count, churn_score
         FROM file_stats WHERE cache_key = ?1 AND churn_score > 0
         ORDER BY churn_score DESC",
    )?;
    let rows = stmt.query_map(params![key], |r| {
        Ok(ChurnRow {
            file: r.get(0)?,
            commit_count: r.get::<_, i64>(1)? as u32,
            churn_score: r.get(2)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
