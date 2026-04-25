mod loading;
mod theme;
mod views;
mod widgets;

pub use loading::{run_with_loading, LoadMsg, LoadStep};

use anyhow::Result;
use crossterm::event::{self, Event};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::TableState;
use ratatui::Terminal;
use std::time::Duration;

use crate::app::{App, SortMode, Theme, ViewMode};
use crate::fmt::Sep;
use theme::*;
use widgets::*;

fn event_loop(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    let mut table_state = TableState::default();
    table_state.select(Some(0));
    let mut file_table_state = TableState::default();

    loop {
        table_state.select(Some(app.view.selected));
        file_table_state.select(Some(app.view.file_selected));

        terminal.draw(|frame| draw(frame, app, &mut table_state, &mut file_table_state))?;

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

fn draw(
    frame: &mut ratatui::Frame,
    app: &App,
    table_state: &mut TableState,
    file_table_state: &mut TableState,
) {
    match app.view.view_mode {
        ViewMode::Detail => views::detail::draw_detail_view(frame, app),
        ViewMode::Files => views::files::draw_files_view(frame, app, file_table_state),
        ViewMode::FileDetail => views::files::draw_file_detail_view(frame, app),
        ViewMode::Table | ViewMode::Graph => {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(4),
                    Constraint::Min(5),
                    Constraint::Length(1),
                ])
                .split(frame.area());

            views::table::draw_header(frame, app, chunks[0]);
            match app.view.view_mode {
                ViewMode::Graph => views::table::draw_graph(frame, app, chunks[1]),
                _ => views::table::draw_table(frame, app, table_state, chunks[1]),
            }
            views::table::draw_help(frame, app, chunks[2]);
        }
    }

    if app.view.theme == Theme::Readable {
        apply_readable_theme(frame);
    }

    if app.view.show_theme_picker {
        draw_theme_picker(frame, app);
    }

    if app.view.questionnaire.is_some() {
        views::files::draw_questionnaire(frame, app);
    }
}

pub fn print_table(app: &App) {
    println!(
        "\x1b[1;36m{}\x1b[0m on \x1b[1;32m{}\x1b[0m",
        app.data.repo_info.name, app.data.repo_info.branch
    );
    println!(
        "{} commits \u{00b7} {} authors\n",
        Sep(app.data.total_commits),
        Sep(app.data.total_authors)
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

    let sort = app.view.sort_mode;
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
