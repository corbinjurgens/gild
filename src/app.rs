use crate::db::{Database, FILE_KIND_ADDED, FILE_KIND_DELETED, FILE_KIND_TOUCHED};
use crate::fmt::{MONTHS, SECONDS_PER_DAY, SECONDS_PER_WEEK};
use crate::git::{Commit, RepoInfo};
use crate::identity::IdentityGroup;
use crate::identity_map::IdentityMap;
use crate::mailmap::MailmapEntry;
use chrono::{DateTime, Datelike, Timelike};
use crossterm::event::{KeyCode, KeyEvent};
use rusqlite::params_from_iter;
use std::cell::{Ref, RefCell};
use std::path::PathBuf;

const SESSION_GAP_SECS: i64 = 30 * 60;
const DETAIL_SCROLL_MAX: usize = 200;
const SURROUND_PERIODS: i32 = 4;
const QUARTERLY_THRESHOLD_DAYS: i64 = 730;
const MONTHLY_THRESHOLD_DAYS: i64 = 120;

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
    pub rename: usize,
    pub trivial: usize,
    pub new_files: usize,
    pub deleted_files: usize,
    pub renamed_files: usize,
    pub whitespace_lines: usize,
    pub merge: usize,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ViewMode {
    Table,
    Graph,
    Detail,
    Files,
    FileDetail,
}

#[derive(Clone, Copy, PartialEq)]
pub enum FileSortMode {
    Commits,
    Authors,
    Churn,
    Coupling,
}

pub struct FileRow {
    pub path: String,
    pub commit_count: u32,
    pub unique_authors: Option<u32>,
    pub churn_score: Option<f64>,
    pub top_coupled: Option<(String, f64)>,
}

pub struct FileDetailData {
    pub path: String,
    pub commit_count: u32,
    pub unique_authors: Option<u32>,
    pub churn_score: Option<f64>,
    pub coupled_files: Vec<(String, f64)>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Theme {
    Normal,
    Readable,
}

#[derive(Clone)]
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
}

type GroupFiles = (Vec<(String, usize)>, Vec<FileEvent>, Vec<FileEvent>);

pub struct TrendPoint {
    pub label: String,
    pub value: u64,
    pub is_current: bool,
}

pub struct DetailData {
    pub author: AuthorStats,
    pub aliases: Vec<(String, String)>,
    pub prev_name: Option<String>,
    pub next_name: Option<String>,
    pub top_files: Vec<(String, usize)>,
    pub activity: [[usize; 24]; 7],
    pub recent_added: Vec<FileEvent>,
    pub recent_deleted: Vec<FileEvent>,
    pub trend: Vec<TrendPoint>,
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

#[derive(Clone, Copy, PartialEq)]
pub enum OwnershipPresence {
    Absent,
    Stale,
    Current,
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

pub struct QuestionnaireCandidate {
    pub group_a: usize,
    pub group_b: usize,
}

pub struct QuestionnaireState {
    pub candidates: Vec<QuestionnaireCandidate>,
    pub current: usize,
    pub changed: bool,
    pub last_action: Option<&'static str>,
}

pub struct AppData {
    pub commits: Vec<Commit>,
    pub groups: Vec<IdentityGroup>,
    pub repo_info: RepoInfo,
    pub total_commits: usize,
    pub total_authors: usize,
    pub identity_map: IdentityMap,
    pub identity_map_path: PathBuf,
    pub mailmap_entries: Vec<MailmapEntry>,
    pub ownership_per_group: Vec<usize>,
    pub ownership_total: usize,
    pub earliest: i64,
    pub latest: i64,
    pub db: Database,
    pub file_rows: Vec<FileRow>,
    pub ownership: OwnershipPresence,
    pub has_commit_types: bool,
    pub has_files_view: bool,
    pub has_file_authors: bool,
    pub has_file_churn: bool,
    pub has_file_coupling: bool,
}

pub struct AppView {
    pub view_mode: ViewMode,
    pub selected: usize,
    pub detail_scroll: usize,
    pub sort_mode: SortMode,
    pub time_mode: TimeMode,
    pub time_offset: i32,
    pub theme: Theme,
    pub show_theme_picker: bool,
    pub file_sort: FileSortMode,
    pub file_selected: usize,
    pub file_detail_scroll: usize,
    pub detail_group_id: Option<usize>,
    pub detail_position: Option<usize>,
    pub questionnaire: Option<QuestionnaireState>,
}

pub struct AppCache {
    pub authors: Vec<AuthorStats>,
    pub filtered_commits: usize,
    pub sorted_indices: Vec<usize>,
    pub graph_cache: RefCell<Option<GraphData>>,
    pub detail_cache: RefCell<Option<(usize, DetailData)>>,
    pub file_detail: Option<FileDetailData>,
}

pub struct App {
    pub data: AppData,
    pub view: AppView,
    pub cache: AppCache,
}

impl App {
    pub fn new(
        commits: Vec<Commit>,
        groups: Vec<IdentityGroup>,
        info: RepoInfo,
        identity_map: IdentityMap,
        identity_map_path: PathBuf,
        mailmap_entries: Vec<MailmapEntry>,
        db: Database,
    ) -> Self {
        let total_commits = commits.len();
        let total_authors = groups.len();
        let (earliest, latest) = commits.iter().fold((i64::MAX, i64::MIN), |(lo, hi), c| {
            (lo.min(c.timestamp), hi.max(c.timestamp))
        });
        let earliest = if total_commits == 0 { 0 } else { earliest };
        let latest = if total_commits == 0 { 0 } else { latest };

        let mut app = Self {
            data: AppData {
                commits,
                groups,
                repo_info: info,
                total_commits,
                total_authors,
                identity_map,
                identity_map_path,
                mailmap_entries,
                ownership_per_group: Vec::new(),
                ownership_total: 0,
                earliest,
                latest,
                db,
                file_rows: Vec::new(),
                ownership: OwnershipPresence::Absent,
                has_commit_types: false,
                has_files_view: false,
                has_file_authors: false,
                has_file_churn: false,
                has_file_coupling: false,
            },
            view: AppView {
                view_mode: ViewMode::Table,
                selected: 0,
                detail_scroll: 0,
                sort_mode: SortMode::Impact,
                time_mode: TimeMode::Year,
                time_offset: 0,
                theme: Theme::Normal,
                show_theme_picker: false,
                file_sort: FileSortMode::Commits,
                file_selected: 0,
                file_detail_scroll: 0,
                detail_group_id: None,
                detail_position: None,
                questionnaire: None,
            },
            cache: AppCache {
                authors: Vec::new(),
                filtered_commits: total_commits,
                sorted_indices: Vec::new(),
                graph_cache: RefCell::new(None),
                detail_cache: RefCell::new(None),
                file_detail: None,
            },
        };
        app.recompute();
        app
    }

