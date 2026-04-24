use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

mod app;
mod cache;
mod db;
mod export;
mod fmt;
mod git;
mod identity;
mod identity_map;
mod mailmap;
mod ownership;
mod questionnaire;
mod remote;
mod storage;
mod ui;
mod util;

#[derive(Parser)]
#[command(name = "gild", about = "Interactive git contribution analyzer")]
struct Cli {
    /// Path to git repository (local path or remote URL)
    #[arg(default_value = ".")]
    path: String,

    /// Branch to analyze
    #[arg(short, long)]
    branch: Option<String>,

    /// Maximum number of commits to analyze
    #[arg(short = 'n', long)]
    max_commits: Option<usize>,

    /// Print table to stdout instead of interactive TUI
    #[arg(long)]
    print: bool,

    /// Skip code ownership analysis
    #[arg(long)]
    no_ownership: bool,

    /// Export data in specified format (json, csv, html)
    #[arg(long, value_name = "FORMAT")]
    export: Option<String>,

    /// Output file for export (default: stdout)
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<PathBuf>,
}

fn clear_progress() {
    eprint!("\r\x1b[2K");
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let path = if remote::is_remote_url(&cli.path) {
        remote::ensure_repo(&cli.path)?
    } else {
        let p = PathBuf::from(&cli.path);
        p.canonicalize().unwrap_or(p)
    };

    let repo_key = storage::resolve_repo_key(&path)?;
    std::fs::create_dir_all(storage::repo_data_dir(&repo_key))?;

    let map_path = storage::identities_path(&repo_key);
    let legacy_gild_dir = path.join(".git/gild");
    let legacy_id_path = legacy_gild_dir.join("identities.toml");
    if !map_path.exists() && legacy_id_path.exists() {
        let _ = std::fs::copy(&legacy_id_path, &map_path);
    }
    for name in &["cache.json", "ownership.json", "identities.toml"] {
        let _ = std::fs::remove_file(legacy_gild_dir.join(name));
    }
    let _ = std::fs::remove_dir(&legacy_gild_dir);

    let db = db::Database::open(&storage::db_path(&repo_key))?;
    let mut commit_cache = cache::Cache::load(&db)?;

    let (info, repo, mut commits) = git::load_commits(
        &path,
        cli.branch.as_deref(),
        cli.max_commits,
        &mut commit_cache,
        |total, new| {
            let step = match total {
                0..=99 => 1,
                100..=999 => 10,
                1000..=9999 => 100,
                _ => 1000,
            };
            if total % step == 0 {
                if new > 0 {
                    eprint!("\r  Scanning... {} commits ({} new)", total, new);
                } else {
                    eprint!("\r  Loading... {} commits (cached)", total);
                }
            }
        },
    )?;
    clear_progress();

    if commits.is_empty() {
        eprintln!("No commits found.");
        return Ok(());
    }

    commit_cache.save(&db)?;

    let mailmap_entries = mailmap::load(&path);
    if !mailmap_entries.is_empty() {
        eprintln!(
            "  .mailmap: {} entries loaded",
            mailmap_entries.len()
        );
    }

    let identity_map = identity_map::IdentityMap::load(&map_path);

    let (groups, assignments) = identity::merge(&commits, &identity_map, &mailmap_entries);
    for (commit, &gid) in commits.iter_mut().zip(assignments.iter()) {
        commit.group_id = gid;
    }

    let mut app = app::App::new(
        commits,
        groups,
        info,
        identity_map,
        map_path,
        mailmap_entries,
    );

    if !cli.no_ownership {
        match ownership::compute(&repo, &db, app.commits(), app.groups(), |done, total| {
            eprint!("\r  Ownership... {}/{} files", done, total);
        }) {
            Ok(data) => {
                clear_progress();
                let (per_group, mapped_total) =
                    ownership::map_to_groups(&data, app.groups());
                app.set_ownership(per_group, mapped_total);
            }
            Err(e) => {
                clear_progress();
                eprintln!("  Ownership: skipped ({})", e);
            }
        }
    }

    if let Some(ref format) = cli.export {
        let output_path = cli.output.as_deref();
        export::export(&app, format, output_path)?;
    } else if cli.print {
        ui::print_table(&app);
    } else {
        ui::run(&mut app)?;
    }

    Ok(())
}
