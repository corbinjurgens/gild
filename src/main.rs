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
mod log;
mod mailmap;
mod ownership;
mod questionnaire;
mod remote;
mod storage;
mod ui;
mod util;

#[derive(Parser)]
#[command(
    name = "gild",
    about = "Interactive git contribution analyzer",
    version
)]
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

    /// Enable optional deep-analysis add-on (can be repeated): ownership, coupling, authors, hotspot, types
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

    /// Write verbose log with timings to the repo data directory
    #[arg(long)]
    log: bool,
}

const ADDON_STEPS: &[(&str, &str, bool)] = &[
    ("ownership", "Analyzing ownership", true),
    ("coupling", "Analyzing file coupling", false),
    ("authors", "Analyzing authors", false),
    ("hotspot", "Analyzing hotspots", false),
    ("types", "Analyzing commit types", false),
];

fn addon_label(name: &str) -> &'static str {
    match name {
        "ownership" => "Analyzing ownership",
        "coupling" => "Analyzing file coupling",
        "authors" => "Analyzing authors",
        "hotspot" => "Analyzing hotspots",
        "types" => "Analyzing commit types",
        _ => unreachable!("validated in main"),
    }
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

    const ALIASES: &[(&str, &str)] = &[
        ("blame", "ownership"),
        ("bus-factor", "authors"),
        ("churn", "hotspot"),
        ("commit-types", "types"),
    ];
    let add_ons: Vec<String> = cli
        .add_ons
        .iter()
        .map(|name| {
            ALIASES
                .iter()
                .find(|&&(alias, _)| alias == name.as_str())
                .map(|&(_, canon)| canon.to_string())
                .unwrap_or_else(|| name.clone())
        })
        .collect();
    for name in &add_ons {
        if !ADDON_STEPS.iter().any(|&(n, _, _)| n == name.as_str()) {
            anyhow::bail!(
                "Unknown add-on '{}'. Valid add-ons: {}",
                name,
                ADDON_STEPS
                    .iter()
                    .map(|&(n, _, _)| n)
                    .collect::<Vec<_>>()
                    .join(", "),
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

    let thread_count = cli.max_threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });
    if let Some(n) = cli.max_threads {
        let _ = rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global();
    }

    let mut logger = if cli.log {
        let log_path = storage::repo_data_dir(&repo_key).join("gild.log");
        match log::Logger::open(&log_path) {
            Some(l) => {
                eprintln!("Logging to {}", log_path.display());
                Some(l)
            }
            None => {
                eprintln!("Warning: could not create log file {}", log_path.display());
                None
            }
        }
    } else {
        None
    };
    if let Some(ref mut l) = logger {
        l.info(&format!("repo: {}", path.display()));
        l.info(&format!("threads: {thread_count}"));
        if !add_ons.is_empty() {
            l.info(&format!("add-ons: {}", add_ons.join(", ")));
        }
        if let Some(n) = cli.max_commits {
            l.info(&format!("max-commits: {n}"));
        }
    }

    if cli.export.is_some() || cli.print {
        let progress = StderrProgress;
        let mut app = load(LoadParams {
            path: &path,
            db_path: &storage::db_path(&repo_key),
            map_path: &map_path,
            branch: cli.branch.as_deref(),
            max_commits: cli.max_commits,
            add_ons: &add_ons,
            logger: &mut logger,
            progress: &progress,
        })?;
        if let Some(ref mut l) = logger {
            l.finish();
        }
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
        std::thread::spawn(move || {
            let progress = ChannelProgress::new(tx.clone(), &add_ons, thread_count);
            let result = load(LoadParams {
                path: &path,
                db_path: &db_path,
                map_path: &map_path,
                branch: branch.as_deref(),
                max_commits,
                add_ons: &add_ons,
                logger: &mut logger,
                progress: &progress,
            });
            if let Some(ref mut l) = logger {
                l.finish();
            }
            match result {
                Ok(app) => {
                    let _ = tx.send(ui::LoadMsg::Done(Box::new(app)));
                }
                Err(e) => {
                    let _ = tx.send(ui::LoadMsg::Failed(e.to_string()));
                }
            }
        });
        ui::run_with_loading(rx)?;
    }

    Ok(())
}

