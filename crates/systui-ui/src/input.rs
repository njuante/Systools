//! Keyboard input handling.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use systui_core::FindingStatus;

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

    // The cron builder captures input while open.
    if app.cron_builder.is_some() {
        match key.code {
            KeyCode::Esc => app.close_cron_form(),
            KeyCode::Enter => app.submit_cron_form(),
            KeyCode::Tab | KeyCode::Down => app.cron_form_focus_next(),
            KeyCode::BackTab | KeyCode::Up => app.cron_form_focus_prev(),
            KeyCode::Left => app.cron_form_decrement(),
            KeyCode::Right => app.cron_form_increment(),
            KeyCode::Backspace => app.cron_form_pop_char(),
            KeyCode::Char(c) => app.cron_form_push_char(c),
            _ => {}
        }
        return;
    }

    // Note entry captures typing for the session note (Dashboard).
    if app.note_draft.is_some() {
        match key.code {
            KeyCode::Esc => app.cancel_note(),
            KeyCode::Enter => app.submit_note(),
            KeyCode::Backspace => app.note_pop_char(),
            KeyCode::Char(c) => app.note_push_char(c),
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
        KeyCode::Char('T') => {
            app.cycle_theme();
        }
        KeyCode::Char('V') => {
            app.cycle_visual_style();
        }
        KeyCode::Char('r') => app.request_refresh(),
        KeyCode::Char('s') => app.toggle_process_sort(),
        KeyCode::Char('/') => app.enter_search(),
        KeyCode::Char('l') => app.cycle_log_level(),
        KeyCode::Char('t') if app.current_tab() == Tab::Processes => app.toggle_process_view(),
        KeyCode::Char('t') => app.cycle_log_window(),
        KeyCode::Char('a') => {
            if app.current_tab() == Tab::Security {
                app.set_selected_finding_status(FindingStatus::Accepted);
            } else if app.current_tab() == Tab::Crons {
                app.open_add_cron_form();
            } else {
                app.request_action();
            }
        }
        KeyCode::Char('o') if app.current_tab() == Tab::Security => {
            app.set_selected_finding_status(FindingStatus::Open);
        }
        KeyCode::Char('i') if app.current_tab() == Tab::Security => {
            app.set_selected_finding_status(FindingStatus::Ignored);
        }
        KeyCode::Char('f') if app.current_tab() == Tab::Security => {
            app.set_selected_finding_status(FindingStatus::Fixed);
        }
        KeyCode::Char('v') if app.current_tab() == Tab::Security => {
            app.set_selected_finding_status(FindingStatus::FalsePositive);
        }
        KeyCode::Char('e') if app.current_tab() == Tab::Crons => app.open_edit_cron_form(),
        KeyCode::Char('d') if app.current_tab() == Tab::Crons => app.request_delete_cron(),
        KeyCode::Char('x') if app.current_tab() == Tab::Crons => app.request_toggle_cron(),
        KeyCode::Char('n') if app.current_tab() == Tab::Crons => app.request_run_cron(),
        KeyCode::Char('f') if app.current_tab() == Tab::Services => app.cycle_service_filter(),
        KeyCode::Char('c') if app.current_tab() == Tab::Network => app.request_connectivity(),
        KeyCode::Char('p') if app.current_tab() == Tab::Docker => app.request_prune_images(),
        KeyCode::Char('n') if app.current_tab() == Tab::Dashboard => app.open_note(),
        KeyCode::Char('S') if app.current_tab() == Tab::Logs => app.save_current_search(),
        KeyCode::Up if app.current_tab() == Tab::Logs => app.saved_search_up(),
        KeyCode::Down if app.current_tab() == Tab::Logs => app.saved_search_down(),
        KeyCode::Enter if app.current_tab() == Tab::Logs => app.apply_saved_search(),
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

    #[test]
    fn security_keys_set_finding_state() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.select_tab(9);
        app.findings = vec![systui_core::Finding::new(
            "policy.port.forbidden.prod-web.6379",
            systui_core::Severity::High,
            systui_core::ModuleId::Security,
            "Forbidden port 6379 is listening",
        )];

        handle_key(&mut app, press(KeyCode::Char('a')));
        assert_eq!(app.findings[0].status, FindingStatus::Accepted);
        assert_eq!(app.finding_counts(), [0, 0, 0, 0, 0]);
        assert!(app.state_dirty);

        handle_key(&mut app, press(KeyCode::Char('o')));
        assert_eq!(app.findings[0].status, FindingStatus::Open);
        assert_eq!(app.finding_counts(), [0, 1, 0, 0, 0]);
    }
}
