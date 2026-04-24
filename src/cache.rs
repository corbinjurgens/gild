use crate::db::Database;
use crate::git::Commit;
use anyhow::Result;
use rusqlite::params;
use std::collections::HashMap;

pub struct Cache {
    staged: HashMap<String, Commit>,
    loaded: HashMap<String, Commit>,
    dirty: bool,
}

impl Cache {
    pub fn load(db: &Database) -> Result<Self> {
        let mut loaded = HashMap::new();
        let mut stmt = db.conn.prepare(
            "SELECT hash, author_name, author_email, lines_added, lines_removed,
                    files_changed, timestamp, files_json, whitespace_added, whitespace_removed,
                    files_added, files_deleted, added_file_names_json, deleted_file_names_json, is_merge
             FROM commits",
        )?;
        let rows = stmt.query_map([], |row| {
            let hash: String = row.get(0)?;
            let files_json: String = row.get(7)?;
            let added_json: String = row.get(12)?;
            let deleted_json: String = row.get(13)?;
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
                    files: serde_json::from_str(&files_json).unwrap_or_default(),
                    whitespace_added: row.get::<_, i64>(8)? as usize,
                    whitespace_removed: row.get::<_, i64>(9)? as usize,
                    files_added: row.get::<_, i64>(10)? as usize,
                    files_deleted: row.get::<_, i64>(11)? as usize,
                    added_file_names: serde_json::from_str(&added_json).unwrap_or_default(),
                    deleted_file_names: serde_json::from_str(&deleted_json).unwrap_or_default(),
                    is_merge: row.get::<_, i64>(14)? != 0,
                },
            ))
        })?;
        for r in rows {
            let (h, c) = r?;
            loaded.insert(h, c);
        }
        Ok(Self {
            staged: HashMap::new(),
            loaded,
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
                    files_changed, timestamp, files_json, whitespace_added, whitespace_removed,
                    files_added, files_deleted, added_file_names_json, deleted_file_names_json, is_merge
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            )?;
            for (hash, c) in self.staged.drain() {
                stmt.execute(params![
                    hash,
                    c.author_name,
                    c.author_email,
                    c.lines_added as i64,
                    c.lines_removed as i64,
                    c.files_changed as i64,
                    c.timestamp,
                    serde_json::to_string(&c.files)?,
                    c.whitespace_added as i64,
                    c.whitespace_removed as i64,
                    c.files_added as i64,
                    c.files_deleted as i64,
                    serde_json::to_string(&c.added_file_names)?,
                    serde_json::to_string(&c.deleted_file_names)?,
                    c.is_merge as i64,
                ])?;
            }
        }
        tx.commit()?;
        self.dirty = false;
        Ok(())
    }

    pub fn get(&self, hash: &str) -> Option<Commit> {
        self.staged
            .get(hash)
            .cloned()
            .or_else(|| self.loaded.get(hash).cloned())
    }

    pub fn insert(&mut self, hash: String, commit: Commit) {
        self.staged.insert(hash, commit);
        self.dirty = true;
    }
}
