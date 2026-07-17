// testing status checks for release
mod app;
mod diagnostics;
mod git;
mod repo_registry;
mod runtime_paths;
mod tree;
mod ui;

use std::{env, time::Duration};

use anyhow::Result;
use app::{App, InputMode, Screen};
use diagnostics::{doctor_report, initialize_logging, parse_runtime_options};
use git::PullRequestMergeMethod;
use log::error;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, prelude::CrosstermBackend};

fn main() -> Result<()> {
    let runtime_options = parse_runtime_options(env::args().skip(1))?;
    if runtime_options.doctor_mode {
        println!("{}", doctor_report(&runtime_options)?);
        return Ok(());
    }

    let log_path = initialize_logging(&runtime_options)?;
    install_panic_hook();

    let mut app = App::new()?;
    if let Some(path) = log_path {
        app.set_runtime_log_path(path.clone());
        app.status_message = format!("Logging enabled: {}", path.display());
    }

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        app.tick();
        terminal.draw(|frame| ui::draw(frame, app))?;

        if !event::poll(Duration::from_millis(35))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if app.in_input_mode() {
            let mut action_result: Option<Result<()>> = None;
            match app.input_mode {
                InputMode::ConfirmDiscard => match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        action_result = Some(app.confirm_discard_selected())
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.cancel_input(),
                    _ => {}
                },
                InputMode::ConfirmPullRequestMerge => match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        action_result = Some(app.confirm_merge_selected_pr())
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.cancel_input(),
                    _ => {}
                },
                InputMode::ConfirmStashDrop => match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        action_result = Some(app.confirm_drop_selected_stash())
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.cancel_input(),
                    _ => {}
                },
                _ => match key.code {
                    KeyCode::Esc => app.cancel_input(),
                    KeyCode::Enter
                        if app.input_mode_allows_multiline()
                            && key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        action_result = Some(app.submit_input())
                    }
                    KeyCode::Enter if app.input_mode_allows_multiline() => app.push_input_char('\n'),
                    KeyCode::F(2) if app.input_mode_allows_multiline() => {
                        action_result = Some(app.submit_input())
                    }
                    KeyCode::Char('s')
                        if app.input_mode_allows_multiline()
                            && key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        action_result = Some(app.submit_input())
                    }
                    KeyCode::Enter => action_result = Some(app.submit_input()),
                    KeyCode::Backspace => app.pop_input_char(),
                    KeyCode::Delete => app.delete_input_char(),
                    KeyCode::Left => app.move_input_cursor_left(),
                    KeyCode::Right => app.move_input_cursor_right(),
                    KeyCode::Up if app.input_mode_allows_multiline() => app.move_input_cursor_up(),
                    KeyCode::Down if app.input_mode_allows_multiline() => {
                        app.move_input_cursor_down()
                    }
                    KeyCode::Home => app.move_input_cursor_home(),
                    KeyCode::End => app.move_input_cursor_end(),
                    KeyCode::Tab => action_result = Some(app.autocomplete_input()),
                    KeyCode::Char(ch) => app.push_input_char(ch),
                    _ => {}
                },
            }
            if let Some(result) = action_result {
                handle_action_result(app, result);
            }
            continue;
        }

        if app.help_visible {
            match key.code {
                KeyCode::Char('?') | KeyCode::Esc => app.close_help(),
                KeyCode::Down | KeyCode::Char('j') => app.scroll_help_down(1),
                KeyCode::Up | KeyCode::Char('k') => app.scroll_help_up(1),
                KeyCode::PageDown => app.scroll_help_down(8),
                KeyCode::PageUp => app.scroll_help_up(8),
                KeyCode::Home => app.scroll_help_to_top(),
                KeyCode::End => app.scroll_help_to_bottom(),
                _ => {}
            }
            continue;
        }

        if app.is_fullscreen_diff_visible() {
            match key.code {
                KeyCode::Esc => app.close_fullscreen_diff(),
                KeyCode::Down | KeyCode::Char('j') => app.fullscreen_diff_move_down(),
                KeyCode::Up | KeyCode::Char('k') => app.fullscreen_diff_move_up(),
                KeyCode::PageDown => app.fullscreen_diff_page_down(20),
                KeyCode::PageUp => app.fullscreen_diff_page_up(20),
                KeyCode::Home => app.fullscreen_diff_home(),
                KeyCode::End => app.fullscreen_diff_end(),
                KeyCode::Char('n') => app.fullscreen_diff_next_hunk(),
                KeyCode::Char('p') => app.fullscreen_diff_prev_hunk(),
                KeyCode::Left => app.fullscreen_diff_scroll_left(),
                KeyCode::Right => app.fullscreen_diff_scroll_right(),
                KeyCode::Char('h') => app.fullscreen_diff_scroll_left(),
                KeyCode::Char('l') => app.fullscreen_diff_scroll_right(),
                _ => {}
            }
            continue;
        }

        let mut action_result: Option<Result<()>> = None;
        match key.code {
            KeyCode::Char('q') => break,
            KeyCode::Char('1') => action_result = Some(app.switch_workspace_tab(1)),
            KeyCode::Char('2') => action_result = Some(app.switch_workspace_tab(2)),
            KeyCode::Char('3') => action_result = Some(app.switch_workspace_tab(3)),
            KeyCode::Char('4') => action_result = Some(app.switch_workspace_tab(4)),
            KeyCode::Char('5') => action_result = Some(app.switch_workspace_tab(5)),
            KeyCode::Char('6') => action_result = Some(app.switch_workspace_tab(6)),
            KeyCode::Char('?') => app.toggle_help(),
            KeyCode::Char('L') => app.show_runtime_log_path_status(),
            KeyCode::Char('r') => action_result = Some(app.refresh()),
            KeyCode::Down | KeyCode::Char('j') => app.move_next(),
            KeyCode::Up | KeyCode::Char('k') => app.move_prev(),
            KeyCode::Tab => app.cycle_focus(),
            KeyCode::Left if app.screen == Screen::HistoryView => app.history_focus_left(),
            KeyCode::Left if app.screen == Screen::RepoView => app.status_tree_focus_left(),
            KeyCode::Right if app.screen == Screen::RepoView => {
                action_result = Some(app.status_tree_focus_right())
            }
            KeyCode::Right if app.screen == Screen::HistoryView => {
                action_result = Some(app.history_focus_right())
            }
            KeyCode::Backspace if app.screen == Screen::RepoBrowser => {
                action_result = Some(app.browser_go_parent())
            }
            KeyCode::Enter => action_result = Some(app.activate_focused_action()),
            KeyCode::Char('a') if app.screen == Screen::RepoPicker => app.begin_add_repo_input(),
            KeyCode::Char('f') if app.screen == Screen::RepoPicker => {
                action_result = Some(app.enter_repo_browser())
            }
            KeyCode::Char('.') if app.screen == Screen::RepoBrowser => {
                action_result = Some(app.toggle_browser_hidden_entries())
            }
            KeyCode::Char('d') if app.screen == Screen::RepoPicker => {
                action_result = Some(app.remove_selected_repo())
            }
            KeyCode::Char('b') if app.screen == Screen::RepoBrowser => app.return_to_picker(),
            KeyCode::Char('b') if app.screen == Screen::RepoView => app.return_to_picker(),
            KeyCode::Char('b') if app.screen == Screen::BranchPicker => app.return_to_repo_view(),
            KeyCode::Char('b') if app.screen == Screen::RemoteBranchPicker => app.return_to_repo_view(),
            KeyCode::Char('b') if app.screen == Screen::HistoryView => app.return_to_repo_view(),
            KeyCode::Char('b') if app.screen == Screen::PullRequestView => app.return_to_repo_view(),
            KeyCode::Char('b') if app.screen == Screen::TrackingStatusView => app.return_to_repo_view(),
            KeyCode::Char('b') if app.screen == Screen::StashView => app.return_to_repo_view(),
            KeyCode::Char('g') if app.screen == Screen::RepoView => {
                action_result = Some(app.enter_branch_picker())
            }
            KeyCode::Char('G') if app.screen == Screen::RepoView => {
                action_result = Some(app.enter_remote_branch_picker())
            }
            KeyCode::Char('G') if app.screen == Screen::BranchPicker => {
                action_result = Some(app.enter_remote_branch_picker())
            }
            KeyCode::Char('g') if app.screen == Screen::RemoteBranchPicker => {
                action_result = Some(app.enter_branch_picker())
            }
            KeyCode::Char('h') if app.screen == Screen::RepoView => {
                action_result = Some(app.enter_history_view())
            }
            KeyCode::Char('i') if app.screen == Screen::RepoView => {
                action_result = Some(app.enter_tracking_status_view())
            }
            KeyCode::Char('t') if app.screen == Screen::RepoView => {
                action_result = Some(app.enter_stash_view())
            }
            KeyCode::Char('P') if app.screen == Screen::RepoView => {
                action_result = Some(app.enter_pull_request_view())
            }
            KeyCode::Char('n') if app.screen == Screen::BranchPicker => app.begin_new_branch_input(),
            KeyCode::Char('n') if app.screen == Screen::PullRequestView => {
                app.begin_create_pull_request_input()
            }
            KeyCode::Char('o') if app.screen == Screen::HistoryView => {
                action_result = Some(app.checkout_selected_commit())
            }
            KeyCode::Char('p') if app.screen == Screen::HistoryView => {
                action_result = Some(app.cherry_pick_selected_commit())
            }
            KeyCode::Char('h') if app.screen == Screen::HistoryView => {
                action_result = Some(app.open_selected_history_file_history())
            }
            KeyCode::Char('o') if app.screen == Screen::PullRequestView => {
                action_result = Some(app.open_selected_pr_in_browser())
            }
            KeyCode::Char('s') if app.screen == Screen::StashView => {
                action_result = Some(app.stash_current_changes())
            }
            KeyCode::Char('a') if app.screen == Screen::StashView => {
                action_result = Some(app.apply_selected_stash())
            }
            KeyCode::Char('p') if app.screen == Screen::StashView => {
                action_result = Some(app.pop_selected_stash())
            }
            KeyCode::Char('d') if app.screen == Screen::StashView => app.begin_drop_selected_stash(),
            KeyCode::Char('c') if app.screen == Screen::PullRequestView => {
                action_result = Some(app.checkout_selected_pr())
            }
            KeyCode::Char('m') if app.screen == Screen::PullRequestView => {
                action_result = Some(app.begin_merge_selected_pr(PullRequestMergeMethod::Merge))
            }
            KeyCode::Char('s') if app.screen == Screen::PullRequestView => {
                action_result = Some(app.begin_merge_selected_pr(PullRequestMergeMethod::Squash))
            }
            KeyCode::Char('R') if app.screen == Screen::PullRequestView => {
                action_result = Some(app.begin_merge_selected_pr(PullRequestMergeMethod::Rebase))
            }
            KeyCode::Char('f') if app.screen == Screen::PullRequestView => {
                action_result = Some(app.cycle_pull_request_filter())
            }
            KeyCode::Char('s') if app.screen == Screen::RepoView => {
                action_result = Some(app.stage_selected())
            }
            KeyCode::Char('S') if app.screen == Screen::RepoView => {
                action_result = Some(app.stage_all_unstaged())
            }
            KeyCode::Char('u') if app.screen == Screen::RepoView => {
                action_result = Some(app.unstage_selected())
            }
            KeyCode::Char('U') if app.screen == Screen::RepoView => {
                action_result = Some(app.unstage_all_staged())
            }
            KeyCode::Char('x') if app.screen == Screen::RepoView => {
                action_result = Some(app.discard_selected_unstaged())
            }
            KeyCode::Char('c') if app.screen == Screen::RepoView => app.begin_commit_input(),
            KeyCode::Char('f') if app.screen == Screen::RepoView => {
                action_result = Some(app.fetch_remotes())
            }
            KeyCode::Char('f') if app.screen == Screen::TrackingStatusView => {
                action_result = Some(app.fetch_remotes())
            }
            KeyCode::Char('l') if app.screen == Screen::RepoView => {
                action_result = Some(app.pull_current_branch())
            }
            KeyCode::Char('l') if app.screen == Screen::TrackingStatusView => {
                action_result = Some(app.pull_current_branch())
            }
            KeyCode::Char('p') if app.screen == Screen::RepoView => {
                action_result = Some(app.push_current_branch())
            }
            KeyCode::Char('p') if app.screen == Screen::TrackingStatusView => {
                action_result = Some(app.push_current_branch())
            }
            KeyCode::Char('v') if app.screen == Screen::RepoView => {
                action_result = Some(app.continue_cherry_pick())
            }
            KeyCode::Char('z') if app.screen == Screen::RepoView => {
                action_result = Some(app.abort_cherry_pick())
            }
            _ => {}
        }
        if let Some(result) = action_result {
            handle_action_result(app, result);
        }
    }

    Ok(())
}

fn handle_action_result(app: &mut App, result: Result<()>) {
    if let Err(err) = result {
        error!("Action failed on screen {:?}: {:#}", app.screen, err);
        app.status_message = format!("Error: {}", err);
    }
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        error!("Application panic: {}\nBacktrace:\n{}", panic_info, backtrace);
        default_hook(panic_info);
    }));
}
