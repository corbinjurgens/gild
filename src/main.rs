use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

mod app;
mod bus_factor;
mod cache;
mod churn;
mod coupling;
mod db;
mod export;
mod fmt;
mod gc;
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
#[command(name = "gild", about = "Interactive git contribution analyzer", version)]
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

    /// Enable optional deep-analysis add-on (can be repeated): ownership, coupling, bus-factor, churn
    #[arg(long = "add-on", value_name = "NAME")]
    add_ons: Vec<String>,

    /// Export data in specified format (json, csv, html)
    #[arg(long, value_name = "FORMAT")]
    export: Option<String>,

    /// Output file for export (default: stdout)
    #[arg(short = 'o', long, value_name = "FILE", requires = "export")]
    output: Option<PathBuf>,

    /// Clear cached commit data for this repository and exit
    #[arg(long)]
    clear_cache: bool,

    /// Maximum number of threads for parallel commit scanning (default: all CPU cores)
    #[arg(long, value_name = "N")]
    max_threads: Option<usize>,
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
        if !p.exists() {
            anyhow::bail!("Path does not exist: {}", p.display());
        }
        p.canonicalize().unwrap_or(p)
    };

    let repo_key = storage::resolve_repo_key(&path)?;

    const VALID_ADDONS: &[&str] = &["ownership", "coupling", "bus-factor", "churn"];
    for name in &cli.add_ons {
        if !VALID_ADDONS.contains(&name.as_str()) {
            anyhow::bail!(
                "Unknown add-on '{}'. Valid add-ons: {}",
                name,
                VALID_ADDONS.join(", ")
            );
        }
    }

    if cli.clear_cache {
        let db_path = storage::db_path(&repo_key);
        if db_path.exists() {
            std::fs::remove_file(&db_path)?;
            eprintln!("Cache cleared: {}", db_path.display());
        } else {
            eprintln!("No cache found ({})", db_path.display());
        }
        return Ok(());
    }

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

    let thread_count = cli.max_threads
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1));
    if let Some(n) = cli.max_threads {
        let _ = rayon::ThreadPoolBuilder::new().num_threads(n).build_global();
    }

    if cli.export.is_some() || cli.print {
        let mut app = load_sync(
            &path,
            &storage::db_path(&repo_key),
            &map_path,
            cli.branch.as_deref(),
            cli.max_commits,
            &cli.add_ons,
        )?;
        if let Some(ref format) = cli.export {
            app.set_time_mode(app::TimeMode::All);
            export::export(&app, format, cli.output.as_deref())?;
        } else {
            ui::print_table(&app);
        }
    } else {
        let (tx, rx) = std::sync::mpsc::channel();
        let db_path = storage::db_path(&repo_key);
        let branch = cli.branch.clone();
        let max_commits = cli.max_commits;
        let add_ons = cli.add_ons.clone();
        std::thread::spawn(move || {
            load_in_background(path, branch, max_commits, add_ons, db_path, map_path, thread_count, tx);
        });
        ui::run_with_loading(rx)?;
    }

    Ok(())
}