    pub fn groups(&self) -> &[IdentityGroup] {
        &self.data.groups
    }

    pub fn set_ownership(&mut self, per_group: Vec<usize>, total: usize) {
        self.data.ownership_per_group = per_group;
        self.data.ownership_total = total;
        self.data.ownership = if total > 0 {
            OwnershipPresence::Current
        } else {
            OwnershipPresence::Absent
        };
        self.recompute();
    }

    pub fn set_file_data(&mut self, rows: Vec<FileRow>) {
        self.data.has_file_authors = rows.iter().any(|r| r.unique_authors.is_some());
        self.data.has_file_churn = rows.iter().any(|r| r.churn_score.is_some());
        self.data.has_file_coupling = rows.iter().any(|r| r.top_coupled.is_some());
        self.data.file_rows = rows;
        self.view.file_sort = FileSortMode::Commits;
        self.view.file_selected = 0;
        self.data.has_files_view = !self.data.file_rows.is_empty();
        self.sort_file_rows();
    }

    fn sort_file_rows(&mut self) {
        match self.view.file_sort {
            FileSortMode::Commits => {
                self.data
                    .file_rows
                    .sort_by_key(|r| std::cmp::Reverse(r.commit_count));
            }
            FileSortMode::Authors => {
                self.data.file_rows.sort_by(|a, b| {
                    b.unique_authors
                        .unwrap_or(0)
                        .cmp(&a.unique_authors.unwrap_or(0))
                });
            }
            FileSortMode::Churn => {
                self.data.file_rows.sort_by(|a, b| {
                    b.churn_score
                        .unwrap_or(0.0)
                        .partial_cmp(&a.churn_score.unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            FileSortMode::Coupling => {
                self.data.file_rows.sort_by(|a, b| {
                    let sa = a.top_coupled.as_ref().map(|(_, s)| *s).unwrap_or(0.0);
                    let sb = b.top_coupled.as_ref().map(|(_, s)| *s).unwrap_or(0.0);
                    sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
                });
            }
        }
    }

    pub fn set_file_sort(&mut self, mode: FileSortMode) {
        if self.view.file_sort != mode {
            self.view.file_sort = mode;
            self.view.file_selected = 0;
            self.sort_file_rows();
        }
    }

    pub fn open_file_detail(&mut self) {
        let row = match self.data.file_rows.get(self.view.file_selected) {
            Some(r) => r,
            None => return,
        };

        let path = row.path.clone();
        let mut coupled_files = Vec::new();

        if let Ok(mut stmt) = self.data.db.prepare(
            "SELECT file_b, score FROM file_coupling WHERE file_a = ?1
             UNION ALL
             SELECT file_a, score FROM file_coupling WHERE file_b = ?1
             ORDER BY score DESC",
        ) {
            if let Ok(rows) = stmt.query_map([&path], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
            }) {
                for r in rows.flatten() {
                    coupled_files.push(r);
                }
            }
        }

        self.cache.file_detail = Some(FileDetailData {
            path,
            commit_count: row.commit_count,
            unique_authors: row.unique_authors,
            churn_score: row.churn_score,
            coupled_files,
        });
        self.view.file_detail_scroll = 0;
        self.view.view_mode = ViewMode::FileDetail;
    }

    fn recompute(&mut self) {
        let (start, end) = self.time_bounds();

        let mut group_data: Vec<Vec<&Commit>> =
            (0..self.data.groups.len()).map(|_| Vec::new()).collect();

        for entry in &self.data.commits {
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

            let author_stats = aggregate_group(gid, data, &self.data.groups[gid].display_name);
            total += author_stats.commits;

            let (ownership_lines, ownership_pct) = self.ownership_for_gid(gid);
            authors.push(AuthorStats {
                ownership_lines,
                ownership_pct,
                ..author_stats
            });
        }

        self.cache.authors = authors;
        self.cache.filtered_commits = total;
        self.resort();
    }

    pub fn time_bounds(&self) -> (i64, i64) {
        self.bounds_for_offset(self.view.time_offset)
    }

    pub fn time_label(&self) -> String {
        self.label_for_offset(self.view.time_offset)
    }

    fn label_for_offset(&self, offset: i32) -> String {
        match self.view.time_mode {
            TimeMode::All => "All time".to_string(),
            TimeMode::Year => {
                let year = ts_year(self.data.latest) + offset;
                format!("{year}")
            }
            TimeMode::Quarter => {
                let dt = ts_to_dt(self.data.latest);
                let ref_q_start = ((dt.month() - 1) / 3) * 3 + 1;
                let (y, m) = offset_month(dt.year(), ref_q_start, offset * 3);
                let q = (m - 1) / 3 + 1;
                format!("{y} Q{q}")
            }
            TimeMode::Month => {
                let dt = ts_to_dt(self.data.latest);
                let (y, m) = offset_month(dt.year(), dt.month(), offset);
                format!("{} {}", MONTHS[(m - 1) as usize], y)
            }
        }
    }

    fn bounds_for_offset(&self, offset: i32) -> (i64, i64) {
        match self.view.time_mode {
            TimeMode::All => (0, i64::MAX),
            TimeMode::Year => {
                let year = ts_year(self.data.latest) + offset;
                (month_ts(year, 1), month_ts(year + 1, 1))
            }
            TimeMode::Quarter => {
                let dt = ts_to_dt(self.data.latest);
                let ref_q_start = ((dt.month() - 1) / 3) * 3 + 1;
                let (y, m) = offset_month(dt.year(), ref_q_start, offset * 3);
                let (ny, nm) = offset_month(y, m, 3);
                (month_ts(y, m), month_ts(ny, nm))
            }
            TimeMode::Month => {
                let dt = ts_to_dt(self.data.latest);
                let (y, m) = offset_month(dt.year(), dt.month(), offset);
                let (ny, nm) = offset_month(y, m, 1);
                (month_ts(y, m), month_ts(ny, nm))
            }
        }
    }

    fn short_label_for_offset(&self, offset: i32) -> String {
        match self.view.time_mode {
            TimeMode::All => String::new(),
            TimeMode::Year => {
                let year = ts_year(self.data.latest) + offset;
                format!("{year}")
            }
            TimeMode::Quarter => {
                let dt = ts_to_dt(self.data.latest);
                let ref_q_start = ((dt.month() - 1) / 3) * 3 + 1;
                let (y, m) = offset_month(dt.year(), ref_q_start, offset * 3);
                let q = (m - 1) / 3 + 1;
                if m == 1 {
                    format!("{y}Q{q}")
                } else {
                    format!("Q{q}")
                }
            }
            TimeMode::Month => {
                let dt = ts_to_dt(self.data.latest);
                let (y, m) = offset_month(dt.year(), dt.month(), offset);
                let show_year = m == 1 || {
                    let (py, _) = offset_month(dt.year(), dt.month(), offset - 1);
                    y != py
                };
                if show_year {
                    format!("{}{}", MONTHS[(m - 1) as usize], y % 100)
                } else {
                    MONTHS[(m - 1) as usize].to_string()
                }
            }
        }
    }

    fn has_earlier_data(&self) -> bool {
        let (start, _) = self.time_bounds();
        start > self.data.earliest
    }

    pub fn sorted_authors(
        &self,
    ) -> impl ExactSizeIterator<Item = &AuthorStats> + DoubleEndedIterator + Clone + '_ {
        self.cache
            .sorted_indices
            .iter()
            .map(|&i| &self.cache.authors[i])
    }

    pub fn sorted_author_at(&self, pos: usize) -> Option<&AuthorStats> {
        self.cache
            .sorted_indices
            .get(pos)
            .map(|&i| &self.cache.authors[i])
    }

    pub fn sorted_author_count(&self) -> usize {
        self.cache.sorted_indices.len()
    }

    pub fn overall_time_range(&self) -> (i64, i64) {
        (self.data.earliest, self.data.latest)
    }

    fn resort(&mut self) {
        self.cache.sorted_indices = (0..self.cache.authors.len()).collect();
        let authors = &self.cache.authors;
        let sort_mode = self.view.sort_mode;
        self.cache.sorted_indices.sort_by(|&a, &b| {
            let va = sort_value_for(sort_mode, &authors[a]);
            let vb = sort_value_for(sort_mode, &authors[b]);
            vb.cmp(&va)
        });
        self.view.selected = 0;
        self.invalidate_caches();
    }

    fn invalidate_caches(&mut self) {
        self.cache.graph_cache.get_mut().take();
        self.cache.detail_cache.get_mut().take();
    }

    fn set_sort(&mut self, mode: SortMode) {
        if mode == SortMode::Ownership && !self.show_ownership() {
            return;
        }
        if self.view.sort_mode != mode {
            self.view.sort_mode = mode;
            self.resort();
        }
    }

    pub fn set_time_mode(&mut self, mode: TimeMode) {
        self.view.time_mode = mode;
        self.view.time_offset = 0;
        self.recompute();
    }

    fn time_navigate(&mut self, delta: i32) {
        if !self.is_time_filtered() {
            return;
        }
        let new_offset = self.view.time_offset + delta;
        if delta < 0 && !self.has_earlier_data() {
            return;
        }
        if delta > 0 && new_offset > 0 {
            return;
        }
        self.view.time_offset = new_offset;
        self.recompute();
    }

    pub fn is_current_window(&self) -> bool {
        self.view.time_mode == TimeMode::All || self.view.time_offset == 0
    }

    pub fn show_ownership(&self) -> bool {
        self.data.ownership == OwnershipPresence::Current && self.is_current_window()
    }

    pub fn set_commit_types(&mut self) {
        self.data.has_commit_types = true;
    }

    pub fn show_commit_types(&self) -> bool {
        self.data.has_commit_types
    }

    pub fn supports_time_nav(&self) -> bool {
        matches!(
            self.view.view_mode,
            ViewMode::Table | ViewMode::Graph | ViewMode::Detail
        )
    }

    pub fn is_time_filtered(&self) -> bool {
        self.view.time_mode != TimeMode::All
    }

    pub fn has_file_authors(&self) -> bool {
        self.data.has_file_authors
    }

    pub fn has_file_churn(&self) -> bool {
        self.data.has_file_churn
    }

    pub fn has_file_coupling(&self) -> bool {
        self.data.has_file_coupling
    }

    fn ownership_for_gid(&self, gid: usize) -> (usize, f64) {
        if !self.is_current_window() {
            return (0, 0.0);
        }
        let lines = self.data.ownership_per_group.get(gid).copied().unwrap_or(0);
        let pct = if self.data.ownership_total > 0 {
            lines as f64 / self.data.ownership_total as f64 * 100.0
        } else {
            0.0
        };
        (lines, pct)
    }

    fn empty_author(&self, gid: usize) -> AuthorStats {
        let (ownership_lines, ownership_pct) = self.ownership_for_gid(gid);
        AuthorStats {
            display_name: self.data.groups[gid].display_name.clone(),
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
        if self.view.view_mode != ViewMode::Detail {
            return None;
        }
        let gid = self.view.detail_group_id?;
        if gid >= self.data.groups.len() {
            return None;
        }
        let cache_hit = self
            .cache
            .detail_cache
            .borrow()
            .as_ref()
            .map(|(k, _)| *k == gid)
            .unwrap_or(false);
        if !cache_hit {
            let data = self.compute_detail_data(gid)?;
            *self.cache.detail_cache.borrow_mut() = Some((gid, data));
        }
        Some(Ref::map(self.cache.detail_cache.borrow(), |o| {
            &o.as_ref().unwrap().1
        }))
    }

    fn compute_detail_data(&self, gid: usize) -> Option<DetailData> {
        let author = self
            .cache
            .authors
            .iter()
            .find(|a| a.group_id == gid)
            .cloned()
            .unwrap_or_else(|| self.empty_author(gid));

        let aliases = self.data.groups[gid].aliases.clone();

        let (prev_name, next_name) = match self.sorted_authors().position(|a| a.group_id == gid) {
            Some(pos) => {
                let prev = pos
                    .checked_sub(1)
                    .and_then(|p| self.sorted_author_at(p))
                    .map(|a| a.display_name.clone());
                let next = self
                    .sorted_author_at(pos + 1)
                    .map(|a| a.display_name.clone());
                (prev, next)
            }
            None => (
                self.sorted_author_at(self.sorted_author_count().saturating_sub(1))
                    .map(|a| a.display_name.clone()),
                self.sorted_author_at(0).map(|a| a.display_name.clone()),
            ),
        };

        let (start, end) = self.time_bounds();

        let mut activity = [[0usize; 24]; 7];
        for entry in &self.data.commits {
            if entry.group_id != gid {
                continue;
            }
            if entry.timestamp < start || entry.timestamp >= end {
                continue;
            }
            if let Some(dt) = DateTime::from_timestamp(entry.timestamp, 0) {
                let local = dt.with_timezone(&chrono::Local);
                let dow = local.weekday().num_days_from_monday() as usize;
                let hour = local.hour() as usize;
                activity[dow][hour] += 1;
            }
        }

        let (top_files, recent_added, recent_deleted) =
            self.query_group_files(gid, start, end).unwrap_or_default();

        let trend = self.compute_trend(gid);

        Some(DetailData {
            author,
            aliases,
            prev_name,
            next_name,
            top_files,
            activity,
            recent_added,
            recent_deleted,
            trend,
        })
    }

    /// Fetch per-file commit counts and recent add/delete events for a group.
    ///
    /// File paths are not kept in memory; each detail-view open issues three
    /// short SQL queries scoped to the group's author emails and window.
    fn query_group_files(&self, gid: usize, start: i64, end: i64) -> rusqlite::Result<GroupFiles> {
        let mut emails: Vec<String> = self.data.groups[gid]
            .aliases
            .iter()
            .map(|(_, e)| e.clone())
            .collect();
        emails.sort();
        emails.dedup();
        if emails.is_empty() {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        }

        // `?N` placeholders for emails start after the three fixed params
        // (kind, start, end).
        let email_placeholders = (0..emails.len())
            .map(|i| format!("?{}", i + 4))
            .collect::<Vec<_>>()
            .join(",");

        let top_files = self.query_top_files(&emails, &email_placeholders, start, end)?;
        let recent_added =
            self.query_recent_events(FILE_KIND_ADDED, &emails, &email_placeholders, start, end)?;
        let recent_deleted =
            self.query_recent_events(FILE_KIND_DELETED, &emails, &email_placeholders, start, end)?;

        Ok((top_files, recent_added, recent_deleted))
    }

    fn query_top_files(
        &self,
        emails: &[String],
        email_placeholders: &str,
        start: i64,
        end: i64,
    ) -> rusqlite::Result<Vec<(String, usize)>> {
        let sql = format!(
            "SELECT cf.file_path, COUNT(*) AS ct
             FROM commit_files cf
             JOIN commits c ON c.hash = cf.commit_hash
             WHERE cf.kind = ?1 AND c.timestamp >= ?2 AND c.timestamp < ?3
               AND c.author_email IN ({email_placeholders})
             GROUP BY cf.file_path
             ORDER BY ct DESC, cf.file_path ASC
             LIMIT 20"
        );
        let mut stmt = self.data.db.prepare(&sql)?;
        let args = bind_args(FILE_KIND_TOUCHED, start, end, emails);
        let mut rows = stmt.query(params_from_iter(args.iter()))?;
        let mut out = Vec::with_capacity(20);
        while let Some(row) = rows.next()? {
            out.push((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize));
        }
        Ok(out)
    }

    fn query_recent_events(
        &self,
        kind: i64,
        emails: &[String],
        email_placeholders: &str,
        start: i64,
        end: i64,
    ) -> rusqlite::Result<Vec<FileEvent>> {
        // Over-fetch then dedupe by path — matches the prior behavior of
        // "most-recent N distinct files" without a correlated subquery.
        let sql = format!(
            "SELECT cf.file_path, c.timestamp
             FROM commit_files cf
             JOIN commits c ON c.hash = cf.commit_hash
             WHERE cf.kind = ?1 AND c.timestamp >= ?2 AND c.timestamp < ?3
               AND c.author_email IN ({email_placeholders})
             ORDER BY c.timestamp DESC
             LIMIT 200"
        );
        let mut stmt = self.data.db.prepare(&sql)?;
        let args = bind_args(kind, start, end, emails);
        let mut rows = stmt.query(params_from_iter(args.iter()))?;
        let mut seen = std::collections::HashSet::<String>::new();
        let mut out = Vec::with_capacity(10);
        while let Some(row) = rows.next()? {
            if out.len() >= 10 {
                break;
            }
            let path: String = row.get(0)?;
            if !seen.insert(path.clone()) {
                continue;
            }
            out.push(FileEvent {
                path,
                timestamp: row.get(1)?,
            });
        }
        Ok(out)
    }

    fn compute_trend(&self, gid: usize) -> Vec<TrendPoint> {
        if !self.is_time_filtered() {
            return Vec::new();
        }

        let periods: Vec<(i64, i64, String, bool)> = (-SURROUND_PERIODS..=SURROUND_PERIODS)
            .filter_map(|delta| {
                let virtual_offset = self.view.time_offset + delta;
                let (s, e) = self.bounds_for_offset(virtual_offset);
                if e <= self.data.earliest || s >= self.data.latest + SECONDS_PER_DAY {
                    return None;
                }
                let label = self.short_label_for_offset(virtual_offset);
                Some((s, e, label, delta == 0))
            })
            .collect();

        let mut author_commits: Vec<(i64, usize, usize)> = self
            .data
            .commits
            .iter()
            .filter(|c| c.group_id == gid)
            .map(|c| {
                (
                    c.timestamp,
                    c.lines_added + c.lines_removed,
                    c.files_changed,
                )
            })
            .collect();
        author_commits.sort_by_key(|&(ts, _, _)| ts);

        periods
            .iter()
            .map(|(start, end, label, is_current)| {
                let mut sess_lines = 0usize;
                let mut sess_files = 0usize;
                let mut sess_last_ts = 0i64;
                let mut total = 0u64;
                let mut first = true;

                for &(ts, lines, files) in &author_commits {
                    if ts < *start || ts >= *end {
                        continue;
                    }
                    if !first && ts - sess_last_ts > SESSION_GAP_SECS && sess_lines + sess_files > 0
                    {
                        total +=
                            (session_value(sess_lines as f64, sess_files as f64) * 10.0) as u64;
                        sess_lines = 0;
                        sess_files = 0;
                    }
                    first = false;
                    sess_lines += lines;
                    sess_files += files;
                    sess_last_ts = ts;
                }
                if sess_lines + sess_files > 0 {
                    total += (session_value(sess_lines as f64, sess_files as f64) * 10.0) as u64;
                }

                TrendPoint {
                    label: label.clone(),
                    value: total,
                    is_current: *is_current,
                }
            })
            .collect()
    }

    fn detail_navigate(&mut self, delta: i32) {
        if self.view.detail_group_id.is_none() {
            return;
        }
        let count = self.sorted_author_count();
        if count == 0 {
            return;
        }
        let pos = self.view.detail_position.unwrap_or(0).min(count - 1);
        let new_pos = if delta < 0 {
            pos.saturating_sub(1)
        } else {
            (pos + 1).min(count - 1)
        };
        if new_pos != pos {
            let new_gid = self.sorted_author_at(new_pos).unwrap().group_id;
            self.view.detail_group_id = Some(new_gid);
            self.view.detail_position = Some(new_pos);
            self.view.detail_scroll = 0;
        }
    }

    fn start_questionnaire(&mut self) {
        let candidates =
            crate::questionnaire::find_candidates(&self.data.groups, &self.data.identity_map);
        if candidates.is_empty() {
            return;
        }
        self.view.questionnaire = Some(QuestionnaireState {
            candidates: candidates
                .into_iter()
                .map(|(a, b)| QuestionnaireCandidate {
                    group_a: a,
                    group_b: b,
                })
                .collect(),
            current: 0,
            changed: false,
            last_action: None,
        });
    }

    fn finish_questionnaire(&mut self) {
        let changed = self.view.questionnaire.as_ref().is_some_and(|q| q.changed);
        self.view.questionnaire = None;

        if changed {
            let _ = self.data.identity_map.save(&self.data.identity_map_path);
            let (new_groups, assignments) = crate::identity::merge(
                &self.data.commits,
                &self.data.identity_map,
                &self.data.mailmap_entries,
            );
            for (commit, &gid) in self.data.commits.iter_mut().zip(assignments.iter()) {
                commit.group_id = gid;
            }
            self.data.total_authors = new_groups.len();
            self.data.groups = new_groups;
            self.view.detail_group_id = None;
            self.view.detail_position = None;
            self.view.selected = 0;
            self.data.ownership_per_group.clear();
            self.data.ownership_total = 0;
            self.data.ownership = OwnershipPresence::Stale;
            self.recompute();
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.view.show_theme_picker {
            match key.code {
                KeyCode::Char('n') | KeyCode::Char('N') => self.view.theme = Theme::Normal,
                KeyCode::Char('r') | KeyCode::Char('R') => self.view.theme = Theme::Readable,
                KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                    self.view.theme = match self.view.theme {
                        Theme::Normal => Theme::Readable,
                        Theme::Readable => Theme::Normal,
                    };
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    self.view.show_theme_picker = false;
                    self.start_questionnaire();
                }
                KeyCode::Esc | KeyCode::Char('q') => return true,
                _ => {}
            }
            return false;
        }

        if self.view.questionnaire.is_some() {
            return self.handle_questionnaire_key(key);
        }

        if self.view.view_mode == ViewMode::Detail {
            return self.handle_detail_key(key);
        }

        if self.view.view_mode == ViewMode::FileDetail {
            return self.handle_file_detail_key(key);
        }

        if self.view.view_mode == ViewMode::Files {
            return self.handle_files_key(key);
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
            KeyCode::Char('T') => self.view.show_theme_picker = true,
            KeyCode::Char('g') => {
                self.view.view_mode = match self.view.view_mode {
                    ViewMode::Table => ViewMode::Graph,
                    ViewMode::Graph => ViewMode::Table,
                    ViewMode::Detail | ViewMode::Files | ViewMode::FileDetail => ViewMode::Table,
                };
            }
            KeyCode::Char('V') if self.data.has_files_view => {
                self.view.view_mode = ViewMode::Files;
                self.view.file_selected = 0;
            }
            KeyCode::Char('t') => self.set_time_mode(self.view.time_mode.next()),
            KeyCode::Left | KeyCode::Char('[') => self.time_navigate(-1),
            KeyCode::Right | KeyCode::Char(']') => self.time_navigate(1),
            KeyCode::Up | KeyCode::Char('k') => {
                self.view.selected = self.view.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j')
                if self.view.selected + 1 < self.cache.authors.len() =>
            {
                self.view.selected += 1;
            }
            KeyCode::Home => self.view.selected = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.view.selected = self.cache.authors.len().saturating_sub(1);
            }
            KeyCode::Enter => {
                if let Some(author) = self.sorted_author_at(self.view.selected) {
                    self.view.detail_group_id = Some(author.group_id);
                    self.view.detail_position = Some(self.view.selected);
                    self.view.view_mode = ViewMode::Detail;
                    self.view.detail_scroll = 0;
                }
            }
            _ => {}
        }
        false
    }

    fn handle_questionnaire_key(&mut self, key: KeyEvent) -> bool {
        let q = self.view.questionnaire.as_ref().unwrap();
        let cand = &q.candidates[q.current];
        let ga = cand.group_a;
        let gb = cand.group_b;

        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.data
                    .identity_map
                    .add_merge(&self.data.groups[ga].aliases, &self.data.groups[gb].aliases);
                let q = self.view.questionnaire.as_mut().unwrap();
                q.changed = true;
                q.last_action = Some("Merged");
                q.current += 1;
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                self.data
                    .identity_map
                    .add_reject(&self.data.groups[ga].aliases, &self.data.groups[gb].aliases);
                let q = self.view.questionnaire.as_mut().unwrap();
                q.changed = true;
                q.last_action = Some("Rejected");
                q.current += 1;
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.data
                    .identity_map
                    .add_unsure(&self.data.groups[ga].aliases, &self.data.groups[gb].aliases);
                let q = self.view.questionnaire.as_mut().unwrap();
                q.changed = true;
                q.last_action = Some("Unsure");
                q.current += 1;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                let q = self.view.questionnaire.as_mut().unwrap();
                q.last_action = Some("Skipped");
                q.current += 1;
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                self.finish_questionnaire();
                return false;
            }
            _ => {}
        }

        if self
            .view
            .questionnaire
            .as_ref()
            .is_some_and(|q| q.current >= q.candidates.len())
        {
            self.finish_questionnaire();
        }

        false
    }

    fn handle_detail_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => {
                if let Some(pos) = self.view.detail_position {
                    self.view.selected = pos;
                }
                self.view.detail_group_id = None;
                self.view.detail_position = None;
                self.view.view_mode = ViewMode::Table;
            }
            KeyCode::Char('q') => return true,
            KeyCode::Char('t') => self.set_time_mode(self.view.time_mode.next()),
            KeyCode::Char('T') => self.view.show_theme_picker = true,
            KeyCode::Left | KeyCode::Char('[') => self.time_navigate(-1),
            KeyCode::Right | KeyCode::Char(']') => self.time_navigate(1),
            KeyCode::Up | KeyCode::Char('k') => self.detail_navigate(-1),
            KeyCode::Down | KeyCode::Char('j') => self.detail_navigate(1),
            KeyCode::PageUp => {
                self.view.detail_scroll = self.view.detail_scroll.saturating_sub(5);
            }
            KeyCode::PageDown => {
                self.view.detail_scroll = (self.view.detail_scroll + 5).min(DETAIL_SCROLL_MAX);
            }
            KeyCode::Home => self.view.detail_scroll = 0,
            _ => {}
        }
        false
    }

