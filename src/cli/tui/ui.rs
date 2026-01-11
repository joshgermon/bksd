//! UI rendering for the TUI.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::core::transfer_engine::TransferStatus;

use super::app::{TuiApp, View};

/// Main render function - dispatches to view-specific renderers.
pub fn render(frame: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Content
            Constraint::Length(3), // Footer/help
        ])
        .split(frame.area());

    render_header(frame, app, chunks[0]);

    match &app.view {
        View::Dashboard { selected } => {
            render_dashboard(frame, app, chunks[1], *selected);
        }
        View::History { selected, .. } => {
            render_history(frame, app, chunks[1], *selected);
        }
        View::Detail { job_id, scroll } => {
            render_detail(frame, app, chunks[1], job_id, *scroll);
        }
    }

    render_footer(frame, app, chunks[2]);
}

fn render_header(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let status = app.data.daemon_status.as_ref();

    let title = match status {
        Some(s) => {
            let mode = if s.simulation { " [SIM]" } else { "" };
            let uptime = format_duration(s.uptime_secs);
            format!("BKSD  v{}  Uptime: {}{}", s.version, uptime, mode)
        }
        None => "BKSD  (connecting...)".to_string(),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    frame.render_widget(block, area);
}

fn render_dashboard(frame: &mut Frame, app: &TuiApp, area: Rect, selected: usize) {
    let has_active = !app.data.active_jobs.is_empty();

    if has_active {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Active transfer banner (compact)
                Constraint::Min(0),    // Recent jobs (fills remaining space)
            ])
            .split(area);

        render_active_banner(frame, app, chunks[0]);
        render_recent_jobs(frame, app, chunks[1], selected);
    } else {
        // No active transfers - recent jobs get full space
        render_recent_jobs(frame, app, area, selected);
    }
}

fn render_active_banner(frame: &mut Frame, app: &TuiApp, area: Rect) {
    // Get first active job (typically only one)
    let (job_id, status) = match app.data.active_jobs.iter().next() {
        Some((id, s)) => (id, s),
        None => return,
    };

    let content = format_active_banner(&job_id[..8.min(job_id.len())], status);

    let block = Block::default()
        .title("Active Transfer")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let paragraph = Paragraph::new(content).block(block);
    frame.render_widget(paragraph, area);
}

