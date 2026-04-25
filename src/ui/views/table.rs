use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use super::super::theme::*;
use super::super::widgets::*;
use crate::app::{App, SortMode, ViewMode};
use crate::fmt::{fmt_date, Sep};

pub fn draw_header(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let block = gild_header_block();

    let mut line2 = vec![
        fg(Sep(app.cache.filtered_commits).to_string(), Color::White),
        dim(" commits  "),
        fg(Sep(app.cache.authors.len()).to_string(), Color::White),
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
            bold_fg(app.data.repo_info.name.clone(), HEADER_COLOR),
            dim(" on "),
            bold_fg(app.data.repo_info.branch.clone(), ACTIVE_SORT),
        ]),
        Line::from(line2),
    ];

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
    sparkle_top_border(frame, area);
}

pub fn draw_table(frame: &mut ratatui::Frame, app: &App, table_state: &mut TableState, area: Rect) {
    const W_COMMITS: usize = 11;
    const W_LINES: usize = 11;
    const W_FILES: usize = 10;
    const W_IMPACT: usize = 10;
    const W_NOISE: usize = 10;
    const W_OWN: usize = 9;

    let sort = app.view.sort_mode;
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

pub fn draw_graph(frame: &mut ratatui::Frame, app: &App, area: Rect) {
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
    let spark_width = (area.width as usize).saturating_sub(name_col + total_col + 2);

    let has_label_row = !graph.labels.is_empty() && spark_width > 4;
    let reserved = if has_label_row { 2 } else { 1 };
    let max_visible = (area.height as usize).saturating_sub(reserved).max(1);

    let total_rows = graph.rows.len();
    let display_rows: Vec<crate::app::GraphRow> = if total_rows <= max_visible {
        graph.rows.clone()
    } else {
        let top = max_visible.saturating_sub(1).max(1);
        let mut rows: Vec<crate::app::GraphRow> = graph.rows[..top].to_vec();
        let bucket_count = graph.rows[0].data.len();
        let mut others_data = vec![0u64; bucket_count];
        for r in &graph.rows[top..] {
            for (slot, &v) in others_data.iter_mut().zip(r.data.iter()) {
                *slot += v;
            }
        }
        rows.push(crate::app::GraphRow {
            name: format!("others ({})", total_rows - top),
            data: others_data,
            color: DIM,
        });
        rows
    };

    let resampled_rows: Vec<Vec<u64>> = display_rows
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

    if has_label_row {
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

    let empty_cell = Color::Rgb(0x30, 0x32, 0x40);

    for (i, row) in display_rows.iter().enumerate() {
        let total: u64 = row.data.iter().sum();

        let name_display = truncate(&row.name, name_col - 2);
        let name_pad = (name_col - 1).saturating_sub(display_width(&name_display));

        let mut spans: Vec<Span> = Vec::with_capacity(spark_width + 2);
        spans.push(fg(
            format!(" {}{}", name_display, " ".repeat(name_pad)),
            row.color,
        ));

        for &v in &resampled_rows[i] {
            let color = if v == 0 {
                empty_cell
            } else {
                let t = (v as f64 / global_max as f64).clamp(0.0, 1.0).sqrt();
                gradient_color(&BAR_STOPS, t)
            };
            spans.push(Span::styled(" ", Style::default().bg(color)));
        }

        spans.push(dim(format!(" {:>7}", Sep(total as usize))));
        lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

pub fn draw_help(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let sort_spans = SortMode::ALL.iter().flat_map(|&mode| {
        let style = if mode == app.view.sort_mode {
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
    let (view_label, view_style) = if app.view.view_mode == ViewMode::Graph {
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
        Span::styled(format!("[t]{}", app.view.time_mode.label()), time_style),
        dim(" [\u{2190}\u{2192}]"),
        dim(" \u{2502} "),
        Span::styled(view_label, view_style),
        dim(" [↑↓]"),
        dim("scroll "),
        dim("[⏎]"),
        dim("detail "),
    ];
    if app.data.has_files_view {
        extra.push(dim("[V]"));
        extra.push(dim("files "));
    }
    extra.extend([dim("[q]"), dim("uit")]);
    let spans: Vec<Span> = sort_spans.chain(extra).collect();

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
