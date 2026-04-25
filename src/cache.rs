use crate::db::{Database, FILE_KIND_ADDED, FILE_KIND_DELETED, FILE_KIND_TOUCHED};
use crate::git::Commit;
use anyhow::Result;
use rusqlite::params;
use std::collections::HashMap;

pub struct CommitFiles {
    pub files: Vec<String>,
    pub added: Vec<String>,
    pub deleted: Vec<String>,
}

/// Per-run commit cache. Stats-only commits are mirrored in `loaded` for
/// phase-1 cache hits; file-path lists only exist in the DB (and in `staged`
/// between insert and save).
pub struct Cache {
    loaded: HashMap<String, Commit>,
    staged_commits: HashMap<String, Commit>,
    staged_files: HashMap<String, CommitFiles>,
    dirty: bool,
}

impl Cache {
    pub fn load(db: &Database) -> Result<Self> {
        let mut loaded = HashMap::new();
        let mut stmt = db.conn.prepare(
            "SELECT hash, author_name, author_email, lines_added, lines_removed,
                    files_changed, timestamp, whitespace_added, whitespace_removed,
                    files_added, files_deleted, is_merge
             FROM commits",
        )?;
        let rows = stmt.query_map([], |row| {
            let hash: String = row.get(0)?;
            Ok((
                hash,
                Commit {
                    author_name: row.get(1)?,
                    author_email: row.get(2)?,
                    group_id: 0,
                    lines_added: row.get::<_, i64>(3)? as usize,
                    lines_removed: row.get::<_, i64>(4)? as usize,
                    files_changed: row.get::<_, i64>(5)? as usize,
                    timestamp: row.get(6)?,
                    whitespace_added: row.get::<_, i64>(7)? as usize,
                    whitespace_removed: row.get::<_, i64>(8)? as usize,
                    files_added: row.get::<_, i64>(9)? as usize,
                    files_deleted: row.get::<_, i64>(10)? as usize,
                    is_merge: row.get::<_, i64>(11)? != 0,
                },
            ))
        })?;
        for r in rows {
            let (h, c) = r?;
            loaded.insert(h, c);
        }
        Ok(Self {
            loaded,
            staged_commits: HashMap::new(),
            staged_files: HashMap::new(),
            dirty: false,
        })
    }

    pub fn save(&mut self, db: &Database) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let tx = db.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO commits (
                    hash, author_name, author_email, lines_added, lines_removed,
                    files_changed, timestamp, whitespace_added, whitespace_removed,
                    files_added, files_deleted, is_merge
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            )?;
            for (hash, c) in self.staged_commits.drain() {
                stmt.execute(params![
                    hash,
                    c.author_name,
                    c.author_email,
                    c.lines_added as i64,
                    c.lines_removed as i64,
                    c.files_changed as i64,
                    c.timestamp,
                    c.whitespace_added as i64,
                    c.whitespace_removed as i64,
                    c.files_added as i64,
                    c.files_deleted as i64,
                    c.is_merge as i64,
                ])?;
            }
        }
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO commit_files (commit_hash, file_path, kind)
                 VALUES (?1, ?2, ?3)",
            )?;
            for (hash, f) in self.staged_files.drain() {
                for path in &f.files {
                    stmt.execute(params![hash, path, FILE_KIND_TOUCHED])?;
                }
                for path in &f.added {
                    stmt.execute(params![hash, path, FILE_KIND_ADDED])?;
                }
                for path in &f.deleted {
                    stmt.execute(params![hash, path, FILE_KIND_DELETED])?;
                }
            }
        }
        tx.commit()?;
        self.dirty = false;
        Ok(())
    }

    pub fn get(&self, hash: &str) -> Option<Commit> {
        self.staged_commits
            .get(hash)
            .cloned()
            .or_else(|| self.loaded.get(hash).cloned())
    }

    pub fn insert(&mut self, hash: String, commit: Commit, files: CommitFiles) {
        self.staged_commits.insert(hash.clone(), commit);
        self.staged_files.insert(hash, files);
        self.dirty = true;
    }
}
