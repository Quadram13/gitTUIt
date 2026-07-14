use anyhow::Result;

use crate::git;

use super::{
    App, AppAsyncEvent, HistoryFocusPane, JOB_HISTORY_DETAILS, JOB_HISTORY_ENTRIES,
    JOB_HISTORY_FILE_HISTORY, Screen,
};

impl App {
    pub fn enter_history_view(&mut self) -> Result<()> {
        if !self.has_open_repo() {
            self.status_message = "Open a repository before viewing history".to_string();
            return Ok(());
        }
        self.screen = Screen::HistoryView;
        self.history_details_visible = false;
        self.history_focus = HistoryFocusPane::Commits;
        self.clear_history_details();
        self.clear_history_file_history();
        self.refresh_history_entries()?;
        self.status_message = "History: [Enter] toggle details, [b] back".to_string();
        Ok(())
    }

    pub fn toggle_history_details(&mut self) -> Result<()> {
        if self.screen != Screen::HistoryView {
            return Ok(());
        }
        self.history_details_visible = !self.history_details_visible;
        self.history_focus = HistoryFocusPane::Commits;
        if self.history_details_visible {
            self.load_selected_commit_details()?;
            self.status_message =
                "History details shown ([Left/Right] focus, [h] file history, [Right] open diff)"
                    .to_string();
        } else {
            self.status_message = "History details hidden".to_string();
        }
        Ok(())
    }

    pub fn load_selected_commit_details(&mut self) -> Result<()> {
        let Some(entry) = self.history_entries.get(self.selected_history) else {
            self.status_message = "No commit selected".to_string();
            return Ok(());
        };
        if self.history_details_for.as_deref() == Some(entry.hash.as_str()) && self.history_details.is_some() {
            self.status_message = format!("Commit details already loaded ({})", entry.short_hash);
            return Ok(());
        }

        let commit_hash = entry.hash.clone();
        let short_hash = entry.short_hash.clone();
        let root = self.current_repo_root()?.to_path_buf();
        self.history_details_request_seq = self.history_details_request_seq.saturating_add(1);
        let request_id = self.history_details_request_seq;
        self.history_details_inflight = Some(request_id);
        self.set_async_running_status(&format!("Loading commit details {}", short_hash));
        let tx = self.async_tx.clone();
        if let Err(err) = self.queue_cancellable_async_task(JOB_HISTORY_DETAILS, request_id, move || {
            let details =
                git::commit_details_structured(&root, &commit_hash).map_err(|err| err.to_string());
            let _ = tx.send(AppAsyncEvent::HistoryDetailsReady {
                request_id,
                commit_hash,
                short_hash,
                details,
            });
        }) {
            self.history_details_inflight = None;
            return Err(err);
        }
        Ok(())
    }

    pub fn history_focus_right(&mut self) -> Result<()> {
        if !self.history_details_visible {
            return Ok(());
        }
        match self.history_focus {
            HistoryFocusPane::Commits => {
                self.history_focus = HistoryFocusPane::ChangedFiles;
                self.status_message = "History focus: changed files".to_string();
            }
            HistoryFocusPane::ChangedFiles => self.open_selected_history_file_diff()?,
            HistoryFocusPane::FileHistory => self.open_selected_history_file_history_diff()?,
        }
        Ok(())
    }

    pub fn history_focus_left(&mut self) {
        match self.history_focus {
            HistoryFocusPane::Commits => {}
            HistoryFocusPane::ChangedFiles => {
                self.history_focus = HistoryFocusPane::Commits;
                self.clear_history_file_history();
                self.status_message = "History focus: commits".to_string();
            }
            HistoryFocusPane::FileHistory => {
                self.history_focus = HistoryFocusPane::ChangedFiles;
                self.status_message = "History focus: changed files".to_string();
            }
        }
    }

    pub fn open_selected_history_file_history(&mut self) -> Result<()> {
        if !self.history_details_visible || self.history_focus == HistoryFocusPane::Commits {
            return Ok(());
        }
        let Some(entry) = self.history_entries.get(self.selected_history) else {
            self.status_message = "No commit selected".to_string();
            return Ok(());
        };
        let Some(path) = self.selected_history_file_path() else {
            self.status_message = "No changed file selected".to_string();
            return Ok(());
        };

        let commit_hash = entry.hash.clone();
        let root = self.current_repo_root()?.to_path_buf();
        self.history_file_history_request_seq = self.history_file_history_request_seq.saturating_add(1);
        let request_id = self.history_file_history_request_seq;
        self.history_file_history_inflight = Some(request_id);
        self.history_file_history_for_path = Some(path.clone());
        let tx = self.async_tx.clone();
        self.set_async_running_status(&format!("Loading file history for {}", path));
        if let Err(err) =
            self.queue_cancellable_async_task(JOB_HISTORY_FILE_HISTORY, request_id, move || {
                let entries = git::file_history(&root, &path, 80).map_err(|err| err.to_string());
                let _ = tx.send(AppAsyncEvent::HistoryFileHistoryReady {
                    request_id,
                    commit_hash,
                    path,
                    entries,
                });
            })
        {
            self.history_file_history_inflight = None;
            return Err(err);
        }
        Ok(())
    }

