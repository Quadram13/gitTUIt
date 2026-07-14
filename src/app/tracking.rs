use anyhow::Result;

use super::{App, Screen};

impl App {
    pub fn enter_tracking_status_view(&mut self) -> Result<()> {
        if !self.has_open_repo() {
            self.status_message = "Open a repository before comparing incoming/outgoing commits".to_string();
            return Ok(());
        }
        self.screen = Screen::TrackingStatusView;
        self.refresh_tracking_status_summary()?;
        self.set_async_running_status("Refreshing incoming/outgoing comparison");
        Ok(())
    }

    pub(crate) fn refresh_tracking_status_summary(&mut self) -> Result<()> {
        self.request_tracking_summary_refresh()?;
        Ok(())
    }
}
