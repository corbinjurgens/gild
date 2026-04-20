use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

mod app;
mod cache;
mod export;
mod git;
mod identity;
mod identity_map;
mod mailmap;
mod ownership;
mod questionnaire;
mod ui;

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

fn main() -> Result<()> {
    let cli = Cli::parse();

    let path = cli
        .path
        .canonicalize()
        .unwrap_or_else(|_| cli.path.clone());

    let mut commit_cache = cache::Cache::load(&path.join(".git"));

    let (info, commits) = git::load_commits(
        &path,
        cli.branch.as_deref(),
        cli.max_commits,
        &mut commit_cache,
        |total, new| {
            if new > 0 {
                eprint!("\r  Scanning... {} commits ({} new)", total, new);
            } else {
                eprint!("\r  Loading... {} commits (cached)", total);
            }
        },
    )?;
    eprint!("\r\x1b[2K");

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

    let (preliminary_groups, _, _) = identity::merge(&commits, &identity_map, &mailmap_entries);

    if !cli.no_questions {
        let changed = questionnaire::run(
            &preliminary_groups,
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
    }

    let (groups, assignments, _pair_to_group) =
        identity::merge(&commits, &identity_map, &mailmap_entries);

    let commit_entries: Vec<app::CommitEntry> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| app::CommitEntry {
            group_id: assignments[i],
            lines_added: c.lines_added,
            lines_removed: c.lines_removed,
            files_changed: c.files_changed,
            timestamp: c.timestamp,
            files: c.files.clone(),
        })
        .collect();

    let num_groups = groups.len();
    let mut app = app::App::new(commit_entries, groups, info);

    if !cli.no_ownership {
        let repo = git2::Repository::open(&path)?;
        let git_dir = repo.path().to_path_buf();
        let commit_entries_ref = app.commit_entries();
        match ownership::compute(&repo, &git_dir, commit_entries_ref, |done, total| {
            eprint!("\r  Ownership... {}/{} files", done, total);
        }) {
            Ok(data) => {
                eprint!("\r\x1b[2K");
                let (per_group, mapped_total) =
                    ownership::map_to_groups(&data, num_groups);
                app.set_ownership(per_group, mapped_total);
            }
            Err(e) => {
                eprint!("\r\x1b[2K");
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
