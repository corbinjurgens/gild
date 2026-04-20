use crate::fmt::{MONTHS, SECONDS_PER_DAY, SECONDS_PER_WEEK};
use crate::git::{Commit, RepoInfo};
use crate::identity::IdentityGroup;
use chrono::{DateTime, Datelike, Timelike};
use crossterm::event::{KeyCode, KeyEvent};
use std::cell::{Ref, RefCell};
use std::collections::HashMap;

#[derive(Clone)]
pub struct AuthorStats {
    pub display_name: String,
    pub group_id: usize,
    pub commits: usize,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub files_changed: usize,
    pub first_commit: i64,
    pub last_commit: i64,
    pub impact: f64,
    pub ownership_lines: usize,
    pub ownership_pct: f64,
    pub change_types: ChangeBreakdown,
}

#[derive(Clone, Default)]
pub struct ChangeBreakdown {
    pub feature: usize,
    pub refactor: usize,
    pub trivial: usize,
    pub new_files: usize,
    pub deleted_files: usize,
    pub whitespace_lines: usize,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ViewMode {
    Table,
    Graph,
    Detail,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Theme {
    Normal,
    Readable,
}

pub struct GraphRow {
    pub name: String,
    pub data: Vec<u64>,
    pub color: ratatui::style::Color,
}

pub struct GraphData {
    pub labels: Vec<String>,
    pub rows: Vec<GraphRow>,
}

pub struct FileEvent {
    pub path: String,
    pub timestamp: i64,
    pub lines: usize,
}

pub struct DetailData {
    pub author: AuthorStats,
    pub prev_name: Option<String>,
    pub next_name: Option<String>,
    pub top_files: Vec<(String, usize)>,
    pub activity: [[usize; 24]; 7],
    pub recent_added: Vec<FileEvent>,
    pub recent_deleted: Vec<FileEvent>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SortMode {
    Commits,
    LinesAdded,
    LinesRemoved,
    NetLines,
    FilesChanged,
    Impact,
    Noise,
    Ownership,
}

impl SortMode {
    pub fn label(self) -> &'static str {
        match self {
            SortMode::Commits => "commits",
            SortMode::LinesAdded => "+added",
            SortMode::LinesRemoved => "-removed",
            SortMode::NetLines => "net",
            SortMode::FilesChanged => "files",
            SortMode::Impact => "impact",
            SortMode::Noise => "noise",
            SortMode::Ownership => "own",
        }
    }

    pub fn key_hint(self) -> &'static str {
        match self {
            SortMode::Commits => "c",
            SortMode::LinesAdded => "+",
            SortMode::LinesRemoved => "-",
            SortMode::NetLines => "n",
            SortMode::FilesChanged => "f",
            SortMode::Impact => "i",
            SortMode::Noise => "N",
            SortMode::Ownership => "o",
        }
    }

    pub const ALL: [SortMode; 8] = [
        SortMode::Commits,
        SortMode::LinesAdded,
        SortMode::LinesRemoved,
        SortMode::NetLines,
        SortMode::FilesChanged,
        SortMode::Impact,
        SortMode::Noise,
        SortMode::Ownership,
    ];
}

#[derive(Clone, Copy, PartialEq)]
pub enum TimeMode {
    All,
    Year,
    Quarter,
    Month,
}

impl TimeMode {
    pub fn label(self) -> &'static str {
        match self {
            TimeMode::All => "all",
            TimeMode::Year => "year",
            TimeMode::Quarter => "quarter",
            TimeMode::Month => "month",
        }
    }

    fn next(self) -> Self {
        match self {
            TimeMode::All => TimeMode::Year,
            TimeMode::Year => TimeMode::Quarter,
            TimeMode::Quarter => TimeMode::Month,
            TimeMode::Month => TimeMode::All,
        }
    }
}

pub fn noise_pct(author: &AuthorStats) -> f64 {
    if author.commits == 0 {
        0.0
    } else {
        author.change_types.trivial as f64 / author.commits as f64 * 100.0
    }
}