    fn handle_files_key(&mut self, key: KeyEvent) -> bool {
        let count = self.data.file_rows.len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('V') => {
                self.view.view_mode = ViewMode::Table;
            }
            KeyCode::Char('q') => return true,
            KeyCode::Enter => self.open_file_detail(),
            KeyCode::Up | KeyCode::Char('k') => {
                self.view.file_selected = self.view.file_selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') if self.view.file_selected + 1 < count => {
                self.view.file_selected += 1;
            }
            KeyCode::Home => self.view.file_selected = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.view.file_selected = count.saturating_sub(1);
            }
            KeyCode::Char('c') => self.set_file_sort(FileSortMode::Commits),
            KeyCode::Char('a') => self.set_file_sort(FileSortMode::Authors),
            KeyCode::Char('h') => self.set_file_sort(FileSortMode::Churn),
            KeyCode::Char('p') => self.set_file_sort(FileSortMode::Coupling),
            _ => {}
        }
        false
    }

    fn handle_file_detail_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => {
                self.cache.file_detail = None;
                self.view.view_mode = ViewMode::Files;
            }
            KeyCode::Char('q') => return true,
            KeyCode::Up | KeyCode::Char('k') => {
                self.view.file_detail_scroll = self.view.file_detail_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = self
                    .cache
                    .file_detail
                    .as_ref()
                    .map(|d| d.coupled_files.len().saturating_sub(1))
                    .unwrap_or(0);
                if self.view.file_detail_scroll < max {
                    self.view.file_detail_scroll += 1;
                }
            }
            KeyCode::PageUp => {
                self.view.file_detail_scroll = self.view.file_detail_scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                let max = self
                    .cache
                    .file_detail
                    .as_ref()
                    .map(|d| d.coupled_files.len().saturating_sub(1))
                    .unwrap_or(0);
                self.view.file_detail_scroll = (self.view.file_detail_scroll + 10).min(max);
            }
            KeyCode::Home => self.view.file_detail_scroll = 0,
            _ => {}
        }
        false
    }

    pub fn graph_data(&self) -> Ref<'_, GraphData> {
        if self.cache.graph_cache.borrow().is_none() {
            let data = self.compute_graph_data();
            *self.cache.graph_cache.borrow_mut() = Some(data);
        }
        Ref::map(self.cache.graph_cache.borrow(), |o| o.as_ref().unwrap())
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
        let start = if raw_start == 0 {
            self.data.earliest
        } else {
            raw_start
        };
        let end = if raw_end == i64::MAX {
            self.data.latest + SECONDS_PER_DAY
        } else {
            raw_end
        };

        let (boundaries, labels) = period_boundaries(start, end);
        let num_periods = labels.len();

        let total_authors = self.sorted_author_count();
        let top_ids: Vec<usize> = self.sorted_authors().map(|a| a.group_id).collect();

        let slot_map: std::collections::HashMap<usize, usize> = top_ids
            .iter()
            .enumerate()
            .map(|(i, &gid)| (gid, i))
            .collect();
        let mut per_author: Vec<Vec<(i64, usize, usize)>> = vec![Vec::new(); total_authors];

        for entry in &self.data.commits {
            if entry.timestamp < start || entry.timestamp >= end {
                continue;
            }
            let Some(&author_idx) = slot_map.get(&entry.group_id) else {
                continue;
            };
            per_author[author_idx].push((
                entry.timestamp,
                entry.lines_added + entry.lines_removed,
                entry.files_changed,
            ));
        }

        let mut buckets: Vec<Vec<u64>> = vec![vec![0; num_periods]; total_authors];

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
                    let val = session_value(sess_lines as f64, sess_files as f64);
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
                let val = session_value(sess_lines as f64, sess_files as f64);
                let bucket = boundaries
                    .partition_point(|&b| b <= sess_last_ts)
                    .saturating_sub(1)
                    .min(num_periods - 1);
                buckets[ai][bucket] += (val * 10.0) as u64;
            }
        }

        let rows: Vec<GraphRow> = self
            .sorted_authors()
            .enumerate()
            .map(|(i, a)| GraphRow {
                name: a.display_name.clone(),
                data: std::mem::take(&mut buckets[i]),
                color: PALETTE[i % PALETTE.len()],
            })
            .collect();

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
        .expect("month always 1..=12 from offset_month")
        .and_hms_opt(0, 0, 0)
        .expect("midnight is always valid")
        .and_utc()
        .timestamp()
}

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
        let label = format!("{y}Q{q}");
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

