use crate::app::{
    App, FileDetailData, FileEvent, FileSortMode, SortMode, Theme, TrendPoint, ViewMode,
};
use crate::fmt::{fmt_date, Sep};
use crate::identity_map::format_identity;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState};
use ratatui::Terminal;
use std::io;
use std::sync::mpsc;
use std::time::Duration;

const GOLD: Color = Color::Rgb(255, 215, 0);
const GILD_AMBER: Color = Color::Rgb(0xf1, 0x8f, 0x3b);
const GILD_SPARK: Color = Color::Rgb(0xff, 0xcb, 0x7a);
const SILVER: Color = Color::Rgb(192, 192, 192);
const BRONZE: Color = Color::Rgb(205, 127, 50);
const ADDED: Color = Color::Rgb(80, 250, 123);
const REMOVED: Color = Color::Rgb(255, 85, 85);
const HEADER_COLOR: Color = Color::Rgb(139, 233, 253);
const DIM: Color = Color::Rgb(108, 118, 148);
const BORDER_TEAL: Color = Color::Rgb(0x4a, 0x93, 0x8f);
const ACTIVE_SORT: Color = Color::Rgb(80, 250, 123);
const EMPTY_WEEKDAY: Color = Color::Rgb(255, 184, 108);

type Stop = (f64, (u8, u8, u8));

// Unified bar palette: steel blue → bronze → amber → gold
const BAR_STOPS: [Stop; 4] = [
    (0.00, (0x4a, 0x5a, 0x7a)),
    (0.35, (0xcd, 0x7f, 0x32)),
    (0.70, (0xf1, 0x8f, 0x3b)),
    (1.00, (0xfb, 0xbf, 0x24)),
];

// Impact column: blue throughout — no pure white at the top, stays on-theme
const SPARKLE_STOPS: [Stop; 4] = [
    (0.00, (0x5a, 0x9c, 0xd6)),
    (0.40, (0x7b, 0xb8, 0xe8)),
    (0.70, (0xa0, 0xd4, 0xf4)),
    (1.00, (0xd5, 0xec, 0xff)),
];

// Value-driven ownership %: muted blue → cyan → green → orange → gold
const OWNERSHIP_STOPS: [Stop; 5] = [
    (0.00, (0x62, 0x72, 0xa4)),
    (0.30, (0x8b, 0xe9, 0xfd)),
    (0.50, (0x50, 0xfa, 0x7b)),
    (0.70, (0xff, 0xb8, 0x6c)),
    (1.00, (0xff, 0xd7, 0x00)),
];

// Value-driven noise %: green (clean) → yellow → orange → red (noisy)
const NOISE_STOPS: [Stop; 4] = [
    (0.00, (0x50, 0xfa, 0x7b)),
    (0.30, (0xf1, 0xfa, 0x8c)),
    (0.60, (0xff, 0xb8, 0x6c)),
    (1.00, (0xff, 0x55, 0x55)),
];

fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
    (a as f64 + (b as f64 - a as f64) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn scale_color(base: Color, factor: f64) -> Color {
    let (r, g, b) = match base {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::White => (255, 255, 255),
        Color::Black => (0, 0, 0),
        _ => return base,
    };
    let f = factor.clamp(0.0, 1.0);
    Color::Rgb(
        ((r as f64) * f).round() as u8,
        ((g as f64) * f).round() as u8,
        ((b as f64) * f).round() as u8,
    )
}

/// Returns a brightness factor in [0.9, 1.0] for `value` within [min, max].
/// The column's max sits at full brightness; the column's min sits 10% dimmer.
fn dim_factor(value: f64, min: f64, max: f64) -> f64 {
    if max <= min {
        return 1.0;
    }
    let t = ((value - min) / (max - min)).clamp(0.0, 1.0);
    0.9 + 0.1 * t
}

fn gradient_color(stops: &[Stop], t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    let mut i = 0;
    while i + 1 < stops.len() && stops[i + 1].0 < t {
        i += 1;
    }
    if i + 1 >= stops.len() {
        let (r, g, b) = stops[stops.len() - 1].1;
        return Color::Rgb(r, g, b);
    }
    let (t0, (r0, g0, b0)) = stops[i];
    let (t1, (r1, g1, b1)) = stops[i + 1];
    let local = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
    Color::Rgb(
        lerp_u8(r0, r1, local),
        lerp_u8(g0, g1, local),
        lerp_u8(b0, b1, local),
    )
}

fn dim<'a, T: Into<std::borrow::Cow<'a, str>>>(text: T) -> Span<'a> {
    Span::styled(text, Style::default().fg(DIM))
}

fn fg<'a, T: Into<std::borrow::Cow<'a, str>>>(text: T, color: Color) -> Span<'a> {
    Span::styled(text, Style::default().fg(color))
}

fn bold_fg<'a, T: Into<std::borrow::Cow<'a, str>>>(text: T, color: Color) -> Span<'a> {
    Span::styled(
        text,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn impact_header(is_sorted: bool, width: usize) -> Cell<'static> {
    const NAME: &str = "Impact";
    let sparkle_color = Color::Rgb(0xff, 0xff, 0xff);
    let name_color = Color::Rgb(0x89, 0xdd, 0xff);

    let content_len = 2 + NAME.chars().count() + if is_sorted { 2 } else { 0 };
    let pad = width.saturating_sub(content_len);

    let mut spans = vec![
        Span::raw(" ".repeat(pad)),
        Span::styled(
            "\u{2726} ",
            Style::default()
                .fg(sparkle_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            NAME,
            Style::default().fg(name_color).add_modifier(Modifier::BOLD),
        ),
    ];

    if is_sorted {
        spans.push(Span::styled(
            " \u{25be}",
            Style::default()
                .fg(ACTIVE_SORT)
                .add_modifier(Modifier::BOLD),
        ));
    }

    Cell::from(Line::from(spans))
}

fn metric_header(name: &str, is_sorted: bool, is_bad: bool, width: usize) -> Cell<'static> {
    let dot_color = if is_bad { REMOVED } else { ADDED };
    let name_style = if is_sorted {
        Style::default()
            .fg(ACTIVE_SORT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(DIM).add_modifier(Modifier::BOLD)
    };

    let content_len = 2 + name.chars().count() + if is_sorted { 2 } else { 0 };
    let pad = width.saturating_sub(content_len);

    let mut spans = vec![
        Span::raw(" ".repeat(pad)),
        Span::styled("\u{2022} ", Style::default().fg(dot_color)),
        Span::styled(name.to_string(), name_style),
    ];
    if is_sorted {
        spans.push(Span::styled(
            " \u{25be}",
            Style::default()
                .fg(ACTIVE_SORT)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Cell::from(Line::from(spans))
}

fn titled_block(title: &str, title_color: Color) -> Block<'_> {
    Block::default()
        .title(Span::styled(
            title.to_string(),
            Style::default()
                .fg(title_color)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER_TEAL))
}

fn gild_header_block() -> Block<'static> {
    Block::default()
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "\u{2726} ",
                Style::default().fg(GILD_SPARK).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "gild",
                Style::default().fg(GILD_AMBER).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " \u{2726} ",
                Style::default().fg(GILD_SPARK).add_modifier(Modifier::BOLD),
            ),
        ]))
        .borders(Borders::ALL)
        .border_set(border::DOUBLE)
        .border_style(Style::default().fg(GILD_AMBER).add_modifier(Modifier::BOLD))
}

fn is_horizontal_border(sym: &str) -> bool {
    matches!(sym, "\u{2500}" | "\u{2550}" | "\u{2501}")
}

fn place_sparkle(buf: &mut Buffer, x: u16, y: u16) {
    if let Some(cell) = buf.cell_mut(Position::new(x, y)) {
        if is_horizontal_border(cell.symbol()) {
            cell.set_symbol("\u{2726}");
            cell.set_fg(GILD_SPARK);
            cell.modifier |= Modifier::BOLD;
        }
    }
}

/// Scatters ✦ sparkles across the top and bottom borders of the gild frame,
/// turning the plain `═` line into a proper gilded edge.
fn sparkle_top_border(frame: &mut ratatui::Frame, area: Rect) {
    if area.height < 2 || area.width < 24 {
        return;
    }
    let buf = frame.buffer_mut();
    let top_y = area.y;
    let bottom_y = area.y + area.height - 1;
    let w = area.width;

    // Top: 4 sparkles, clustered to the right of the title
    let top = [
        area.x + w / 3,
        area.x + w / 2 + 2,
        area.x + (w * 2) / 3,
        area.x + (w * 5) / 6,
    ];
    for &x in &top {
        place_sparkle(buf, x, top_y);
    }

    // Bottom: 3 sparkles, evenly distributed for balance
    let bottom = [area.x + w / 5, area.x + w / 2, area.x + (w * 4) / 5];
    for &x in &bottom {
        place_sparkle(buf, x, bottom_y);
    }
}