fn sort_value_for(mode: SortMode, author: &AuthorStats) -> i64 {
    match mode {
        SortMode::Commits => author.commits as i64,
        SortMode::LinesAdded => author.lines_added as i64,
        SortMode::LinesRemoved => author.lines_removed as i64,
        SortMode::NetLines => author.lines_added as i64 - author.lines_removed as i64,
        SortMode::FilesChanged => author.files_changed as i64,
        SortMode::Impact => (author.impact * 100.0) as i64,
        SortMode::Noise => (noise_pct(author) * 100.0) as i64,
        SortMode::Ownership => author.ownership_lines as i64,
    }
}

pub struct App {
    pub authors: Vec<AuthorStats>,
    pub sort_mode: SortMode,
    pub time_mode: TimeMode,
    pub time_offset: i32,
    pub view_mode: ViewMode,
    pub selected: usize,
    pub detail_scroll: usize,
    pub repo_info: RepoInfo,
    pub total_commits: usize,
    pub total_authors: usize,
    pub filtered_commits: usize,
    pub has_ownership: bool,
    pub theme: Theme,
    pub show_theme_picker: bool,
    detail_group_id: Option<usize>,
    sorted_indices: Vec<usize>,
    graph_cache: RefCell<Option<GraphData>>,
    detail_cache: RefCell<Option<(usize, DetailData)>>,
    commits: Vec<Commit>,
    groups: Vec<IdentityGroup>,
    ownership_per_group: Vec<usize>,
    ownership_total: usize,
    earliest: i64,
    latest: i64,
}

impl App {
    pub fn new(
        commits: Vec<Commit>,
        groups: Vec<IdentityGroup>,
        info: RepoInfo,
    ) -> Self {
        let total_commits = commits.len();
        let earliest = commits.iter().map(|c| c.timestamp).min().unwrap_or(0);
        let latest = commits.iter().map(|c| c.timestamp).max().unwrap_or(0);

        let mut app = Self {
            authors: Vec::new(),
            sort_mode: SortMode::Impact,
            time_mode: TimeMode::Year,
            time_offset: 0,
            view_mode: ViewMode::Table,
            selected: 0,
            detail_scroll: 0,
            repo_info: info,
            total_commits,
            total_authors: groups.len(),
            filtered_commits: total_commits,
            has_ownership: false,
            theme: Theme::Normal,
            show_theme_picker: true,
            detail_group_id: None,
            sorted_indices: Vec::new(),
            graph_cache: RefCell::new(None),
            detail_cache: RefCell::new(None),
            commits,
            groups,
            ownership_per_group: Vec::new(),
            ownership_total: 0,
            earliest,
            latest,
        };
        app.recompute();
        app
    }

    pub fn commits(&self) -> &[Commit] {
        &self.commits
    }

    pub fn set_ownership(&mut self, per_group: Vec<usize>, total: usize) {
        self.ownership_per_group = per_group;
        self.ownership_total = total;
        self.has_ownership = total > 0;
        self.recompute();
    }

    fn recompute(&mut self) {
        let (start, end) = self.time_bounds();

        let mut group_data: Vec<Vec<&Commit>> =
            (0..self.groups.len()).map(|_| Vec::new()).collect();

        for entry in &self.commits {
            if entry.timestamp < start || entry.timestamp >= end {
                continue;
            }
            group_data[entry.group_id].push(entry);
        }

        let mut authors = Vec::new();
        let mut total = 0usize;

        for (gid, data) in group_data.iter_mut().enumerate() {
            if data.is_empty() {
                continue;
            }

            data.sort_by_key(|c| c.timestamp);

            let author_stats = aggregate_group(gid, data, &self.groups[gid].display_name);
            total += author_stats.commits;

            let ownership_lines = self.ownership_per_group.get(gid).copied().unwrap_or(0);
            let ownership_pct = if self.ownership_total > 0 {
                ownership_lines as f64 / self.ownership_total as f64 * 100.0
            } else {
                0.0
            };

            authors.push(AuthorStats {
                ownership_lines,
                ownership_pct,
                ..author_stats
            });
        }

        self.authors = authors;
        self.filtered_commits = total;
        self.resort();
    }