fn render_recent_jobs(frame: &mut Frame, app: &TuiApp, area: Rect, selected: usize) {
    let block = Block::default()
        .title("Recent Jobs")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    if app.data.recent_jobs.is_empty() {
        let text = Paragraph::new("  No recent jobs")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(text, area);
        return;
    }

    let items: Vec<ListItem> = app
        .data
        .recent_jobs
        .iter()
        .enumerate()
        .map(|(i, job)| {
            let is_selected = i == selected;
            let style = if is_selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let status_icon = match job.status.as_str() {
                "Complete" => Span::styled("✓", Style::default().fg(Color::Green)),
                "Failed" => Span::styled("✗", Style::default().fg(Color::Red)),
                _ => Span::styled("•", Style::default().fg(Color::Yellow)),
            };

            // Safe substring handling for job id and created_at
            let job_id_short = if job.id.len() >= 8 {
                &job.id[..8]
            } else {
                &job.id
            };
            let created_short = if job.created_at.len() >= 16 {
                &job.created_at[..16]
            } else {
                &job.created_at
            };

            let line = Line::from(vec![
                Span::raw(if is_selected { "> " } else { "  " }),
                status_icon,
                Span::raw(format!("  {}  {}  {}", job_id_short, created_short, &job.status)),
            ]);

            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_history(frame: &mut Frame, app: &TuiApp, area: Rect, selected: usize) {
    let block = Block::default()
        .title("Job History")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    if app.data.all_jobs.is_empty() {
        let text = Paragraph::new("  No jobs found")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(text, area);
        return;
    }

    let items: Vec<ListItem> = app
        .data
        .all_jobs
        .iter()
        .enumerate()
        .map(|(i, job)| {
            let is_selected = i == selected;
            let style = if is_selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let status_icon = match job.status.as_str() {
                "Complete" => Span::styled("✓", Style::default().fg(Color::Green)),
                "Failed" => Span::styled("✗", Style::default().fg(Color::Red)),
                _ => Span::styled("•", Style::default().fg(Color::Yellow)),
            };

            let line = Line::from(vec![
                Span::raw("  "),
                status_icon,
                Span::raw(format!(
                    "  {}  {}  {}",
                    &job.id[..8],
                    &job.created_at[..16],
                    job.status
                )),
            ]);

            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn render_detail(frame: &mut Frame, app: &TuiApp, area: Rect, _job_id: &str, _scroll: u16) {
    let block = Block::default()
        .title("Job Details")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let job = match &app.data.selected_job {
        Some(j) => j,
        None => {
            let text = Paragraph::new("  Loading...")
                .style(Style::default().fg(Color::DarkGray))
                .block(block);
            frame.render_widget(text, area);
            return;
        }
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled("  Job ID:      ", Style::default().fg(Color::Cyan)),
            Span::raw(&job.job.id),
        ]),
        Line::from(vec![
            Span::styled("  Target:      ", Style::default().fg(Color::Cyan)),
            Span::raw(&job.job.target_id),
        ]),
        Line::from(vec![
            Span::styled("  Destination: ", Style::default().fg(Color::Cyan)),
            Span::raw(job.job.destination_path.as_deref().unwrap_or("-")),
        ]),
        Line::from(vec![
            Span::styled("  Created:     ", Style::default().fg(Color::Cyan)),
            Span::raw(&job.job.created_at),
        ]),
        Line::from(vec![
            Span::styled("  Status:      ", Style::default().fg(Color::Cyan)),
            Span::raw(&job.job.status),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Status History",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  ─────────────────────────────────────────"),
    ];

    for entry in &job.history {
        let timestamp = if entry.created_at.len() >= 19 {
            &entry.created_at[11..19]
        } else {
            &entry.created_at
        };

        let mut parts = vec![
            Span::raw(format!("  {}  ", timestamp)),
            Span::styled(
                format!("{:<12}", entry.status),
                Style::default().fg(Color::White),
            ),
        ];

        if let Some(desc) = &entry.description {
            parts.push(Span::raw(format!("  {}", desc)));
        }

        if let (Some(bytes), Some(secs)) = (entry.total_bytes, entry.duration_secs) {
            parts.push(Span::styled(
                format!("  {} in {}s", format_bytes(bytes), secs),
                Style::default().fg(Color::Green),
            ));
        }

        lines.push(Line::from(parts));
    }

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_footer(frame: &mut Frame, app: &TuiApp, area: Rect) {
    let help_text = match &app.view {
        View::Dashboard { .. } => "[↑↓] Navigate  [Enter] Details  [h] History  [r] Refresh  [q] Quit",
        View::History { .. } => "[↑↓] Navigate  [Enter] Details  [Esc] Back  [q] Quit",
        View::Detail { .. } => "[Esc] Back  [q] Quit",
    };

    let mut spans = vec![Span::raw(format!("  {}", help_text))];

    if let Some(error) = &app.error {
        spans.push(Span::styled(
            format!("  Error: {}", error),
            Style::default().fg(Color::Red),
        ));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(Line::from(spans)).block(block);
    frame.render_widget(paragraph, area);
}

fn format_active_banner(job_id: &str, status: &TransferStatus) -> Line<'static> {
    match status {
        TransferStatus::Ready => Line::from(vec![
            Span::styled("▶ ", Style::default().fg(Color::Green)),
            Span::raw(format!("{}  Preparing...", job_id)),
        ]),
        TransferStatus::InProgress {
            percentage,
            current_file,
            ..
        } => {
            let bar = progress_bar(*percentage, 25);
            let file = if current_file.len() > 30 {
                format!("...{}", &current_file[current_file.len() - 27..])
            } else {
                current_file.clone()
            };
            Line::from(vec![
                Span::styled("▶ ", Style::default().fg(Color::Green)),
                Span::raw(format!("{}  {} {:>3}%  {}", job_id, bar, percentage, file)),
            ])
        }
        TransferStatus::CopyComplete => Line::from(vec![
            Span::styled("▶ ", Style::default().fg(Color::Yellow)),
            Span::raw(format!("{}  Copy complete, verifying...", job_id)),
        ]),
        TransferStatus::Verifying { current, total } => {
            let pct = if *total > 0 {
                (current * 100 / total) as u8
            } else {
                100
            };
            let bar = progress_bar(pct, 25);
            Line::from(vec![
                Span::styled("▶ ", Style::default().fg(Color::Yellow)),
                Span::raw(format!(
                    "{}  {} {:>3}%  Verifying {}/{}",
                    job_id, bar, pct, current, total
                )),
            ])
        }
        TransferStatus::Complete {
            total_bytes,
            duration_secs,
        } => Line::from(vec![
            Span::styled("✓ ", Style::default().fg(Color::Green)),
            Span::raw(format!(
                "{}  Complete: {} in {}s",
                job_id,
                format_bytes(*total_bytes),
                duration_secs
            )),
        ]),
        TransferStatus::Failed(msg) => Line::from(vec![
            Span::styled("✗ ", Style::default().fg(Color::Red)),
            Span::raw(format!("{}  ", job_id)),
            Span::styled(format!("Failed: {}", msg), Style::default().fg(Color::Red)),
        ]),
    }
}

fn progress_bar(percentage: u8, width: usize) -> String {
    let percentage = percentage.min(100) as usize;
    let filled = (percentage * width) / 100;
    let empty = width - filled;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn format_duration(secs: u64) -> String {
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let secs = secs % 60;

    if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else if mins > 0 {
        format!("{}m {}s", mins, secs)
    } else {
        format!("{}s", secs)
    }
}