#[derive(Clone, Copy)]
pub struct LoadStep {
    pub label: &'static str,
    /// Whether this step may dispatch work across rayon threads. Drives the
    /// "N threads" badge and the --max-threads hint in the loading UI.
    pub parallel: bool,
}

pub enum LoadMsg {
    Plan(Vec<LoadStep>),
    StepStart(usize),
    CommitTotal(usize),
    CommitProgress {
        processed: usize,
        new_count: usize,
    },
    AddonProgress {
        label: &'static str,
        done: usize,
        total: usize,
    },
    ScanThreads(usize),
    Done(Box<App>),
    Failed(String),
}

#[derive(Default)]
struct LoadState {
    steps: Vec<LoadStep>,
    current_step: usize,
    commits_total: usize,
    commits_processed: usize,
    new_commits: usize,
    addon_label: &'static str,
    addon_done: usize,
    addon_total: usize,
    scan_threads: usize,
}

struct RawModeGuard;

impl RawModeGuard {
    fn enter() -> Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen);
    }
}

pub fn run_with_loading(rx: mpsc::Receiver<LoadMsg>) -> Result<()> {
    let _guard = RawModeGuard::enter()?;
    let backend = ratatui::backend::CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    loading_loop(&mut terminal, rx)
}

fn loading_loop(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
    rx: mpsc::Receiver<LoadMsg>,
) -> Result<()> {
    let mut state = LoadState::default();

    loop {
        loop {
            match rx.try_recv() {
                Ok(LoadMsg::Plan(steps)) => {
                    state.steps = steps;
                }
                Ok(LoadMsg::StepStart(idx)) => {
                    state.current_step = idx;
                    state.addon_label = "";
                    state.addon_done = 0;
                    state.addon_total = 0;
                }
                Ok(LoadMsg::CommitTotal(total)) => {
                    state.commits_total = total;
                }
                Ok(LoadMsg::CommitProgress {
                    processed,
                    new_count,
                }) => {
                    state.commits_processed = processed;
                    state.new_commits = new_count;
                }
                Ok(LoadMsg::AddonProgress { label, done, total }) => {
                    state.addon_label = label;
                    state.addon_done = done;
                    state.addon_total = total;
                    if let Some(idx) = state.steps.iter().position(|s| s.label == label) {
                        state.current_step = idx;
                    }
                }
                Ok(LoadMsg::ScanThreads(n)) => {
                    state.scan_threads = n;
                }
                Ok(LoadMsg::Done(boxed)) => {
                    let mut app = *boxed;
                    terminal.clear()?;
                    return event_loop(terminal, &mut app);
                }
                Ok(LoadMsg::Failed(msg)) => return Err(anyhow::anyhow!("{msg}")),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(anyhow::anyhow!("loader thread exited unexpectedly"));
                }
            }
        }

        terminal.draw(|frame| draw_loading(frame, &state))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                    return Ok(());
                }
            }
        }
    }
}

fn draw_progress_bar(frame: &mut ratatui::Frame, area: Rect, done: usize, total: usize) {
    if area.width < 10 {
        return;
    }
    let pct = (done as f64 / total as f64).min(1.0);
    let bar_w = area.width as usize;
    let filled = (pct * bar_w as f64) as usize;
    let empty = bar_w - filled;
    let spans = vec![
        Span::styled("\u{2588}".repeat(filled), Style::default().fg(GILD_AMBER)),
        Span::styled(
            "\u{2591}".repeat(empty),
            Style::default().fg(Color::Rgb(0x44, 0x47, 0x5a)),
        ),
    ];
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_loading(frame: &mut ratatui::Frame, state: &LoadState) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Rgb(0x28, 0x2a, 0x36))),
        area,
    );
    if area.height < 4 {
        return;
    }

    let margin = 4u16;
    let inner_w = area.width.saturating_sub(margin * 2);
    let ghost = Style::default().fg(Color::Rgb(0x44, 0x47, 0x5a));

    let is_commit_phase = state.current_step == 0;
    let (done, total) = if is_commit_phase {
        (state.commits_processed, state.commits_total)
    } else {
        (state.addon_done, state.addon_total)
    };

    let phase_label: String = if is_commit_phase {
        if state.commits_total == 0 {
            "Reading repository\u{2026}".to_string()
        } else if state.new_commits > 0 {
            "Scanning commits".to_string()
        } else {
            "Loading from cache".to_string()
        }
    } else if !state.addon_label.is_empty() {
        state.addon_label.to_string()
    } else if let Some(step) = state.steps.get(state.current_step) {
        step.label.to_string()
    } else {
        "Working\u{2026}".to_string()
    };

    let count_label: Option<String> = if total > 0 {
        if is_commit_phase && state.new_commits > 0 {
            Some(format!(
                "{} / {}  \u{2502}  {} new",
                Sep(done),
                Sep(total),
                Sep(state.new_commits),
            ))
        } else {
            Some(format!("{} / {}", Sep(done), Sep(total)))
        }
    } else {
        None
    };

    // Is the current step actually running parallel work right now?
    // The plan declares whether a step MAY use threads; at runtime we also
    // require that real work is happening (not a warm cache hit).
    let step_is_parallel = state
        .steps
        .get(state.current_step)
        .map(|s| s.parallel)
        .unwrap_or(false);
    let currently_computing = if is_commit_phase {
        state.new_commits > 0
    } else {
        state.addon_total > 0
    };
    let is_parallel_phase = step_is_parallel && currently_computing;
    let show_threads_hint = is_parallel_phase && state.scan_threads > 1;
    let show_first_run_note = currently_computing;

    // Lay out vertically: title, gap, phase, count?, gap, bar?, gap, steps, gap, note?, thread hint?
    let steps_rows = state.steps.len().max(1) as u16;
    let mut content_h: u16 = 1 + 1 + 1; // title + gap + phase
    if count_label.is_some() {
        content_h += 1;
    }
    content_h += 1; // gap
    if total > 0 {
        content_h += 1;
    } // progress bar
    content_h += 1; // gap before steps
    content_h += steps_rows;
    if show_first_run_note {
        content_h += 2;
    } // gap + note
    if show_threads_hint {
        content_h += 1;
    }

    let start_y = area.y + area.height.saturating_sub(content_h) / 2;
    let mut y = start_y;

    // Title
    if y < area.y + area.height {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "\u{2726} ",
                    Style::default().fg(GILD_SPARK).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "gild",
                    Style::default().fg(GILD_AMBER).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " \u{2726}",
                    Style::default().fg(GILD_SPARK).add_modifier(Modifier::BOLD),
                ),
            ]))
            .centered(),
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            },
        );
        y += 2; // title + gap
    }

    // Phase label
    if y < area.y + area.height {
        let mut spans = vec![Span::styled(
            phase_label.clone(),
            Style::default().fg(SILVER).add_modifier(Modifier::BOLD),
        )];
        if is_parallel_phase && state.scan_threads > 1 {
            spans.push(Span::styled(
                format!("  \u{00b7}  {} threads", state.scan_threads),
                Style::default().fg(DIM),
            ));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)).centered(),
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            },
        );
        y += 1;
    }

    // Count
    if let Some(ref count) = count_label {
        if y < area.y + area.height {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    count.clone(),
                    Style::default().fg(DIM),
                )))
                .centered(),
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                },
            );
            y += 1;
        }
    }

    y += 1; // gap before bar

    // Progress bar
    if total > 0 && y < area.y + area.height {
        draw_progress_bar(
            frame,
            Rect {
                x: area.x + margin,
                y,
                width: inner_w,
                height: 1,
            },
            done,
            total,
        );
        y += 1;
    }

    y += 1; // gap before step list

    // Step list
    if !state.steps.is_empty() {
        let list_w = state
            .steps
            .iter()
            .map(|s| s.label.chars().count())
            .max()
            .unwrap_or(0)
            + 2;
        let list_x = area.x + area.width.saturating_sub(list_w as u16) / 2;
        for (idx, step) in state.steps.iter().enumerate() {
            if y >= area.y + area.height {
                break;
            }
            let (sym, sym_style, label_style) = if idx < state.current_step {
                (
                    "\u{2713}",
                    Style::default().fg(ADDED),
                    Style::default().fg(DIM),
                )
            } else if idx == state.current_step {
                (
                    "\u{25b8}",
                    Style::default().fg(GILD_AMBER).add_modifier(Modifier::BOLD),
                    Style::default().fg(SILVER).add_modifier(Modifier::BOLD),
                )
            } else {
                (
                    "\u{00b7}",
                    Style::default().fg(Color::Rgb(0x44, 0x47, 0x5a)),
                    Style::default().fg(Color::Rgb(0x44, 0x47, 0x5a)),
                )
            };
            let line = Line::from(vec![
                Span::styled(sym, sym_style),
                Span::raw(" "),
                Span::styled(step.label.to_string(), label_style),
            ]);
            frame.render_widget(
                Paragraph::new(line),
                Rect {
                    x: list_x,
                    y,
                    width: area.width.saturating_sub(list_x - area.x),
                    height: 1,
                },
            );
            y += 1;
        }
    }

    // First-run note
    if show_first_run_note {
        y += 1;
        if y < area.y + area.height {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "First run \u{2014} results will be cached for instant future launches",
                    ghost,
                )))
                .centered(),
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                },
            );
            y += 1;
        }
    }

    // Thread hint
    if show_threads_hint && y < area.y + area.height {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "use --max-threads <N> to limit CPU usage",
                ghost,
            )))
            .centered(),
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            },
        );
    }

    // Quit hint
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled("  q quit", ghost))),
        Rect {
            x: area.x,
            y: area.y + area.height - 1,
            width: area.width,
            height: 1,
        },
    );
}

