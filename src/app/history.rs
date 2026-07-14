use anyhow::Result;

use crate::git;

use super::{App, AppAsyncEvent, JOB_HISTORY_DETAILS, JOB_HISTORY_ENTRIES, Screen};

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
        self.history_details_request_seq = self.history_details_request_seq.saturating_add(1);
        let request_id = self.history_details_request_seq;
        self.history_details_inflight = Some(request_id);
        self.set_async_running_status(&format!("Loading commit details {}", short_hash));
        let tx = self.async_tx.clone();
        if let Err(err) = self.queue_cancellable_async_task(JOB_HISTORY_DETAILS, request_id, move || {
            let details = git::commit_details(&root, &commit_hash).map_err(|err| err.to_string());
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
        self.history_details_inflight = None;
    }
}
