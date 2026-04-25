use anyhow::Result;
use rusqlite::params;
use std::collections::HashMap;

use crate::db::{file_addon_cache_key, Database, FILE_KIND_TOUCHED};

pub struct FileCouplingRow {
    pub file: String,
    pub commit_count: u32,
    pub top_partner: Option<(String, f64)>,
}

const MAX_FILES_PER_COMMIT: i64 = 50;
const MIN_CO_OCCURRENCES: u32 = 3;
const MIN_SCORE: f64 = 0.1;

pub fn compute(
    _repo: &gix::Repository,
    db: &mut Database,
    head_hash: &str,
    on_progress: impl Fn(usize, usize),
) -> Result<Vec<FileCouplingRow>> {
    let key = file_addon_cache_key(db, head_hash)?;

    let cached_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM file_stats WHERE cache_key = ?1",
            params![&key],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if cached_count > 0 {
        return load_from_db(db, &key);
    }

    let total_commits: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM commits WHERE files_changed <= ?1 AND is_merge = 0",
            params![MAX_FILES_PER_COMMIT],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let total_commits = total_commits.max(0) as usize;
    on_progress(0, total_commits);

    let mut intern: HashMap<String, u32> = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    let mut file_counts: HashMap<u32, u32> = HashMap::new();
    let mut co_occur: HashMap<(u32, u32), u32> = HashMap::new();

    {
        let mut stmt = db.prepare(
            "SELECT cf.commit_hash, cf.file_path
             FROM commit_files cf
             JOIN commits c ON c.hash = cf.commit_hash
             WHERE cf.kind = ?1 AND c.files_changed <= ?2 AND c.is_merge = 0
             ORDER BY cf.commit_hash",
        )?;
        let mut rows = stmt.query(params![FILE_KIND_TOUCHED, MAX_FILES_PER_COMMIT])?;

        let mut current_hash: Option<String> = None;
        let mut current_files: Vec<u32> = Vec::new();
        let mut processed = 0usize;

        while let Some(row) = rows.next()? {
            let hash: String = row.get(0)?;
            let path: String = row.get(1)?;
            if current_hash.as_ref() != Some(&hash) {
                if current_hash.is_some() {
                    accumulate(&current_files, &mut file_counts, &mut co_occur);
                    processed += 1;
                    if processed % 500 == 0 {
                        on_progress(processed, total_commits);
                    }
                }
                current_hash = Some(hash);
                current_files.clear();
            }
            let id = match intern.get(&path) {
                Some(&id) => id,
                None => {
                    let id = names.len() as u32;
                    intern.insert(path.clone(), id);
                    names.push(path);
                    id
                }
            };
            current_files.push(id);
        }
        if current_hash.is_some() {
            accumulate(&current_files, &mut file_counts, &mut co_occur);
            processed += 1;
        }
        on_progress(processed, total_commits);
    }

    persist(db, &key, &names, &file_counts, &co_occur)?;
    load_from_db(db, &key)
}

fn accumulate(
    files: &[u32],
    file_counts: &mut HashMap<u32, u32>,
    co_occur: &mut HashMap<(u32, u32), u32>,
) {
    for &f in files {
        *file_counts.entry(f).or_insert(0) += 1;
    }
    for j in 0..files.len() {
        for k in (j + 1)..files.len() {
            let pair = if files[j] <= files[k] {
                (files[j], files[k])
            } else {
                (files[k], files[j])
            };
            *co_occur.entry(pair).or_insert(0) += 1;
        }
    }
}

fn persist(
    db: &mut Database,
    key: &str,
    names: &[String],
    file_counts: &HashMap<u32, u32>,
    co_occur: &HashMap<(u32, u32), u32>,
) -> Result<()> {
    let tx = db.transaction()?;
    tx.execute("DELETE FROM file_stats WHERE cache_key != ?1", params![key])?;
    tx.execute("DELETE FROM file_coupling", [])?;

    {
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO file_stats (file, commit_count, cache_key)
             VALUES (?1, ?2, ?3)",
        )?;
        for (&id, &count) in file_counts {
            stmt.execute(params![&names[id as usize], count as i64, key])?;
        }
    }
    {
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO file_coupling (file_a, file_b, co_occurrences, score)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (&(id_a, id_b), &co) in co_occur {
            if co < MIN_CO_OCCURRENCES {
                continue;
            }
            let ca = *file_counts.get(&id_a).unwrap_or(&1) as f64;
            let cb = *file_counts.get(&id_b).unwrap_or(&1) as f64;
            let score = co as f64 / ca.min(cb);
            if score < MIN_SCORE {
                continue;
            }
            stmt.execute(params![
                &names[id_a as usize],
                &names[id_b as usize],
                co as i64,
                score
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn load_from_db(db: &Database, key: &str) -> Result<Vec<FileCouplingRow>> {
    let mut file_counts: HashMap<String, u32> = HashMap::new();
    {
        let mut stmt =
            db.prepare("SELECT file, commit_count FROM file_stats WHERE cache_key = ?1")?;
        let rows = stmt.query_map(params![key], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
        })?;
        for row in rows {
            let (file, count) = row?;
            file_counts.insert(file, count);
        }
    }

    let mut best_partner: HashMap<String, (String, f64)> = HashMap::new();
    {
        let mut stmt =
            db.prepare("SELECT file_a, file_b, score FROM file_coupling ORDER BY score DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
            ))
        })?;
        for row in rows {
            let (file_a, file_b, score) = row?;
            best_partner
                .entry(file_a.clone())
                .or_insert((file_b.clone(), score));
            best_partner.entry(file_b).or_insert((file_a, score));
        }
    }

    let mut results: Vec<FileCouplingRow> = file_counts
        .into_iter()
        .map(|(file, commit_count)| {
            let top_partner = best_partner.get(&file).cloned();
            FileCouplingRow {
                file,
                commit_count,
                top_partner,
            }
        })
        .collect();

    results.sort_by_key(|r| std::cmp::Reverse(r.commit_count));
    Ok(results)
}