#[derive(Debug, PartialEq)]
enum ChangeKind {
    Feature,
    Refactor,
    Rename,
    Trivial,
    Merge,
}

fn bind_args(kind: i64, start: i64, end: i64, emails: &[String]) -> Vec<rusqlite::types::Value> {
    use rusqlite::types::Value;
    let mut args = Vec::with_capacity(3 + emails.len());
    args.push(Value::Integer(kind));
    args.push(Value::Integer(start));
    args.push(Value::Integer(end));
    for e in emails {
        args.push(Value::Text(e.clone()));
    }
    args
}

fn classify_commit(c: &Commit) -> ChangeKind {
    if c.is_merge {
        return ChangeKind::Merge;
    }

    let total_lines = c.lines_added + c.lines_removed;
    let ws_total = c.whitespace_added + c.whitespace_removed;
    let is_whitespace_only = total_lines > 0 && ws_total >= total_lines;
    let is_trivial = total_lines <= 5 && c.files_changed <= 1;

    if is_whitespace_only || is_trivial {
        return ChangeKind::Trivial;
    }

    if c.files_renamed > 0 {
        let file_ops = c.files_added + c.files_deleted + c.files_renamed;
        if c.files_renamed * 2 >= file_ops && total_lines <= c.files_renamed * 20 {
            return ChangeKind::Rename;
        }
    }

    if c.files_added > 0 && c.files_deleted == 0 {
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
        files_changed += c.files_changed;
        change_types.whitespace_lines += c.whitespace_added + c.whitespace_removed;
        change_types.new_files += c.files_added;
        change_types.deleted_files += c.files_deleted;
        change_types.renamed_files += c.files_renamed;
        match classify_commit(c) {
            ChangeKind::Feature => change_types.feature += 1,
            ChangeKind::Refactor => change_types.refactor += 1,
            ChangeKind::Rename => change_types.rename += 1,
            ChangeKind::Trivial => change_types.trivial += 1,
            ChangeKind::Merge => change_types.merge += 1,
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
    let mut total_files = 0usize;
    let mut merge_files = 0usize;
    let mut sessions = 1usize;
    let mut last_ts = sorted_commits[0].timestamp;

    for c in sorted_commits {
        total_lines += c.lines_added + c.lines_removed;
        total_ws += c.whitespace_added + c.whitespace_removed;
        if c.is_merge {
            merge_files += c.files_changed;
        } else {
            total_files += c.files_changed;
        }
        if c.timestamp - last_ts > SESSION_GAP_SECS {
            sessions += 1;
        }
        last_ts = c.timestamp;
    }

    let substantive_lines = total_lines.saturating_sub(total_ws);
    let ws_contribution = total_ws as f64 * 0.1;
    let effective_lines = substantive_lines as f64 + ws_contribution;

    let effective_files = total_files as f64 + merge_files as f64 * 0.1;
    let substance = session_value(effective_lines, effective_files);
    substance * (1.0 + 0.15 * (sessions as f64).ln())
}

fn session_value(lines: f64, files: f64) -> f64 {
    (1.0 + (1.0 + lines).ln()) * (1.0 + 0.5 * (1.0 + files).ln())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Commit;

    fn assert_approx(got: f64, expected: f64) {
        assert!(
            (got - expected).abs() < 1e-6,
            "got {got}, expected {expected}"
        );
    }

    fn default_commit() -> Commit {
        Commit {
            author_name: "Test".into(),
            author_email: "test@example.com".into(),
            group_id: 0,
            timestamp: 0,
            lines_added: 0,
            lines_removed: 0,
            files_changed: 0,
            whitespace_added: 0,
            whitespace_removed: 0,
            files_added: 0,
            files_deleted: 0,
            files_renamed: 0,
            is_merge: false,
        }
    }

    fn make_author(commits: usize, trivial: usize) -> AuthorStats {
        AuthorStats {
            display_name: String::new(),
            group_id: 0,
            commits,
            lines_added: 0,
            lines_removed: 0,
            files_changed: 0,
            first_commit: 0,
            last_commit: 0,
            impact: 0.0,
            ownership_lines: 0,
            ownership_pct: 0.0,
            change_types: ChangeBreakdown {
                trivial,
                ..ChangeBreakdown::default()
            },
        }
    }

    // --- noise_pct ---

    #[test]
    fn noise_pct_zero_commits() {
        assert_approx(noise_pct(&make_author(0, 0)), 0.0);
    }

    #[test]
    fn noise_pct_no_trivial() {
        assert_approx(noise_pct(&make_author(10, 0)), 0.0);
    }

    #[test]
    fn noise_pct_all_trivial() {
        assert_approx(noise_pct(&make_author(1, 1)), 100.0);
    }

    #[test]
    fn noise_pct_mixed() {
        assert_approx(noise_pct(&make_author(10, 3)), 30.0);
    }

    // --- session_value ---

    #[test]
    fn session_value_zeros() {
        assert_approx(session_value(0.0, 0.0), 1.0);
    }

    #[test]
    fn session_value_lines_only() {
        assert_approx(session_value(1.0, 0.0), 1.0 + 2.0_f64.ln());
    }

    #[test]
    fn session_value_both() {
        let expected = (1.0 + 11.0_f64.ln()) * (1.0 + 0.5 * 3.0_f64.ln());
        assert_approx(session_value(10.0, 2.0), expected);
    }

    // --- classify_commit ---

    #[test]
    fn classify_merge() {
        let c = Commit {
            lines_added: 100,
            lines_removed: 50,
            files_changed: 10,
            files_added: 5,
            files_deleted: 2,
            is_merge: true,
            ..default_commit()
        };
        assert_eq!(classify_commit(&c), ChangeKind::Merge);
    }

    #[test]
    fn classify_trivial_whitespace_only() {
        let c = Commit {
            lines_added: 5,
            lines_removed: 5,
            files_changed: 3,
            whitespace_added: 5,
            whitespace_removed: 5,
            ..default_commit()
        };
        assert_eq!(classify_commit(&c), ChangeKind::Trivial);
    }

    #[test]
    fn classify_trivial_small() {
        let c = Commit {
            lines_added: 3,
            lines_removed: 2,
            files_changed: 1,
            ..default_commit()
        };
        assert_eq!(classify_commit(&c), ChangeKind::Trivial);
    }

    #[test]
    fn classify_trivial_empty() {
        let c = default_commit();
        assert_eq!(classify_commit(&c), ChangeKind::Trivial);
    }

    #[test]
    fn classify_rename() {
        let c = Commit {
            lines_added: 15,
            lines_removed: 15,
            files_changed: 4,
            files_renamed: 2,
            ..default_commit()
        };
        assert_eq!(classify_commit(&c), ChangeKind::Rename);
    }

    #[test]
    fn classify_rename_too_many_lines() {
        let c = Commit {
            lines_added: 21,
            files_changed: 2,
            files_renamed: 1,
            ..default_commit()
        };
        assert_ne!(classify_commit(&c), ChangeKind::Rename);
    }

    #[test]
    fn classify_feature_new_files() {
        let c = Commit {
            lines_added: 20,
            files_changed: 3,
            files_added: 1,
            ..default_commit()
        };
        assert_eq!(classify_commit(&c), ChangeKind::Feature);
    }

    #[test]
    fn classify_refactor_deleted_files() {
        let c = Commit {
            lines_removed: 20,
            files_changed: 2,
            files_deleted: 1,
            ..default_commit()
        };
        assert_eq!(classify_commit(&c), ChangeKind::Refactor);
    }

    #[test]
    fn classify_feature_high_add_ratio() {
        let c = Commit {
            lines_added: 9,
            lines_removed: 3,
            files_changed: 3,
            ..default_commit()
        };
        assert_eq!(classify_commit(&c), ChangeKind::Feature);
    }

    #[test]
    fn classify_refactor_low_add_ratio() {
        let c = Commit {
            lines_added: 3,
            lines_removed: 3,
            files_changed: 3,
            ..default_commit()
        };
        assert_eq!(classify_commit(&c), ChangeKind::Refactor);
    }

    // --- compute_impact ---

    #[test]
    fn impact_empty() {
        let commits: Vec<&Commit> = vec![];
        assert_approx(compute_impact(&commits), 0.0);
    }

    #[test]
    fn impact_single_commit() {
        let c = Commit {
            lines_added: 10,
            files_changed: 3,
            timestamp: 1000,
            ..default_commit()
        };
        let expected = session_value(10.0, 3.0);
        assert_approx(compute_impact(&[&c]), expected);
    }

    #[test]
    fn impact_same_session() {
        let c1 = Commit {
            lines_added: 5,
            files_changed: 1,
            ..default_commit()
        };
        let c2 = Commit {
            lines_added: 5,
            files_changed: 1,
            timestamp: 1000,
            ..default_commit()
        };
        let one_session = session_value(10.0, 2.0) * 1.0;
        assert_approx(compute_impact(&[&c1, &c2]), one_session);
    }

    #[test]
    fn impact_boundary_not_new_session() {
        let c1 = Commit {
            lines_added: 5,
            files_changed: 1,
            ..default_commit()
        };
        let c2 = Commit {
            lines_added: 5,
            files_changed: 1,
            timestamp: SESSION_GAP_SECS,
            ..default_commit()
        };
        let one_session = session_value(10.0, 2.0);
        assert_approx(compute_impact(&[&c1, &c2]), one_session);
    }

    #[test]
    fn impact_boundary_new_session() {
        let c1 = Commit {
            lines_added: 5,
            files_changed: 1,
            ..default_commit()
        };
        let c2 = Commit {
            lines_added: 5,
            files_changed: 1,
            timestamp: SESSION_GAP_SECS + 1,
            ..default_commit()
        };
        let two_sessions = session_value(10.0, 2.0) * (1.0 + 0.15 * 2.0_f64.ln());
        assert_approx(compute_impact(&[&c1, &c2]), two_sessions);
    }

    #[test]
    fn impact_whitespace_discount() {
        let c = Commit {
            lines_added: 10,
            lines_removed: 10,
            files_changed: 2,
            whitespace_added: 10,
            whitespace_removed: 10,
            ..default_commit()
        };
        let effective_lines = 20.0 * 0.1; // 2.0
        let expected = session_value(effective_lines, 2.0);
        assert_approx(compute_impact(&[&c]), expected);
    }

    #[test]
    fn impact_merge_file_discount() {
        let c = Commit {
            files_changed: 10,
            is_merge: true,
            ..default_commit()
        };
        let effective_files = 10.0 * 0.1; // 1.0
        let expected = session_value(0.0, effective_files);
        assert_approx(compute_impact(&[&c]), expected);
    }
}
