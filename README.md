# Gild — interactive git contribution analyzer

Fair impact scoring, identity deduplication, code ownership tracking.

## Requirements

- [Rust](https://www.rust-lang.org/tools/install) 1.70+ (2021 edition)
- A C compiler (for the `git2`/`libgit2` dependency)
  - **macOS**: Xcode Command Line Tools (`xcode-select --install`)
  - **Ubuntu/Debian**: `sudo apt install build-essential pkg-config libssl-dev`
  - **Fedora**: `sudo dnf install gcc openssl-devel`
  - **Arch**: `sudo pacman -S base-devel openssl`
  - **Windows**: Visual Studio Build Tools with C++ workload

## Install

### From source (recommended)

```sh
git clone https://github.com/user/gild.git
cd gild
cargo install --path .
```

### Build without installing

```sh
git clone https://github.com/user/gild.git
cd gild
cargo build --release
./target/release/gild --help
```

> **Tip for development:** `cargo run` compiles and immediately runs the binary — it is equivalent to `cargo build && ./target/debug/gild`. Use `--` to separate cargo's own flags from gild's arguments:
> ```sh
> cargo run -- /path/to/repo              # same as running the binary directly
> cargo run -- /path/to/repo --add-on coupling
> cargo run --release -- /path/to/repo   # optimized build
> ```
> Once installed via `cargo install`, just use `gild` directly.

### Via cargo (once published)

```sh
cargo install gild
```

## Usage

```sh
gild /path/to/repo              # interactive TUI
gild /path/to/repo --print      # static table output
gild -n 500                     # limit to 500 commits
gild -b develop                 # analyze specific branch
gild --export json              # export as JSON (also: csv, html)
gild --export html -o report.html
gild --clear-cache              # delete cached commit data for this repo
gild --max-threads 4            # limit CPU threads used during first-run scanning
```

Run `gild` with no arguments to analyze the current directory.

### Add-ons

Deep-analysis features are opt-in via `--add-on`. They run after the normal commit scan and cache their results — subsequent runs are instant.

```sh
gild --add-on ownership         # blame-based code ownership (Who% column)
gild --add-on coupling          # file co-occurrence analysis (Files view)
gild --add-on bus-factor        # unique-author risk per file (Files view)
gild --add-on churn             # change-frequency hotspots (Files view)
gild --add-on coupling --add-on bus-factor --add-on churn   # stack multiple
```

| Add-on | Time window | Displayed in |
|--------|-------------|--------------|
| `ownership` | No — blame always reflects current HEAD | Author table (`Own%` column) |
| `coupling` | Yes | Files view |
| `bus-factor` | Yes | Files view |
| `churn` | Yes | Files view |

The `coupling`, `bus-factor`, and `churn` add-ons contribute to the Files view (press `V` in the TUI). Columns appear only for the add-ons that ran. Each time window is cached separately, so switching between All / Year / Quarter / Month in the Files view is instant after the first computation.

## TUI keys

### Table / graph view

| Key | Action |
|-----|--------|
| `c` | Sort by commits |
| `+` / `a` | Sort by lines added |
| `-` / `d` | Sort by lines removed |
| `n` | Sort by net lines |
| `f` | Sort by files changed |
| `i` | Sort by impact |
| `N` | Sort by noise % |
| `o` | Sort by ownership |
| `t` | Cycle time window (all / year / quarter / month) |
| `[` / `]` / `Left` / `Right` | Navigate time period |
| `g` | Toggle table / graph view |
| `V` | Open Files view (when coupling / bus-factor / churn add-ons active) |
| `T` | Theme picker (Normal / Readable) |
| `Enter` | Detail drill-down (top files, activity heatmap, new/deleted files) |
| `j` / `k` / `Up` / `Down` | Select author |
| `Home` / `End` / `G` | Jump to top / bottom |
| `Esc` / `q` | Quit |

### Files view (requires at least one file-level add-on)

| Key | Action |
|-----|--------|
| `j` / `k` / `Up` / `Down` | Select file |
| `G` | Jump to bottom |
| `c` | Sort by commits |
| `a` | Sort by unique authors (bus-factor) |
| `h` | Sort by churn score |
| `p` | Sort by coupling score |
| `V` / `Esc` | Return to table |
| `q` | Quit |

### Detail view

| Key | Action |
|-----|--------|
| `j` / `k` / `Up` / `Down` | Navigate between authors |
| `t` | Cycle time window |
| `[` / `]` / `Left` / `Right` | Navigate time period |
| `T` | Theme picker |
| `PageUp` / `PageDown` | Scroll detail content |
| `Home` | Scroll to top |
| `Esc` / `Backspace` | Back to table |
| `q` | Quit |

## How it works

- **Impact scoring** — measures total work substance, not commit count. The base score is `(1 + ln(1 + lines)) × (1 + 0.5 × ln(1 + total_files_changed))` computed on all lines added/removed and the sum of per-commit file counts in the window. A small consistency bonus `× (1 + 0.15 × ln(sessions))` rewards regular activity, where sessions are commit groups separated by 30-minute gaps. Both terms are log-compressed, so a single clean commit with 200 lines outscores a flurry of tiny typo-fix commits with the same total lines — commit-splitting can't inflate impact meaningfully
- **Identity deduplication** — union-find merges authors by email, `.mailmap`, and saved confirmations; an interactive questionnaire catches fuzzy matches (Levenshtein, substring, email heuristics)
- **Code ownership** (`--add-on ownership`) — runs `git blame` on every non-binary file in HEAD and attributes each surviving line to its author. This is more accurate than last-touch (who last committed the file) because it measures who wrote the code that is actually alive today. Slow on first run for large repos; cached forever per HEAD hash
- **File coupling** (`--add-on coupling`) — counts how often pairs of files appear in the same commit. Score = `co_occurrences / min(commit_count_a, commit_count_b)`. High-scoring pairs are implicit dependencies: change one, you likely need to change the other
- **Bus factor** (`--add-on bus-factor`) — counts the number of distinct authors who have ever touched each file. Files touched by only one or two people are single points of failure if those people leave
- **Churn** (`--add-on churn`) — measures change frequency relative to file size: `commit_count / max(1, current_line_count)`. A 20-line file touched 50 times is a hotter spot than a 2000-line file touched 50 times
- **Caching** — commit stats cached in a per-repo SQLite database by commit hash; subsequent runs are near-instant. The first scan parallelises diff computation across all CPU cores so even large repositories index quickly; use `--max-threads` to cap usage on shared machines. Add-on results are also cached in the same database and only recomputed when the commit history or HEAD changes. Database lives in the platform data dir (`~/Library/Application Support/gild/` on macOS), keyed by remote origin URL so local clones and remote URL inputs share the same cache
- **Memory footprint on large repos** — only numeric commit stats live in memory (~80 bytes per commit). Per-commit file paths are held in a normalized SQLite table and queried on demand by the detail view and file-level add-ons. A 1M-commit repository stays under ~100 MB resident regardless of how many files each commit touched

## How merges are counted

Gild walks all commits reachable from HEAD, deduplicated by hash. Each commit is counted exactly once regardless of how many branches it's reachable through.

| Merge strategy | How it's counted |
|----------------|-----------------|
| **Regular merge** | Original branch commits keep their hashes and are attributed to the original author. The merge commit itself is separate (usually 0 lines unless conflict resolution). |
| **Fast-forward** | No merge commit created. Original commits become part of the branch as-is. |
| **Squash merge** | Original branch commits are **not** in the target history. A single new commit is created — attributed to whoever performed the merge, not the original authors. |

If your team uses squash merges, contribution data will be skewed toward the person merging. This is a git-level limitation — the original commits aren't reachable from HEAD after a squash.

## Export formats

- **JSON** — structured data with all metrics, suitable for dashboards and scripts
- **CSV** — spreadsheet-ready, one row per author
- **HTML** — self-contained Dracula-themed report with styled table

## Architecture

| File | Role |
|------|------|
| `main.rs` | CLI via clap, orchestration |
| `storage.rs` | Resolves per-repo data dir from origin URL or path hash |
| `db.rs` | SQLite connection, schema migrations; tables for `commits` (stats) and `commit_files` (normalized per-commit paths) |
| `git.rs` | Three-phase commit loader: pre-count walk, sequential cache check, parallel diff computation (rayon); file paths emitted as a side channel, not stored on in-memory `Commit` |
| `cache.rs` | SQLite-backed cache; writes stats to `commits` and paths to `commit_files`, loads stats only |
| `identity.rs` | Union-find identity merging from email, mailmap, saved rules |
| `identity_map.rs` | Load/save confirmed merges and rejects (`identities.toml`) |
| `mailmap.rs` | Parse `.mailmap` for standard git identity mapping |
| `questionnaire.rs` | Interactive fuzzy identity matcher |
| `ownership.rs` | Blame-based code ownership; `walk_tree_sizes()` shared by churn/bus-factor; SQLite-backed |
| `coupling.rs` | File co-occurrence matrix; scores pairs by normalized co-commit frequency |
| `bus_factor.rs` | Counts unique authors per file from commit history; cached per HEAD hash |
| `churn.rs` | Churn score = commit_count / current_lines; shares `file_stats` with coupling |
| `app.rs` | State machine: sort, time windows, impact scoring, commit classification, views (Table / Graph / Detail / Files). Owns `Database` for on-demand file-path queries in the detail view |
| `ui.rs` | Ratatui TUI with Dracula colors, table/graph/detail/files views |
| `export.rs` | JSON, CSV, HTML export |
| `fmt.rs` | Number/date formatting helpers |
| `util.rs` | Atomic file writes, safe TOML/text loading |
