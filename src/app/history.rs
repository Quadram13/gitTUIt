use anyhow::Result;

use crate::git;

use super::{App, Screen, truncate_lines};

impl App {
    pub fn enter_history_view(&mut self) -> Result<()> {
        if !self.has_open_repo() {
            self.status_message = "Open a repository before viewing history".to_string();
            return Ok(());
        }
        self.screen = Screen::HistoryView;
        self.refresh_history_entries()?;
        self.status_message = "History: [Enter] load commit details, [b] back".to_string();
        Ok(())
    }

    pub fn load_selected_commit_details(&mut self) -> Result<()> {
        let Some(entry) = self.history_entries.get(self.selected_history) else {
            self.status_message = "No commit selected".to_string();
            return Ok(());
        };
        if self.history_details_for.as_deref() == Some(entry.hash.as_str()) {
            self.status_message = format!("Commit details already loaded ({})", entry.short_hash);
            return Ok(());
        }

        let commit_hash = entry.hash.clone();
        let short_hash = entry.short_hash.clone();
        let root = self.current_repo_root()?.to_path_buf();
        let details = git::commit_details(&root, &commit_hash)?;
        self.history_details = truncate_lines(details, 240);
        self.history_details_for = Some(commit_hash);
        self.status_message = format!("Loaded commit details {}", short_hash);
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
        match git::checkout_detached(&root, &commit_hash) {
            Ok(_) => {
                self.snapshot = git::snapshot(&root)?;
                self.clear_history_details();
                self.status_message = format!("Checked out {} (detached HEAD)", short_hash);
            }
            Err(err) => {
                self.status_message = format!("Checkout failed: {err}");
            }
        }
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
        match git::cherry_pick(&root, &commit_hash) {
            Ok(_) => {
                self.snapshot = git::snapshot(&root)?;
                self.refresh_history_entries()?;
                self.status_message = format!("Cherry-picked {}", short_hash);
            }
            Err(err) => {
                self.status_message = format!("Cherry-pick failed: {err}");
            }
        }
        Ok(())
    }

    pub(crate) fn refresh_history_entries(&mut self) -> Result<()> {
        let root = self.current_repo_root()?.to_path_buf();
        self.history_entries = git::commit_history(&root, 80)?;
        self.selected_history = std::cmp::min(
            self.selected_history,
            self.history_entries.len().saturating_sub(1),
        );
        self.clear_history_details();
        Ok(())
    }

    pub(crate) fn history_preview_text(&self) -> String {
        let Some(entry) = self.history_entries.get(self.selected_history) else {
            return "No commits found.\nPress [b] to return.".to_string();
        };
        if self.history_details_for.as_deref() == Some(entry.hash.as_str()) && !self.history_details.is_empty()
        {
            return self.history_details.clone();
        }
        format!(
            "Commit {}\n{}\n\nAuthor: {}\nWhen: {}\n\n[Enter] details | [o] checkout detached | [p] cherry-pick",
            entry.short_hash, entry.summary, entry.author, entry.relative_time
        )
    }

    pub(crate) fn clear_history_details(&mut self) {
        self.history_details.clear();
        self.history_details_for = None;
    }
}
