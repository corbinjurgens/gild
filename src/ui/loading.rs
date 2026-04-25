use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Terminal;
use std::io;
use std::sync::mpsc;
use std::time::Duration;

use super::event_loop;
use super::theme::*;
use crate::app::App;
use crate::fmt::Sep;

#[derive(Clone, Copy)]
pub struct LoadStep {
    pub label: &'static str,
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

    let phase_label: &str = if is_commit_phase {
        if state.commits_total == 0 {
            "Reading repository\u{2026}"
        } else if state.new_commits > 0 {
            "Scanning commits"
        } else {
            "Loading from cache"
        }
    } else if !state.addon_label.is_empty() {
        state.addon_label
    } else if let Some(step) = state.steps.get(state.current_step) {
        step.label
    } else {
        "Working\u{2026}"
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

    let steps_rows = state.steps.len().max(1) as u16;
    let mut content_h: u16 = 1 + 1 + 1;
    if count_label.is_some() {
        content_h += 1;
    }
    content_h += 1;
    if total > 0 {
        content_h += 1;
    }
    content_h += 1;
    content_h += steps_rows;
    if show_first_run_note {
        content_h += 2;
    }
    if show_threads_hint {
        content_h += 1;
    }

    let start_y = area.y + area.height.saturating_sub(content_h) / 2;
    let mut y = start_y;

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
        y += 2;
    }

    if y < area.y + area.height {
        let mut spans = vec![Span::styled(
            phase_label,
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

    y += 1;

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

    y += 1;

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
                Span::styled(step.label, label_style),
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
