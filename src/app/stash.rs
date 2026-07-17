use anyhow::Result;

use crate::git;

use super::{App, AppAsyncEvent, InputMode, JOB_STASH_DETAILS, JOB_STASH_ENTRIES, Screen};

impl App {
    pub fn enter_stash_view(&mut self) -> Result<()> {
        if !self.has_open_repo() {
            self.status_message = "Open a repository before managing stash".to_string();
            return Ok(());
        }
        self.screen = Screen::StashView;
        self.refresh_stash_entries()?;
        self.status_message =
            "Stash view: [s] stash current changes, [a]/[p]/[d] apply/pop/drop".to_string();
        Ok(())
    }

    pub fn stash_current_changes(&mut self) -> Result<()> {
        if self.screen != Screen::StashView {
            self.status_message = "Open stash view first".to_string();
            return Ok(());
        }
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            "Stashing current changes",
            super::AsyncWriteOp::StashPush,
            move || git::stash_push(&root),
        );
        Ok(())
    }

    pub fn apply_selected_stash(&mut self) -> Result<()> {
        let Some(entry) = self.stash_entries.get(self.selected_stash) else {
            self.status_message = "No stash selected".to_string();
            return Ok(());
        };
        let reference = entry.reference.clone();
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            &format!("Applying {}", reference),
            super::AsyncWriteOp::StashApply {
                reference: reference.clone(),
            },
            move || git::stash_apply(&root, &reference),
        );
        Ok(())
    }

    pub fn pop_selected_stash(&mut self) -> Result<()> {
        let Some(entry) = self.stash_entries.get(self.selected_stash) else {
            self.status_message = "No stash selected".to_string();
            return Ok(());
        };
        let reference = entry.reference.clone();
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            &format!("Popping {}", reference),
            super::AsyncWriteOp::StashPop {
                reference: reference.clone(),
            },
            move || git::stash_pop(&root, &reference),
        );
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
        self.input_mode = InputMode::None;
        self.request_write_op(
            &format!("Dropping {}", reference),
            super::AsyncWriteOp::StashDrop {
                reference: reference.clone(),
            },
            move || git::stash_drop(&root, &reference),
        );
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
        self.stash_details_request_seq = self.stash_details_request_seq.saturating_add(1);
        let request_id = self.stash_details_request_seq;
        self.stash_details_inflight = Some(request_id);
        self.set_async_running_status(&format!("Loading stash details {}", reference));
        let tx = self.async_tx.clone();
        if let Err(err) =
            self.queue_cancellable_async_task(JOB_STASH_DETAILS, request_id, move || {
                let details = git::stash_show(&root, &reference).map_err(|err| err.to_string());
                let _ = tx.send(AppAsyncEvent::StashDetailsReady {
                    request_id,
                    reference,
                    details,
                });
            })
        {
            self.stash_details_inflight = None;
            return Err(err);
        }
        Ok(())
    }

    pub(crate) fn refresh_stash_entries(&mut self) -> Result<()> {
        let root = self.current_repo_root()?.to_path_buf();
        self.stash_entries_request_seq = self.stash_entries_request_seq.saturating_add(1);
        let request_id = self.stash_entries_request_seq;
        self.stash_entries_inflight = Some(request_id);
        let tx = self.async_tx.clone();
        if let Err(err) =
            self.queue_cancellable_async_task(JOB_STASH_ENTRIES, request_id, move || {
                let entries = git::list_stashes(&root).map_err(|err| err.to_string());
                let _ = tx.send(AppAsyncEvent::StashEntriesReady {
                    request_id,
                    entries,
                });
            })
        {
            self.stash_entries_inflight = None;
            return Err(err);
        }
        Ok(())
    }
}
