use anyhow::Result;
use rusqlite::Connection;
use std::fs;
use std::path::Path;
use std::time::Duration;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        migrate(&mut conn)?;
        Ok(Database { conn })
    }

    pub(crate) fn prepare(&self, sql: &str) -> rusqlite::Result<rusqlite::Statement<'_>> {
        self.conn.prepare(sql)
    }

    pub(crate) fn query_row<T, P, F>(&self, sql: &str, params: P, f: F) -> rusqlite::Result<T>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        self.conn.query_row(sql, params, f)
    }

    pub(crate) fn transaction(&mut self) -> rusqlite::Result<rusqlite::Transaction<'_>> {
        self.conn.transaction()
    }
}

// `commit_files.kind`: 0 = touched, 1 = added, 2 = deleted.
// Separate rows for added/deleted instead of a flag on "touched" so queries
// for added-file events can hit the index directly.
pub fn file_addon_cache_key(db: &Database, head_hash: &str) -> Result<String> {
    let (count, max_ts): (i64, i64) = db.query_row(
        "SELECT COUNT(*), COALESCE(MAX(timestamp), 0) FROM commits",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(format!("n={count},ts={max_ts},head={head_hash}"))
}

pub const FILE_KIND_TOUCHED: i64 = 0;
pub const FILE_KIND_ADDED: i64 = 1;
pub const FILE_KIND_DELETED: i64 = 2;

// Append to this list for schema changes; each entry runs once, ordered, and
// its index + 1 becomes the on-disk version number.
const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE commits (
        hash TEXT PRIMARY KEY,
        author_name TEXT NOT NULL,
        author_email TEXT NOT NULL,
        lines_added INTEGER NOT NULL,
        lines_removed INTEGER NOT NULL,
        files_changed INTEGER NOT NULL,
        timestamp INTEGER NOT NULL,
        whitespace_added INTEGER NOT NULL,
        whitespace_removed INTEGER NOT NULL,
        files_added INTEGER NOT NULL,
        files_deleted INTEGER NOT NULL,
        is_merge INTEGER NOT NULL
    );
    CREATE INDEX idx_commits_ts ON commits(timestamp);
    CREATE INDEX idx_commits_author ON commits(author_email);

    -- Per-commit file paths live only here; the in-memory Commit struct holds
    -- numeric stats only. `WITHOUT ROWID` stores rows directly in the PK
    -- b-tree, roughly halving disk use for this large append-only table.
    CREATE TABLE commit_files (
        commit_hash TEXT NOT NULL,
        file_path   TEXT NOT NULL,
        kind        INTEGER NOT NULL,
        PRIMARY KEY (commit_hash, file_path, kind)
    ) WITHOUT ROWID;

    CREATE TABLE ownership (
        head_hash TEXT NOT NULL,
        author_email TEXT NOT NULL,
        author_name TEXT NOT NULL,
        lines INTEGER NOT NULL,
        PRIMARY KEY (head_hash, author_email)
    );
    CREATE TABLE ownership_meta (
        head_hash TEXT PRIMARY KEY,
        total_lines INTEGER NOT NULL
    );

    CREATE TABLE file_coupling (
        file_a         TEXT NOT NULL,
        file_b         TEXT NOT NULL,
        co_occurrences INTEGER NOT NULL,
        score          REAL NOT NULL,
        PRIMARY KEY (file_a, file_b)
    );
    CREATE TABLE file_stats (
        file          TEXT PRIMARY KEY,
        commit_count  INTEGER NOT NULL DEFAULT 0,
        current_lines INTEGER NOT NULL DEFAULT 0,
        churn_score   REAL NOT NULL DEFAULT 0.0,
        cache_key     TEXT NOT NULL DEFAULT ''
    );
    CREATE TABLE file_bus_factor (
        file           TEXT NOT NULL,
        head_hash      TEXT NOT NULL,
        unique_authors INTEGER NOT NULL,
        PRIMARY KEY (file, head_hash)
    );
    "#,
    // Per-file blame output, written chunk-by-chunk during ownership compute
    // so long runs can resume after interrupts. Rows for a head_hash are
    // deleted once the final aggregate lands in `ownership`.
    // A sentinel row with author_email='' records "file processed, no blame
    // emails" so the file isn't retried on every resume.
    r#"
    CREATE TABLE blame_file (
        head_hash    TEXT NOT NULL,
        file_path    TEXT NOT NULL,
        author_email TEXT NOT NULL,
        lines        INTEGER NOT NULL,
        PRIMARY KEY (head_hash, file_path, author_email)
    ) WITHOUT ROWID;
    "#,
    r#"
    ALTER TABLE commits ADD COLUMN files_renamed INTEGER NOT NULL DEFAULT 0;
    "#,
];

fn migrate(conn: &mut Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY)",
        [],
    )?;
    let current: u32 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |r| r.get(0),
    )?;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let target = (i + 1) as u32;
        if target > current {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.execute("INSERT INTO schema_version (version) VALUES (?1)", [target])?;
            tx.commit()?;
        }
    }
    Ok(())
}
