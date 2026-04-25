use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState};

use super::super::theme::*;
use super::super::widgets::*;
use super::table::draw_header;
use crate::app::{App, FileDetailData, FileSortMode};
use crate::fmt::Sep;
use crate::identity_map::format_identity;

pub fn draw_files_view(frame: &mut ratatui::Frame, app: &App, file_table_state: &mut TableState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_header(frame, app, chunks[0]);
    draw_files_table(frame, app, file_table_state, chunks[1]);
    draw_files_help(frame, app, chunks[2]);
}

fn draw_files_table(
    frame: &mut ratatui::Frame,
    app: &App,
    file_table_state: &mut TableState,
    area: Rect,
) {
    const W_BAR: usize = 8;
    const W_COUNT: usize = 6;
    const W_COMMITS: usize = W_BAR + W_COUNT;
    const W_AUTHORS: usize = 9;
    const W_CHURN: usize = 9;
    const W_COUPLING: usize = 24;

    if app.data.file_rows.is_empty() {
        frame.render_widget(
            Paragraph::new("  No file data.").style(Style::default().fg(DIM)),
            area,
        );
        return;
    }

    let has_authors = app.has_file_authors();
    let has_churn = app.has_file_churn();
    let has_coupling = app.has_file_coupling();
    let sort = app.view.file_sort;

    let max_commits = app
        .data
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
        .data
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

    let table = Table::new(rows, widths).header(header).row_highlight_style(
        Style::default()
            .bg(Color::Rgb(0x44, 0x48, 0x5c))
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(table, area, file_table_state);
}

fn draw_files_help(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let has_authors = app.has_file_authors();
    let has_churn = app.has_file_churn();
    let has_coupling = app.has_file_coupling();
    let sort = app.view.file_sort;

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

pub fn draw_file_detail_view(frame: &mut ratatui::Frame, app: &App) {
    let detail = match &app.cache.file_detail {
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
    draw_file_detail_coupling(frame, detail, app.view.file_detail_scroll, chunks[2]);

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

pub fn draw_questionnaire(frame: &mut ratatui::Frame, app: &App) {
    let q = match &app.view.questionnaire {
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
