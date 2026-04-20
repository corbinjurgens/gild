use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

mod app;
mod cache;
mod export;
mod fmt;
mod git;
mod identity;
mod identity_map;
mod mailmap;
mod ownership;
mod questionnaire;
mod ui;
mod util;

#[derive(Parser)]
#[command(name = "gild", about = "Interactive git contribution analyzer")]
struct Cli {
    /// Path to git repository
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Branch to analyze
    #[arg(short, long)]
    branch: Option<String>,

    /// Maximum number of commits to analyze
    #[arg(short = 'n', long)]
    max_commits: Option<usize>,

    /// Print table to stdout instead of interactive TUI
    #[arg(long)]
    print: bool,

    /// Skip identity questionnaire
    #[arg(long)]
    no_questions: bool,

    /// Skip code ownership analysis
    #[arg(long)]
    no_ownership: bool,

    /// Export data in specified format (json, csv, html)
    #[arg(long, value_name = "FORMAT")]
    export: Option<String>,

    /// Output file for export (default: stdout)
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Force questionnaire even without TTY (for testing)
    #[arg(long, hide = true)]
    force_questions: bool,
}

fn clear_progress() {
    eprint!("\r\x1b[2K");
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let path = cli
        .path
        .canonicalize()
        .unwrap_or_else(|_| cli.path.clone());

    let mut commit_cache = cache::Cache::load(&path.join(".git"));

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

    commit_cache.save(&info.git_dir)?;

    let mailmap_entries = mailmap::load(&path);
    if !mailmap_entries.is_empty() {
        eprintln!(
            "  .mailmap: {} entries loaded",
            mailmap_entries.len()
        );
    }

    let map_path = cache::identities_path(&info.git_dir);
    let mut identity_map = identity_map::IdentityMap::load(&map_path);

    let initial_merge = identity::merge(&commits, &identity_map, &mailmap_entries);

    let identity_changed = if !cli.no_questions {
        let changed = questionnaire::run(
            &initial_merge.0,
            &mut identity_map,
            &map_path,
            cli.force_questions,
        );
        if changed {
            eprintln!(
                "  Identity map: \x1b[90m{}\x1b[0m",
                map_path.display()
            );
        }
        changed
    } else {
        false
    };

    let (groups, assignments) = if identity_changed {
        identity::merge(&commits, &identity_map, &mailmap_entries)
    } else {
        initial_merge
    };
    for (commit, &gid) in commits.iter_mut().zip(assignments.iter()) {
        commit.group_id = gid;
    }

    let num_groups = groups.len();
    let git_dir = info.git_dir.clone();
    let mut app = app::App::new(commits, groups, info);

    if !cli.no_ownership {
        match ownership::compute(&repo, &git_dir, app.commits(), |done, total| {
            eprint!("\r  Ownership... {}/{} files", done, total);
        }) {
            Ok(data) => {
                clear_progress();
                let (per_group, mapped_total) =
                    ownership::map_to_groups(&data, num_groups);
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
