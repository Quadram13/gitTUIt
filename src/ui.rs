use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::app::{App, FocusPane, HistoryFocusPane, InputMode, Screen};
use crate::tree::changed_files_tree::TreeRowKind;

pub fn draw(frame: &mut Frame, app: &App) {
    let root = frame.area();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(8),
            Constraint::Length(4),
        ])
        .split(root);

    draw_header(frame, app, layout[0]);
    draw_body(frame, app, layout[1]);
    draw_footer(frame, app, layout[2]);

    if app.in_input_mode() {
        draw_commit_popup(frame, app);
    }
    if app.help_visible {
        draw_help_popup(frame, app);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let primary_line = if app.is_repo_workspace_screen() {
        Line::from(vec![
            "Repo: ".into(),
            app.active_repo_label().yellow(),
            " | Branch: ".into(),
            app.branch_summary().green(),
            " | ".into(),
            app.last_fetch_summary().green(),
        ])
    } else {
        match app.screen {
            Screen::RepoPicker => Line::from(vec![
                "Main Menu".yellow(),
                " | Tracked Repos: ".into(),
                app.registry.repos.len().to_string().green(),
            ]),
            Screen::RepoBrowser => Line::from(vec![
                "Repo Browser".yellow(),
                " | Directory: ".into(),
                app.active_repo_label().green(),
            ]),
            _ => Line::from("gitTUIt"),
        }
    };
    let mut lines = vec![primary_line];
    if app.is_repo_workspace_screen() {
        lines.push(workspace_tabs_line(app));
    }

    let header = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title("gitTUIt"));
    frame.render_widget(header, area);
}

fn workspace_tabs_line(app: &App) -> Line<'static> {
    let active = app.active_workspace_tab_index();
    let labels = [
        (1u8, "Status"),
        (2u8, "Branches"),
        (3u8, "History"),
        (4u8, "Incoming/Outgoing"),
        (5u8, "Stash"),
        (6u8, "Pull Requests"),
    ];
    let mut spans = vec!["Tabs: ".into()];
    for (idx, label) in labels {
        let text = format!("{label} [{idx}]");
        if Some(idx) == active {
            spans.push(text.yellow());
        } else {
            spans.push(text.into());
        }
        spans.push("  ".into());
    }
    Line::from(spans)
}

fn draw_body(frame: &mut Frame, app: &App, area: Rect) {
    if app.is_fullscreen_diff_visible() {
        draw_fullscreen_diff(frame, app, area);
        return;
    }
    if app.screen == Screen::RepoPicker {
        draw_repo_picker(frame, app, area);
        return;
    }
    if app.screen == Screen::RepoBrowser {
        draw_repo_browser(frame, app, area);
        return;
    }
    if app.screen == Screen::BranchPicker {
        draw_branch_picker(frame, app, area);
        return;
    }
    if app.screen == Screen::RemoteBranchPicker {
        draw_remote_branch_picker(frame, app, area);
        return;
    }
    if app.screen == Screen::HistoryView {
        draw_history_view(frame, app, area);
        return;
    }
    if app.screen == Screen::PullRequestView {
        draw_pull_request_view(frame, app, area);
        return;
    }
    if app.screen == Screen::TrackingStatusView {
        draw_tracking_status_view(frame, app, area);
        return;
    }
    if app.screen == Screen::StashView {
        draw_stash_view(frame, app, area);
        return;
    }

    draw_repo_status_view(frame, app, area);
}

