use anyhow::Result;

use crate::git;

use super::{App, Screen};

impl App {
    pub fn enter_tracking_status_view(&mut self) -> Result<()> {
        if !self.has_open_repo() {
            self.status_message = "Open a repository before comparing incoming/outgoing commits".to_string();
            return Ok(());
        }
        self.screen = Screen::TrackingStatusView;
        self.refresh_tracking_status_summary()?;
        self.status_message = "Incoming/outgoing commit comparison view".to_string();
        Ok(())
    }

    pub(crate) fn refresh_tracking_status_summary(&mut self) -> Result<()> {
        let root = self.current_repo_root()?.to_path_buf();
        self.tracking_summary = Some(git::tracking_commit_summary(&root, 30)?);
        Ok(())
    }
}
