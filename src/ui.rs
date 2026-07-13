use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::app::{App, FocusPane, Screen};

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
    let primary_line = match app.screen {
        Screen::RepoPicker => Line::from(vec![
            "Mode: ".into(),
            "Main Menu".yellow(),
            " | Tracked Repos: ".into(),
            app.registry.repos.len().to_string().green(),
        ]),
        Screen::RepoBrowser => Line::from(vec![
            "Mode: ".into(),
            "Repo Browser".yellow(),
            " | Directory: ".into(),
            app.active_repo_label().green(),
        ]),
        Screen::BranchPicker => Line::from(vec![
            "Mode: ".into(),
            "Branch Picker".yellow(),
            " | Repo: ".into(),
            app.active_repo_label().green(),
            " | ".into(),
            app.last_fetch_summary().green(),
        ]),
        Screen::RemoteBranchPicker => Line::from(vec![
            "Mode: ".into(),
            "Remote Branches".yellow(),
            " | Repo: ".into(),
            app.active_repo_label().green(),
            " | ".into(),
            app.last_fetch_summary().green(),
        ]),
        Screen::HistoryView => Line::from(vec![
            "Mode: ".into(),
            "History".yellow(),
            " | Repo: ".into(),
            app.active_repo_label().green(),
            " | ".into(),
            app.last_fetch_summary().green(),
        ]),
        Screen::PullRequestView => Line::from(vec![
            "Mode: ".into(),
            "Pull Requests".yellow(),
            " | Filter: ".into(),
            app.pull_request_filter_label().green(),
            " | ".into(),
            app.last_fetch_summary().green(),
        ]),
        Screen::TrackingStatusView => Line::from(vec![
            "Mode: ".into(),
            "Incoming/Outgoing".yellow(),
            " | Repo: ".into(),
            app.active_repo_label().green(),
            " | ".into(),
            app.last_fetch_summary().green(),
        ]),
        Screen::StashView => Line::from(vec![
            "Mode: ".into(),
            "Stash".yellow(),
            " | Repo: ".into(),
            app.active_repo_label().green(),
            " | ".into(),
            app.last_fetch_summary().green(),
        ]),
        Screen::RepoView => Line::from(vec![
            "Repo: ".into(),
            app.active_repo_label().yellow(),
            " | Branch: ".into(),
            app.branch_summary().green(),
            " | ".into(),
            app.last_fetch_summary().green(),
        ]),
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
}

fn draw_repo_picker(frame: &mut Frame, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

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
                .title("Main Menu ([Enter] open, [a] add path, [f] browse folders, [d] remove)"),
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
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

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
                .title("Browser ([Enter] open/add, [Backspace] up, [.] hidden, [b] back)"),
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
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

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
                .title("Branches ([Enter] switch, [n] new branch, [b] back)"),
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
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

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
                .title("History ([Enter] details, [o] checkout, [p] cherry-pick, [b] back)"),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.history_entries.is_empty() {
        state.select(Some(app.selected_history));
    }
    frame.render_stateful_widget(list, columns[0], &mut state);

    let details = Paragraph::new(app.preview_text())
        .block(Block::default().borders(Borders::ALL).title("Commit Details"))
        .wrap(Wrap { trim: false });
    frame.render_widget(details, columns[1]);
}

