use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::theme::*;

pub fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
    (a as f64 + (b as f64 - a as f64) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

pub fn scale_color(base: Color, factor: f64) -> Color {
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

pub fn dim_factor(value: f64, min: f64, max: f64) -> f64 {
    if max <= min {
        return 1.0;
    }
    let t = ((value - min) / (max - min)).clamp(0.0, 1.0);
    0.9 + 0.1 * t
}

pub fn gradient_color(stops: &[Stop], t: f64) -> Color {
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

pub fn dim<'a, T: Into<std::borrow::Cow<'a, str>>>(text: T) -> Span<'a> {
    Span::styled(text, Style::default().fg(DIM))
}

pub fn fg<'a, T: Into<std::borrow::Cow<'a, str>>>(text: T, color: Color) -> Span<'a> {
    Span::styled(text, Style::default().fg(color))
}

pub fn bold_fg<'a, T: Into<std::borrow::Cow<'a, str>>>(text: T, color: Color) -> Span<'a> {
    Span::styled(
        text,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

pub fn impact_header(is_sorted: bool, width: usize) -> Cell<'static> {
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

pub fn metric_header(
    name: &'static str,
    is_sorted: bool,
    is_bad: bool,
    width: usize,
) -> Cell<'static> {
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
        Span::styled(name, name_style),
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

pub fn titled_block(title: &str, title_color: Color) -> Block<'_> {
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

pub fn file_path_cell(path: &str, max_width: usize) -> Cell<'static> {
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

pub fn coupling_cell(partner: &str, score: f64, max_width: usize) -> Cell<'static> {
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

pub fn char_display_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

pub fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

pub fn truncate(s: &str, max_width: usize) -> String {
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

pub fn truncate_left(s: &str, max_width: usize) -> String {
    let w = display_width(s);
    if w <= max_width {
        return s.to_string();
    }
    let mut current_width = 1;
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

pub fn format_impact(impact: f64) -> String {
    if impact >= 1000.0 {
        format!("{:.1}k", impact / 1000.0)
    } else {
        format!("{impact:.0}")
    }
}

pub fn resample(data: &[u64], target_len: usize) -> Vec<u64> {
    if data.is_empty() || target_len == 0 {
        return vec![0; target_len];
    }
    if data.len() == target_len {
        return data.to_vec();
    }
    if target_len > data.len() {
        let mut result = vec![0u64; target_len];
        for (j, slot) in result.iter_mut().enumerate() {
            let i = j * data.len() / target_len;
            *slot = data[i.min(data.len() - 1)];
        }
        return result;
    }
    let mut result = vec![0u64; target_len];
    for (i, &v) in data.iter().enumerate() {
        let t = i * target_len / data.len();
        result[t.min(target_len - 1)] += v;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_identity() {
        assert_eq!(resample(&[1, 2, 3], 3), vec![1, 2, 3]);
    }

    #[test]
    fn resample_downsample_aggregates() {
        // 6 → 3, two-into-one: each output is the sum of two inputs
        assert_eq!(resample(&[1, 2, 3, 4, 5, 6], 3), vec![3, 7, 11]);
    }

    #[test]
    fn resample_upsample_stretches_no_gaps() {
        // 3 → 9: each input replicated across 3 output cells, no zero gaps
        assert_eq!(resample(&[1, 2, 3], 9), vec![1, 1, 1, 2, 2, 2, 3, 3, 3]);
    }

    #[test]
    fn resample_upsample_no_zero_gap_for_nonzero_input() {
        // The key invariant: if all input is nonzero, no output cell is zero.
        // (regression test for the =____=____ heatmap bug)
        let out = resample(&[5, 5, 5, 5, 5], 100);
        assert!(
            out.iter().all(|&v| v > 0),
            "found zero gap in upsample: {out:?}"
        );
    }
}
