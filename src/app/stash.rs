use std::cmp::min;

use anyhow::Result;

use crate::git;

use super::{App, InputMode, Screen, truncate_lines};

impl App {
    pub fn enter_stash_view(&mut self) -> Result<()> {
        if !self.has_open_repo() {
            self.status_message = "Open a repository before managing stash".to_string();
            return Ok(());
        }
        self.screen = Screen::StashView;
        self.refresh_stash_entries()?;
        self.status_message = "Stash view: [s] stash current changes, [a]/[p]/[d] apply/pop/drop".to_string();
        Ok(())
    }

    pub fn stash_current_changes(&mut self) -> Result<()> {
        if self.screen != Screen::StashView {
            self.status_message = "Open stash view first".to_string();
            return Ok(());
        }
        let root = self.current_repo_root()?.to_path_buf();
        git::stash_push(&root)?;
        self.snapshot = git::snapshot(&root)?;
        self.refresh_stash_entries()?;
        self.clear_stash_details();
        self.status_message = "Stashed current changes".to_string();
        Ok(())
    }

    pub fn apply_selected_stash(&mut self) -> Result<()> {
        let Some(entry) = self.stash_entries.get(self.selected_stash) else {
            self.status_message = "No stash selected".to_string();
            return Ok(());
        };
        let reference = entry.reference.clone();
        let root = self.current_repo_root()?.to_path_buf();
        git::stash_apply(&root, &reference)?;
        self.snapshot = git::snapshot(&root)?;
        self.status_message = format!("Applied {}", reference);
        Ok(())
    }

    pub fn pop_selected_stash(&mut self) -> Result<()> {
        let Some(entry) = self.stash_entries.get(self.selected_stash) else {
            self.status_message = "No stash selected".to_string();
            return Ok(());
        };
        let reference = entry.reference.clone();
        let root = self.current_repo_root()?.to_path_buf();
        git::stash_pop(&root, &reference)?;
        self.snapshot = git::snapshot(&root)?;
        self.refresh_stash_entries()?;
        self.clear_stash_details();
        self.status_message = format!("Popped {}", reference);
        Ok(())
    }

    pub fn begin_drop_selected_stash(&mut self) {
        let Some(entry) = self.stash_entries.get(self.selected_stash) else {
            self.status_message = "No stash selected".to_string();
            return;
        };
        self.pending_stash_drop = Some(entry.reference.clone());
        self.input_mode = InputMode::ConfirmStashDrop;
        self.status_message = "Confirm stash drop: [y] confirm, [n]/[Esc] cancel".to_string();
    }

    pub fn confirm_drop_selected_stash(&mut self) -> Result<()> {
        let Some(reference) = self.pending_stash_drop.take() else {
            self.input_mode = InputMode::None;
            self.status_message = "No stash drop action to confirm".to_string();
            return Ok(());
        };
        let root = self.current_repo_root()?.to_path_buf();
        git::stash_drop(&root, &reference)?;
        self.input_mode = InputMode::None;
        self.refresh_stash_entries()?;
        self.clear_stash_details();
        self.status_message = format!("Dropped {}", reference);
        Ok(())
    }

    pub fn load_selected_stash_details(&mut self) -> Result<()> {
        let Some(entry) = self.stash_entries.get(self.selected_stash) else {
            self.status_message = "No stash selected".to_string();
            return Ok(());
        };
        if self.stash_details_for.as_deref() == Some(entry.reference.as_str()) {
            self.status_message = format!("Stash details already loaded ({})", entry.reference);
            return Ok(());
        }
        let reference = entry.reference.clone();
        let root = self.current_repo_root()?.to_path_buf();
        let details = git::stash_show(&root, &reference)?;
        self.stash_details = truncate_lines(details, 220);
        self.stash_details_for = Some(reference.clone());
        self.status_message = format!("Loaded stash details {}", reference);
        Ok(())
    }

    pub(crate) fn refresh_stash_entries(&mut self) -> Result<()> {
        let root = self.current_repo_root()?.to_path_buf();
        self.stash_entries = git::list_stashes(&root)?;
        self.selected_stash = min(self.selected_stash, self.stash_entries.len().saturating_sub(1));
        Ok(())
    }
}