fn draw_repo_picker(frame: &mut Frame, app: &App, area: Rect) {
    let columns = split_master_detail(area, 55, 45);

    let items = if app.registry.repos.is_empty() {
        vec![ListItem::new("No repositories tracked")]
    } else {
        app.registry
            .repos
            .iter()
            .enumerate()
            .map(|(idx, repo)| {
                let label = app
                    .repo_picker_label(idx)
                    .map(str::to_string)
                    .unwrap_or_else(|| app.format_path_for_ui(&repo.path));
                ListItem::new(label)
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Main Menu"),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.registry.repos.is_empty() {
        state.select(Some(app.selected_repo));
    }
    frame.render_stateful_widget(list, columns[0], &mut state);

    let help = Paragraph::new(app.preview_text())
        .block(Block::default().borders(Borders::ALL).title("Details"))
        .wrap(Wrap { trim: false });
    frame.render_widget(help, columns[1]);
}

fn draw_repo_browser(frame: &mut Frame, app: &App, area: Rect) {
    let columns = split_master_detail(area, 55, 45);

    let items = if app.browser_entries.is_empty() {
        vec![ListItem::new("(empty directory)")]
    } else {
        app.browser_entries
            .iter()
            .map(|entry| {
                let mut label = if entry.is_dir {
                    format!("[D] {}", entry.name)
                } else {
                    format!("[F] {}", entry.name)
                };
                if entry.is_git_root {
                    label.push_str("  [git]");
                }
                ListItem::new(label)
            })
            .collect()
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Repo Browser"),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.browser_entries.is_empty() {
        state.select(Some(app.selected_browser));
    }
    frame.render_stateful_widget(list, columns[0], &mut state);

    let details = Paragraph::new(app.preview_text())
        .block(Block::default().borders(Borders::ALL).title("Details"))
        .wrap(Wrap { trim: false });
    frame.render_widget(details, columns[1]);
}

fn draw_branch_picker(frame: &mut Frame, app: &App, area: Rect) {
    let columns = split_master_detail(area, 55, 45);

    let items = if app.branch_entries.is_empty() {
        vec![ListItem::new("No local branches")]
    } else {
        app.branch_entries
            .iter()
            .map(|entry| {
                let label = if entry.is_current {
                    format!("* {}", entry.name)
                } else {
                    format!("  {}", entry.name)
                };
                ListItem::new(label)
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Branches"),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.branch_entries.is_empty() {
        state.select(Some(app.selected_branch));
    }
    frame.render_stateful_widget(list, columns[0], &mut state);

    let details = Paragraph::new(app.preview_text())
        .block(Block::default().borders(Borders::ALL).title("Details"))
        .wrap(Wrap { trim: false });
    frame.render_widget(details, columns[1]);
}

fn draw_history_view(frame: &mut Frame, app: &App, area: Rect) {
    if !app.history_details_visible() {
        let items = if app.history_entries.is_empty() {
            vec![ListItem::new("No commits found")]
        } else {
            app.history_entries
                .iter()
                .map(|entry| {
                    ListItem::new(format!(
                        "{} {} ({}, {})",
                        entry.short_hash, entry.summary, entry.author, entry.relative_time
                    ))
                })
                .collect()
        };
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("History ([Enter] show details)"),
            )
            .highlight_style(Style::default().bg(Color::DarkGray))
            .highlight_symbol("> ");
        let mut state = ListState::default();
        if !app.history_entries.is_empty() {
            state.select(Some(app.selected_history));
        }
        frame.render_stateful_widget(list, area, &mut state);
        return;
    }

    let columns = split_master_detail(area, 45, 55);

    let items = if app.history_entries.is_empty() {
        vec![ListItem::new("No commits found")]
    } else {
        app.history_entries
            .iter()
            .map(|entry| {
                ListItem::new(format!(
                    "{} {} ({}, {})",
                    entry.short_hash, entry.summary, entry.author, entry.relative_time
                ))
            })
            .collect()
    };

    let list_border_style = if app.history_focus() == HistoryFocusPane::Commits {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("History ([Enter] hide details)")
                .border_style(list_border_style),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.history_entries.is_empty() {
        state.select(Some(app.selected_history));
    }
    frame.render_stateful_widget(list, columns[0], &mut state);

    let detail_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Percentage(35),
            Constraint::Min(6),
        ])
        .split(columns[1]);
    let metadata = Paragraph::new(app.history_metadata_text())
        .block(Block::default().borders(Borders::ALL).title("Metadata"))
        .wrap(Wrap { trim: false });
    frame.render_widget(metadata, detail_rows[0]);

    let message = Paragraph::new(app.history_message_text())
        .block(Block::default().borders(Borders::ALL).title("Message"))
        .wrap(Wrap { trim: false });
    frame.render_widget(message, detail_rows[1]);

    let file_border_style = if app.history_focus() != HistoryFocusPane::Commits {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let file_items = app
        .history_files_items()
        .into_iter()
        .map(ListItem::new)
        .collect::<Vec<_>>();
    let files = List::new(file_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(app.history_files_title())
                .border_style(file_border_style),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");
    let mut files_state = ListState::default();
    files_state.select(app.history_files_selected_index());
    frame.render_stateful_widget(files, detail_rows[2], &mut files_state);
}

fn draw_remote_branch_picker(frame: &mut Frame, app: &App, area: Rect) {
    let columns = split_master_detail(area, 55, 45);

    let items = if app.remote_branch_entries.is_empty() {
        vec![ListItem::new("No remote branches found")]
    } else {
        app.remote_branch_entries
            .iter()
            .map(|entry| ListItem::new(entry.name.clone()))
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Remote Branches"),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.remote_branch_entries.is_empty() {
        state.select(Some(app.selected_remote_branch));
    }
    frame.render_stateful_widget(list, columns[0], &mut state);

    let details = Paragraph::new(app.preview_text())
        .block(Block::default().borders(Borders::ALL).title("Details"))
        .wrap(Wrap { trim: false });
    frame.render_widget(details, columns[1]);
}

fn draw_pull_request_view(frame: &mut Frame, app: &App, area: Rect) {
    let columns = split_master_detail(area, 55, 45);

    let items = if app.pull_requests.is_empty() {
        vec![ListItem::new("No pull requests for current filter")]
    } else {
        app.pull_requests
            .iter()
            .map(|pr| {
                let draft_marker = if pr.is_draft { " [draft]" } else { "" };
                ListItem::new(format!("#{} {}{}", pr.number, pr.title, draft_marker))
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Pull Requests"),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.pull_requests.is_empty() {
        state.select(Some(app.selected_pr));
    }
    frame.render_stateful_widget(list, columns[0], &mut state);

    let details = Paragraph::new(app.preview_text())
        .block(Block::default().borders(Borders::ALL).title("PR Details"))
        .wrap(Wrap { trim: false });
    frame.render_widget(details, columns[1]);
}

fn draw_tracking_status_view(frame: &mut Frame, app: &App, area: Rect) {
    let details = Paragraph::new(app.preview_text())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Incoming / Outgoing Commits"),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(details, area);
}

fn draw_stash_view(frame: &mut Frame, app: &App, area: Rect) {
    let columns = split_master_detail(area, 45, 55);

    let items = if app.stash_entries.is_empty() {
        vec![ListItem::new("No stash entries")]
    } else {
        app.stash_entries
            .iter()
            .map(|entry| ListItem::new(format!("{} {}", entry.reference, entry.message)))
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Stash"),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.stash_entries.is_empty() {
        state.select(Some(app.selected_stash));
    }
    frame.render_stateful_widget(list, columns[0], &mut state);

    let details = Paragraph::new(app.preview_text())
        .block(Block::default().borders(Borders::ALL).title("Stash Details"))
        .wrap(Wrap { trim: false });
    frame.render_widget(details, columns[1]);
}

fn draw_unstaged(frame: &mut Frame, app: &App, area: Rect) {
    let items = if app.unstaged_tree_rows().is_empty() {
        vec![ListItem::new("No unstaged changes")]
    } else {
        app.unstaged_tree_rows()
            .iter()
            .map(|entry| {
                let indent = "  ".repeat(entry.depth);
                let label = match entry.kind {
                    TreeRowKind::Directory => {
                        let marker = if entry.expanded { "[-]" } else { "[+]" };
                        format!("{indent}{marker} {}", entry.name)
                    }
                    TreeRowKind::File => {
                        format!(
                            "{indent}[{}] {}",
                            entry.status.as_deref().unwrap_or("?"),
                            entry.name
                        )
                    }
                };
                ListItem::new(label)
            })
            .collect()
    };
    let border_style = if app.focus == FocusPane::Unstaged {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Unstaged ([s/S] stage/all, [x] discard file, [Left/Right] fold/expand)")
                .border_style(border_style),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.unstaged_tree_rows().is_empty() {
        state.select(Some(app.selected_unstaged));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_staged(frame: &mut Frame, app: &App, area: Rect) {
    let items = if app.staged_tree_rows().is_empty() {
        vec![ListItem::new("No staged changes")]
    } else {
        app.staged_tree_rows()
            .iter()
            .map(|entry| {
                let indent = "  ".repeat(entry.depth);
                let label = match entry.kind {
                    TreeRowKind::Directory => {
                        let marker = if entry.expanded { "[-]" } else { "[+]" };
                        format!("{indent}{marker} {}", entry.name)
                    }
                    TreeRowKind::File => {
                        format!(
                            "{indent}[{}] {}",
                            entry.status.as_deref().unwrap_or("?"),
                            entry.name
                        )
                    }
                };
                ListItem::new(label)
            })
            .collect()
    };
    let border_style = if app.focus == FocusPane::Staged {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Staged ([u/U] unstage/all, [c] commit, [Left/Right] fold/expand)")
                .border_style(border_style),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.staged_tree_rows().is_empty() {
        state.select(Some(app.selected_staged));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_preview(frame: &mut Frame, app: &App, area: Rect) {
    let preview = Paragraph::new(diff_colored_text(&app.preview_text()))
        .block(Block::default().borders(Borders::ALL).title("Preview / Output"))
        .wrap(Wrap { trim: false });
    frame.render_widget(preview, area);
}

fn draw_repo_status_view(frame: &mut Frame, app: &App, area: Rect) {
    if area.width >= 140 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(30),
                Constraint::Percentage(30),
                Constraint::Percentage(40),
            ])
            .split(area);
        draw_unstaged(frame, app, columns[0]);
        draw_staged(frame, app, columns[1]);
        draw_preview(frame, app, columns[2]);
        return;
    }

    if area.width >= 96 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        let left_stack = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(columns[0]);
        draw_unstaged(frame, app, left_stack[0]);
        draw_staged(frame, app, left_stack[1]);
        draw_preview(frame, app, columns[1]);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);
    draw_unstaged(frame, app, rows[0]);
    draw_staged(frame, app, rows[1]);
    draw_preview(frame, app, rows[2]);
}

fn split_master_detail(area: Rect, master_percent: u16, detail_percent: u16) -> Vec<Rect> {
    if area.width >= 96 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(master_percent),
                Constraint::Percentage(detail_percent),
            ])
            .split(area)
            .to_vec()
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
            .split(area)
            .to_vec()
    }
}

fn draw_footer(frame: &mut Frame, app: &App, _area: Rect) {
    let lines = vec![
        Line::from(app.status_message.clone()),
        Line::from("[?] help | [q] quit | [r] refresh | [L] log"),
    ];

    let footer = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title("Status"));
    frame.render_widget(footer, _area);
}

fn draw_fullscreen_diff(frame: &mut Frame, app: &App, area: Rect) {
    let body_height = area.height.saturating_sub(2) as usize;
    let body_width = area.width.saturating_sub(2) as usize;
    let visible_lines = app.fullscreen_diff_visible_lines(body_width, body_height);
    let (scroll_y, scroll_x) = app.fullscreen_diff_scroll_position().unwrap_or((0, 0));
    let title = app
        .fullscreen_diff_title()
        .map(|title| {
            format!(
                "{title} (y:{scroll_y} x:{scroll_x}) [Esc] close | [j/k] scroll | [Left/Right]/[h/l] horizontal | [n/p] hunks"
            )
        })
        .unwrap_or_else(|| "Diff Viewer".to_string());
    let paragraph = Paragraph::new(diff_colored_text_from_lines(visible_lines))
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn diff_colored_text(input: &str) -> Text<'static> {
    let lines = input.lines().map(|line| colorize_diff_line(line.to_string())).collect::<Vec<_>>();
    Text::from(lines)
}

fn diff_colored_text_from_lines(lines: Vec<String>) -> Text<'static> {
    Text::from(
        lines
            .into_iter()
            .map(colorize_diff_line)
            .collect::<Vec<_>>(),
    )
}

fn colorize_diff_line(line: String) -> Line<'static> {
    if line.starts_with("+++") || line.starts_with("---") {
        return Line::from(Span::styled(line, Style::default().fg(Color::Yellow)));
    }
    if line.starts_with("@@") {
        return Line::from(Span::styled(line, Style::default().fg(Color::Cyan)));
    }
    if line.starts_with('+') {
        return Line::from(Span::styled(line, Style::default().fg(Color::Green)));
    }
    if line.starts_with('-') {
        return Line::from(Span::styled(line, Style::default().fg(Color::Red)));
    }
    Line::from(line)
}

fn draw_commit_popup(frame: &mut Frame, app: &App) {
    let popup = match app.input_mode {
        InputMode::CommitBody => centered_rect_with_min(92, 45, frame.area(), 56, 10),
        _ => centered_rect_with_min(82, 24, frame.area(), 52, 5),
    };
    let title = truncate_for_width(app.popup_title(), popup.width.saturating_sub(4) as usize);
    frame.render_widget(Clear, popup);
    let (popup_text, cursor_line, cursor_col) = popup_text_with_cursor(app);
    let body_height = popup.height.saturating_sub(2) as usize;
    let body_width = popup.width.saturating_sub(2) as usize;
    let scroll_y = if body_height == 0 {
        0
    } else {
        cursor_line.saturating_sub(body_height.saturating_sub(1))
    };
    let scroll_x = if body_width == 0 {
        0
    } else {
        cursor_col.saturating_sub(body_width.saturating_sub(1))
    };
    let input = Paragraph::new(popup_text)
        .scroll((
            scroll_y.min(u16::MAX as usize) as u16,
            scroll_x.min(u16::MAX as usize) as u16,
        ))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title),
        );
    frame.render_widget(input, popup);
}