fn event_loop(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    let mut table_state = TableState::default();
    table_state.select(Some(0));

    loop {
        table_state.select(Some(app.selected));

        terminal.draw(|frame| draw(frame, app, &mut table_state))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if app.handle_key(key) {
                    break;
                }
            }
        }
    }

    Ok(())
}

fn draw(frame: &mut ratatui::Frame, app: &App, table_state: &mut TableState) {
    match app.view_mode {
        ViewMode::Detail => draw_detail_view(frame, app),
        ViewMode::Files => draw_files_view(frame, app),
        ViewMode::FileDetail => draw_file_detail_view(frame, app),
        _ => {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(4),
                    Constraint::Min(5),
                    Constraint::Length(1),
                ])
                .split(frame.area());

            draw_header(frame, app, chunks[0]);
            if app.view_mode == ViewMode::Graph {
                draw_graph(frame, app, chunks[1]);
            } else {
                draw_table(frame, app, table_state, chunks[1]);
            }
            draw_help(frame, app, chunks[2]);
        }
    }

    if app.theme == Theme::Readable {
        apply_readable_theme(frame);
    }

    if app.show_theme_picker {
        draw_theme_picker(frame, app);
    }

    if app.questionnaire.is_some() {
        draw_questionnaire(frame, app);
    }
}

const FORCED_BG: Color = Color::Rgb(0x1e, 0x1e, 0x2e);

fn apply_readable_theme(frame: &mut ratatui::Frame) {
    let area = frame.area();
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut(Position::new(x, y)) {
                if cell.bg == Color::Reset {
                    cell.bg = FORCED_BG;
                }
                cell.fg = simplify_color(cell.fg);
            }
        }
    }
}

fn simplify_color(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => {
            let lum = 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;

            if lum < 25.0 {
                return Color::Rgb(0x3a, 0x3a, 0x4e);
            }

            if r < 80 && g < 80 && b < 80 {
                return Color::DarkGray;
            }

            if lum < 75.0 {
                return Color::Gray;
            }

            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let sat = if max == 0 {
                0.0
            } else {
                (max - min) as f64 / max as f64
            };

            if sat < 0.2 {
                if lum > 200.0 {
                    return Color::White;
                }
                return Color::Gray;
            }

            if r > g && r > b {
                if g > 180 {
                    Color::Yellow
                } else if g > 100 {
                    Color::Rgb(0xff, 0xb0, 0x60)
                } else {
                    Color::LightRed
                }
            } else if g > r && g > b {
                Color::LightGreen
            } else if b > r && b > g {
                if r > 160 {
                    Color::Rgb(0xcc, 0xaa, 0xff)
                } else {
                    Color::LightCyan
                }
            } else if r > 200 && g > 200 {
                Color::Yellow
            } else if g > 200 && b > 200 {
                Color::LightCyan
            } else if r > 200 && b > 200 {
                Color::LightMagenta
            } else {
                Color::White
            }
        }
        _ => c,
    }
}

fn draw_theme_picker(frame: &mut ratatui::Frame, app: &App) {
    let screen = frame.area();
    let popup_w = 56u16.min(screen.width.saturating_sub(2));
    let popup_h = 10u16.min(screen.height.saturating_sub(2));
    if popup_w < 30 || popup_h < 8 {
        return;
    }
    let x = screen.x + (screen.width - popup_w) / 2;
    let y = screen.y + (screen.height - popup_h) / 3;
    let popup = Rect::new(x, y, popup_w, popup_h);

    frame.render_widget(Clear, popup);

    let popup_bg = Color::Rgb(0x1e, 0x1e, 0x2e);
    let highlight = Style::default()
        .fg(Color::LightGreen)
        .add_modifier(Modifier::BOLD);
    let muted = Style::default().fg(Color::Gray);

    let block = Block::default()
        .title(Span::styled(
            " Theme ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Gray))
        .style(Style::default().bg(popup_bg));

    let normal_label = if app.theme == Theme::Normal {
        "[N] Normal  \u{2713}"
    } else {
        "[N] Normal"
    };
    let readable_label = if app.theme == Theme::Readable {
        "[R] Readable  \u{2713}"
    } else {
        "[R] Readable"
    };

    let hint = match app.theme {
        Theme::Normal => "  uses your terminal's background",
        Theme::Readable => "  dark background + simplified colors",
    };

    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Does gild look right in your terminal?",
            muted,
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            Span::styled(
                normal_label,
                if app.theme == Theme::Normal {
                    highlight
                } else {
                    muted
                },
            ),
            Span::raw("      "),
            Span::styled(
                readable_label,
                if app.theme == Theme::Readable {
                    highlight
                } else {
                    muted
                },
            ),
        ]),
        Line::from(Span::styled(hint, muted)),
        Line::from(Span::styled(
            "  N / R or \u{2190}\u{2192} to preview, Enter to confirm",
            muted,
        )),
        Line::from(Span::styled("  Reopen anytime with [T]", muted)),
    ];

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, popup);
}