    pub fn time_bounds(&self) -> (i64, i64) {
        match self.time_mode {
            TimeMode::All => (0, i64::MAX),
            TimeMode::Year => {
                let ref_year = ts_year(self.latest);
                let year = ref_year + self.time_offset;
                (month_ts(year, 1), month_ts(year + 1, 1))
            }
            TimeMode::Quarter => {
                let dt = ts_to_dt(self.latest);
                let ref_q_start = ((dt.month() - 1) / 3) * 3 + 1;
                let (y, m) = offset_month(dt.year(), ref_q_start, self.time_offset * 3);
                let (ny, nm) = offset_month(y, m, 3);
                (month_ts(y, m), month_ts(ny, nm))
            }
            TimeMode::Month => {
                let dt = ts_to_dt(self.latest);
                let (y, m) = offset_month(dt.year(), dt.month(), self.time_offset);
                let (ny, nm) = offset_month(y, m, 1);
                (month_ts(y, m), month_ts(ny, nm))
            }
        }
    }

    pub fn time_label(&self) -> String {
        match self.time_mode {
            TimeMode::All => "All time".to_string(),
            TimeMode::Year => {
                let year = ts_year(self.latest) + self.time_offset;
                format!("{}", year)
            }
            TimeMode::Quarter => {
                let dt = ts_to_dt(self.latest);
                let ref_q_start = ((dt.month() - 1) / 3) * 3 + 1;
                let (y, m) = offset_month(dt.year(), ref_q_start, self.time_offset * 3);
                let q = (m - 1) / 3 + 1;
                format!("{} Q{}", y, q)
            }
            TimeMode::Month => {
                let dt = ts_to_dt(self.latest);
                let (y, m) = offset_month(dt.year(), dt.month(), self.time_offset);
                format!("{} {}", MONTHS[(m - 1) as usize], y)
            }
        }
    }

    fn has_earlier_data(&self) -> bool {
        let (start, _) = self.time_bounds();
        start > self.earliest
    }

