use crate::app::{App, SortMode, ViewMode};
use anyhow::Result;
use chrono::DateTime;
use crossterm::event::{self, Event};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::Terminal;
use std::io;
use std::time::Duration;

const GOLD: Color = Color::Rgb(255, 215, 0);
const SILVER: Color = Color::Rgb(192, 192, 192);
const BRONZE: Color = Color::Rgb(205, 127, 50);
const ADDED: Color = Color::Rgb(80, 250, 123);
const REMOVED: Color = Color::Rgb(255, 85, 85);
const HEADER_COLOR: Color = Color::Rgb(139, 233, 253);
const DIM: Color = Color::Rgb(108, 118, 148);
const BAR_FULL: Color = Color::Rgb(189, 147, 249);
const BAR_EMPTY: Color = Color::Rgb(68, 71, 90);
const ACTIVE_SORT: Color = Color::Rgb(80, 250, 123);

const HEAT_COLORS: [Color; 5] = [
    Color::Rgb(68, 71, 90),
    Color::Rgb(54, 120, 88),
    Color::Rgb(57, 170, 96),
    Color::Rgb(62, 210, 105),
    Color::Rgb(80, 250, 123),
];

pub fn run(app: &mut App) -> Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, app);

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    result
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
}

fn draw_header(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(Span::styled(
            " gild ",
            Style::default()
                .fg(BAR_FULL)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM));

    let mut line2 = vec![
        Span::styled(
            format_number(app.filtered_commits),
            Style::default().fg(Color::White),
        ),
        Span::styled(" commits  ", Style::default().fg(DIM)),
        Span::styled(
            format_number(app.authors.len()),
            Style::default().fg(Color::White),
        ),
        Span::styled(" authors", Style::default().fg(DIM)),
    ];

    if app.time_mode != crate::app::TimeMode::All {
        line2.push(Span::styled("  \u{25c0} ", Style::default().fg(DIM)));
        line2.push(Span::styled(
            app.time_label(),
            Style::default()
                .fg(HEADER_COLOR)
                .add_modifier(Modifier::BOLD),
        ));
        line2.push(Span::styled(" \u{25b6}", Style::default().fg(DIM)));
    } else {
        let first_ts = app.sorted_authors().iter().map(|a| a.first_commit).min().unwrap_or(0);
        let last_ts = app.sorted_authors().iter().map(|a| a.last_commit).max().unwrap_or(0);
        line2.push(Span::styled("  ", Style::default().fg(DIM)));
        line2.push(Span::styled(format_date(first_ts), Style::default().fg(Color::White)));
        line2.push(Span::styled(" \u{2192} ", Style::default().fg(DIM)));
        line2.push(Span::styled(format_date(last_ts), Style::default().fg(Color::White)));
    }

    let text = vec![
        Line::from(vec![
            Span::styled(
                &app.repo_info.name,
                Style::default()
                    .fg(HEADER_COLOR)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" on ", Style::default().fg(DIM)),
            Span::styled(
                &app.repo_info.branch,
                Style::default()
                    .fg(ACTIVE_SORT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(line2),
    ];

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_table(frame: &mut ratatui::Frame, app: &App, table_state: &mut TableState, area: Rect) {
    let sorted = app.sorted_authors();
    let max_value = sorted
        .first()
        .map(|a| app.sort_value(a).max(1))
        .unwrap_or(1) as f64;

    let mut header_cells = vec![
        Cell::from(" # "),
        Cell::from("Author"),
        Cell::from("Commits"),
        Cell::from("  +Lines"),
        Cell::from("  -Lines"),
        Cell::from(" Files"),
        Cell::from("Impact"),
    ];
    if app.has_ownership {
        header_cells.push(Cell::from("  Own%"));
    }
    header_cells.push(Cell::from("Share"));

    let header = Row::new(header_cells).style(
        Style::default()
            .fg(DIM)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = sorted
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

            let value = app.sort_value(author) as f64;
            let pct = (value / max_value * 100.0).max(0.0);
            let bar_filled = ((pct / 100.0) * 10.0).round() as usize;
            let bar_empty = 10usize.saturating_sub(bar_filled);

            let net = author.lines_added as i64 - author.lines_removed as i64;
            let net_color = if net >= 0 { ADDED } else { REMOVED };

            let mut cells = vec![
                Cell::from(format!("{:>2} ", rank))
                    .style(Style::default().fg(rank_color).add_modifier(Modifier::BOLD)),
                Cell::from(truncate(&author.display_name, 24))
                    .style(Style::default().fg(rank_color)),
                Cell::from(format!("{:>7}", format_number(author.commits)))
                    .style(Style::default().fg(Color::White)),
                Cell::from(format!("{:>8}", format_number(author.lines_added)))
                    .style(Style::default().fg(ADDED)),
                Cell::from(format!("{:>8}", format_number(author.lines_removed)))
                    .style(Style::default().fg(REMOVED)),
                Cell::from(format!("{:>6}", format_number(author.files_changed)))
                    .style(Style::default().fg(Color::White)),
                Cell::from(format!("{:>7}", format_impact(author.impact)))
                    .style(Style::default().fg(HEADER_COLOR)),
            ];

            if app.has_ownership {
                cells.push(
                    Cell::from(format!("{:>5.1}", author.ownership_pct))
                        .style(Style::default().fg(Color::Rgb(255, 184, 108))),
                );
            }

            cells.push(Cell::from(Line::from(vec![
                Span::styled(
                    "\u{2588}".repeat(bar_filled),
                    Style::default().fg(BAR_FULL),
                ),
                Span::styled(
                    "\u{2591}".repeat(bar_empty),
                    Style::default().fg(BAR_EMPTY),
                ),
                Span::styled(
                    format!(" {:>3.0}%", pct),
                    Style::default().fg(net_color),
                ),
            ])));

            Row::new(cells)
        })
        .collect();

    let mut widths = vec![
        Constraint::Length(4),
        Constraint::Length(25),
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(7),
        Constraint::Length(8),
    ];
    if app.has_ownership {
        widths.push(Constraint::Length(6));
    }
    widths.push(Constraint::Min(16));

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 42, 54))
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(table, area, table_state);
}

fn draw_graph(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let graph = app.graph_data();
    if graph.rows.is_empty() {
        let msg = Paragraph::new("  No data for this period.")
            .style(Style::default().fg(DIM));
        frame.render_widget(msg, area);
        return;
    }

    let name_col = 20usize;
    let total_col = 9usize;
    let spark_width = (area.width as usize).saturating_sub(name_col + total_col + 2).min(60);

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
        let last = graph.labels.last().unwrap();
        let pad = spark_width.saturating_sub(first.len() + last.len());
        lines.push(Line::from(vec![
            Span::styled(format!(" {:>width$}", "", width = name_col - 1), Style::default()),
            Span::styled(first.clone(), Style::default().fg(DIM)),
            Span::styled(format!("{:>width$}", last, width = pad + last.len()), Style::default().fg(DIM)),
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
            Span::styled(
                format!(" {}{}", name_display, " ".repeat(name_pad)),
                Style::default().fg(row.color),
            ),
            Span::styled(spark_str, Style::default().fg(row.color)),
            Span::styled(
                format!(" {:>7}", format_number(total as usize)),
                Style::default().fg(DIM),
            ),
        ];
        lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

fn draw_detail_view(frame: &mut ratatui::Frame, app: &App) {
    let detail = match app.detail_data() {
        Some(d) => d,
        None => return,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let a = &detail.author;
    let ownership_line = if a.ownership_lines > 0 {
        format!(
            "  Owns {} lines ({:.1}% of codebase)",
            format_number(a.ownership_lines),
            a.ownership_pct
        )
    } else {
        String::new()
    };

    let header_block = Block::default()
        .title(Span::styled(
            " gild ",
            Style::default().fg(BAR_FULL).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM));

    let header_text = vec![
        Line::from(vec![
            Span::styled(
                &a.display_name,
                Style::default().fg(GOLD).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(format_number(a.commits), Style::default().fg(Color::White)),
            Span::styled(" commits  ", Style::default().fg(DIM)),
            Span::styled(format!("+{}", format_number(a.lines_added)), Style::default().fg(ADDED)),
            Span::styled("  ", Style::default().fg(DIM)),
            Span::styled(format!("-{}", format_number(a.lines_removed)), Style::default().fg(REMOVED)),
            Span::styled("  ", Style::default().fg(DIM)),
            Span::styled(format!("{} files", format_number(a.files_changed)), Style::default().fg(Color::White)),
            Span::styled("  impact ", Style::default().fg(DIM)),
            Span::styled(format_impact(a.impact), Style::default().fg(HEADER_COLOR)),
            Span::styled(&ownership_line, Style::default().fg(Color::Rgb(255, 184, 108))),
        ]),
    ];

    let header = Paragraph::new(header_text).block(header_block);
    frame.render_widget(header, chunks[0]);

    let content_area = chunks[1];
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(content_area);

    draw_detail_files(frame, &detail.top_files, app.detail_scroll, content_chunks[0]);
    draw_detail_activity(frame, &detail.activity, content_chunks[1]);

    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Esc]", Style::default().fg(DIM)),
        Span::styled("back ", Style::default().fg(DIM)),
        Span::styled("[↑↓]", Style::default().fg(DIM)),
        Span::styled("scroll ", Style::default().fg(DIM)),
        Span::styled("[q]", Style::default().fg(DIM)),
        Span::styled("uit", Style::default().fg(DIM)),
    ]));
    frame.render_widget(help, chunks[2]);
}

fn draw_detail_files(frame: &mut ratatui::Frame, files: &[(String, usize)], scroll: usize, area: Rect) {
    let block = Block::default()
        .title(Span::styled(
            " Top Files ",
            Style::default().fg(HEADER_COLOR).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if files.is_empty() {
        let msg = Paragraph::new("  No file data (try clearing cache)")
            .style(Style::default().fg(DIM));
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
            let bar_width: usize = 6;
            let filled = (*count as f64 / max_count as f64 * bar_width as f64).round() as usize;
            let bar: String = "\u{2588}".repeat(filled)
                + &"\u{2591}".repeat(bar_width.saturating_sub(filled));

            let max_path_len = (inner.width as usize).saturating_sub(bar_width + 8);
            let display_path = truncate_left(path, max_path_len);

            Line::from(vec![
                Span::styled(
                    format!(" {:>4} ", count),
                    Style::default().fg(Color::White),
                ),
                Span::styled(bar, Style::default().fg(BAR_FULL)),
                Span::styled(
                    format!(" {}", display_path),
                    Style::default().fg(HEADER_COLOR),
                ),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn draw_detail_activity(frame: &mut ratatui::Frame, activity: &[[usize; 24]; 7], area: Rect) {
    let block = Block::default()
        .title(Span::styled(
            " Activity Pattern ",
            Style::default().fg(HEADER_COLOR).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let max_val = activity
        .iter()
        .flat_map(|row| row.iter())
        .copied()
        .max()
        .unwrap_or(0);

    if max_val == 0 {
        let msg = Paragraph::new("  No activity data")
            .style(Style::default().fg(DIM));
        frame.render_widget(msg, inner);
        return;
    }

    let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let mut lines: Vec<Line> = Vec::new();

    let hour_labels: Vec<String> = (0..24).step_by(3).map(|h| format!("{:>2}", h)).collect();
    let mut header_spans = vec![Span::styled("      ", Style::default())];
    for label in &hour_labels {
        header_spans.push(Span::styled(
            format!("{} ", label),
            Style::default().fg(DIM),
        ));
    }
    lines.push(Line::from(header_spans));
    lines.push(Line::from(""));

    for (day_idx, day_name) in days.iter().enumerate() {
        let mut spans = vec![Span::styled(
            format!(" {} ", day_name),
            Style::default().fg(Color::White),
        )];
        spans.push(Span::styled("  ", Style::default()));

        for hour in 0..24 {
            let val = activity[day_idx][hour];
            let intensity = if max_val > 0 {
                (val as f64 / max_val as f64 * 4.0).round() as usize
            } else {
                0
            };
            let color = HEAT_COLORS[intensity.min(4)];
            spans.push(Span::styled("\u{2588}", Style::default().fg(color)));
        }

        let total: usize = activity[day_idx].iter().sum();
        spans.push(Span::styled(
            format!(" {:>3}", total),
            Style::default().fg(DIM),
        ));

        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));

    let total_all: usize = activity.iter().flat_map(|r| r.iter()).sum();
    let mut hour_totals = vec![0usize; 24];
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
        Span::styled(" Total: ", Style::default().fg(DIM)),
        Span::styled(format!("{}", total_all), Style::default().fg(Color::White)),
        Span::styled("  Peak: ", Style::default().fg(DIM)),
        Span::styled(format!("{}:00", peak_hour), Style::default().fg(ACTIVE_SORT)),
    ]));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn sparkline_char(value: u64, max: u64) -> char {
    if value == 0 {
        return '\u{2581}';
    }
    const BLOCKS: [char; 8] = [
        '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}',
        '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}',
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
    let spans: Vec<Span> = SortMode::ALL
        .iter()
        .flat_map(|&mode| {
            let is_active = mode == app.sort_mode;
            let style = if is_active {
                Style::default()
                    .fg(ACTIVE_SORT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DIM)
            };
            vec![
                Span::styled(format!("[{}]", mode.key_hint()), style),
                Span::styled(format!("{} ", mode.label()), style),
            ]
        })
        .chain(vec![
            Span::styled(" \u{2502} ", Style::default().fg(DIM)),
            Span::styled(
                format!("[t]{}", app.time_mode.label()),
                if app.time_mode != crate::app::TimeMode::All {
                    Style::default().fg(HEADER_COLOR).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(DIM)
                },
            ),
            Span::styled(
                " [\u{2190}\u{2192}]",
                Style::default().fg(DIM),
            ),
            Span::styled(" \u{2502} ", Style::default().fg(DIM)),
            Span::styled(
                if app.view_mode == ViewMode::Graph {
                    "[g]table"
                } else {
                    "[g]raph"
                },
                if app.view_mode == ViewMode::Graph {
                    Style::default().fg(HEADER_COLOR).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(DIM)
                },
            ),
            Span::styled(" ", Style::default().fg(DIM)),
            Span::styled("[↑↓]", Style::default().fg(DIM)),
            Span::styled("scroll ", Style::default().fg(DIM)),
            Span::styled("[⏎]", Style::default().fg(DIM)),
            Span::styled("detail ", Style::default().fg(DIM)),
            Span::styled("[q]", Style::default().fg(DIM)),
            Span::styled("uit", Style::default().fg(DIM)),
        ])
        .collect();

    let help = Paragraph::new(Line::from(spans));
    frame.render_widget(help, area);
}

fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn format_date(timestamp: i64) -> String {
    DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%b %Y").to_string())
        .unwrap_or_else(|| "unknown".to_string())
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
    let mut result = String::from("\u{2026}");
    let mut current_width = 1;
    for c in s.chars().rev() {
        let cw = char_display_width(c);
        if current_width + cw > max_width {
            break;
        }
        result.insert(1, c);
        current_width += cw;
    }
    result
}

fn format_impact(impact: f64) -> String {
    if impact >= 1000.0 {
        format!("{:.1}k", impact / 1000.0)
    } else {
        format!("{:.0}", impact)
    }
}

pub fn print_table(app: &App) {
    let sorted = app.sorted_authors();
    let max_value = sorted
        .first()
        .map(|a| app.sort_value(a).max(1))
        .unwrap_or(1) as f64;

    println!(
        "\x1b[1;36m{}\x1b[0m on \x1b[1;32m{}\x1b[0m",
        app.repo_info.name, app.repo_info.branch
    );
    println!(
        "{} commits \u{00b7} {} authors\n",
        format_number(app.total_commits),
        format_number(app.total_authors)
    );

    let own_header = if app.has_ownership { " Own%" } else { "" };
    println!(
        "\x1b[1m{:>3}  {:<24} {:>7} {:>9} {:>9} {:>6} {:>7}{:>6}  {}\x1b[0m",
        "#", "Author", "Commits", "+Lines", "-Lines", "Files", "Impact", own_header, "Share"
    );

    for (i, author) in sorted.iter().enumerate() {
        let rank = i + 1;
        let value = app.sort_value(author) as f64;
        let pct = (value / max_value * 100.0).max(0.0);
        let filled = ((pct / 100.0) * 10.0).round() as usize;
        let empty = 10usize.saturating_sub(filled);

        let rank_color = match rank {
            1 => "\x1b[1;33m",
            2 => "\x1b[1;37m",
            3 => "\x1b[33m",
            _ => "\x1b[0m",
        };

        let own_col = if app.has_ownership {
            format!(" \x1b[33m{:>5.1}\x1b[0m", author.ownership_pct)
        } else {
            String::new()
        };

        println!(
            "{}{:>3}\x1b[0m  {}{:<24}\x1b[0m {:>7} \x1b[32m{:>9}\x1b[0m \x1b[31m{:>9}\x1b[0m {:>6} \x1b[36m{:>7}\x1b[0m{}  \x1b[35m{}\x1b[90m{}\x1b[0m {:>3.0}%",
            rank_color,
            rank,
            rank_color,
            truncate(&author.display_name, 24),
            format_number(author.commits),
            format_number(author.lines_added),
            format_number(author.lines_removed),
            format_number(author.files_changed),
            format_impact(author.impact),
            own_col,
            "\u{2588}".repeat(filled),
            "\u{2591}".repeat(empty),
            pct,
        );
    }
}