    pub fn open_selected_history_file_diff(&mut self) -> Result<()> {
        let Some(commit) = self.history_entries.get(self.selected_history) else {
            self.status_message = "No commit selected".to_string();
            return Ok(());
        };
        let Some(path) = self.selected_history_file_path() else {
            self.status_message = "No changed file selected".to_string();
            return Ok(());
        };
        let root = self.current_repo_root()?.to_path_buf();
        let commit_hash = commit.hash.clone();
        let commit_short = commit.short_hash.clone();
        let title = format!("Commit {commit_short} :: {path}");
        let key = format!("history:{commit_hash}:{path}");
        self.request_fullscreen_diff(title, key, move || {
            git::commit_file_diff(&root, &commit_hash, &path)
        })?;
        Ok(())
    }

    pub fn open_selected_history_file_history_diff(&mut self) -> Result<()> {
        let Some(path) = self.history_file_history_for_path.clone() else {
            self.status_message = "No file history loaded".to_string();
            return Ok(());
        };
        let Some(entry) = self
            .history_file_history_entries
            .get(self.history_file_history_selected)
        else {
            self.status_message = "No file-history commit selected".to_string();
            return Ok(());
        };
        let root = self.current_repo_root()?.to_path_buf();
        let commit_hash = entry.hash.clone();
        let short_hash = entry.short_hash.clone();
        let title = format!("File History {short_hash} :: {path}");
        let key = format!("history-file:{commit_hash}:{path}");
        self.request_fullscreen_diff(title, key, move || {
            git::commit_file_diff(&root, &commit_hash, &path)
        })?;
        Ok(())
    }

