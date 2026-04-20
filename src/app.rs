use crate::git::RepoInfo;
use crate::identity::IdentityGroup;
use chrono::{DateTime, Datelike, Timelike};
use crossterm::event::{KeyCode, KeyEvent};
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
}

#[derive(Clone, Copy, PartialEq)]
pub enum ViewMode {
    Table,
    Graph,
    Detail,
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

pub struct DetailData {
    pub author: AuthorStats,
    pub top_files: Vec<(String, usize)>,
    pub activity: [[usize; 24]; 7],
}

#[derive(Clone, Copy, PartialEq)]
pub enum SortMode {
    Commits,
    LinesAdded,
    LinesRemoved,
    NetLines,
    FilesChanged,
    Impact,
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
            SortMode::Ownership => "o",
        }
    }

    pub const ALL: [SortMode; 7] = [
        SortMode::Commits,
        SortMode::LinesAdded,
        SortMode::LinesRemoved,
        SortMode::NetLines,
        SortMode::FilesChanged,
        SortMode::Impact,
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

fn sort_value_for(mode: SortMode, author: &AuthorStats) -> i64 {
    match mode {
        SortMode::Commits => author.commits as i64,
        SortMode::LinesAdded => author.lines_added as i64,
        SortMode::LinesRemoved => author.lines_removed as i64,
        SortMode::NetLines => author.lines_added as i64 - author.lines_removed as i64,
        SortMode::FilesChanged => author.files_changed as i64,
        SortMode::Impact => (author.impact * 100.0) as i64,
        SortMode::Ownership => author.ownership_lines as i64,
    }
}

pub struct CommitEntry {
    pub group_id: usize,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub files_changed: usize,
    pub timestamp: i64,
    pub files: Vec<String>,
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
    sorted_indices: Vec<usize>,
    commits: Vec<CommitEntry>,
    groups: Vec<IdentityGroup>,
    ownership_per_group: Vec<usize>,
    ownership_total: usize,
    earliest: i64,
    latest: i64,
}

impl App {
    pub fn new(
        commits: Vec<CommitEntry>,
        groups: Vec<IdentityGroup>,
        info: RepoInfo,
    ) -> Self {
        let total_commits = commits.len();
        let earliest = commits.iter().map(|c| c.timestamp).min().unwrap_or(0);
        let latest = commits.iter().map(|c| c.timestamp).max().unwrap_or(0);

        let mut app = Self {
            authors: Vec::new(),
            sort_mode: SortMode::Impact,
            time_mode: TimeMode::All,
            time_offset: 0,
            view_mode: ViewMode::Table,
            selected: 0,
            detail_scroll: 0,
            repo_info: info,
            total_commits,
            total_authors: groups.len(),
            filtered_commits: total_commits,
            has_ownership: false,
            sorted_indices: Vec::new(),
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

    pub fn commit_entries(&self) -> &[CommitEntry] {
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

        let mut group_data: Vec<Vec<(i64, usize, usize, usize)>> =
            vec![Vec::new(); self.groups.len()];

        for entry in &self.commits {
            if entry.timestamp < start || entry.timestamp >= end {
                continue;
            }
            group_data[entry.group_id].push((
                entry.timestamp,
                entry.lines_added,
                entry.lines_removed,
                entry.files_changed,
            ));
        }

        let mut authors = Vec::new();
        let mut total = 0usize;

        for (gid, data) in group_data.iter_mut().enumerate() {
            if data.is_empty() {
                continue;
            }

            data.sort_by_key(|&(ts, _, _, _)| ts);

            let mut commits = 0;
            let mut lines_added = 0;
            let mut lines_removed = 0;
            let mut files_changed = 0;

            for &(_, la, lr, fc) in data.iter() {
                commits += 1;
                lines_added += la;
                lines_removed += lr;
                files_changed += fc;
            }
            total += commits;

            let first_commit = data.first().unwrap().0;
            let last_commit = data.last().unwrap().0;

            let impact = compute_impact(data);

            let ownership_lines = if gid < self.ownership_per_group.len() {
                self.ownership_per_group[gid]
            } else {
                0
            };
            let ownership_pct = if self.ownership_total > 0 {
                ownership_lines as f64 / self.ownership_total as f64 * 100.0
            } else {
                0.0
            };

            authors.push(AuthorStats {
                display_name: self.groups[gid].display_name.clone(),
                group_id: gid,
                commits,
                lines_added,
                lines_removed,
                files_changed,
                first_commit,
                last_commit,
                impact,
                ownership_lines,
                ownership_pct,
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
                const MONTHS: [&str; 12] = [
                    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
                    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
                ];
                format!("{} {}", MONTHS[(m - 1) as usize], y)
            }
        }
    }

    fn has_earlier_data(&self) -> bool {
        let (start, _) = self.time_bounds();
        start > self.earliest
    }

    pub fn sorted_authors(&self) -> Vec<&AuthorStats> {
        self.sorted_indices
            .iter()
            .map(|&i| &self.authors[i])
            .collect()
    }

    pub fn sort_value(&self, author: &AuthorStats) -> i64 {
        sort_value_for(self.sort_mode, author)
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
    }

    fn set_sort(&mut self, mode: SortMode) {
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

    pub fn detail_data(&self) -> Option<DetailData> {
        if self.view_mode != ViewMode::Detail {
            return None;
        }
        let sorted = self.sorted_authors();
        let author = (*sorted.get(self.selected)?).clone();
        let gid = author.group_id;
        let (start, end) = self.time_bounds();

        let mut file_counts: HashMap<String, usize> = HashMap::new();
        let mut activity = [[0usize; 24]; 7];

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
        }

        let mut top_files: Vec<(String, usize)> = file_counts.into_iter().collect();
        top_files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        top_files.truncate(20);

        Some(DetailData {
            author,
            top_files,
            activity,
        })
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
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
            KeyCode::Char('o') => self.set_sort(SortMode::Ownership),
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
                if !self.authors.is_empty() {
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
                self.view_mode = ViewMode::Table;
            }
            KeyCode::Char('q') => return true,
            KeyCode::Up | KeyCode::Char('k') => {
                self.detail_scroll = self.detail_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.detail_scroll += 1;
            }
            KeyCode::Home => self.detail_scroll = 0,
            _ => {}
        }
        false
    }

    pub fn graph_data(&self) -> GraphData {
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
            self.latest + 86400
        } else {
            raw_end
        };

        let (boundaries, labels) = period_boundaries(start, end);
        let num_periods = labels.len();

        let sorted = self.sorted_authors();
        let top_n = 8.min(sorted.len());
        let top_ids: Vec<usize> = sorted.iter().take(top_n).map(|a| a.group_id).collect();

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

        let mut rows: Vec<GraphRow> = (0..top_n)
            .map(|i| GraphRow {
                name: sorted[i].display_name.clone(),
                data: buckets[i].clone(),
                color: PALETTE[i % PALETTE.len()],
            })
            .collect();

        if sorted.len() > top_n {
            rows.push(GraphRow {
                name: "others".to_string(),
                data: buckets[top_n].clone(),
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

fn period_boundaries(start: i64, end: i64) -> (Vec<i64>, Vec<String>) {
    let span_days = (end - start) / 86400;
    if span_days > 730 {
        gen_quarterly_boundaries(start, end)
    } else if span_days > 120 {
        gen_monthly_boundaries(start, end)
    } else {
        gen_weekly_boundaries(start, end)
    }
}

fn gen_quarterly_boundaries(start: i64, end: i64) -> (Vec<i64>, Vec<String>) {
    let dt = ts_to_dt(start);
    let mut boundaries = Vec::new();
    let mut labels = Vec::new();
    let q_start = ((dt.month() - 1) / 3) * 3 + 1;
    let (mut y, mut m) = (dt.year(), q_start);
    loop {
        let ts = month_ts(y, m);
        if ts >= end {
            break;
        }
        boundaries.push(ts);
        let q = (m - 1) / 3 + 1;
        labels.push(format!("{}Q{}", y, q));
        let next = offset_month(y, m, 3);
        y = next.0;
        m = next.1;
    }
    boundaries.push(end);
    (boundaries, labels)
}

fn gen_monthly_boundaries(start: i64, end: i64) -> (Vec<i64>, Vec<String>) {
    let dt = ts_to_dt(start);
    let mut boundaries = Vec::new();
    let mut labels = Vec::new();
    let (mut y, mut m) = (dt.year(), dt.month());
    loop {
        let ts = month_ts(y, m);
        if ts >= end {
            break;
        }
        boundaries.push(ts);
        const MO: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun",
            "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        labels.push(if m == 1 {
            format!("{}", y)
        } else {
            MO[(m - 1) as usize].to_string()
        });
        let next = offset_month(y, m, 1);
        y = next.0;
        m = next.1;
    }
    boundaries.push(end);
    (boundaries, labels)
}

fn gen_weekly_boundaries(start: i64, end: i64) -> (Vec<i64>, Vec<String>) {
    let mut boundaries = Vec::new();
    let mut labels = Vec::new();
    let aligned = (start / 604800) * 604800;
    let mut ts = aligned;
    loop {
        if ts >= end {
            break;
        }
        boundaries.push(ts);
        let dt = ts_to_dt(ts);
        labels.push(format!("{:02}/{:02}", dt.month(), dt.day()));
        ts += 604800;
    }
    boundaries.push(end);
    (boundaries, labels)
}

const SESSION_GAP_SECS: i64 = 30 * 60;

fn compute_impact(sorted_commits: &[(i64, usize, usize, usize)]) -> f64 {
    if sorted_commits.is_empty() {
        return 0.0;
    }

    let mut total_impact = 0.0;
    let mut sess_lines = 0usize;
    let mut sess_files = 0usize;
    let mut last_ts = sorted_commits[0].0;
    let mut in_session = false;

    for &(ts, la, lr, fc) in sorted_commits {
        if in_session && ts - last_ts > SESSION_GAP_SECS {
            total_impact += session_value(sess_lines, sess_files);
            sess_lines = 0;
            sess_files = 0;
        }
        sess_lines += la + lr;
        sess_files += fc;
        last_ts = ts;
        in_session = true;
    }

    if in_session {
        total_impact += session_value(sess_lines, sess_files);
    }

    total_impact
}

fn session_value(lines: usize, files: usize) -> f64 {
    (1.0 + (1.0 + lines as f64).ln()) * (1.0 + 0.5 * (1.0 + files as f64).ln())
}
