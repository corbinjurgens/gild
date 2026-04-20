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
gild --no-questions             # skip identity questionnaire
gild --no-ownership             # skip code ownership analysis
gild --export json              # export as JSON (also: csv, html)
gild --export html -o report.html
```

Run `gild` with no arguments to analyze the current directory.

## TUI keys

| Key | Action |
|-----|--------|
| `c` | Sort by commits |
| `+` / `a` | Sort by lines added |
| `-` / `d` | Sort by lines removed |
| `n` | Sort by net lines |
| `f` | Sort by files changed |
| `i` | Sort by impact |
| `o` | Sort by ownership |
| `t` | Cycle time window (all / year / quarter / month) |
| `[` / `]` | Navigate time period |
| `g` | Toggle table / graph view |
| `Enter` | Detail drill-down (top files + activity heatmap) |
| `j` / `k` / `Up` / `Down` | Scroll |
| `Home` / `End` / `G` | Jump to top / bottom |
| `Esc` | Back (from detail) or quit |
| `q` | Quit |

## How it works

- **Impact scoring** — sessions of commits within 30-minute gaps are grouped; impact uses a logarithmic formula weighing lines and files touched, rewarding focused effort over raw volume
- **Identity deduplication** — union-find merges authors by email, `.mailmap`, and saved confirmations; an interactive questionnaire catches fuzzy matches (Levenshtein, substring, email heuristics)
- **Code ownership** — for each file in HEAD, finds the most recent commit that touched it and counts lines; cached per HEAD hash in `.git/gild/ownership.json`
- **Caching** — commit stats cached in `.git/gild/cache.json` by hash; subsequent runs are near-instant

## Export formats

- **JSON** — structured data with all metrics, suitable for dashboards and scripts
- **CSV** — spreadsheet-ready, one row per author
- **HTML** — self-contained Dracula-themed report with styled table

## Architecture

| File | Role |
|------|------|
| `git.rs` | Walk commits via git2, compute diff stats + file paths |
| `cache.rs` | On-disk commit stats cache (`.git/gild/cache.json`) |
| `identity.rs` | Union-find identity merging from email, mailmap, saved rules |
| `identity_map.rs` | Load/save confirmed merges and rejects (`.git/gild/identities.toml`) |
| `mailmap.rs` | Parse `.mailmap` for standard git identity mapping |
| `questionnaire.rs` | Interactive fuzzy identity matcher |
| `ownership.rs` | Code ownership via last-touch analysis + blob line counts |
| `app.rs` | State machine: sort, time windows, impact scoring, views |
| `ui.rs` | Ratatui TUI with Dracula colors, ranking, sparklines, heatmap |
| `export.rs` | JSON, CSV, HTML export |
| `main.rs` | CLI via clap, orchestration |
