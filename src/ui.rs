use std::io;

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Sparkline, Wrap},
    Frame, Terminal,
};

use crate::app::App;

pub type Term = Terminal<CrosstermBackend<io::Stdout>>;

pub fn setup_terminal() -> anyhow::Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

pub fn restore_terminal(terminal: &mut Term) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(4),
            Constraint::Length(8),
        ])
        .split(frame.size());

    render_header(frame, chunks[0], app);
    render_logs(frame, chunks[1], app);
    render_timeline(frame, chunks[2], app);
    render_status(frame, chunks[3], app);

    if app.show_help {
        let area = centered_rect(70, 60, frame.size());
        frame.render_widget(Clear, area);
        let help = Paragraph::new(vec![
            Line::from("Keys:"),
            Line::from(" q/ctrl-c quit | space pause/resume | g/end go live"),
            Line::from(" arrows/pgup/pgdn scroll | left/right timeline"),
            Line::from(" / filter (Enter apply, Esc cancel) | R toggle regex | F/C clear"),
            Line::from(" 1=info 2=warn 3=error level toggles | n/p next/prev error"),
            Line::from(" b add bookmark | ]/[ next/prev bookmark"),
            Line::from(" Filters match level/target/timestamp/message."),
            Line::from(
                " Timeline: red=error, yellow=warn, white=info; ^ cursor, * bookmark, # overlap.",
            ),
            Line::from(""),
            Line::from("While scrolling up we auto-pause; queued lines show as +N."),
            Line::from("Timeline cursor moves with left/right; markers show bookmarks and cursor."),
            Line::from("Press ? or Esc to close this help."),
        ])
        .block(
            Block::default()
                .title("Help")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta)),
        );
        frame.render_widget(help, area);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let filter_display = match &app.filters().text {
        Some(t) if !t.is_empty() => {
            if app.filters().regex_mode {
                format!("/{t}/")
            } else {
                format!("\"{t}\"")
            }
        }
        _ => "none".to_string(),
    };
    let level_display = format!(
        "1={}  2={}  3={}",
        if app.filters().info { "INFO" } else { "info" },
        if app.filters().warn { "WARN" } else { "warn" },
        if app.filters().error {
            "ERROR"
        } else {
            "error"
        },
    );
    let input_status = match app.input_mode() {
        crate::filters::InputMode::FilterText(buf) => {
            format!("typing: {buf}_ (Enter apply, Esc cancel)")
        }
        crate::filters::InputMode::Normal => "normal".to_string(),
    };
    let queued = app.queued_len();
    let timeline_hint = app.timeline_cursor_from_end().map_or_else(
        || "timeline: live (left/right to scrub)".to_string(),
        |cursor| {
            let len = app.timeline().len();
            let idx_from_oldest = len.saturating_sub(cursor + 1);
            let position = if idx_from_oldest == 0 {
                "at oldest"
            } else if idx_from_oldest + 1 == len {
                "at newest"
            } else {
                "inside range"
            };
            format!(
                "timeline cursor: bin {}/{} ({})",
                idx_from_oldest + 1,
                len,
                position
            )
        },
    );

    let header = Paragraph::new(vec![
        Line::from(format!(
            "Source: {}   Mode: {}   Queued: +{}   {}",
            app.source_label(),
            app.mode.label(),
            queued,
            timeline_hint
        )),
        Line::from(
            "space pause/resume | arrows/pgup/pgdn scroll | g/end go live | left/right timeline | n/p next/prev error | b add bookmark | ]/[ jump mark | ?: help",
        ),
        Line::from(format!(
            "Filter (/ start, R regex, F/C clear): {} | Levels: {} | Input: {}",
            filter_display, level_display, input_status
        )),
    ])
    .block(
        Block::default()
            .title("Overview")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(header, area);
}

fn render_logs(frame: &mut Frame, area: Rect, app: &App) {
    let max_visible = area.height.saturating_sub(2) as usize;
    let visible_logs = app.visible_logs(max_visible);

    let selected_idx_from_end = app.selected_from_end();
    let filtered_total = app.filtered_len();
    let items: Vec<ListItem> = visible_logs
        .into_iter()
        .map(|(filtered_idx, entry)| {
            let selected = filtered_total.saturating_sub(filtered_idx + 1) == selected_idx_from_end;
            to_list_item(entry, selected)
        })
        .collect();

    let list = List::new(items).block(Block::default().title("Logs").borders(Borders::ALL));
    frame.render_widget(list, area);
}

