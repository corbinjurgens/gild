use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::Theme;

pub const GOLD: Color = Color::Rgb(255, 215, 0);
pub const GILD_AMBER: Color = Color::Rgb(0xf1, 0x8f, 0x3b);
pub const GILD_SPARK: Color = Color::Rgb(0xff, 0xcb, 0x7a);
pub const SILVER: Color = Color::Rgb(192, 192, 192);
pub const BRONZE: Color = Color::Rgb(205, 127, 50);
pub const ADDED: Color = Color::Rgb(80, 250, 123);
pub const REMOVED: Color = Color::Rgb(255, 85, 85);
pub const HEADER_COLOR: Color = Color::Rgb(139, 233, 253);
pub const DIM: Color = Color::Rgb(108, 118, 148);
pub const BORDER_TEAL: Color = Color::Rgb(0x4a, 0x93, 0x8f);
pub const ACTIVE_SORT: Color = Color::Rgb(80, 250, 123);
pub const EMPTY_WEEKDAY: Color = Color::Rgb(255, 184, 108);

pub type Stop = (f64, (u8, u8, u8));

pub const BAR_STOPS: [Stop; 4] = [
    (0.00, (0x4a, 0x5a, 0x7a)),
    (0.35, (0xcd, 0x7f, 0x32)),
    (0.70, (0xf1, 0x8f, 0x3b)),
    (1.00, (0xfb, 0xbf, 0x24)),
];

pub const SPARKLE_STOPS: [Stop; 4] = [
    (0.00, (0x5a, 0x9c, 0xd6)),
    (0.40, (0x7b, 0xb8, 0xe8)),
    (0.70, (0xa0, 0xd4, 0xf4)),
    (1.00, (0xd5, 0xec, 0xff)),
];

pub const OWNERSHIP_STOPS: [Stop; 5] = [
    (0.00, (0x62, 0x72, 0xa4)),
    (0.30, (0x8b, 0xe9, 0xfd)),
    (0.50, (0x50, 0xfa, 0x7b)),
    (0.70, (0xff, 0xb8, 0x6c)),
    (1.00, (0xff, 0xd7, 0x00)),
];

pub const NOISE_STOPS: [Stop; 4] = [
    (0.00, (0x50, 0xfa, 0x7b)),
    (0.30, (0xf1, 0xfa, 0x8c)),
    (0.60, (0xff, 0xb8, 0x6c)),
    (1.00, (0xff, 0x55, 0x55)),
];

const FORCED_BG: Color = Color::Rgb(0x1e, 0x1e, 0x2e);

pub fn apply_readable_theme(frame: &mut ratatui::Frame) {
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

pub fn simplify_color(c: Color) -> Color {
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

pub fn draw_theme_picker(frame: &mut ratatui::Frame, app: &crate::app::App) {
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

    let normal_label = if app.view.theme == Theme::Normal {
        "[N] Normal  \u{2713}"
    } else {
        "[N] Normal"
    };
    let readable_label = if app.view.theme == Theme::Readable {
        "[R] Readable  \u{2713}"
    } else {
        "[R] Readable"
    };

    let hint = match app.view.theme {
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
                if app.view.theme == Theme::Normal {
                    highlight
                } else {
                    muted
                },
            ),
            Span::raw("      "),
            Span::styled(
                readable_label,
                if app.view.theme == Theme::Readable {
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

pub fn sparkle_top_border(frame: &mut ratatui::Frame, area: Rect) {
    if area.height < 2 || area.width < 24 {
        return;
    }
    let buf = frame.buffer_mut();
    let top_y = area.y;
    let bottom_y = area.y + area.height - 1;
    let w = area.width;

    let top = [
        area.x + w / 3,
        area.x + w / 2 + 2,
        area.x + (w * 2) / 3,
        area.x + (w * 5) / 6,
    ];
    for &x in &top {
        place_sparkle(buf, x, top_y);
    }

    let bottom = [area.x + w / 5, area.x + w / 2, area.x + (w * 4) / 5];
    for &x in &bottom {
        place_sparkle(buf, x, bottom_y);
    }
}

pub fn gild_header_block() -> Block<'static> {
    use ratatui::symbols::border;
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
