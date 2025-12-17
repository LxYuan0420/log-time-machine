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
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Sparkline},
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
            Constraint::Length(2),
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
            Line::from(" q/ctrl-c quit | space pause | g/end go live | arrows/pgup/pgdn scroll"),
            Line::from(" left/right timeline | s/S jump spikes"),
            Line::from(
                " / filter | R toggle regex | F clear | 1/2/3 toggle levels | n/p next/prev error",
            ),
            Line::from(" b add bookmark | ]/[ next/prev bookmark"),
            Line::from(" A/B set diff markers | E export diff slice"),
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
    let header = Paragraph::new(vec![
        Line::from(format!(
            "Source: {}   Mode: {}",
            app.source_label(),
            app.mode.label()
        )),
        Line::from(
            "q: quit  space: pause/resume  arrows: scroll  left/right: timeline  g/end: go live  ?: help",
        ),
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
    let data: Vec<u64> = app.timeline().data();
    let max_value = data.iter().copied().max().unwrap_or(1);
    let (start, end) = app.timeline().range();
    let cursor_text = app.timeline_cursor_from_end().map(|cursor| {
        let len = app.timeline().len();
        let idx_from_oldest = len.saturating_sub(cursor + 1);
        let ts = app.timeline().bin_start(idx_from_oldest);
        format!("  cursor: {}", ts.format("%H:%M:%S"))
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
            Constraint::Length(area.height.saturating_sub(1).max(1)),
            Constraint::Length(1),
        ])
        .split(area);

    let sparkline = Sparkline::default()
        .block(Block::default().title(title).borders(Borders::ALL))
        .data(&data)
        .max(max_value)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(sparkline, parts[0]);

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
    if let Some(a) = app.diff_a() {
        if let Some(idx) = app.timeline().bin_index_for(a) {
            if let Some(slot) = marks.get_mut(idx) {
                *slot = 'A';
            }
        }
    }
    if let Some(b) = app.diff_b() {
        if let Some(idx) = app.timeline().bin_index_for(b) {
            if let Some(slot) = marks.get_mut(idx) {
                *slot = 'B';
            }
        }
    }

    let marker_str: String = marks.iter().collect();
    let trimmed = if marker_str.len() > parts[1].width as usize {
        marker_str
            .chars()
            .take(parts[1].width as usize)
            .collect::<String>()
    } else {
        marker_str
    };
    let markers = Paragraph::new(trimmed).block(
        Block::default()
            .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(markers, parts[1]);
}

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let queued = app
        .paused_head_len()
        .map(|start| app.total_logs().saturating_sub(start))
        .unwrap_or(0);
    let levels = format!(
        "levels: {}{}{}",
        if app.filters().info { "I" } else { "i" },
        if app.filters().warn { "W" } else { "w" },
        if app.filters().error { "E" } else { "e" }
    );
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
        crate::filters::InputMode::FilterText(buf) => format!("typing filter: {buf}_"),
        crate::filters::InputMode::Normal => "".to_string(),
    };
    let bookmarks = format!("bookmarks: {}", app.bookmarks().len());

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
        Span::raw(format!("queued: +{queued}")),
        Span::raw(" · "),
        Span::raw(levels),
        Span::raw(" · "),
        Span::raw(filter_text),
        Span::raw(" · "),
        Span::raw(input_hint),
        Span::raw(" · "),
        Span::raw(bookmarks),
        Span::raw(" · "),
        Span::raw(
            "keys: pgup/pgdn scroll, left/right timeline, / filter, F clear, R regex, b add mark, ]/[ jump mark, n/p error, s/S spike, A/B diff, E export",
        ),
    ])];

    if !app.bookmarks().is_empty() {
        let labels: Vec<String> = app.bookmarks().iter().map(|b| b.label.clone()).collect();
        lines.push(Line::from(format!("Bookmarks: {}", labels.join(", "))));
    }

    if let Some(err) = app.filter_error() {
        lines.push(Line::from(Span::styled(
            format!("filter error: {err}"),
            Style::default().fg(Color::Red),
        )));
    }

    if let (Some(a), Some(b)) = (app.diff_a(), app.diff_b()) {
        lines.push(Line::from(format!(
            "Diff A..B: A={}  B={}",
            a.format("%H:%M:%S"),
            b.format("%H:%M:%S")
        )));
        if let Some(stats) = app.diff_summary() {
            let targets = if stats.top_targets.is_empty() {
                "top targets: (none)".to_string()
            } else {
                let joined = stats
                    .top_targets
                    .iter()
                    .map(|(t, c)| format!("{t}({c})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("top targets: {joined}")
            };
            lines.push(Line::from(format!(
                "Counts in range (filtered): total={} info={} warn={} error={} · {}",
                stats.total, stats.info, stats.warn, stats.error, targets
            )));
        }
    }

    if let Some(msg) = app.last_notice() {
        lines.push(Line::from(Span::styled(
            msg.clone(),
            Style::default().fg(Color::Green),
        )));
    }

    let status = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
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