    pub fn checkout_selected_commit(&mut self) -> Result<()> {
        let Some(entry) = self.history_entries.get(self.selected_history) else {
            self.status_message = "No commit selected".to_string();
            return Ok(());
        };

        let commit_hash = entry.hash.clone();
        let short_hash = entry.short_hash.clone();
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            &format!("Checking out {}", short_hash),
            super::AsyncWriteOp::CheckoutDetached {
                short_hash: short_hash.clone(),
            },
            move || git::checkout_detached(&root, &commit_hash),
        );
        Ok(())
    }

    pub fn cherry_pick_selected_commit(&mut self) -> Result<()> {
        let Some(entry) = self.history_entries.get(self.selected_history) else {
            self.status_message = "No commit selected".to_string();
            return Ok(());
        };

        let commit_hash = entry.hash.clone();
        let short_hash = entry.short_hash.clone();
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            &format!("Cherry-picking {}", short_hash),
            super::AsyncWriteOp::CherryPickCommit {
                short_hash: short_hash.clone(),
            },
            move || git::cherry_pick(&root, &commit_hash),
        );
        Ok(())
    }

    pub(crate) fn refresh_history_entries(&mut self) -> Result<()> {
        let root = self.current_repo_root()?.to_path_buf();
        self.history_entries_request_seq = self.history_entries_request_seq.saturating_add(1);
        let request_id = self.history_entries_request_seq;
        self.history_entries_inflight = Some(request_id);
        let tx = self.async_tx.clone();
        if let Err(err) = self.queue_cancellable_async_task(JOB_HISTORY_ENTRIES, request_id, move || {
            let entries = git::commit_history(&root, 50).map_err(|err| err.to_string());
            let _ = tx.send(AppAsyncEvent::HistoryEntriesReady {
                request_id,
                entries,
            });
        }) {
            self.history_entries_inflight = None;
            return Err(err);
        }
        Ok(())
    }

    pub(crate) fn history_preview_text(&self) -> String {
        let Some(entry) = self.history_entries.get(self.selected_history) else {
            return "No commits found.\nPress [b] to return.".to_string();
        };
        if !self.history_details_visible {
            return format!(
                "Commit {}\n{}\n\nAuthor: {}\nWhen: {}\n\n[Enter] show details | [o] checkout detached | [p] cherry-pick",
                entry.short_hash, entry.summary, entry.author, entry.relative_time
            );
        }

        if self.history_details_for.as_deref() != Some(entry.hash.as_str()) {
            return "Loading commit details...".to_string();
        }
        let Some(details) = self.history_details.as_ref() else {
            return "No details loaded for selected commit.".to_string();
        };
        let focus = match self.history_focus {
            HistoryFocusPane::Commits => "commits",
            HistoryFocusPane::ChangedFiles => "changed files",
            HistoryFocusPane::FileHistory => "file history",
        };
        let file_section = if self.history_focus == HistoryFocusPane::FileHistory {
            if self.history_file_history_entries.is_empty() {
                "  (no history entries)".to_string()
            } else {
                self.history_file_history_entries
                    .iter()
                    .enumerate()
                    .take(18)
                    .map(|(idx, file_commit)| {
                        let marker = if idx == self.history_file_history_selected { ">" } else { " " };
                        format!(
                            "{marker} {} {} ({})",
                            file_commit.short_hash, file_commit.summary, file_commit.relative_time
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        } else if details.files.is_empty() {
            "  (no changed files)".to_string()
        } else {
            details
                .files
                .iter()
                .enumerate()
                .take(24)
                .map(|(idx, file)| {
                    let marker = if idx == self.history_file_tree_selected { ">" } else { " " };
                    format!("{marker} [{}] {}", file.status, file.path)
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            "Commit {}\n\nMetadata\n- Hash: {}\n- Author: {} <{}>\n- Date: {}\n\nMessage\n{}\n{}\n\nFiles ({}) [focus: {}]\n{}\n\n[Enter] hide details | [Left/Right] focus/open | [h] file history | [o] checkout | [p] cherry-pick",
            entry.short_hash,
            details.hash,
            details.author_name,
            details.author_email,
            details.authored_at,
            details.subject,
            details.body,
            details.files.len(),
            focus,
            file_section
        )
    }

    pub(crate) fn clear_history_details(&mut self) {
        self.history_details = None;
        self.history_details_for = None;
        self.history_details_inflight = None;
        self.history_file_tree_selected = 0;
        self.history_focus = HistoryFocusPane::Commits;
        self.clear_history_file_history();
    }

    pub(crate) fn clear_history_file_history(&mut self) {
        self.history_file_history_entries.clear();
        self.history_file_history_selected = 0;
        self.history_file_history_for_path = None;
        self.history_file_history_inflight = None;
    }

    pub(crate) fn selected_history_file_path(&self) -> Option<String> {
        self.history_details
            .as_ref()
            .and_then(|details| details.files.get(self.history_file_tree_selected))
            .map(|file| file.path.clone())
    }

    pub fn history_details_visible(&self) -> bool {
        self.history_details_visible
    }

    pub fn history_focus(&self) -> HistoryFocusPane {
        self.history_focus
    }

    pub fn history_metadata_text(&self) -> String {
        let Some(details) = self.history_details.as_ref() else {
            return "No commit details loaded.".to_string();
        };
        format!(
            "Hash: {}\nAuthor: {} <{}>\nDate: {}",
            details.hash, details.author_name, details.author_email, details.authored_at
        )
    }

    pub fn history_message_text(&self) -> String {
        let Some(details) = self.history_details.as_ref() else {
            return "No commit details loaded.".to_string();
        };
        let body = if details.body.is_empty() {
            "(No body)".to_string()
        } else {
            details.body.clone()
        };
        format!("{}\n\n{}", details.subject, body)
    }

    pub fn history_files_title(&self) -> String {
        if self.history_focus == HistoryFocusPane::FileHistory {
            let path = self
                .history_file_history_for_path
                .clone()
                .unwrap_or_else(|| "<none>".to_string());
            return format!("File History ({path}) [Right open diff, Left back]");
        }
        let count = self
            .history_details
            .as_ref()
            .map(|details| details.files.len())
            .unwrap_or(0);
        format!("Changed Files ({count}) [h history, Right diff]")
    }

    pub fn history_files_items(&self) -> Vec<String> {
        if self.history_focus == HistoryFocusPane::FileHistory {
            if self.history_file_history_entries.is_empty() {
                return vec!["No file history entries".to_string()];
            }
            return self
                .history_file_history_entries
                .iter()
                .map(|entry| format!("{} {} ({})", entry.short_hash, entry.summary, entry.relative_time))
                .collect();
        }

        let Some(details) = self.history_details.as_ref() else {
            return vec!["No changed files".to_string()];
        };
        if details.files.is_empty() {
            return vec!["No changed files".to_string()];
        }
        details
            .files
            .iter()
            .map(|file| {
                let depth = file.path.matches('/').count() + file.path.matches('\\').count();
                let name = file
                    .path
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(file.path.as_str());
                format!("[{}] {}{}", file.status, "  ".repeat(depth), name)
            })
            .collect()
    }

    pub fn history_files_selected_index(&self) -> Option<usize> {
        if self.history_focus == HistoryFocusPane::FileHistory {
            if self.history_file_history_entries.is_empty() {
                None
            } else {
                Some(self.history_file_history_selected)
            }
        } else {
            let count = self
                .history_details
                .as_ref()
                .map(|details| details.files.len())
                .unwrap_or(0);
            (count > 0).then_some(self.history_file_tree_selected)
        }
    }
}
