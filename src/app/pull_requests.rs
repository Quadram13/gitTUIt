use anyhow::Result;

use crate::git::{self, PullRequestFilter as GitPullRequestFilter, PullRequestMergeMethod as GitPullRequestMergeMethod};

use super::{App, InputMode, PendingPullRequestMerge, format_gh_error_for_status, pull_request_merge_method_label};

impl App {
    pub fn enter_pull_request_view(&mut self) -> Result<()> {
        if !self.has_open_repo() {
            self.status_message = "Open a repository before viewing pull requests".to_string();
            return Ok(());
        }
        self.screen = super::Screen::PullRequestView;
        self.pr_filter = GitPullRequestFilter::Open;
        self.pending_pr_title = None;
        match self.refresh_pull_requests() {
            Ok(_) => {
                self.set_async_running_status("Refreshing pull requests");
            }
            Err(err) => {
                self.status_message = format_gh_error_for_status(&err);
            }
        }
        Ok(())
    }

    pub fn cycle_pull_request_filter(&mut self) -> Result<()> {
        if self.screen != super::Screen::PullRequestView {
            self.status_message = "Open pull request view first".to_string();
            return Ok(());
        }
        self.pr_filter = match self.pr_filter {
            GitPullRequestFilter::Open => GitPullRequestFilter::Draft,
            GitPullRequestFilter::Draft => GitPullRequestFilter::Merged,
            GitPullRequestFilter::Merged => GitPullRequestFilter::Open,
        };
        match self.refresh_pull_requests() {
            Ok(_) => {
                self.set_async_running_status(&format!(
                    "Refreshing pull requests ({})",
                    self.pull_request_filter_label()
                ));
            }
            Err(err) => {
                self.status_message = format_gh_error_for_status(&err);
            }
        }
        Ok(())
    }

    pub fn begin_create_pull_request_input(&mut self) {
        if self.screen != super::Screen::PullRequestView {
            self.status_message = "Open pull request view first".to_string();
            return;
        }
        self.pending_pr_title = None;
        self.input_mode = InputMode::CreatePullRequestTitle;
        self.input_buffer.clear();
        self.move_input_cursor_home();
        self.status_message = "Enter PR title and press [Enter]".to_string();
    }

    pub fn open_selected_pr_in_browser(&mut self) -> Result<()> {
        let Some(pr) = self.pull_requests.get(self.selected_pr) else {
            self.status_message = "No pull request selected".to_string();
            return Ok(());
        };
        let number = pr.number;
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            &format!("Opening PR #{} in browser", number),
            super::AsyncWriteOp::OpenPrInBrowser { number },
            move || git::open_pr_in_browser(&root, number),
        );
        Ok(())
    }

    pub fn checkout_selected_pr(&mut self) -> Result<()> {
        let Some(pr) = self.pull_requests.get(self.selected_pr) else {
            self.status_message = "No pull request selected".to_string();
            return Ok(());
        };
        let number = pr.number;
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            &format!("Checking out PR #{}", number),
            super::AsyncWriteOp::CheckoutPullRequest { number },
            move || git::checkout_pr(&root, number),
        );
        Ok(())
    }

    pub fn begin_merge_selected_pr(&mut self, method: GitPullRequestMergeMethod) -> Result<()> {
        if self.screen != super::Screen::PullRequestView {
            self.status_message = "Open pull request view first".to_string();
            return Ok(());
        }
        let Some(pr) = self.pull_requests.get(self.selected_pr) else {
            self.status_message = "No pull request selected".to_string();
            return Ok(());
        };

        self.pending_pr_merge = Some(PendingPullRequestMerge {
            number: pr.number,
            title: pr.title.clone(),
            method,
        });
        self.input_mode = InputMode::ConfirmPullRequestMerge;
        self.status_message = "Confirm PR merge: [y] confirm, [n]/[Esc] cancel".to_string();
        Ok(())
    }

    pub fn confirm_merge_selected_pr(&mut self) -> Result<()> {
        let Some(pending) = self.pending_pr_merge.take() else {
            self.input_mode = InputMode::None;
            self.status_message = "No PR merge action to confirm".to_string();
            return Ok(());
        };

        let root = self.current_repo_root()?.to_path_buf();
        let method = pending.method;
        let number = pending.number;
        self.input_mode = InputMode::None;
        self.request_write_op(
            &format!(
                "Merging PR #{} with {}",
                number,
                pull_request_merge_method_label(method)
            ),
            super::AsyncWriteOp::MergePullRequest { number, method },
            move || git::merge_pull_request(&root, number, method),
        );
        Ok(())
    }

    pub fn pull_request_filter_label(&self) -> &'static str {
        match self.pr_filter {
            GitPullRequestFilter::Open => "open",
            GitPullRequestFilter::Draft => "draft",
            GitPullRequestFilter::Merged => "merged",
        }
    }
}