fn popup_text_with_cursor(app: &App) -> (Text<'static>, usize, usize) {
    let Some(input) = app.popup_input_text() else {
        return (Text::from(app.popup_body()), 0, 0);
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(prefix) = app.popup_input_prefix() {
        if !prefix.is_empty() {
            for line in prefix.split('\n') {
                lines.push(Line::from(line.to_string()));
            }
        }
    }

    let chars = input.chars().collect::<Vec<_>>();
    let cursor = app.popup_input_cursor().min(chars.len());
    let mut input_cursor_col = 0usize;
    for ch in chars.iter().take(cursor) {
        if *ch == '\n' {
            input_cursor_col = 0;
        } else {
            input_cursor_col += 1;
        }
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut current_line_idx = lines.len();
    let mut cursor_line: Option<usize> = None;
    for (idx, ch) in chars.iter().enumerate() {
        if idx == cursor && *ch == '\n' {
            if cursor_line.is_none() {
                cursor_line = Some(current_line_idx);
            }
            spans.push(Span::styled(
                " ".to_string(),
                Style::default().add_modifier(Modifier::REVERSED),
            ));
            lines.push(Line::from(std::mem::take(&mut spans)));
            current_line_idx += 1;
            continue;
        }
        if *ch == '\n' {
            lines.push(Line::from(std::mem::take(&mut spans)));
            current_line_idx += 1;
            continue;
        }
        if idx == cursor {
            if cursor_line.is_none() {
                cursor_line = Some(current_line_idx);
            }
            spans.push(Span::styled(
                ch.to_string(),
                Style::default().add_modifier(Modifier::REVERSED),
            ));
        } else {
            spans.push(Span::raw(ch.to_string()));
        }
    }
    if cursor == chars.len() {
        if cursor_line.is_none() {
            cursor_line = Some(current_line_idx);
        }
        spans.push(Span::styled(
            " ".to_string(),
            Style::default().add_modifier(Modifier::REVERSED),
        ));
    }

    if spans.is_empty() && lines.is_empty() {
        spans.push(Span::styled(
            " ".to_string(),
            Style::default().add_modifier(Modifier::REVERSED),
        ));
    }

    if !spans.is_empty() || lines.is_empty() {
        lines.push(Line::from(spans));
    }
    (
        Text::from(lines),
        cursor_line.unwrap_or(current_line_idx),
        input_cursor_col,
    )
}

fn draw_help_popup(frame: &mut Frame, app: &App) {
    let popup = centered_rect(80, 70, frame.area());
    frame.render_widget(Clear, popup);
    let body_height = popup.height.saturating_sub(2) as usize;
    let total_lines = app.help_line_count();
    let has_overflow = total_lines > body_height && body_height > 0;
    let title = if has_overflow {
        format!(
            "{} ([j/k]/[PgUp]/[PgDn]/[Home]/[End] scroll, [?]/[Esc] close)",
            app.help_popup_title()
        )
    } else {
        format!("{} ([?]/[Esc] close)", app.help_popup_title())
    };
    let help = Paragraph::new(app.help_popup_body())
        .scroll((app.help_scroll.min(u16::MAX as usize) as u16, 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(help, popup);
}


fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn centered_rect_with_min(
    percent_x: u16,
    percent_y: u16,
    area: Rect,
    min_width: u16,
    min_height: u16,
) -> Rect {
    let max_width = area.width.saturating_sub(2).max(1);
    let max_height = area.height.saturating_sub(2).max(1);
    let desired_width = area.width.saturating_mul(percent_x).saturating_div(100);
    let desired_height = area.height.saturating_mul(percent_y).saturating_div(100);
    let width = desired_width.max(min_width).min(max_width);
    let height = desired_height.max(min_height).min(max_height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}

fn truncate_for_width(input: &str, width: usize) -> String {
    let length = input.chars().count();
    if length <= width {
        return input.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let truncated: String = input.chars().take(width - 3).collect();
    format!("{truncated}...")
}