fn render_timeline(frame: &mut Frame, area: Rect, app: &App) {
    let data = app.timeline().data();
    let max_value = data
        .iter()
        .map(|b| b.info + b.warn + b.error)
        .max()
        .unwrap_or(1);
    let (start, end) = app.timeline().range();
    let cursor_text = app.timeline_cursor_from_end().map(|cursor| {
        let len = app.timeline().len();
        let idx_from_oldest = len.saturating_sub(cursor + 1);
        let ts = app.timeline().bin_start(idx_from_oldest);
        format!(
            "  cursor: {} (bin {}/{})",
            ts.format("%H:%M:%S"),
            idx_from_oldest + 1,
            len
        )
    });
    let title = format!(
        "Activity timeline ({} - {}){}",
        start.format("%H:%M:%S"),
        end.format("%H:%M:%S"),
        cursor_text.unwrap_or_default()
    );

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(3).max(1)),
            Constraint::Length(1),
            Constraint::Length(2),
        ])
        .split(area);

    let combined: Vec<u64> = data.iter().map(|b| b.info + b.warn + b.error).collect();
    let sparkline = Sparkline::default()
        .block(Block::default().title(title).borders(Borders::ALL))
        .data(&combined)
        .max(max_value)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(sparkline, parts[0]);

    let band = build_band_spans(&data, parts[1].width as usize);
    let band_para = Paragraph::new(Line::from(band)).block(
        Block::default()
            .borders(Borders::LEFT | Borders::RIGHT)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(band_para, parts[1]);

    let mut marks = vec!['.'; data.len()];
    if let Some(cursor) = app.timeline_cursor_from_end() {
        let len = data.len();
        if len > 0 {
            let idx_from_oldest = len.saturating_sub(cursor + 1);
            if idx_from_oldest < marks.len() {
                marks[idx_from_oldest] = '^';
            }
        }
    }
    for bm in app.bookmarks() {
        if let Some(idx) = app.timeline().bin_index_for(bm.timestamp) {
            if let Some(slot) = marks.get_mut(idx) {
                *slot = if *slot == '^' { '#' } else { '*' };
            }
        }
    }
    let legend = Line::from(vec![
        Span::styled(" error", Style::default().bg(Color::Red).fg(Color::Black)),
        Span::raw(" "),
        Span::styled(" warn", Style::default().bg(Color::Yellow).fg(Color::Black)),
        Span::raw(" "),
        Span::styled(" info", Style::default().bg(Color::White).fg(Color::Black)),
        Span::raw("   ^ cursor  * bookmark  # cursor+bookmark"),
    ]);

    let marker_str: String = marks.iter().collect();
    let trimmed = if marker_str.len() > parts[2].width as usize {
        marker_str
            .chars()
            .take(parts[2].width as usize)
            .collect::<String>()
    } else {
        marker_str
    };
    let markers = Paragraph::new(vec![Line::from(trimmed), legend]).block(
        Block::default()
            .borders(Borders::LEFT | Borders::RIGHT)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(markers, parts[2]);
}

fn build_band_spans(data: &[crate::timeline::Bin], width: usize) -> Vec<Span<'static>> {
    if data.is_empty() || width == 0 {
        return vec![Span::raw("")];
    }
    let len = data.len();
    let step = len.div_ceil(width).max(1);
    let mut spans = Vec::new();
    let mut idx = 0;
    while idx < len && spans.len() < width {
        let mut info = 0;
        let mut warn = 0;
        let mut error = 0;
        for bin in data.iter().skip(idx).take(step) {
            info += bin.info;
            warn += bin.warn;
            error += bin.error;
        }
        let color = if error > 0 {
            Color::Red
        } else if warn > 0 {
            Color::Yellow
        } else if info > 0 {
            Color::White
        } else {
            Color::DarkGray
        };
        spans.push(Span::styled(" ", Style::default().bg(color)));
        idx += step;
    }
    spans
}

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let queued = app.queued_len();
    let filter_text = match &app.filters().text {
        Some(text) if !text.is_empty() => {
            if app.filters().regex_mode {
                format!("filter: /{text}/")
            } else {
                format!("filter: \"{text}\"")
            }
        }
        _ => "filter: none".to_string(),
    };
    let input_hint = match app.input_mode() {
        crate::filters::InputMode::FilterText(buf) => Some(format!("typing filter: {buf}_")),
        crate::filters::InputMode::Normal => None,
    };
    let levels = (
        level_chip("INFO", app.filters().info, Color::White),
        level_chip("WARN", app.filters().warn, Color::Yellow),
        level_chip("ERROR", app.filters().error, Color::Red),
    );
    let timeline_status = app.timeline_cursor_from_end().map_or_else(
        || "timeline: live (left/right to scrub)".to_string(),
        |cursor| {
            let len = app.timeline().len();
            let idx_from_oldest = len.saturating_sub(cursor + 1);
            let position = if idx_from_oldest == 0 {
                "at oldest"
            } else if idx_from_oldest + 1 == len {
                "at newest"
            } else {
                "inside range"
            };
            format!(
                "timeline cursor {}/{} ({})",
                idx_from_oldest + 1,
                len,
                position
            )
        },
    );

    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!(" {} ", app.mode.label()),
            Style::default()
                .fg(app.mode.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" · "),
        Span::styled(
            format!("logs buffered: {}", app.total_logs()),
            Style::default().fg(Color::Gray),
        ),
        Span::raw(" · "),
        Span::raw(format!("scroll offset: {}", app.scroll_offset())),
        Span::raw(" · "),
        Span::raw(if app.scroll_offset() > 0 {
            format!("INSPECTING -{}", app.scroll_offset())
        } else {
            "live view".to_string()
        }),
        Span::raw(" · "),
        Span::styled(
            format!("queued: +{queued}"),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw(" · "),
        Span::raw(timeline_status),
    ])];
    if matches!(app.mode, crate::app::Mode::Paused) {
        lines.push(Line::from(vec![
            Span::styled(
                "PAUSED - view frozen; new lines are buffered",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" · press space/g to resume"),
        ]));
    }

    let filter_spans = {
        let mut spans = vec![Span::styled(
            filter_text.clone(),
            if app.filters().text.is_some() {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            },
        )];
        if app.filters().text.is_some() {
            spans.push(Span::raw(" (F/C to clear)"));
        }
        spans.push(Span::raw(" · "));
        spans.push(Span::styled(
            format!(
                "regex: {}",
                if app.filters().regex_mode {
                    "on"
                } else {
                    "off"
                }
            ),
            if app.filters().regex_mode {
                Style::default().fg(Color::Magenta)
            } else {
                Style::default().fg(Color::Gray)
            },
        ));
        spans.push(Span::raw(" · "));
        spans.extend_from_slice(&[
            levels.0.clone(),
            Span::raw(" "),
            levels.1.clone(),
            Span::raw(" "),
            levels.2.clone(),
        ]);
        spans
    };
    lines.push(Line::from(filter_spans));
    let bookmark_line = if let Some((idx, bm)) = app.current_bookmark_position() {
        format!(
            "Bookmarks: {} (at {}/{} -> {} @ {})",
            app.bookmarks().len(),
            idx + 1,
            app.bookmarks().len(),
            bm.label,
            bm.timestamp.format("%H:%M:%S")
        )
    } else {
        format!(
            "Bookmarks: {} (b to add, ]/[ to jump)",
            app.bookmarks().len()
        )
    };
    lines.push(Line::from(bookmark_line));
    let command_bar = Line::from(vec![
        Span::styled("Commands: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(
            "Quit q/ctrl-c | Pause/Live space/g | Scroll \u{2191}/\u{2193}/PgUp/PgDn/Home/End | Timeline \u{2190}/\u{2192} | Filters / type, Enter apply, Esc cancel, F/C clear, R regex | Levels 1/2/3 | Errors n/p | Bookmarks b add, ]/[ jump",
        ),
    ]);
    lines.push(command_bar);
    if let Some(hint) = input_hint {
        lines.push(Line::from(format!("Input: {}", hint)));
    }

    if let Some(err) = app.filter_error() {
        lines.push(Line::from(Span::styled(
            format!("filter error: {err}"),
            Style::default().fg(Color::Red),
        )));
    }

    if let Some(msg) = app.last_notice() {
        lines.push(Line::from(Span::styled(
            msg.clone(),
            Style::default().fg(Color::Green),
        )));
    }

    let status = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(status, area);
}

fn to_list_item(entry: &crate::log_entry::LogEntry, selected: bool) -> ListItem<'static> {
    let ts = entry.timestamp.format("%H:%M:%S").to_string();
    let mut spans = vec![
        Span::styled(
            format!("{ts} "),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
        Span::styled(
            format!("{:5}", entry.level.label()),
            Style::default()
                .fg(entry.level.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:<7}", entry.target),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw(" "),
        Span::raw(entry.message.clone()),
    ];
    if selected {
        for span in spans.iter_mut() {
            span.style = span.style.add_modifier(Modifier::REVERSED);
        }
    }
    ListItem::new(Line::from(spans))
}

fn level_chip(label: &str, enabled: bool, color: Color) -> Span<'static> {
    let mut style = Style::default().fg(color).add_modifier(Modifier::BOLD);
    if !enabled {
        style = style.add_modifier(Modifier::CROSSED_OUT | Modifier::DIM);
    }
    Span::styled(label.to_string(), style)
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
