//! Keyboard input handling.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;

/// Apply a key press to the application state.
pub fn handle_key(app: &mut App, key: KeyEvent) {
    // Ctrl+C always quits.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.quit();
        return;
    }

    // While the help overlay is open, only dismissal keys are handled.
    if app.show_help {
        if matches!(
            key.code,
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
        ) {
            app.show_help = false;
        }
        return;
    }

    match key.code {
        KeyCode::Char('q') => app.quit(),
        KeyCode::Char('?') => app.toggle_help(),
        KeyCode::Char('r') => app.request_refresh(),
        KeyCode::Tab | KeyCode::Right => app.next_tab(),
        KeyCode::BackTab | KeyCode::Left => app.prev_tab(),
        KeyCode::Char(c @ '1'..='9') => app.select_tab(c as usize - '1' as usize),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_core::ExecutionMode;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn q_quits() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        handle_key(&mut app, press(KeyCode::Char('q')));
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(app.should_quit);
    }

    #[test]
    fn help_opens_and_closes() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        handle_key(&mut app, press(KeyCode::Char('?')));
        assert!(app.show_help);
        handle_key(&mut app, press(KeyCode::Esc));
        assert!(!app.show_help);
    }

    #[test]
    fn number_key_jumps_to_tab() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        handle_key(&mut app, press(KeyCode::Char('3')));
        assert_eq!(app.current_tab(), crate::app::Tab::Services);
    }

    #[test]
    fn keys_are_swallowed_while_help_is_open() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.toggle_help();
        handle_key(&mut app, press(KeyCode::Tab));
        // tab did not change because help intercepted the key
        assert_eq!(app.current_tab(), crate::app::Tab::Dashboard);
    }
}