trait Progress {
    fn step_start(&self, _label: &'static str) {}
    fn commit_total(&self, _total: usize) {}
    fn commit_progress(&self, _processed: usize, _new_count: usize) {}
    fn addon_progress(&self, _label: &'static str, _done: usize, _total: usize) {}
    fn clear_line(&self) {}
    fn info(&self, _msg: &str) {}
}

struct StderrProgress;

impl Progress for StderrProgress {
    fn commit_progress(&self, processed: usize, new: usize) {
        let step = match processed {
            0..=99 => 1,
            100..=999 => 10,
            1000..=9999 => 100,
            _ => 1000,
        };
        if processed.is_multiple_of(step) {
            if new > 0 {
                eprint!("\r  Scanning... {processed} commits ({new} new)");
            } else {
                eprint!("\r  Loading... {processed} commits (cached)");
            }
        }
    }

    fn addon_progress(&self, label: &'static str, done: usize, total: usize) {
        eprint!("\r  {label}... {done}/{total}");
    }

    fn clear_line(&self) {
        eprint!("\r\x1b[2K");
    }

    fn info(&self, msg: &str) {
        eprintln!("  {msg}");
    }
}

struct ChannelProgress {
    tx: std::sync::mpsc::Sender<ui::LoadMsg>,
    steps: Vec<ui::LoadStep>,
}

impl ChannelProgress {
    fn new(
        tx: std::sync::mpsc::Sender<ui::LoadMsg>,
        add_ons: &[String],
        thread_count: usize,
    ) -> Self {
        let mut steps = vec![ui::LoadStep {
            label: "Scanning commits",
            parallel: true,
        }];
        for &(name, label, parallel) in ADDON_STEPS {
            if add_ons.iter().any(|a| a == name) {
                steps.push(ui::LoadStep { label, parallel });
            }
        }
        let _ = tx.send(ui::LoadMsg::ScanThreads(thread_count));
        let _ = tx.send(ui::LoadMsg::Plan(steps.clone()));
        Self { tx, steps }
    }
}

impl Progress for ChannelProgress {
    fn step_start(&self, label: &'static str) {
        let idx = self
            .steps
            .iter()
            .position(|s| s.label == label)
            .unwrap_or(0);
        let _ = self.tx.send(ui::LoadMsg::StepStart(idx));
    }

    fn commit_total(&self, total: usize) {
        let _ = self.tx.send(ui::LoadMsg::CommitTotal(total));
    }

    fn commit_progress(&self, processed: usize, new_count: usize) {
        let _ = self.tx.send(ui::LoadMsg::CommitProgress {
            processed,
            new_count,
        });
    }

    fn addon_progress(&self, label: &'static str, done: usize, total: usize) {
        let _ = self.tx.send(ui::LoadMsg::AddonProgress { label, done, total });
    }
}

struct LoadParams<'a> {
    path: &'a std::path::Path,
    db_path: &'a std::path::Path,
    map_path: &'a std::path::Path,
    branch: Option<&'a str>,
    max_commits: Option<usize>,
    add_ons: &'a [String],
    logger: &'a mut Option<log::Logger>,
    progress: &'a (dyn Progress + Sync),
}

fn load(p: LoadParams) -> Result<app::App> {
    let LoadParams { path, db_path, map_path, branch, max_commits, add_ons, logger, progress } = p;
    progress.step_start("Scanning commits");

    let db = db::Database::open(db_path)?;
    gc::run(path, &db)?;
    if let Some(ref mut l) = logger {
        l.phase_start("cache load");
    }
    let mut commit_cache = cache::Cache::load(&db)?;

    if let Some(ref mut l) = logger {
        l.phase_start("commit scan");
    }
    let (info, mut commits) = git::load_commits(
        path,
        branch,
        max_commits,
        &mut commit_cache,
        |total| progress.commit_total(total),
        |processed, new| progress.commit_progress(processed, new),
    )?;
    progress.clear_line();

    if commits.is_empty() {
        anyhow::bail!("No commits found.");
    }

    let repo = gix::open(path)?;

    let new_count = commit_cache.staged_count();
    if let Some(ref mut l) = logger {
        l.info(&format!("total commits: {}", commits.len()));
        l.info(&format!("new commits: {new_count}"));
        l.info(&format!("cached commits: {}", commits.len() - new_count));
        l.info(&format!("authors (raw): {}", {
            let mut emails: Vec<&str> = commits.iter().map(|c| c.author_email.as_str()).collect();
            emails.sort_unstable();
            emails.dedup();
            emails.len()
        }));
    }

    if let Some(ref mut l) = logger {
        l.phase_start("cache save");
    }
    commit_cache.save(&db)?;

    if let Some(ref mut l) = logger {
        l.phase_start("identity merge");
    }
    let mailmap_entries = mailmap::load(path);
    if !mailmap_entries.is_empty() {
        progress.info(&format!(
            ".mailmap: {} entries loaded",
            mailmap_entries.len()
        ));
    }

    let identity_map = identity_map::IdentityMap::load(map_path);
    let (groups, assignments) = identity::merge(&commits, &identity_map, &mailmap_entries);
    for (commit, &gid) in commits.iter_mut().zip(assignments.iter()) {
        commit.group_id = gid;
    }
    if let Some(ref mut l) = logger {
        l.info(&format!("identity groups: {}", groups.len()));
    }

    let ownership_data = if add_ons.iter().any(|a| a == "ownership") {
        let label = addon_label("ownership");
        progress.step_start(label);
        if let Some(ref mut l) = logger {
            l.phase_start(label);
        }
        match ownership::compute(&repo, &db, &groups, |done, total| {
            progress.addon_progress(label, done, total);
        }) {
            Ok(data) => {
                progress.clear_line();
                Some(data)
            }
            Err(e) => {
                progress.clear_line();
                progress.info(&format!("--add-on ownership: skipped ({e})"));
                if let Some(ref mut l) = logger {
                    l.info(&format!("ownership skipped: {e}"));
                }
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
        |label| progress.step_start(label),
        |label, done, total| progress.addon_progress(label, done, total),
        logger,
    );
    progress.clear_line();

    if let Some(ref mut l) = logger {
        l.phase_start("app init");
    }
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
    if add_ons.iter().any(|a| a == "types") {
        let label = addon_label("types");
        progress.step_start(label);
        if let Some(ref mut l) = logger {
            l.phase_start(label);
        }
        app.set_commit_types();
    }

    Ok(app)
}

fn run_file_addons(
    repo: &gix::Repository,
    db: &db::Database,
    add_ons: &[String],
    on_step_start: impl Fn(&'static str),
    on_progress: impl Fn(&'static str, usize, usize),
    logger: &mut Option<log::Logger>,
) -> Vec<app::FileRow> {
    let need_coupling = add_ons.iter().any(|a| a == "coupling");
    let need_bus_factor = add_ons.iter().any(|a| a == "authors");
    let need_churn = add_ons.iter().any(|a| a == "hotspot");

    if !need_coupling && !need_bus_factor && !need_churn {
        return Vec::new();
    }

    let head_hash = match repo.head().map(|h| h.into_peeled_id()) {
        Ok(Ok(id)) => id.to_string(),
        _ => return Vec::new(),
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
        let label = addon_label("coupling");
        on_step_start(label);
        if let Some(ref mut l) = logger {
            l.phase_start(label);
        }
        if let Ok(rows) = coupling::compute(db, &head_hash, |done, total| {
            on_progress(label, done, total);
        }) {
            if let Some(ref mut l) = logger {
                l.info(&format!("coupling pairs: {}", rows.len()));
            }
            for row in rows {
                let entry = row_entry(&mut file_map, &row.file);
                entry.commit_count = row.commit_count;
                entry.top_coupled = row.top_partner;
            }
        }
    }

    if need_bus_factor {
        let label = addon_label("authors");
        on_step_start(label);
        if let Some(ref mut l) = logger {
            l.phase_start(label);
        }
        if let Ok(rows) = bus_factor::compute(repo, db, |done, total| {
            on_progress(label, done, total);
        }) {
            if let Some(ref mut l) = logger {
                l.info(&format!("files with authors: {}", rows.len()));
            }
            for row in rows {
                row_entry(&mut file_map, &row.file).unique_authors = Some(row.unique_authors);
            }
        }
    }

    if need_churn {
        let label = addon_label("hotspot");
        on_step_start(label);
        if let Some(ref mut l) = logger {
            l.phase_start(label);
        }
        if let Ok(rows) = churn::compute(repo, db, &head_hash, |done, total| {
            on_progress(label, done, total);
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