    pub fn sorted_authors(
        &self,
    ) -> impl ExactSizeIterator<Item = &AuthorStats> + DoubleEndedIterator + Clone + '_ {
        self.sorted_indices.iter().map(|&i| &self.authors[i])
    }

    pub fn sorted_author_at(&self, pos: usize) -> Option<&AuthorStats> {
        self.sorted_indices.get(pos).map(|&i| &self.authors[i])
    }

    pub fn sorted_author_count(&self) -> usize {
        self.sorted_indices.len()
    }

    pub fn overall_time_range(&self) -> (i64, i64) {
        (self.earliest, self.latest)
    }



    fn resort(&mut self) {
        self.sorted_indices = (0..self.authors.len()).collect();
        let authors = &self.authors;
        let sort_mode = self.sort_mode;
        self.sorted_indices.sort_by(|&a, &b| {
            let va = sort_value_for(sort_mode, &authors[a]);
            let vb = sort_value_for(sort_mode, &authors[b]);
            vb.cmp(&va)
        });
        self.selected = 0;
        self.invalidate_caches();
    }

    fn invalidate_caches(&mut self) {
        self.graph_cache.get_mut().take();
        self.detail_cache.get_mut().take();
    }

    fn set_sort(&mut self, mode: SortMode) {
        if mode == SortMode::Ownership && !self.has_ownership {
            return;
        }
        if self.sort_mode != mode {
            self.sort_mode = mode;
            self.resort();
        }
    }

    fn set_time_mode(&mut self, mode: TimeMode) {
        self.time_mode = mode;
        self.time_offset = 0;
        self.recompute();
    }

    fn time_navigate(&mut self, delta: i32) {
        if self.time_mode == TimeMode::All {
            return;
        }
        let new_offset = self.time_offset + delta;
        if delta < 0 && !self.has_earlier_data() {
            return;
        }
        if delta > 0 && new_offset > 0 {
            return;
        }
        self.time_offset = new_offset;
        self.recompute();
    }

    fn empty_author(&self, gid: usize) -> AuthorStats {
        let ownership_lines = self.ownership_per_group.get(gid).copied().unwrap_or(0);
        let ownership_pct = if self.ownership_total > 0 {
            ownership_lines as f64 / self.ownership_total as f64 * 100.0
        } else {
            0.0
        };
        AuthorStats {
            display_name: self.groups[gid].display_name.clone(),
            group_id: gid,
            commits: 0,
            lines_added: 0,
            lines_removed: 0,
            files_changed: 0,
            first_commit: 0,
            last_commit: 0,
            impact: 0.0,
            ownership_lines,
            ownership_pct,
            change_types: ChangeBreakdown::default(),
        }
    }

    pub fn detail_data(&self) -> Option<Ref<'_, DetailData>> {
        if self.view_mode != ViewMode::Detail {
            return None;
        }
        let gid = self.detail_group_id?;
        if gid >= self.groups.len() {
            return None;
        }
        let cache_hit = self
            .detail_cache
            .borrow()
            .as_ref()
            .map(|(k, _)| *k == gid)
            .unwrap_or(false);
        if !cache_hit {
            let data = self.compute_detail_data(gid)?;
            *self.detail_cache.borrow_mut() = Some((gid, data));
        }
        Some(Ref::map(self.detail_cache.borrow(), |o| &o.as_ref().unwrap().1))
    }

    fn compute_detail_data(&self, gid: usize) -> Option<DetailData> {
        let author = self
            .authors
            .iter()
            .find(|a| a.group_id == gid)
            .cloned()
            .unwrap_or_else(|| self.empty_author(gid));

        let (prev_name, next_name) = match self.sorted_authors().position(|a| a.group_id == gid) {
            Some(pos) => {
                let prev = pos
                    .checked_sub(1)
                    .and_then(|p| self.sorted_author_at(p))
                    .map(|a| a.display_name.clone());
                let next = self.sorted_author_at(pos + 1).map(|a| a.display_name.clone());
                (prev, next)
            }
            None => (
                self.sorted_author_at(self.sorted_author_count().saturating_sub(1))
                    .map(|a| a.display_name.clone()),
                self.sorted_author_at(0).map(|a| a.display_name.clone()),
            ),
        };

        let (start, end) = self.time_bounds();

        let mut file_counts: HashMap<String, usize> = HashMap::new();
        let mut activity = [[0usize; 24]; 7];

        let mut added_files: Vec<FileEvent> = Vec::new();
        let mut deleted_files: Vec<FileEvent> = Vec::new();

        for entry in &self.commits {
            if entry.group_id != gid {
                continue;
            }
            if entry.timestamp < start || entry.timestamp >= end {
                continue;
            }

            for f in &entry.files {
                *file_counts.entry(f.clone()).or_insert(0) += 1;
            }

            if let Some(dt) = DateTime::from_timestamp(entry.timestamp, 0) {
                let local = dt.with_timezone(&chrono::Local);
                let dow = local.weekday().num_days_from_monday() as usize;
                let hour = local.hour() as usize;
                activity[dow][hour] += 1;
            }

            let lines_per_file = if entry.files_changed > 0 {
                (entry.lines_added + entry.lines_removed) / entry.files_changed.max(1)
            } else {
                0
            };
            for f in &entry.added_file_names {
                added_files.push(FileEvent {
                    path: f.clone(),
                    timestamp: entry.timestamp,
                    lines: lines_per_file,
                });
            }
            for f in &entry.deleted_file_names {
                deleted_files.push(FileEvent {
                    path: f.clone(),
                    timestamp: entry.timestamp,
                    lines: lines_per_file,
                });
            }
        }

        let mut top_files: Vec<(String, usize)> = file_counts.into_iter().collect();
        top_files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        top_files.truncate(20);

        added_files.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then(b.lines.cmp(&a.lines)));
        deleted_files.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then(b.lines.cmp(&a.lines)));
        added_files.truncate(10);
        deleted_files.truncate(10);

        Some(DetailData {
            author,
            prev_name,
            next_name,
            top_files,
            activity,
            recent_added: added_files,
            recent_deleted: deleted_files,
        })
    }

    fn detail_navigate(&mut self, delta: i32) {
        let gid = match self.detail_group_id {
            Some(g) => g,
            None => return,
        };
        let count = self.sorted_author_count();
        if count == 0 {
            return;
        }
        let new_gid = match self.sorted_authors().position(|a| a.group_id == gid) {
            Some(pos) => {
                let new_pos = if delta < 0 {
                    pos.saturating_sub(1)
                } else {
                    (pos + 1).min(count - 1)
                };
                self.sorted_author_at(new_pos).unwrap().group_id
            }
            None => {
                let fallback = if delta < 0 { count - 1 } else { 0 };
                self.sorted_author_at(fallback).unwrap().group_id
            }
        };
        if Some(new_gid) != self.detail_group_id {
            self.detail_group_id = Some(new_gid);
            self.detail_scroll = 0;
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.show_theme_picker {
            match key.code {
                KeyCode::Char('n') | KeyCode::Char('N') => self.theme = Theme::Normal,
                KeyCode::Char('r') | KeyCode::Char('R') => self.theme = Theme::Readable,
                KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                    self.theme = match self.theme {
                        Theme::Normal => Theme::Readable,
                        Theme::Readable => Theme::Normal,
                    };
                }
                KeyCode::Enter | KeyCode::Char(' ') => self.show_theme_picker = false,
                KeyCode::Esc | KeyCode::Char('q') => return true,
                _ => {}
            }
            return false;
        }

        if self.view_mode == ViewMode::Detail {
            return self.handle_detail_key(key);
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('c') => self.set_sort(SortMode::Commits),
            KeyCode::Char('+') | KeyCode::Char('a') => self.set_sort(SortMode::LinesAdded),
            KeyCode::Char('-') | KeyCode::Char('d') => self.set_sort(SortMode::LinesRemoved),
            KeyCode::Char('n') => self.set_sort(SortMode::NetLines),
            KeyCode::Char('f') => self.set_sort(SortMode::FilesChanged),
            KeyCode::Char('i') => self.set_sort(SortMode::Impact),
            KeyCode::Char('N') => self.set_sort(SortMode::Noise),
            KeyCode::Char('o') => self.set_sort(SortMode::Ownership),
            KeyCode::Char('T') => self.show_theme_picker = true,
            KeyCode::Char('g') => {
                self.view_mode = match self.view_mode {
                    ViewMode::Table => ViewMode::Graph,
                    ViewMode::Graph => ViewMode::Table,
                    ViewMode::Detail => ViewMode::Table,
                };
            }
            KeyCode::Char('t') => self.set_time_mode(self.time_mode.next()),
            KeyCode::Left | KeyCode::Char('[') => self.time_navigate(-1),
            KeyCode::Right | KeyCode::Char(']') => self.time_navigate(1),
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected + 1 < self.authors.len() {
                    self.selected += 1;
                }
            }
            KeyCode::Home => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.authors.len().saturating_sub(1);
            }
            KeyCode::Enter => {
                if let Some(author) = self.sorted_author_at(self.selected) {
                    self.detail_group_id = Some(author.group_id);
                    self.view_mode = ViewMode::Detail;
                    self.detail_scroll = 0;
                }
            }
            _ => {}
        }
        false
    }

    fn handle_detail_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => {
                if let Some(gid) = self.detail_group_id {
                    let pos = self.sorted_authors().position(|a| a.group_id == gid);
                    if let Some(pos) = pos {
                        self.selected = pos;
                    }
                }
                self.detail_group_id = None;
                self.view_mode = ViewMode::Table;
            }
            KeyCode::Char('q') => return true,
            KeyCode::Char('t') => self.set_time_mode(self.time_mode.next()),
            KeyCode::Char('T') => self.show_theme_picker = true,
            KeyCode::Left | KeyCode::Char('[') => self.time_navigate(-1),
            KeyCode::Right | KeyCode::Char(']') => self.time_navigate(1),
            KeyCode::Up | KeyCode::Char('k') => self.detail_navigate(-1),
            KeyCode::Down | KeyCode::Char('j') => self.detail_navigate(1),
            KeyCode::PageUp => {
                self.detail_scroll = self.detail_scroll.saturating_sub(5);
            }
            KeyCode::PageDown => {
                self.detail_scroll = (self.detail_scroll + 5).min(DETAIL_SCROLL_MAX);
            }
            KeyCode::Home => self.detail_scroll = 0,
            _ => {}
        }
        false
    }

    pub fn graph_data(&self) -> Ref<'_, GraphData> {
        if self.graph_cache.borrow().is_none() {
            let data = self.compute_graph_data();
            *self.graph_cache.borrow_mut() = Some(data);
        }
        Ref::map(self.graph_cache.borrow(), |o| o.as_ref().unwrap())
    }

    fn compute_graph_data(&self) -> GraphData {
        use ratatui::style::Color;

        const PALETTE: [Color; 8] = [
            Color::Rgb(80, 250, 123),
            Color::Rgb(139, 233, 253),
            Color::Rgb(255, 184, 108),
            Color::Rgb(189, 147, 249),
            Color::Rgb(255, 85, 85),
            Color::Rgb(255, 121, 198),
            Color::Rgb(241, 250, 140),
            Color::Rgb(255, 255, 255),
        ];

        let (raw_start, raw_end) = self.time_bounds();
        let start = if raw_start == 0 { self.earliest } else { raw_start };
        let end = if raw_end == i64::MAX {
            self.latest + SECONDS_PER_DAY
        } else {
            raw_end
        };

        let (boundaries, labels) = period_boundaries(start, end);
        let num_periods = labels.len();

        let total_authors = self.sorted_author_count();
        let top_n = GRAPH_TOP_N.min(total_authors);
        let top_ids: Vec<usize> = self
            .sorted_authors()
            .take(top_n)
            .map(|a| a.group_id)
            .collect();

        let num_slots = top_n + 1;
        let mut per_author: Vec<Vec<(i64, usize, usize)>> = vec![Vec::new(); num_slots];

        for entry in &self.commits {
            if entry.timestamp < start || entry.timestamp >= end {
                continue;
            }
            let author_idx = top_ids
                .iter()
                .position(|&id| id == entry.group_id)
                .unwrap_or(top_n);
            per_author[author_idx].push((
                entry.timestamp,
                entry.lines_added + entry.lines_removed,
                entry.files_changed,
            ));
        }

        let mut buckets: Vec<Vec<u64>> = vec![vec![0; num_periods]; num_slots];

        for (ai, commits) in per_author.iter_mut().enumerate() {
            if commits.is_empty() {
                continue;
            }
            commits.sort_by_key(|&(ts, _, _)| ts);

            let mut sess_lines = 0usize;
            let mut sess_files = 0usize;
            let mut sess_last_ts = commits[0].0;

            for &(ts, lines, files) in commits.iter() {
                if ts - sess_last_ts > SESSION_GAP_SECS && sess_lines + sess_files > 0 {
                    let val = session_value(sess_lines, sess_files);
                    let bucket = boundaries
                        .partition_point(|&b| b <= sess_last_ts)
                        .saturating_sub(1)
                        .min(num_periods - 1);
                    buckets[ai][bucket] += (val * 10.0) as u64;
                    sess_lines = 0;
                    sess_files = 0;
                }
                sess_lines += lines;
                sess_files += files;
                sess_last_ts = ts;
            }
            if sess_lines + sess_files > 0 {
                let val = session_value(sess_lines, sess_files);
                let bucket = boundaries
                    .partition_point(|&b| b <= sess_last_ts)
                    .saturating_sub(1)
                    .min(num_periods - 1);
                buckets[ai][bucket] += (val * 10.0) as u64;
            }
        }

        let mut rows: Vec<GraphRow> = self
            .sorted_authors()
            .take(top_n)
            .enumerate()
            .map(|(i, a)| GraphRow {
                name: a.display_name.clone(),
                data: std::mem::take(&mut buckets[i]),
                color: PALETTE[i % PALETTE.len()],
            })
            .collect();

        if total_authors > top_n {
            rows.push(GraphRow {
                name: "others".to_string(),
                data: std::mem::take(&mut buckets[top_n]),
                color: Color::Rgb(108, 118, 148),
            });
        }

        GraphData { labels, rows }
    }
}

