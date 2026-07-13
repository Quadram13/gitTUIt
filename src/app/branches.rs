use std::cmp::min;

use anyhow::Result;

use crate::git;

use super::{App, InputMode, Screen};

impl App {
    pub fn enter_branch_picker(&mut self) -> Result<()> {
        if !self.has_open_repo() {
            self.status_message = "Open a repository before managing branches".to_string();
            return Ok(());
        }
        self.screen = Screen::BranchPicker;
        self.refresh_branch_entries()?;
        self.status_message = "Branch picker: [Enter] switch, [n] new branch".to_string();
        Ok(())
    }

    pub fn enter_remote_branch_picker(&mut self) -> Result<()> {
        if !self.has_open_repo() {
            self.status_message = "Open a repository before browsing remote branches".to_string();
            return Ok(());
        }
        self.screen = Screen::RemoteBranchPicker;
        self.refresh_remote_branch_entries()?;
        self.status_message = "Remote branches: [Enter] checkout tracking branch".to_string();
        Ok(())
    }

    pub fn begin_new_branch_input(&mut self) {
        if self.screen != Screen::BranchPicker {
            self.status_message = "Open branch picker first".to_string();
            return;
        }
        self.input_mode = InputMode::NewBranchName;
        self.input_buffer.clear();
        self.status_message = "Enter new branch name and press [Enter]".to_string();
    }

    pub fn switch_selected_branch(&mut self) -> Result<()> {
        let Some(entry) = self.branch_entries.get(self.selected_branch) else {
            self.status_message = "No branch selected".to_string();
            return Ok(());
        };

        if entry.is_current {
            self.status_message = format!("Already on branch {}", entry.name);
            self.return_to_repo_view();
            return Ok(());
        }

        let branch_name = entry.name.clone();
        let root = self.current_repo_root()?.to_path_buf();
        git::switch_branch(&root, &branch_name)?;
        self.snapshot = git::snapshot(&root)?;
        self.refresh_branch_entries()?;
        self.return_to_repo_view();
        self.status_message = format!("Switched to branch {}", branch_name);
        Ok(())
    }

    pub fn checkout_selected_remote_branch(&mut self) -> Result<()> {
        let Some(entry) = self.remote_branch_entries.get(self.selected_remote_branch) else {
            self.status_message = "No remote branch selected".to_string();
            return Ok(());
        };
        let branch_name = entry.name.clone();
        let root = self.current_repo_root()?.to_path_buf();
        let local_branch = git::checkout_remote_tracking_branch(&root, &branch_name)?;
        self.snapshot = git::snapshot(&root)?;
        self.refresh_remote_branch_entries()?;
        self.return_to_repo_view();
        self.status_message = format!("Checked out remote branch {} as {}", branch_name, local_branch);
        Ok(())
    }

    pub(crate) fn create_branch_from_input(&mut self) -> Result<()> {
        let branch_name = self.input_buffer.trim();
        if branch_name.is_empty() {
            self.status_message = "Branch name cannot be empty".to_string();
            return Ok(());
        }
        let branch_name = branch_name.to_string();
        let root = self.current_repo_root()?.to_path_buf();
        git::create_and_switch_branch(&root, &branch_name)?;
        self.input_mode = InputMode::None;
        self.input_buffer.clear();
        self.snapshot = git::snapshot(&root)?;
        self.refresh_branch_entries()?;
        self.return_to_repo_view();
        self.status_message = format!("Created and switched to branch {}", branch_name);
        Ok(())
    }

    pub(crate) fn refresh_branch_entries(&mut self) -> Result<()> {
        let root = self.current_repo_root()?.to_path_buf();
        self.branch_entries = git::list_local_branches(&root)?;
        self.selected_branch = self
            .branch_entries
            .iter()
            .position(|entry| entry.is_current)
            .unwrap_or(0);
        Ok(())
    }

    pub(crate) fn refresh_remote_branch_entries(&mut self) -> Result<()> {
        let root = self.current_repo_root()?.to_path_buf();
        self.remote_branch_entries = git::list_remote_branches(&root)?;
        self.selected_remote_branch = min(
            self.selected_remote_branch,
            self.remote_branch_entries.len().saturating_sub(1),
        );
        Ok(())
    }
}