fn draw_remote_branch_picker(frame: &mut Frame, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

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
                .title("Remote Branches ([Enter] checkout tracking branch, [b] back)"),
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
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

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
                .title("Pull Requests ([Enter/o] open, [c] checkout, [m]/[s]/[R] merge, [n] create, [f] filter, [b] back)"),
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
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

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
                .title("Stash ([s] stash now, [a] apply, [p] pop, [d] drop, [b] back)"),
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
    let items = if app.snapshot.unstaged.is_empty() {
        vec![ListItem::new("No unstaged changes")]
    } else {
        app.snapshot
            .unstaged
            .iter()
            .map(|entry| ListItem::new(format!("[{}] {}", entry.status, entry.path)))
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
                .title("Unstaged ([s] stage, [S] stage all, [x] discard selected)")
                .border_style(border_style),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.snapshot.unstaged.is_empty() {
        state.select(Some(app.selected_unstaged));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_staged(frame: &mut Frame, app: &App, area: Rect) {
    let items = if app.snapshot.staged.is_empty() {
        vec![ListItem::new("No staged changes")]
    } else {
        app.snapshot
            .staged
            .iter()
            .map(|entry| ListItem::new(format!("[{}] {}", entry.status, entry.path)))
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
                .title("Staged ([u] unstage, [U] unstage all, [c] commit)")
                .border_style(border_style),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.snapshot.staged.is_empty() {
        state.select(Some(app.selected_staged));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_preview(frame: &mut Frame, app: &App, area: Rect) {
    let preview = Paragraph::new(app.preview_text())
        .block(Block::default().borders(Borders::ALL).title("Preview / Output"))
        .wrap(Wrap { trim: false });
    frame.render_widget(preview, area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let lines = match app.screen {
        Screen::RepoPicker => vec![
            Line::from(app.status_message.clone()),
            Line::from(compose_compact_footer(
                area.width,
                &[
                    "[j/k] move",
                    "[Enter] open",
                    "[a] add",
                    "[f] browse",
                    "[d] remove",
                    "[r] refresh",
                    "[L] log",
                    "[?] help",
                    "[q] quit",
                ],
            )),
        ],
        Screen::RepoBrowser => vec![
            Line::from(app.status_message.clone()),
            Line::from(compose_compact_footer(
                area.width,
                &[
                    "[j/k] move",
                    "[Enter] open/add",
                    "[Backspace] parent",
                    "[.] hidden",
                    "[b] back",
                    "[r] refresh",
                    "[L] log",
                    "[?] help",
                    "[q] quit",
                ],
            )),
        ],
        Screen::BranchPicker => vec![
            Line::from(app.status_message.clone()),
            Line::from(compose_compact_footer(
                area.width,
                &[
                    "[j/k] move",
                    "[Enter] switch",
                    "[n] new",
                    "[1-6] tabs",
                    "[b] back",
                    "[r] refresh",
                    "[L] log",
                    "[?] help",
                    "[q] quit",
                ],
            )),
        ],
        Screen::RemoteBranchPicker => vec![
            Line::from(app.status_message.clone()),
            Line::from(compose_compact_footer(
                area.width,
                &[
                    "[j/k] move",
                    "[Enter] checkout",
                    "[g] local",
                    "[1-6] tabs",
                    "[b] back",
                    "[r] refresh",
                    "[L] log",
                    "[?] help",
                    "[q] quit",
                ],
            )),
        ],
        Screen::HistoryView => vec![
            Line::from(app.status_message.clone()),
            Line::from(compose_compact_footer(
                area.width,
                &[
                    "[j/k] move",
                    "[Enter] details",
                    "[o] checkout",
                    "[p] cherry-pick",
                    "[1-6] tabs",
                    "[b] back",
                    "[r] refresh",
                    "[L] log",
                    "[?] help",
                    "[q] quit",
                ],
            )),
        ],
        Screen::PullRequestView => vec![
            Line::from(app.status_message.clone()),
            Line::from(compose_compact_footer(
                area.width,
                &[
                    "[j/k] move",
                    "[Enter/o] open",
                    "[c] checkout",
                    "[m/s/R] merge",
                    "[n] create",
                    "[f] filter",
                    "[1-6] tabs",
                    "[b] back",
                    "[r] refresh",
                    "[L] log",
                    "[?] help",
                    "[q] quit",
                ],
            )),
        ],
        Screen::TrackingStatusView => vec![
            Line::from(app.status_message.clone()),
            Line::from(compose_compact_footer(
                area.width,
                &[
                    "[1-6] tabs",
                    "[b] back",
                    "[r] refresh",
                    "[L] log",
                    "[?] help",
                    "[q] quit",
                ],
            )),
        ],
        Screen::StashView => vec![
            Line::from(app.status_message.clone()),
            Line::from(compose_compact_footer(
                area.width,
                &[
                    "[j/k] move",
                    "[Enter] details",
                    "[s] stash",
                    "[a] apply",
                    "[p] pop",
                    "[d] drop",
                    "[1-6] tabs",
                    "[b] back",
                    "[r] refresh",
                    "[L] log",
                    "[?] help",
                    "[q] quit",
                ],
            )),
        ],
        Screen::RepoView => vec![
            Line::from(app.status_message.clone()),
            Line::from(compose_compact_footer(
                area.width,
                &[
                    "[Tab] pane",
                    "[j/k] move",
                    "[s/S] stage",
                    "[u/U] unstage",
                    "[c] commit",
                    "[g/G] branches",
                    "[h] history",
                    "[i] in/out",
                    "[t] stash",
                    "[P] PRs",
                    "[f/l/p] sync",
                    "[1-6] tabs",
                    "[b] repos",
                    "[r] refresh",
                    "[L] log",
                    "[?] help",
                    "[q] quit",
                ],
            )),
        ],
    };

    let footer = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title("Status"));
    frame.render_widget(footer, area);
}

fn draw_commit_popup(frame: &mut Frame, app: &App) {
    let popup = centered_rect(70, 18, frame.area());
    frame.render_widget(Clear, popup);
    let popup_text = popup_text_with_cursor(app);
    let input = Paragraph::new(popup_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(app.popup_title()),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(input, popup);
}

fn popup_text_with_cursor(app: &App) -> Text<'static> {
    let Some(input) = app.popup_input_text() else {
        return Text::from(app.popup_body());
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
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (idx, ch) in chars.iter().enumerate() {
        if idx == cursor {
            spans.push(Span::styled(
                ch.to_string(),
                Style::default().add_modifier(Modifier::REVERSED),
            ));
        } else {
            spans.push(Span::raw(ch.to_string()));
        }
    }
    if cursor == chars.len() {
        spans.push(Span::styled(
            " ".to_string(),
            Style::default().add_modifier(Modifier::REVERSED),
        ));
    }

    if spans.is_empty() {
        spans.push(Span::styled(
            " ".to_string(),
            Style::default().add_modifier(Modifier::REVERSED),
        ));
    }

    lines.push(Line::from(spans));
    Text::from(lines)
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

fn compose_compact_footer(width: u16, hints: &[&str]) -> String {
    let available = width.saturating_sub(4) as usize;
    if available == 0 {
        return String::new();
    }

    let mut parts: Vec<String> = Vec::new();
    for hint in hints {
        let candidate = if parts.is_empty() {
            hint.to_string()
        } else {
            format!("{} | {}", parts.join(" | "), hint)
        };
        if candidate.len() > available {
            break;
        }
        parts.push((*hint).to_string());
    }

    if parts.is_empty() {
        return truncate_for_width(hints.first().copied().unwrap_or(""), available);
    }

    let output = parts.join(" | ");
    if output.len() <= available {
        output
    } else {
        truncate_for_width(&output, available)
    }
}

fn truncate_for_width(input: &str, width: usize) -> String {
    if input.len() <= width {
        return input.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    format!("{}...", &input[..width - 3])
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
