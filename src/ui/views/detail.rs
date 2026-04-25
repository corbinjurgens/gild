use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::theme::*;
use super::super::widgets::*;
use crate::app::{App, FileEvent, TrendPoint};
use crate::fmt::{fmt_date, Sep};

pub fn draw_detail_view(frame: &mut ratatui::Frame, app: &App) {
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

    let time_label = app.time_label();

    let name_line = Line::from(vec![
        bold_fg(a.display_name.clone(), GOLD),
        Span::raw("  "),
        dim(time_label.as_str()),
    ]);

    let stats_line = if a.commits == 0 {
        Line::from(vec![Span::styled(
            format!("No activity in {time_label}"),
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

    draw_detail_files(
        frame,
        &detail.top_files,
        app.view.detail_scroll,
        top_cols[0],
    );
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
                    spans.push(fg("\u{2591}", Color::Rgb(0x30, 0x32, 0x40)));
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

    const HOUR_LABELS: [&str; 8] = [" 0 ", " 3 ", " 6 ", " 9 ", "12 ", "15 ", "18 ", "21 "];
    let mut header_spans = vec![Span::raw("      ")];
    for label in HOUR_LABELS {
        header_spans.push(dim(label));
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
