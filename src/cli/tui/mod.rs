//! Interactive TUI for BKSD.
//!
//! Provides an interactive terminal interface for browsing jobs and monitoring
//! active transfers.

mod app;
mod input;
mod ui;

use std::io::{self, stdout};
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    event, execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::{TuiApp, View};

/// Run the TUI connected to the daemon at the given address.
pub async fn run(addr: SocketAddr) -> Result<()> {
    // Setup terminal
    enable_raw_mode().context("Failed to enable raw mode")?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen).context("Failed to enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("Failed to create terminal")?;

    // Create app and run
    let mut app = TuiApp::new(addr);
    let result = run_app(&mut terminal, &mut app).await;

    // Restore terminal
    disable_raw_mode().context("Failed to disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("Failed to leave alternate screen")?;
    terminal.show_cursor().context("Failed to show cursor")?;

    result
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TuiApp,
) -> Result<()> {
    // Initial data fetch
    app.init().await?;

    // Polling interval for active jobs
    let poll_interval = Duration::from_millis(500);

    loop {
        // Render
        terminal.draw(|frame| ui::render(frame, app))?;

        // Check for input with timeout (for polling)
        let timeout = if matches!(app.view, View::Dashboard { .. }) {
            poll_interval
        } else {
            Duration::from_secs(60) // Longer timeout when not on dashboard
        };

        if event::poll(timeout)? {
            let event = event::read()?;
            if let Some(action) = input::handle_event(event) {
                app.handle_action(action).await;
            }
        } else if matches!(app.view, View::Dashboard { .. }) {
            // Poll active jobs on timeout (only on dashboard)
            app.refresh_active_jobs().await;
        }

        if !app.running {
            break;
        }
    }

    Ok(())
}
