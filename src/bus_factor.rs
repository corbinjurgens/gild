use anyhow::Result;
use rusqlite::params;

use crate::db::{Database, FILE_KIND_TOUCHED};

pub struct BusFactorRow {
    pub file: String,
    pub unique_authors: u32,
}

pub fn compute(
    repo: &gix::Repository,
    db: &Database,
    on_progress: impl Fn(usize, usize),
) -> Result<Vec<BusFactorRow>> {
    let head_hash = repo.head()?.into_peeled_id()?.to_string();

    let cached_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM file_bus_factor WHERE head_hash = ?1",
            params![&head_hash],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if cached_count > 0 {
        return load_from_db(db, &head_hash);
    }

    let head_files = crate::ownership::walk_tree_sizes(repo);
    let total = head_files.len();
    on_progress(0, total);

    // SQLite counts unique authors per file directly. LOWER() matches the
    // prior `to_lowercase()` treatment; non-ASCII emails (rare) degrade to
    // the same result as the old code for ASCII-only input.
    let mut stmt = db.prepare(
        "SELECT cf.file_path, COUNT(DISTINCT LOWER(c.author_email))
         FROM commit_files cf
         JOIN commits c ON c.hash = cf.commit_hash
         WHERE cf.kind = ?1
         GROUP BY cf.file_path",
    )?;
    let rows = stmt.query_map(params![FILE_KIND_TOUCHED], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
    })?;

    let tx_conn = db.transaction()?;
    tx_conn.execute(
        "DELETE FROM file_bus_factor WHERE head_hash = ?1",
        params![&head_hash],
    )?;
    {
        let mut insert = tx_conn.prepare(
            "INSERT INTO file_bus_factor (file, head_hash, unique_authors)
             VALUES (?1, ?2, ?3)",
        )?;
        for row in rows {
            let (file, unique) = row?;
            if !head_files.contains_key(&file) {
                continue;
            }
            insert.execute(params![file, &head_hash, unique as i64])?;
        }
    }
    tx_conn.commit()?;

    on_progress(total, total);
    load_from_db(db, &head_hash)
}

fn load_from_db(db: &Database, head_hash: &str) -> Result<Vec<BusFactorRow>> {
    let mut stmt =
        db.prepare("SELECT file, unique_authors FROM file_bus_factor WHERE head_hash = ?1")?;
    let rows = stmt.query_map(params![head_hash], |r| {
        Ok(BusFactorRow {
            file: r.get(0)?,
            unique_authors: r.get::<_, i64>(1)? as u32,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