fn load_sync(
    path: &std::path::Path,
    db_path: &std::path::Path,
    map_path: &std::path::Path,
    branch: Option<&str>,
    max_commits: Option<usize>,
    add_ons: &[String],
) -> Result<app::App> {
    let db = db::Database::open(db_path)?;
    gc::run(path, &db)?;
    let mut commit_cache = cache::Cache::load(&db)?;

    let (info, repo, mut commits) = git::load_commits(
        path,
        branch,
        max_commits,
        &mut commit_cache,
        |_| {},
        |processed, new| {
            let step = match processed {
                0..=99 => 1,
                100..=999 => 10,
                1000..=9999 => 100,
                _ => 1000,
            };
            if processed % step == 0 {
                if new > 0 {
                    eprint!("\r  Scanning... {} commits ({} new)", processed, new);
                } else {
                    eprint!("\r  Loading... {} commits (cached)", processed);
                }
            }
        },
    )?;
    clear_progress();

    if commits.is_empty() {
        anyhow::bail!("No commits found.");
    }

    commit_cache.save(&db)?;

    let mailmap_entries = mailmap::load(path);
    if !mailmap_entries.is_empty() {
        eprintln!("  .mailmap: {} entries loaded", mailmap_entries.len());
    }

    let identity_map = identity_map::IdentityMap::load(map_path);
    let (groups, assignments) = identity::merge(&commits, &identity_map, &mailmap_entries);
    for (commit, &gid) in commits.iter_mut().zip(assignments.iter()) {
        commit.group_id = gid;
    }

    let ownership_data = if add_ons.iter().any(|a| a == "ownership") {
        match ownership::compute(&repo, &db, &groups, |done, total| {
            eprint!("\r  Ownership... {}/{} files", done, total);
        }) {
            Ok(data) => {
                clear_progress();
                Some(data)
            }
            Err(e) => {
                clear_progress();
                eprintln!("  --add-on ownership: skipped ({})", e);
                None
            }
        }
    } else {
        None
    };

    let file_rows = run_file_addons(
        &repo,
        &db,
        add_ons,
        |_label| {},
        |label, done, total| {
            eprint!("\r  {}... {}/{}", label, done, total);
        },
    );
    clear_progress();

    let mut app = app::App::new(
        commits,
        groups,
        info,
        identity_map,
        map_path.to_path_buf(),
        mailmap_entries,
        db,
    );
    if let Some(data) = ownership_data {
        let (per_group, mapped_total) = ownership::map_to_groups(&data, app.groups());
        app.set_ownership(per_group, mapped_total);
    }
    if !file_rows.is_empty() {
        app.set_file_data(file_rows);
    }

    Ok(app)
}

fn load_in_background(
    path: PathBuf,
    branch: Option<String>,
    max_commits: Option<usize>,
    add_ons: Vec<String>,
    db_path: PathBuf,
    identities_path: PathBuf,
    thread_count: usize,
    tx: std::sync::mpsc::Sender<ui::LoadMsg>,
) {
    tx.send(ui::LoadMsg::ScanThreads(thread_count)).ok();

    // Each step declares whether it may dispatch work across rayon threads so
    // the loading UI can show the "N threads" badge without re-hardcoding the
    // set of parallel phases. Keep this list in sync with the actual work
    // dispatched below.
    let mut plan: Vec<ui::LoadStep> = vec![ui::LoadStep {
        label: "Scanning commits",
        parallel: true,
    }];
    if add_ons.iter().any(|a| a == "ownership") {
        plan.push(ui::LoadStep { label: "Analyzing ownership", parallel: true });
    }
    if add_ons.iter().any(|a| a == "coupling") {
        plan.push(ui::LoadStep { label: "Analyzing file coupling", parallel: false });
    }
    if add_ons.iter().any(|a| a == "bus-factor") {
        plan.push(ui::LoadStep { label: "Analyzing bus factor", parallel: false });
    }
    if add_ons.iter().any(|a| a == "churn") {
        plan.push(ui::LoadStep { label: "Analyzing churn", parallel: false });
    }
    let plan_for_lookup = plan.clone();
    let step_idx = move |label: &'static str| -> usize {
        plan_for_lookup
            .iter()
            .position(|s| s.label == label)
            .unwrap_or(0)
    };
    tx.send(ui::LoadMsg::Plan(plan)).ok();
    tx.send(ui::LoadMsg::StepStart(0)).ok();

    let result = (|| -> Result<Box<app::App>> {
        let db = db::Database::open(&db_path)?;
        gc::run(&path, &db)?;
        let mut commit_cache = cache::Cache::load(&db)?;

        let (info, repo, mut commits) = git::load_commits(
            &path,
            branch.as_deref(),
            max_commits,
            &mut commit_cache,
            |total| {
                tx.send(ui::LoadMsg::CommitTotal(total)).ok();
            },
            |processed, new| {
                tx.send(ui::LoadMsg::CommitProgress { processed, new_count: new }).ok();
            },
        )?;

        if commits.is_empty() {
            anyhow::bail!("No commits found.");
        }

        commit_cache.save(&db)?;

        let mailmap_entries = mailmap::load(&path);
        let identity_map = identity_map::IdentityMap::load(&identities_path);
        let (groups, assignments) = identity::merge(&commits, &identity_map, &mailmap_entries);
        for (commit, &gid) in commits.iter_mut().zip(assignments.iter()) {
            commit.group_id = gid;
        }

        let ownership_data = if add_ons.iter().any(|a| a == "ownership") {
            tx.send(ui::LoadMsg::StepStart(step_idx("Analyzing ownership"))).ok();
            ownership::compute(&repo, &db, &groups, |done, total| {
                tx.send(ui::LoadMsg::AddonProgress {
                    label: "Analyzing ownership",
                    done,
                    total,
                })
                .ok();
            })
            .ok()
        } else {
            None
        };

        let file_rows = run_file_addons(
            &repo,
            &db,
            &add_ons,
            |label| {
                tx.send(ui::LoadMsg::StepStart(step_idx(label))).ok();
            },
            |label, done, total| {
                tx.send(ui::LoadMsg::AddonProgress { label, done, total }).ok();
            },
        );

        let mut app = app::App::new(
            commits,
            groups,
            info,
            identity_map,
            identities_path,
            mailmap_entries,
            db,
        );
        if let Some(data) = ownership_data {
            let (per_group, mapped_total) = ownership::map_to_groups(&data, app.groups());
            app.set_ownership(per_group, mapped_total);
        }
        if !file_rows.is_empty() {
            app.set_file_data(file_rows);
        }

        Ok(Box::new(app))
    })();

    match result {
        Ok(app) => { tx.send(ui::LoadMsg::Done(app)).ok(); }
        Err(e) => { tx.send(ui::LoadMsg::Failed(e.to_string())).ok(); }
    }
}

