//! Keyboard input handling.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{ActionStage, App, InputMode, Tab};

/// Apply a key press to the application state.
pub fn handle_key(app: &mut App, key: KeyEvent) {
    // Ctrl+C always quits.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.quit();
        return;
    }

    // The action overlay captures all input while open.
    if let Some(stage) = app.action.as_ref().map(|m| m.stage) {
        match stage {
            ActionStage::Result => app.close_action(),
            ActionStage::Ready => match key.code {
                KeyCode::Enter => app.submit_action(),
                KeyCode::Esc => app.close_action(),
                _ => {}
            },
            ActionStage::Confirm => match key.code {
                KeyCode::Enter => app.submit_action(),
                KeyCode::Esc => app.close_action(),
                KeyCode::Backspace => app.pop_action_char(),
                KeyCode::Char(c) => app.push_action_char(c),
                _ => {}
            },
        }
        return;
    }

    // Form overlays capture input while open.
    if app.cron_form.is_some() {
        match key.code {
            KeyCode::Esc => app.close_cron_form(),
            KeyCode::Enter => app.submit_cron_form(),
            KeyCode::Tab | KeyCode::Down => app.cron_form_focus_next(),
            KeyCode::BackTab | KeyCode::Up => app.cron_form_focus_prev(),
            KeyCode::Backspace => app.cron_form_pop_char(),
            KeyCode::Char(c) => app.cron_form_push_char(c),
            _ => {}
        }
        return;
    }

    // Search mode captures typing for the log filter.
    if app.input_mode == InputMode::Search {
        match key.code {
            KeyCode::Esc => app.exit_search(),
            KeyCode::Enter => app.input_mode = InputMode::Normal,
            KeyCode::Backspace => app.pop_search_char(),
            KeyCode::Char(c) => app.push_search_char(c),
            _ => {}
        }
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
        KeyCode::Char('s') => app.toggle_process_sort(),
        KeyCode::Char('/') => app.enter_search(),
        KeyCode::Char('l') => app.cycle_log_level(),
        KeyCode::Char('t') => app.cycle_log_window(),
        KeyCode::Char('a') => {
            if app.current_tab() == Tab::Crons {
                app.open_add_cron_form();
            } else {
                app.request_action();
            }
        }
        KeyCode::Char('e') if app.current_tab() == Tab::Crons => app.open_edit_cron_form(),
        KeyCode::Char('d') if app.current_tab() == Tab::Crons => app.request_delete_cron(),
        KeyCode::Char('x') if app.current_tab() == Tab::Crons => app.request_toggle_cron(),
        KeyCode::Char('n') if app.current_tab() == Tab::Crons => app.request_run_cron(),
        KeyCode::Char('f') if app.current_tab() == Tab::Services => app.cycle_service_filter(),
        KeyCode::Char('c') if app.current_tab() == Tab::Network => app.request_connectivity(),
        KeyCode::Char('p') if app.current_tab() == Tab::Docker => app.request_prune_images(),
        KeyCode::Up => app.select_up(),
        KeyCode::Down => app.select_down(),
        KeyCode::Tab | KeyCode::Right => app.next_tab(),
        KeyCode::BackTab | KeyCode::Left => app.prev_tab(),
        KeyCode::Char(c @ '1'..='9') => app.select_tab(c as usize - '1' as usize),
        KeyCode::Char('0') => app.select_tab(9),
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
        handle_key(&mut app, press(KeyCode::Char('4')));
        assert_eq!(app.current_tab(), crate::app::Tab::Services);
        handle_key(&mut app, press(KeyCode::Char('0')));
        assert_eq!(app.current_tab(), crate::app::Tab::Security);
    }

    #[test]
    fn s_toggles_process_sort() {
        use crate::app::ProcessSort;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        assert_eq!(app.process_sort, ProcessSort::Cpu);
        handle_key(&mut app, press(KeyCode::Char('s')));
        assert_eq!(app.process_sort, ProcessSort::Mem);
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
