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
        self.set_async_running_status("Refreshing local branches");
        Ok(())
    }

    pub fn enter_remote_branch_picker(&mut self) -> Result<()> {
        if !self.has_open_repo() {
            self.status_message = "Open a repository before browsing remote branches".to_string();
            return Ok(());
        }
        self.screen = Screen::RemoteBranchPicker;
        self.refresh_remote_branch_entries()?;
        self.set_async_running_status("Refreshing remote branches");
        Ok(())
    }

    pub fn begin_new_branch_input(&mut self) {
        if self.screen != Screen::BranchPicker {
            self.status_message = "Open branch picker first".to_string();
            return;
        }
        self.input_mode = InputMode::NewBranchName;
        self.input_buffer.clear();
        self.move_input_cursor_home();
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
        self.request_write_op(
            &format!("Switching to branch {}", branch_name),
            super::AsyncWriteOp::SwitchBranch {
                branch_name: branch_name.clone(),
            },
            move || git::switch_branch(&root, &branch_name),
        );
        Ok(())
    }

    pub fn checkout_selected_remote_branch(&mut self) -> Result<()> {
        let Some(entry) = self.remote_branch_entries.get(self.selected_remote_branch) else {
            self.status_message = "No remote branch selected".to_string();
            return Ok(());
        };
        let branch_name = entry.name.clone();
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            &format!("Checking out remote branch {}", branch_name),
            super::AsyncWriteOp::CheckoutRemoteBranch {
                branch_name: branch_name.clone(),
            },
            move || git::checkout_remote_tracking_branch(&root, &branch_name).map(|_| ()),
        );
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
        self.input_mode = InputMode::None;
        self.input_buffer.clear();
        self.move_input_cursor_home();
        self.request_write_op(
            &format!("Creating branch {}", branch_name),
            super::AsyncWriteOp::CreateBranch {
                branch_name: branch_name.clone(),
            },
            move || git::create_and_switch_branch(&root, &branch_name),
        );
        Ok(())
    }

    pub(crate) fn refresh_branch_entries(&mut self) -> Result<()> {
        self.request_branch_entries_refresh()?;
        Ok(())
    }

    pub(crate) fn refresh_remote_branch_entries(&mut self) -> Result<()> {
        self.request_remote_branch_entries_refresh()?;
        Ok(())
    }
}
