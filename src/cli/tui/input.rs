//! Input handling for the TUI.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use super::app::Action;

/// Convert a crossterm key event to an Action.
pub fn handle_key_event(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Quit),
        KeyCode::Esc => Some(Action::Back),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Up),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Down),
        KeyCode::Left | KeyCode::Char('h') if !key.modifiers.contains(KeyModifiers::NONE) => {
            Some(Action::Left)
        }
        KeyCode::Left => Some(Action::Left),
        KeyCode::Right | KeyCode::Char('l') => Some(Action::Right),
        KeyCode::Enter | KeyCode::Char(' ') => Some(Action::Select),
        KeyCode::Char('r') => Some(Action::Refresh),
        KeyCode::Char('h') => Some(Action::History),
        KeyCode::F(5) => Some(Action::Refresh),
        _ => None,
    }
}

/// Convert a crossterm Event to an Action.
pub fn handle_event(event: Event) -> Option<Action> {
    match event {
        Event::Key(key) => handle_key_event(key),
        _ => None,
    }
}