fn ts_to_dt(ts: i64) -> DateTime<chrono::Utc> {
    DateTime::from_timestamp(ts, 0).unwrap_or_default()
}

fn ts_year(ts: i64) -> i32 {
    ts_to_dt(ts).year()
}

fn offset_month(year: i32, month: u32, offset: i32) -> (i32, u32) {
    let total = year * 12 + (month as i32 - 1) + offset;
    let y = total.div_euclid(12);
    let m = total.rem_euclid(12) as u32 + 1;
    (y, m)
}

fn month_ts(year: i32, month: u32) -> i64 {
    chrono::NaiveDate::from_ymd_opt(year, month, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp()
}

const QUARTERLY_THRESHOLD_DAYS: i64 = 730;
const MONTHLY_THRESHOLD_DAYS: i64 = 120;

fn period_boundaries(start: i64, end: i64) -> (Vec<i64>, Vec<String>) {
    let span_days = (end - start) / SECONDS_PER_DAY;
    if span_days > QUARTERLY_THRESHOLD_DAYS {
        collect_boundaries(quarterly_iter(start), end)
    } else if span_days > MONTHLY_THRESHOLD_DAYS {
        collect_boundaries(monthly_iter(start), end)
    } else {
        collect_boundaries(weekly_iter(start), end)
    }
}

fn collect_boundaries(
    iter: impl Iterator<Item = (i64, String)>,
    end: i64,
) -> (Vec<i64>, Vec<String>) {
    let mut boundaries = Vec::new();
    let mut labels = Vec::new();
    for (ts, label) in iter {
        if ts >= end {
            break;
        }
        boundaries.push(ts);
        labels.push(label);
    }
    boundaries.push(end);
    (boundaries, labels)
}

fn quarterly_iter(start: i64) -> impl Iterator<Item = (i64, String)> {
    let dt = ts_to_dt(start);
    let q_start = ((dt.month() - 1) / 3) * 3 + 1;
    let (mut y, mut m) = (dt.year(), q_start);
    std::iter::from_fn(move || {
        let ts = month_ts(y, m);
        let q = (m - 1) / 3 + 1;
        let label = format!("{}Q{}", y, q);
        (y, m) = offset_month(y, m, 3);
        Some((ts, label))
    })
}

fn monthly_iter(start: i64) -> impl Iterator<Item = (i64, String)> {
    let dt = ts_to_dt(start);
    let (mut y, mut m) = (dt.year(), dt.month());
    std::iter::from_fn(move || {
        let ts = month_ts(y, m);
        let label = if m == 1 {
            y.to_string()
        } else {
            MONTHS[(m - 1) as usize].to_string()
        };
        (y, m) = offset_month(y, m, 1);
        Some((ts, label))
    })
}

fn weekly_iter(start: i64) -> impl Iterator<Item = (i64, String)> {
    let mut ts = (start / SECONDS_PER_WEEK) * SECONDS_PER_WEEK;
    std::iter::from_fn(move || {
        let current = ts;
        let dt = ts_to_dt(current);
        let label = format!("{:02}/{:02}", dt.month(), dt.day());
        ts += SECONDS_PER_WEEK;
        Some((current, label))
    })
}

const SESSION_GAP_SECS: i64 = 30 * 60;
const GRAPH_TOP_N: usize = 8;
const DETAIL_SCROLL_MAX: usize = 200;

enum ChangeKind {
    Feature,
    Refactor,
    Trivial,
}

fn classify_commit(c: &Commit) -> ChangeKind {
    let total_lines = c.lines_added + c.lines_removed;
    let ws_total = c.whitespace_added + c.whitespace_removed;
    let is_whitespace_only = total_lines > 0 && ws_total >= total_lines;
    let is_trivial = total_lines <= 5 && c.files.len() <= 1;

    if is_whitespace_only || is_trivial {
        ChangeKind::Trivial
    } else if c.files_added > 0 && c.files_deleted == 0 {
        ChangeKind::Feature
    } else if c.files_deleted > 0 && c.files_added == 0 {
        ChangeKind::Refactor
    } else {
        let add_ratio = c.lines_added as f64 / (c.lines_removed.max(1)) as f64;
        if add_ratio > 2.0 || c.files_added > 0 {
            ChangeKind::Feature
        } else {
            ChangeKind::Refactor
        }
    }
}

fn aggregate_group(gid: usize, sorted: &[&Commit], display_name: &str) -> AuthorStats {
    let mut change_types = ChangeBreakdown::default();
    let mut lines_added = 0;
    let mut lines_removed = 0;
    let mut files_changed = 0;

    for c in sorted {
        lines_added += c.lines_added;
        lines_removed += c.lines_removed;
        files_changed += c.files.len();
        change_types.whitespace_lines += c.whitespace_added + c.whitespace_removed;
        change_types.new_files += c.files_added;
        change_types.deleted_files += c.files_deleted;
        match classify_commit(c) {
            ChangeKind::Feature => change_types.feature += 1,
            ChangeKind::Refactor => change_types.refactor += 1,
            ChangeKind::Trivial => change_types.trivial += 1,
        }
    }

    AuthorStats {
        display_name: display_name.to_string(),
        group_id: gid,
        commits: sorted.len(),
        lines_added,
        lines_removed,
        files_changed,
        first_commit: sorted.first().unwrap().timestamp,
        last_commit: sorted.last().unwrap().timestamp,
        impact: compute_impact(sorted),
        ownership_lines: 0,
        ownership_pct: 0.0,
        change_types,
    }
}

fn compute_impact(sorted_commits: &[&Commit]) -> f64 {
    if sorted_commits.is_empty() {
        return 0.0;
    }

    let mut total_lines = 0usize;
    let mut total_ws = 0usize;
    let mut unique_files = std::collections::HashSet::new();
    let mut sessions = 1usize;
    let mut last_ts = sorted_commits[0].timestamp;

    for c in sorted_commits {
        total_lines += c.lines_added + c.lines_removed;
        total_ws += c.whitespace_added + c.whitespace_removed;
        for f in &c.files {
            unique_files.insert(f.as_str());
        }
        if c.timestamp - last_ts > SESSION_GAP_SECS {
            sessions += 1;
        }
        last_ts = c.timestamp;
    }

    let substantive_lines = total_lines.saturating_sub(total_ws);
    let ws_contribution = total_ws as f64 * 0.1;
    let effective_lines = substantive_lines as f64 + ws_contribution;

    let substance = session_value(effective_lines as usize, unique_files.len());
    substance * (1.0 + 0.15 * (sessions as f64).ln())
}

fn session_value(lines: usize, files: usize) -> f64 {
    (1.0 + (1.0 + lines as f64).ln()) * (1.0 + 0.5 * (1.0 + files as f64).ln())
}