fn run_file_addons(
    repo: &git2::Repository,
    db: &db::Database,
    add_ons: &[String],
    on_step_start: impl Fn(&'static str),
    on_progress: impl Fn(&'static str, usize, usize),
) -> Vec<app::FileRow> {
    let need_coupling = add_ons.iter().any(|a| a == "coupling");
    let need_bus_factor = add_ons.iter().any(|a| a == "bus-factor");
    let need_churn = add_ons.iter().any(|a| a == "churn");

    if !need_coupling && !need_bus_factor && !need_churn {
        return Vec::new();
    }

    let head_hash = match repo.head().and_then(|r| r.peel_to_commit()) {
        Ok(c) => c.id().to_string(),
        Err(_) => return Vec::new(),
    };

    let mut file_map: std::collections::HashMap<String, app::FileRow> =
        std::collections::HashMap::new();

    fn row_entry<'a>(
        map: &'a mut std::collections::HashMap<String, app::FileRow>,
        path: &str,
    ) -> &'a mut app::FileRow {
        map.entry(path.to_string()).or_insert_with(|| app::FileRow {
            path: path.to_string(),
            commit_count: 0,
            unique_authors: None,
            churn_score: None,
            top_coupled: None,
        })
    }

    if need_coupling {
        on_step_start("Analyzing file coupling");
        if let Ok(rows) = coupling::compute(db, &head_hash, |done, total| {
            on_progress("Analyzing file coupling", done, total);
        }) {
            for row in rows {
                let entry = row_entry(&mut file_map, &row.file);
                entry.commit_count = row.commit_count;
                entry.top_coupled = row.top_partner;
            }
        }
    }

    if need_bus_factor {
        on_step_start("Analyzing bus factor");
        if let Ok(rows) = bus_factor::compute(repo, db, |done, total| {
            on_progress("Analyzing bus factor", done, total);
        }) {
            for row in rows {
                row_entry(&mut file_map, &row.file).unique_authors = Some(row.unique_authors);
            }
        }
    }

    if need_churn {
        on_step_start("Analyzing churn");
        if let Ok(rows) = churn::compute(repo, db, &head_hash, |done, total| {
            on_progress("Analyzing churn", done, total);
        }) {
            for row in rows {
                let entry = row_entry(&mut file_map, &row.file);
                if entry.commit_count == 0 {
                    entry.commit_count = row.commit_count;
                }
                entry.churn_score = Some(row.churn_score);
            }
        }
    }

    file_map.into_values().collect()
}