fn draw_questionnaire(frame: &mut ratatui::Frame, app: &App) {
    let q = match &app.questionnaire {
        Some(q) => q,
        None => return,
    };

    let cand = &q.candidates[q.current];
    let groups = app.groups();
    let group_a = &groups[cand.group_a];
    let group_b = &groups[cand.group_b];

    let a_lines = group_a.aliases.len();
    let b_lines = group_b.aliases.len();
    let content_h = a_lines + b_lines + 7;
    let popup_h = (content_h as u16 + 2).min(frame.area().height.saturating_sub(2));
    let popup_w = 64u16.min(frame.area().width.saturating_sub(4));
    if popup_w < 30 || popup_h < 8 {
        return;
    }

    let x = (frame.area().width - popup_w) / 2;
    let y = (frame.area().height - popup_h) / 3;
    let popup = Rect::new(x, y, popup_w, popup_h);

    frame.render_widget(Clear, popup);

    let popup_bg = Color::Rgb(0x1e, 0x1e, 0x2e);
    let block = Block::default()
        .title(Span::styled(
            " Identity Match ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Gray))
        .style(Style::default().bg(popup_bg));

    let muted = Style::default().fg(Color::Gray);
    let max_w = popup_w as usize - 4;

    let mut text: Vec<Line> = Vec::new();

    text.push(Line::from(Span::styled(
        format!("  ({}/{}) Same person?", q.current + 1, q.candidates.len()),
        muted,
    )));
    text.push(Line::from(""));

    for (name, email) in &group_a.aliases {
        let ident = format_identity(name, email);
        let display = if ident.len() > max_w {
            format!("{}\u{2026}", &ident[..max_w - 1])
        } else {
            ident
        };
        text.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(display, Style::default().fg(HEADER_COLOR)),
        ]));
    }

    text.push(Line::from(Span::styled("    vs", muted)));

    for (name, email) in &group_b.aliases {
        let ident = format_identity(name, email);
        let display = if ident.len() > max_w {
            format!("{}\u{2026}", &ident[..max_w - 1])
        } else {
            ident
        };
        text.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(display, Style::default().fg(GOLD)),
        ]));
    }

    text.push(Line::from(""));

    if let Some(action) = q.last_action {
        let color = match action {
            "Merged" => Color::LightGreen,
            "Rejected" => Color::LightRed,
            "Unsure" => Color::Yellow,
            _ => Color::Gray,
        };
        text.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("\u{2190} {action}"), Style::default().fg(color)),
        ]));
    } else {
        text.push(Line::from(""));
    }

    text.push(Line::from(vec![
        Span::styled(
            "  [y]",
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("es  ", muted),
        Span::styled(
            "[n]",
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("o  ", muted),
        Span::styled(
            "[d]",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("on't know  ", muted),
        Span::styled(
            "[s]",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("kip  ", muted),
        Span::styled(
            "[q]",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("uit", muted),
    ]));

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, popup);
}

fn draw_header(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let block = gild_header_block();

    let mut line2 = vec![
        fg(Sep(app.filtered_commits).to_string(), Color::White),
        dim(" commits  "),
        fg(Sep(app.authors.len()).to_string(), Color::White),
        dim(" authors"),
    ];

    if app.supports_time_nav() {
        if app.is_time_filtered() {
            line2.push(dim("  \u{25c0} "));
            line2.push(bold_fg(app.time_label(), HEADER_COLOR));
            line2.push(dim(" \u{25b6}"));
        } else {
            let (first_ts, last_ts) = app.overall_time_range();
            line2.push(dim("  "));
            line2.push(fg(fmt_date(first_ts, "%b %Y"), Color::White));
            line2.push(dim(" \u{2192} "));
            line2.push(fg(fmt_date(last_ts, "%b %Y"), Color::White));
        }
    }

    let text = vec![
        Line::from(vec![
            bold_fg(app.repo_info.name.clone(), HEADER_COLOR),
            dim(" on "),
            bold_fg(app.repo_info.branch.clone(), ACTIVE_SORT),
        ]),
        Line::from(line2),
    ];

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
    sparkle_top_border(frame, area);
}

fn draw_table(frame: &mut ratatui::Frame, app: &App, table_state: &mut TableState, area: Rect) {
    const W_COMMITS: usize = 11;
    const W_LINES: usize = 11;
    const W_FILES: usize = 10;
    const W_IMPACT: usize = 10;
    const W_NOISE: usize = 10;
    const W_OWN: usize = 9;

    let sort = app.sort_mode;
    let mut header_cells = vec![
        Cell::from(" # "),
        Cell::from("Author"),
        impact_header(sort == SortMode::Impact, W_IMPACT),
        metric_header("Commits", sort == SortMode::Commits, false, W_COMMITS),
        metric_header("+Lines", sort == SortMode::LinesAdded, false, W_LINES),
        metric_header("-Lines", sort == SortMode::LinesRemoved, false, W_LINES),
        metric_header("Files", sort == SortMode::FilesChanged, false, W_FILES),
        metric_header("Noise%", sort == SortMode::Noise, true, W_NOISE),
    ];
    if app.show_ownership() {
        header_cells.push(metric_header(
            "Own%",
            sort == SortMode::Ownership,
            false,
            W_OWN,
        ));
    }
    let header = Row::new(header_cells);

    let authors: Vec<&crate::app::AuthorStats> = app.sorted_authors().collect();

    let range = |f: fn(&crate::app::AuthorStats) -> f64| -> (f64, f64) {
        authors
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), a| {
                let v = f(a);
                (lo.min(v), hi.max(v))
            })
    };
    let r_commits = range(|a| a.commits as f64);
    let r_added = range(|a| a.lines_added as f64);
    let r_removed = range(|a| a.lines_removed as f64);
    let r_files = range(|a| a.files_changed as f64);
    let r_impact = range(|a| a.impact);

    let rows: Vec<Row> = authors
        .iter()
        .enumerate()
        .map(|(i, author)| {
            let rank = i + 1;
            let rank_color = match rank {
                1 => GOLD,
                2 => SILVER,
                3 => BRONZE,
                _ => Color::White,
            };

            let f_commits = dim_factor(author.commits as f64, r_commits.0, r_commits.1);
            let f_added = dim_factor(author.lines_added as f64, r_added.0, r_added.1);
            let f_removed = dim_factor(author.lines_removed as f64, r_removed.0, r_removed.1);
            let f_files = dim_factor(author.files_changed as f64, r_files.0, r_files.1);

            let impact_t = if r_impact.1 > r_impact.0 {
                ((author.impact - r_impact.0) / (r_impact.1 - r_impact.0)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let impact_color = gradient_color(&SPARKLE_STOPS, impact_t);

            let mut cells = vec![
                Cell::from(format!("{rank:>2} "))
                    .style(Style::default().fg(rank_color).add_modifier(Modifier::BOLD)),
                Cell::from(truncate(&author.display_name, 24))
                    .style(Style::default().fg(rank_color)),
                Cell::from(format!(
                    "{:>width$}",
                    format_impact(author.impact),
                    width = W_IMPACT
                ))
                .style(
                    Style::default()
                        .fg(impact_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(format!(
                    "{:>width$}",
                    Sep(author.commits),
                    width = W_COMMITS
                ))
                .style(Style::default().fg(scale_color(Color::White, f_commits))),
                Cell::from(format!(
                    "{:>width$}",
                    Sep(author.lines_added),
                    width = W_LINES
                ))
                .style(Style::default().fg(scale_color(ADDED, f_added))),
                Cell::from(format!(
                    "{:>width$}",
                    Sep(author.lines_removed),
                    width = W_LINES
                ))
                .style(Style::default().fg(scale_color(REMOVED, f_removed))),
                Cell::from(format!(
                    "{:>width$}",
                    Sep(author.files_changed),
                    width = W_FILES
                ))
                .style(Style::default().fg(scale_color(Color::White, f_files))),
            ];

            let noise = crate::app::noise_pct(author);
            cells.push(
                Cell::from(format!("{noise:>W_NOISE$.1}"))
                    .style(Style::default().fg(gradient_color(&NOISE_STOPS, noise / 100.0))),
            );

            if app.show_ownership() {
                let t = (author.ownership_pct / 100.0).clamp(0.0, 1.0);
                cells.push(
                    Cell::from(format!("{:>width$.1}", author.ownership_pct, width = W_OWN))
                        .style(Style::default().fg(gradient_color(&OWNERSHIP_STOPS, t))),
                );
            }

            Row::new(cells)
        })
        .collect();

    let mut widths = vec![
        Constraint::Length(4),
        Constraint::Length(25),
        Constraint::Length(W_IMPACT as u16),
        Constraint::Length(W_COMMITS as u16),
        Constraint::Length(W_LINES as u16),
        Constraint::Length(W_LINES as u16),
        Constraint::Length(W_FILES as u16),
        Constraint::Length(W_NOISE as u16),
    ];
    if app.show_ownership() {
        widths.push(Constraint::Length(W_OWN as u16));
    }

    let table = Table::new(rows, widths).header(header).row_highlight_style(
        Style::default()
            .bg(Color::Rgb(0x44, 0x48, 0x5c))
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(table, area, table_state);
}

fn draw_graph(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let graph = app.graph_data();
    if graph.rows.is_empty() {
        frame.render_widget(
            Paragraph::new("  No data for this period.").style(Style::default().fg(DIM)),
            area,
        );
        return;
    }

    let name_col = 20usize;
    let total_col = 9usize;
    let spark_width = (area.width as usize)
        .saturating_sub(name_col + total_col + 2)
        .min(60);

    let resampled_rows: Vec<Vec<u64>> = graph
        .rows
        .iter()
        .map(|r| resample(&r.data, spark_width))
        .collect();

    let global_max = resampled_rows
        .iter()
        .flat_map(|r| r.iter())
        .copied()
        .max()
        .unwrap_or(1)
        .max(1);

    let mut lines: Vec<Line> = Vec::new();

    if !graph.labels.is_empty() && spark_width > 4 {
        let first = &graph.labels[0];
        let last = graph.labels.last().expect("labels checked non-empty above");
        let pad = spark_width.saturating_sub(first.len() + last.len());
        lines.push(Line::from(vec![
            Span::raw(format!(" {:>width$}", "", width = name_col - 1)),
            dim(first.clone()),
            dim(format!("{:>width$}", last, width = pad + last.len())),
        ]));
    }

    lines.push(Line::from(""));

    for (i, row) in graph.rows.iter().enumerate() {
        let spark_str: String = resampled_rows[i]
            .iter()
            .map(|&v| sparkline_char(v, global_max))
            .collect();

        let total: u64 = row.data.iter().sum();

        let name_display = truncate(&row.name, name_col - 2);
        let name_pad = (name_col - 1).saturating_sub(display_width(&name_display));

        let spans = vec![
            fg(
                format!(" {}{}", name_display, " ".repeat(name_pad)),
                row.color,
            ),
            fg(spark_str, row.color),
            dim(format!(" {:>7}", Sep(total as usize))),
        ];
        lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn draw_files_view(frame: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_header(frame, app, chunks[0]);
    draw_files_table(frame, app, chunks[1]);
    draw_files_help(frame, app, chunks[2]);
}

fn file_path_cell(path: &str, max_width: usize) -> Cell<'static> {
    let (dir, name) = match path.rfind('/') {
        Some(pos) => (&path[..pos + 1], &path[pos + 1..]),
        None => ("", path),
    };

    let name_w = display_width(name);
    if name_w >= max_width {
        return Cell::from(Line::from(vec![Span::styled(
            truncate(name, max_width),
            Style::default().fg(HEADER_COLOR),
        )]));
    }

    let budget = max_width - name_w;
    let dir_display = if budget < 4 || dir.is_empty() {
        String::new()
    } else {
        truncate_left(dir, budget)
    };

    Cell::from(Line::from(vec![
        Span::styled(dir_display, Style::default().fg(DIM)),
        Span::styled(name.to_string(), Style::default().fg(HEADER_COLOR)),
    ]))
}

fn coupling_cell(partner: &str, score: f64, max_width: usize) -> Cell<'static> {
    let score_text = format!(" {:.0}%", score * 100.0);
    let score_w = display_width(&score_text);
    let path_budget = max_width.saturating_sub(score_w);

    let name = match partner.rfind('/') {
        Some(pos) => &partner[pos + 1..],
        None => partner,
    };
    let display = truncate(name, path_budget);

    let t = (score * 2.0).min(1.0);
    Cell::from(Line::from(vec![
        Span::styled(display, Style::default().fg(HEADER_COLOR)),
        Span::styled(
            score_text,
            Style::default().fg(gradient_color(&BAR_STOPS, t)),
        ),
    ]))
}

fn draw_files_table(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    const W_BAR: usize = 8;
    const W_COUNT: usize = 6;
    const W_COMMITS: usize = W_BAR + W_COUNT;
    const W_AUTHORS: usize = 9;
    const W_CHURN: usize = 9;
    const W_COUPLING: usize = 24;

    if app.file_rows.is_empty() {
        frame.render_widget(
            Paragraph::new("  No file data.").style(Style::default().fg(DIM)),
            area,
        );
        return;
    }

    let has_authors = app.has_file_authors();
    let has_churn = app.has_file_churn();
    let has_coupling = app.has_file_coupling();
    let sort = app.file_sort;

    let max_commits = app
        .file_rows
        .iter()
        .map(|r| r.commit_count)
        .max()
        .unwrap_or(1)
        .max(1);

    let mut header_cells = vec![
        Cell::from(""),
        Cell::from(Line::from(Span::styled(
            "File",
            Style::default().fg(DIM).add_modifier(Modifier::BOLD),
        ))),
        metric_header("Commits", sort == FileSortMode::Commits, false, W_COMMITS),
    ];
    if has_authors {
        header_cells.push(metric_header(
            "Authors",
            sort == FileSortMode::Authors,
            false,
            W_AUTHORS,
        ));
    }
    if has_churn {
        header_cells.push(metric_header(
            "Hotspot",
            sort == FileSortMode::Churn,
            true,
            W_CHURN,
        ));
    }
    if has_coupling {
        header_cells.push(metric_header(
            "Coupled With",
            sort == FileSortMode::Coupling,
            false,
            W_COUPLING,
        ));
    }
    let header =
        Row::new(header_cells).style(Style::default().fg(DIM).add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = app
        .file_rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let rank_color = match i {
                0 => GOLD,
                1 => SILVER,
                2 => BRONZE,
                _ => DIM,
            };

            let frac = row.commit_count as f64 / max_commits as f64;
            let filled = (frac * W_BAR as f64).round() as usize;
            let mut bar_spans: Vec<Span> = Vec::with_capacity(W_BAR + W_COUNT);
            bar_spans.push(Span::styled(
                format!(
                    "{:>width$} ",
                    Sep(row.commit_count as usize),
                    width = W_COUNT - 1
                ),
                Style::default().fg(Color::White),
            ));
            let denom = (W_BAR.saturating_sub(1)).max(1) as f64;
            for j in 0..W_BAR {
                if j < filled {
                    let t = j as f64 / denom;
                    bar_spans.push(Span::styled(
                        "\u{2588}",
                        Style::default().fg(gradient_color(&BAR_STOPS, t)),
                    ));
                } else {
                    bar_spans.push(Span::styled(
                        "\u{2591}",
                        Style::default().fg(Color::Rgb(0x30, 0x32, 0x40)),
                    ));
                }
            }

            let path_budget = (area.width as usize).saturating_sub(
                4 + W_COMMITS
                    + if has_authors { W_AUTHORS } else { 0 }
                    + if has_churn { W_CHURN } else { 0 }
                    + if has_coupling { W_COUPLING } else { 0 }
                    + 4,
            );

            let mut cells: Vec<Cell> = vec![
                Cell::from(format!("{:>2} ", i + 1)).style(Style::default().fg(rank_color)),
                file_path_cell(&row.path, path_budget.max(12)),
                Cell::from(Line::from(bar_spans)),
            ];

            if has_authors {
                let n = row.unique_authors.unwrap_or(0);
                let author_color = match n {
                    0 | 1 => REMOVED,
                    2 => Color::Rgb(0xff, 0xb8, 0x6c),
                    _ => ADDED,
                };
                cells.push(
                    Cell::from(format!("{n:>W_AUTHORS$}")).style(Style::default().fg(author_color)),
                );
            }
            if has_churn {
                let score = row.churn_score.unwrap_or(0.0);
                let t = (score / 5.0).min(1.0);
                cells.push(
                    Cell::from(format!("{:>width$.2}x", score, width = W_CHURN - 1))
                        .style(Style::default().fg(gradient_color(&NOISE_STOPS, t))),
                );
            }
            if has_coupling {
                if let Some((ref partner, score)) = row.top_coupled {
                    cells.push(coupling_cell(partner, score, W_COUPLING));
                } else {
                    cells.push(Cell::from(""));
                }
            }

            Row::new(cells)
        })
        .collect();

    let mut widths = vec![
        Constraint::Length(4),
        Constraint::Min(20),
        Constraint::Length(W_COMMITS as u16),
    ];
    if has_authors {
        widths.push(Constraint::Length(W_AUTHORS as u16));
    }
    if has_churn {
        widths.push(Constraint::Length(W_CHURN as u16));
    }
    if has_coupling {
        widths.push(Constraint::Length(W_COUPLING as u16));
    }

    let mut file_table_state = TableState::default();
    file_table_state.select(Some(app.file_selected));

    let table = Table::new(rows, widths).header(header).row_highlight_style(
        Style::default()
            .bg(Color::Rgb(0x44, 0x48, 0x5c))
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(table, area, &mut file_table_state);
}

fn draw_files_help(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let has_authors = app.has_file_authors();
    let has_churn = app.has_file_churn();
    let has_coupling = app.has_file_coupling();
    let sort = app.file_sort;

    let active = Style::default()
        .fg(ACTIVE_SORT)
        .add_modifier(Modifier::BOLD);
    let inactive = Style::default().fg(DIM);

    let mut spans: Vec<Span> = vec![dim(" Sort: ")];

    let sort_item =
        |key: &'static str, label: &'static str, is_active: bool| -> Vec<Span<'static>> {
            let s = if is_active { active } else { inactive };
            vec![Span::styled(key, s), Span::styled(label, s), Span::raw(" ")]
        };

    spans.extend(sort_item("c", " Commits", sort == FileSortMode::Commits));
    if has_authors {
        spans.extend(sort_item("a", " Authors", sort == FileSortMode::Authors));
    }
    if has_churn {
        spans.extend(sort_item("h", " Hotspot", sort == FileSortMode::Churn));
    }
    if has_coupling {
        spans.extend(sort_item("p", " Coupled", sort == FileSortMode::Coupling));
    }
    spans.extend([
        dim(" \u{2502} "),
        dim("Enter detail  "),
        dim("↑↓ scroll  "),
        dim("V back  "),
        dim("q quit"),
    ]);

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_file_detail_view(frame: &mut ratatui::Frame, app: &App) {
    let detail = match &app.file_detail {
        Some(d) => d,
        None => return,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(5),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_header(frame, app, chunks[0]);
    draw_file_detail_stats(frame, detail, chunks[1]);
    draw_file_detail_coupling(frame, detail, app.file_detail_scroll, chunks[2]);

    let spans = vec![dim(" ↑↓ scroll  "), dim("Esc back  "), dim("q quit")];
    frame.render_widget(Paragraph::new(Line::from(spans)), chunks[3]);
}

fn draw_file_detail_stats(frame: &mut ratatui::Frame, detail: &FileDetailData, area: Rect) {
    let (dir, name) = match detail.path.rfind('/') {
        Some(pos) => (&detail.path[..pos + 1], &detail.path[pos + 1..]),
        None => ("", detail.path.as_str()),
    };

    let title_line = Line::from(vec![
        Span::styled(format!(" {dir}"), Style::default().fg(DIM)),
        Span::styled(
            name.to_string(),
            Style::default()
                .fg(HEADER_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let mut stats_spans = vec![fg(
        format!(" {} commits", Sep(detail.commit_count as usize)),
        Color::White,
    )];
    if let Some(authors) = detail.unique_authors {
        let c = match authors {
            0 | 1 => REMOVED,
            2 => Color::Rgb(0xff, 0xb8, 0x6c),
            _ => ADDED,
        };
        stats_spans.push(fg("  \u{2022}  ", DIM));
        stats_spans.push(fg(format!("{authors} authors"), c));
    }
    if let Some(churn) = detail.churn_score {
        let t = (churn / 5.0).min(1.0);
        stats_spans.push(fg("  \u{2022}  ", DIM));
        stats_spans.push(fg(
            format!("{churn:.2}x hotspot"),
            gradient_color(&NOISE_STOPS, t),
        ));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(0x44, 0x48, 0x5c)))
        .border_set(border::ROUNDED);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![title_line, Line::from(vec![]), Line::from(stats_spans)];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_file_detail_coupling(
    frame: &mut ratatui::Frame,
    detail: &FileDetailData,
    scroll: usize,
    area: Rect,
) {
    let title = format!(" Coupled Files ({}) ", detail.coupled_files.len());
    let block = titled_block(&title, HEADER_COLOR);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if detail.coupled_files.is_empty() {
        let msg = Paragraph::new("  No coupling data — run with --add-on coupling")
            .style(Style::default().fg(DIM));
        frame.render_widget(msg, inner);
        return;
    }

    let max_score = detail
        .coupled_files
        .first()
        .map(|(_, s)| *s)
        .unwrap_or(1.0)
        .max(0.01);
    let visible = inner.height as usize;
    let start = scroll.min(detail.coupled_files.len().saturating_sub(visible));
    let bar_width: usize = 12;

    let lines: Vec<Line> = detail
        .coupled_files
        .iter()
        .skip(start)
        .take(visible)
        .map(|(path, score)| {
            let pct = score * 100.0;
            let frac = score / max_score;
            let filled = (frac * bar_width as f64).round() as usize;

            let (dir, name) = match path.rfind('/') {
                Some(pos) => (&path[..pos + 1], &path[pos + 1..]),
                None => ("", path.as_str()),
            };

            let max_path_len = (inner.width as usize).saturating_sub(bar_width + 10);
            let name_budget = max_path_len.min(display_width(name));
            let dir_budget = max_path_len.saturating_sub(name_budget);
            let dir_display = if dir_budget < 4 || dir.is_empty() {
                String::new()
            } else {
                truncate_left(dir, dir_budget)
            };

            let mut spans = vec![fg(format!(" {pct:>4.0}% "), Color::White)];
            let denom = (bar_width.saturating_sub(1)).max(1) as f64;
            for j in 0..bar_width {
                if j < filled {
                    let t = j as f64 / denom;
                    spans.push(fg("\u{2588}", gradient_color(&BAR_STOPS, t)));
                } else {
                    spans.push(fg("\u{2591}", Color::Rgb(0x30, 0x32, 0x40)));
                }
            }
            spans.push(fg(format!(" {dir_display}"), DIM));
            spans.push(fg(name.to_string(), HEADER_COLOR));
            Line::from(spans)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_detail_view(frame: &mut ratatui::Frame, app: &App) {
    let detail = match app.detail_data() {
        Some(d) => d,
        None => return,
    };

    let has_aliases = !detail.aliases.is_empty();
    let header_height = if has_aliases { 7 } else { 6 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let a = &detail.author;
    let ownership_line = if a.ownership_lines > 0 {
        format!(
            "  Owns {} lines ({:.1}% of codebase)",
            Sep(a.ownership_lines),
            a.ownership_pct
        )
    } else {
        String::new()
    };

    let ct = &a.change_types;
    let ct_total = ct.feature + ct.refactor + ct.rename + ct.trivial + ct.merge;
    let ws_pct = if a.lines_added + a.lines_removed > 0 {
        ct.whitespace_lines as f64 / (a.lines_added + a.lines_removed) as f64 * 100.0
    } else {
        0.0
    };

    let header_block = gild_header_block();

    let name_line = Line::from(vec![
        bold_fg(a.display_name.clone(), GOLD),
        Span::raw("  "),
        dim(app.time_label()),
    ]);

    let stats_line = if a.commits == 0 {
        Line::from(vec![Span::styled(
            format!("No activity in {}", app.time_label()),
            Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
        )])
    } else {
        Line::from(vec![
            fg(Sep(a.commits).to_string(), Color::White),
            dim(" commits  "),
            fg(format!("+{}", Sep(a.lines_added)), ADDED),
            dim("  "),
            fg(format!("-{}", Sep(a.lines_removed)), REMOVED),
            dim("  "),
            fg(format!("{} files", Sep(a.files_changed)), Color::White),
            fg(format!(" (+{}", Sep(ct.new_files)), ADDED),
            fg(format!(" -{}", Sep(ct.deleted_files)), REMOVED),
            dim(")  "),
            dim("impact "),
            fg(format_impact(a.impact), HEADER_COLOR),
            fg(ownership_line.clone(), Color::Rgb(255, 184, 108)),
        ])
    };

    let mut header_text = vec![name_line];

    if !detail.aliases.is_empty() {
        let mut emails: Vec<&str> = Vec::new();
        let mut other_names: Vec<&str> = Vec::new();
        for (name, email) in &detail.aliases {
            if !emails.contains(&email.as_str()) {
                emails.push(email);
            }
            if name != &a.display_name && !other_names.contains(&name.as_str()) {
                other_names.push(name);
            }
        }

        let mut spans: Vec<Span> = Vec::new();
        let max_show = 3;
        for (i, email) in emails.iter().take(max_show).enumerate() {
            if i > 0 {
                spans.push(dim(", "));
            }
            spans.push(fg((*email).to_string(), Color::Rgb(189, 147, 249)));
        }
        if emails.len() > max_show {
            spans.push(dim(format!(" +{}", emails.len() - max_show)));
        }
        if !other_names.is_empty() {
            spans.push(dim("  aka "));
            for (i, name) in other_names.iter().take(max_show).enumerate() {
                if i > 0 {
                    spans.push(dim(", "));
                }
                spans.push(fg((*name).to_string(), SILVER));
            }
            if other_names.len() > max_show {
                spans.push(dim(format!(" +{}", other_names.len() - max_show)));
            }
        }
        header_text.push(Line::from(spans));
    }

    header_text.push(stats_line);
    if ct_total > 0 && app.show_commit_types() {
        let mut spans = vec![
            fg(format!("{} feature", ct.feature), ADDED),
            dim("  "),
            fg(
                format!("{} refactor", ct.refactor),
                Color::Rgb(139, 233, 253),
            ),
        ];
        if ct.rename > 0 {
            spans.push(dim("  "));
            spans.push(fg(
                format!("{} rename", ct.rename),
                Color::Rgb(255, 121, 198),
            ));
        }
        spans.push(dim("  "));
        spans.push(dim(format!("{} trivial", ct.trivial)));
        if ct.merge > 0 {
            spans.push(dim("  "));
            spans.push(fg(format!("{} merge", ct.merge), Color::Rgb(189, 147, 249)));
        }
        if ct.new_files > 0 {
            spans.push(fg(format!("  +{} new files", ct.new_files), ADDED));
        }
        if ct.deleted_files > 0 {
            spans.push(fg(format!("  -{} deleted", ct.deleted_files), REMOVED));
        }
        if ct.renamed_files > 0 {
            spans.push(fg(
                format!("  ~{} renamed", ct.renamed_files),
                Color::Rgb(255, 121, 198),
            ));
        }
        if ws_pct > 1.0 {
            spans.push(fg(
                format!("  {ws_pct:.0}% whitespace"),
                Color::Rgb(255, 85, 85),
            ));
        }
        header_text.push(Line::from(spans));
    }

    let header_inner = header_block.inner(chunks[0]);
    frame.render_widget(header_block, chunks[0]);
    sparkle_top_border(frame, chunks[0]);

    let spindle_width: u16 = 24;
    let (body_area, spindle_area) = if header_inner.width > spindle_width + 20 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(spindle_width)])
            .split(header_inner);
        (cols[0], Some(cols[1]))
    } else {
        (header_inner, None)
    };

    frame.render_widget(Paragraph::new(header_text), body_area);

    if let Some(area) = spindle_area {
        let max_name = (spindle_width as usize).saturating_sub(4);
        let neighbor_line = |arrow: &'static str, name: &Option<String>| match name {
            Some(n) => Line::from(vec![
                fg(arrow, ACTIVE_SORT),
                fg(truncate(n, max_name), Color::White),
            ]),
            None => Line::from(dim(format!("{arrow}\u{2014}"))),
        };
        let spindle_text = vec![
            neighbor_line("\u{25b2} ", &detail.prev_name),
            neighbor_line("\u{25bc} ", &detail.next_name),
        ];
        frame.render_widget(Paragraph::new(spindle_text), area);
    }

    let content_area = chunks[1];
    let has_trend = detail.trend.len() >= 2;

    let added_count = detail.recent_added.len();
    let deleted_count = detail.recent_deleted.len();
    let bottom_content = added_count.max(deleted_count);
    let bottom_h = if bottom_content == 0 {
        3
    } else {
        (bottom_content as u16 + 2).min(8)
    };

    let content_rows = if has_trend {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(10),
                Constraint::Min(10),
                Constraint::Length(bottom_h),
            ])
            .split(content_area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(bottom_h)])
            .split(content_area)
    };

    if has_trend {
        draw_detail_trend(frame, &detail.trend, content_rows[0]);
    }

    let panels_idx = if has_trend { 1 } else { 0 };
    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(content_rows[panels_idx]);

    let bottom_idx = if has_trend { 2 } else { 1 };
    let bottom_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(content_rows[bottom_idx]);

    draw_detail_files(frame, &detail.top_files, app.detail_scroll, top_cols[0]);
    draw_detail_activity(frame, &detail.activity, top_cols[1]);
    draw_file_events(
        frame,
        &detail.recent_added,
        " New files ",
        ADDED,
        bottom_cols[0],
    );
    draw_file_events(
        frame,
        &detail.recent_deleted,
        " Deleted files ",
        REMOVED,
        bottom_cols[1],
    );

    let help = Paragraph::new(Line::from(vec![
        dim("[Esc]"),
        dim("back "),
        dim("[t]"),
        dim("ime "),
        dim("[←→]"),
        dim("period "),
        dim("[↑↓]"),
        dim("author "),
        dim("[PgUp/Dn]"),
        dim("scroll "),
        dim("[q]"),
        dim("uit"),
    ]));
    frame.render_widget(help, chunks[2]);
}

fn draw_detail_files(
    frame: &mut ratatui::Frame,
    files: &[(String, usize)],
    scroll: usize,
    area: Rect,
) {
    let block = titled_block(" Top Files ", HEADER_COLOR);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if files.is_empty() {
        let msg =
            Paragraph::new("  No file data (try clearing cache)").style(Style::default().fg(DIM));
        frame.render_widget(msg, inner);
        return;
    }

    let max_count = files.first().map(|(_, c)| *c).unwrap_or(1).max(1);
    let visible_height = inner.height as usize;
    let start = scroll.min(files.len().saturating_sub(visible_height));

    let lines: Vec<Line> = files
        .iter()
        .skip(start)
        .take(visible_height)
        .map(|(path, count)| {
            let bar_width: usize = 10;
            let filled = (*count as f64 / max_count as f64 * bar_width as f64).round() as usize;

            let max_path_len = (inner.width as usize).saturating_sub(bar_width + 8);
            let display_path = truncate_left(path, max_path_len);

            let mut spans = vec![fg(format!(" {count:>4} "), Color::White)];
            let denom = (bar_width.saturating_sub(1)).max(1) as f64;
            for i in 0..bar_width {
                if i < filled {
                    let t = i as f64 / denom;
                    spans.push(fg("\u{2588}", gradient_color(&BAR_STOPS, t)));
                } else {
                    spans.push(fg("\u{2591}", DIM));
                }
            }
            spans.push(fg(format!(" {display_path}"), HEADER_COLOR));
            Line::from(spans)
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn draw_detail_activity(frame: &mut ratatui::Frame, activity: &[[usize; 24]; 7], area: Rect) {
    let block = titled_block(" Activity Pattern ", HEADER_COLOR);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let max_val = activity
        .iter()
        .flat_map(|row| row.iter())
        .copied()
        .max()
        .unwrap_or(0);

    if max_val == 0 {
        let msg = Paragraph::new("  No activity data").style(Style::default().fg(DIM));
        frame.render_widget(msg, inner);
        return;
    }

    let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let mut lines: Vec<Line> = Vec::new();

    let hour_labels: Vec<String> = (0..24).step_by(3).map(|h| format!("{h:>2}")).collect();
    let mut header_spans = vec![Span::raw("      ")];
    for label in &hour_labels {
        header_spans.push(dim(format!("{label} ")));
    }
    lines.push(Line::from(header_spans));
    lines.push(Line::from(""));

    for (day_idx, day_name) in days.iter().enumerate() {
        let total: usize = activity[day_idx].iter().sum();
        let is_weekend = day_idx >= 5;
        let empty_weekday = total == 0 && !is_weekend;

        let (day_color, marker) = if empty_weekday {
            (EMPTY_WEEKDAY, "\u{2205} ")
        } else if is_weekend {
            (DIM, "  ")
        } else {
            (Color::White, "  ")
        };

        let mut spans = vec![
            fg(format!(" {day_name} "), day_color),
            fg(marker, day_color),
        ];

        for &val in &activity[day_idx] {
            let t = if val == 0 {
                0.0
            } else {
                (val as f64 / max_val as f64).powf(0.6)
            };
            spans.push(fg("\u{2588}", gradient_color(&BAR_STOPS, t)));
        }

        spans.push(dim(format!(" {total:>3}")));

        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));

    let total_all: usize = activity.iter().flat_map(|r| r.iter()).sum();
    let mut hour_totals = [0usize; 24];
    for row in activity {
        for (h, &v) in row.iter().enumerate() {
            hour_totals[h] += v;
        }
    }
    let peak_hour = hour_totals
        .iter()
        .enumerate()
        .max_by_key(|(_, &v)| v)
        .map(|(h, _)| h)
        .unwrap_or(0);

    lines.push(Line::from(vec![
        dim(" Total: "),
        fg(total_all.to_string(), Color::White),
        dim("  Peak: "),
        fg(format!("{peak_hour}:00"), ACTIVE_SORT),
    ]));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn draw_detail_trend(frame: &mut ratatui::Frame, trend: &[TrendPoint], area: Rect) {
    let block = titled_block(" Trend ", HEADER_COLOR);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if trend.is_empty() {
        return;
    }

    let max_val = trend.iter().map(|p| p.value).max().unwrap_or(1).max(1);
    let bar_height = inner.height.saturating_sub(2) as usize;
    if bar_height == 0 || inner.width < 3 {
        return;
    }

    let n = trend.len();
    let avail = inner.width as usize;
    let bar_w = ((avail - 1) / n).clamp(1, 10);
    let gap = if bar_w >= 3 { 1 } else { 0 };
    let total_w = n * bar_w + (n - 1) * gap;
    let left_pad = (avail.saturating_sub(total_w)) / 2;

    let fills: Vec<f64> = trend
        .iter()
        .map(|p| p.value as f64 / max_val as f64)
        .collect();

    let bar_tops: Vec<usize> = fills
        .iter()
        .map(|&f| {
            let filled_rows = (f * bar_height as f64).round() as usize;
            bar_height - filled_rows.min(bar_height)
        })
        .collect();

    let mut buf_lines: Vec<Line> = Vec::new();

    for row in 0..bar_height {
        let threshold = (bar_height - row) as f64 / bar_height as f64;
        let mut spans: Vec<Span> = vec![Span::raw(" ".repeat(left_pad))];

        for (i, point) in trend.iter().enumerate() {
            let is_top_row = row == bar_tops[i] && fills[i] > 0.0;

            if fills[i] >= threshold {
                if is_top_row && bar_w >= 3 {
                    let val_str = Sep(point.value as usize).to_string();
                    let val_display = truncate(&val_str, bar_w);
                    let vw = display_width(&val_display);
                    let vpad = bar_w.saturating_sub(vw);
                    let pl = vpad / 2;
                    let pr = vpad - pl;
                    let label = format!("{}{}{}", " ".repeat(pl), val_display, " ".repeat(pr));
                    if point.is_current {
                        spans.push(Span::styled(
                            label,
                            Style::default()
                                .fg(Color::Rgb(0x28, 0x2a, 0x36))
                                .bg(gradient_color(&BAR_STOPS, fills[i].clamp(0.3, 1.0))),
                        ));
                    } else {
                        spans.push(Span::styled(
                            label,
                            Style::default()
                                .fg(Color::Rgb(0x28, 0x2a, 0x36))
                                .bg(Color::Rgb(0x5a, 0x5e, 0x76)),
                        ));
                    }
                } else if point.is_current {
                    spans.push(fg(
                        "\u{2588}".repeat(bar_w),
                        gradient_color(&BAR_STOPS, fills[i].clamp(0.3, 1.0)),
                    ));
                } else {
                    spans.push(fg("\u{2588}".repeat(bar_w), Color::Rgb(0x5a, 0x5e, 0x76)));
                }
            } else {
                spans.push(Span::raw(" ".repeat(bar_w)));
            }
            if gap > 0 && i + 1 < n {
                spans.push(Span::raw(" ".repeat(gap)));
            }
        }

        buf_lines.push(Line::from(spans));
    }

    let mut label_spans: Vec<Span> = vec![Span::raw(" ".repeat(left_pad))];
    for (i, point) in trend.iter().enumerate() {
        let lbl = truncate(&point.label, bar_w);
        let lw = display_width(&lbl);
        let pad_r = bar_w.saturating_sub(lw);
        let pl = pad_r / 2;
        let pr = pad_r - pl;
        if point.is_current {
            label_spans.push(fg(
                format!("{}{}{}", " ".repeat(pl), lbl, " ".repeat(pr)),
                GOLD,
            ));
        } else {
            label_spans.push(dim(format!("{}{}{}", " ".repeat(pl), lbl, " ".repeat(pr))));
        }
        if gap > 0 && i + 1 < n {
            label_spans.push(Span::raw(" ".repeat(gap)));
        }
    }
    buf_lines.push(Line::from(label_spans));

    let paragraph = Paragraph::new(buf_lines);
    frame.render_widget(paragraph, inner);
}

fn draw_file_events(
    frame: &mut ratatui::Frame,
    events: &[FileEvent],
    title: &str,
    color: Color,
    area: Rect,
) {
    let block = titled_block(title, color);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if events.is_empty() {
        let msg = Paragraph::new("  None").style(Style::default().fg(DIM));
        frame.render_widget(msg, inner);
        return;
    }

    let lines: Vec<Line> = events
        .iter()
        .take(inner.height as usize)
        .map(|ev| {
            let date = fmt_date(ev.timestamp, "%m-%d");
            let max_w = (inner.width as usize).saturating_sub(12);
            let path = truncate_left(&ev.path, max_w);
            Line::from(vec![dim(format!(" {date} ")), fg(path, color)])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn sparkline_char(value: u64, max: u64) -> char {
    if value == 0 {
        return '\u{2581}';
    }
    const BLOCKS: [char; 8] = [
        '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}',
        '\u{2588}',
    ];
    let ratio = value as f64 / max as f64;
    let idx = (ratio * 7.0).round() as usize;
    BLOCKS[idx.clamp(1, 7)]
}

fn char_display_width(c: char) -> usize {
    if ('\u{1100}'..='\u{115F}').contains(&c)
        || ('\u{2E80}'..='\u{A4CF}').contains(&c)
        || ('\u{AC00}'..='\u{D7AF}').contains(&c)
        || ('\u{F900}'..='\u{FAFF}').contains(&c)
        || ('\u{FE10}'..='\u{FE6F}').contains(&c)
        || ('\u{FF01}'..='\u{FF60}').contains(&c)
        || ('\u{FFE0}'..='\u{FFE6}').contains(&c)
        || c > '\u{1F000}'
    {
        2
    } else {
        1
    }
}

fn display_width(s: &str) -> usize {
    s.chars().map(char_display_width).sum()
}

fn resample(data: &[u64], target_len: usize) -> Vec<u64> {
    if data.is_empty() || target_len == 0 {
        return vec![0; target_len];
    }
    if data.len() == target_len {
        return data.to_vec();
    }
    let mut result = vec![0u64; target_len];
    for (i, &v) in data.iter().enumerate() {
        let t = i * target_len / data.len();
        result[t.min(target_len - 1)] += v;
    }
    result
}

fn draw_help(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let sort_spans = SortMode::ALL.iter().flat_map(|&mode| {
        let style = if mode == app.sort_mode {
            Style::default()
                .fg(ACTIVE_SORT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(DIM)
        };
        [
            Span::styled(format!("[{}]", mode.key_hint()), style),
            Span::styled(format!("{} ", mode.label()), style),
        ]
    });

    let time_style = if app.is_time_filtered() {
        Style::default()
            .fg(HEADER_COLOR)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(DIM)
    };
    let (view_label, view_style) = if app.view_mode == ViewMode::Graph {
        (
            "[g]table",
            Style::default()
                .fg(HEADER_COLOR)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        ("[g]raph", Style::default().fg(DIM))
    };

    let mut extra: Vec<Span> = vec![
        dim(" \u{2502} "),
        Span::styled(format!("[t]{}", app.time_mode.label()), time_style),
        dim(" [\u{2190}\u{2192}]"),
        dim(" \u{2502} "),
        Span::styled(view_label, view_style),
        dim(" [↑↓]"),
        dim("scroll "),
        dim("[⏎]"),
        dim("detail "),
    ];
    if app.has_files_view {
        extra.push(dim("[V]"));
        extra.push(dim("files "));
    }
    extra.extend([dim("[q]"), dim("uit")]);
    let spans: Vec<Span> = sort_spans.chain(extra).collect();

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn truncate(s: &str, max_width: usize) -> String {
    let mut width = 0;
    let mut result = String::new();
    for c in s.chars() {
        let cw = char_display_width(c);
        if width + cw > max_width {
            result.push('\u{2026}');
            break;
        }
        result.push(c);
        width += cw;
    }
    result
}

fn truncate_left(s: &str, max_width: usize) -> String {
    let w = display_width(s);
    if w <= max_width {
        return s.to_string();
    }
    let mut current_width = 1; // reserve 1 for the ellipsis
    let mut split_byte = s.len();
    for (byte_pos, c) in s.char_indices().rev() {
        let cw = char_display_width(c);
        if current_width + cw > max_width {
            break;
        }
        current_width += cw;
        split_byte = byte_pos;
    }
    let mut result = String::from("\u{2026}");
    result.push_str(&s[split_byte..]);
    result
}

fn format_impact(impact: f64) -> String {
    if impact >= 1000.0 {
        format!("{:.1}k", impact / 1000.0)
    } else {
        format!("{impact:.0}")
    }
}

pub fn print_table(app: &App) {
    println!(
        "\x1b[1;36m{}\x1b[0m on \x1b[1;32m{}\x1b[0m",
        app.repo_info.name, app.repo_info.branch
    );
    println!(
        "{} commits \u{00b7} {} authors\n",
        Sep(app.total_commits),
        Sep(app.total_authors)
    );

    fn hdr(name: &str, sorted: bool, bad: bool, width: usize) -> String {
        let dot = if bad {
            "\x1b[31m\u{2022}\x1b[0m"
        } else {
            "\x1b[32m\u{2022}\x1b[0m"
        };
        let sort_sfx = if sorted {
            " \x1b[1;36m\u{25be}\x1b[0m"
        } else {
            ""
        };
        let visible = 2 + name.chars().count() + if sorted { 2 } else { 0 };
        let pad = width.saturating_sub(visible);
        format!(
            "{}{} \x1b[1m{}\x1b[0m{}",
            " ".repeat(pad),
            dot,
            name,
            sort_sfx
        )
    }

    fn impact_hdr(sorted: bool, width: usize) -> String {
        let sort_sfx = if sorted {
            " \x1b[1;36m\u{25be}\x1b[0m"
        } else {
            ""
        };
        let visible = 2 + 6 + if sorted { 2 } else { 0 };
        let pad = width.saturating_sub(visible);
        format!(
            "{}\x1b[1;38;2;255;255;255m\u{2726}\x1b[0m \x1b[1;38;2;137;221;255mImpact\x1b[0m{}",
            " ".repeat(pad),
            sort_sfx
        )
    }

    let sort = app.sort_mode;
    let mut header_line = format!("\x1b[1m{:>3}  {:<24}\x1b[0m", "#", "Author");
    header_line.push(' ');
    header_line.push_str(&impact_hdr(sort == SortMode::Impact, 10));
    header_line.push(' ');
    header_line.push_str(&hdr("Commits", sort == SortMode::Commits, false, 11));
    header_line.push(' ');
    header_line.push_str(&hdr("+Lines", sort == SortMode::LinesAdded, false, 10));
    header_line.push(' ');
    header_line.push_str(&hdr("-Lines", sort == SortMode::LinesRemoved, false, 10));
    header_line.push(' ');
    header_line.push_str(&hdr("Files", sort == SortMode::FilesChanged, false, 9));
    header_line.push(' ');
    header_line.push_str(&hdr("Noise%", sort == SortMode::Noise, true, 10));
    if app.show_ownership() {
        header_line.push(' ');
        header_line.push_str(&hdr("Own%", sort == SortMode::Ownership, false, 8));
    }
    println!("{header_line}");

    for (i, author) in app.sorted_authors().enumerate() {
        let rank = i + 1;

        let rank_color = match rank {
            1 => "\x1b[1;33m",
            2 => "\x1b[1;37m",
            3 => "\x1b[33m",
            _ => "\x1b[0m",
        };

        let noise = crate::app::noise_pct(author);
        let noise_color = if noise >= 60.0 {
            "\x1b[31m"
        } else if noise >= 30.0 {
            "\x1b[33m"
        } else {
            "\x1b[32m"
        };

        let own_col = if app.show_ownership() {
            format!(" \x1b[33m{:>8.1}\x1b[0m", author.ownership_pct)
        } else {
            String::new()
        };

        println!(
            "{}{:>3}\x1b[0m  {}{:<24}\x1b[0m \x1b[1;38;2;184;228;255m{:>10}\x1b[0m {:>11} \x1b[32m{:>10}\x1b[0m \x1b[31m{:>10}\x1b[0m {:>9} {}{:>10.1}\x1b[0m{}",
            rank_color,
            rank,
            rank_color,
            truncate(&author.display_name, 24),
            format_impact(author.impact),
            Sep(author.commits),
            Sep(author.lines_added),
            Sep(author.lines_removed),
            Sep(author.files_changed),
            noise_color,
            noise,
            own_col,
        );
    }
}
