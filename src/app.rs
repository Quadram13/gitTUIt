use std::{
    cmp::min,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Result, anyhow};

use crate::{
    diagnostics,
    git::{
        self, BranchEntry as GitBranchEntry, CommitDetails as GitCommitDetails,
        CommitEntry as GitCommitEntry,
        PullRequestMergeMethod as GitPullRequestMergeMethod,
        PullRequestEntry as GitPullRequestEntry, PullRequestFilter as GitPullRequestFilter,
        PullRequestStatusSummary as GitPullRequestStatusSummary,
        RemoteBranchEntry as GitRemoteBranchEntry, RepoSnapshot, StashEntry as GitStashEntry,
        TrackingCommitSummary as GitTrackingCommitSummary,
    },
    repo_registry::{RepoRegistry, canonical_repo_path, normalize_repo_path_input},
    tree::changed_files_tree::{ChangedFilesTree, FileLeaf, TreeRow, TreeRowKind},
};

mod pull_requests;
mod branches;
mod history;
mod stash;
mod tracking;

const ASYNC_WORKER_COUNT: usize = 4;
const ASYNC_QUEUE_CAPACITY: usize = 96;

const JOB_WRITE: &str = "write";
const JOB_SNAPSHOT: &str = "snapshot";
const JOB_REPO_PREVIEW: &str = "repo_preview";
const JOB_HISTORY_DETAILS: &str = "history_details";
const JOB_STASH_DETAILS: &str = "stash_details";
const JOB_HISTORY_ENTRIES: &str = "history_entries";
const JOB_HISTORY_FILE_HISTORY: &str = "history_file_history";
const JOB_STASH_ENTRIES: &str = "stash_entries";
const JOB_PULL_REQUESTS: &str = "pull_requests";
const JOB_PR_STATUS: &str = "pr_status";
const JOB_TRACKING: &str = "tracking";
const JOB_BRANCH_ENTRIES: &str = "branch_entries";
const JOB_REMOTE_BRANCH_ENTRIES: &str = "remote_branch_entries";
const JOB_FULLSCREEN_DIFF: &str = "fullscreen_diff";

type AsyncTask = Box<dyn FnOnce() + Send + 'static>;

enum AsyncWorkerMessage {
    Run(AsyncTask),
}

#[derive(Clone)]
struct AsyncScheduler {
    tx: SyncSender<AsyncWorkerMessage>,
    latest_tokens: Arc<Mutex<HashMap<&'static str, u64>>>,
}

impl AsyncScheduler {
    fn new(worker_count: usize, queue_capacity: usize) -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<AsyncWorkerMessage>(queue_capacity);
        let shared_rx = Arc::new(Mutex::new(rx));
        for idx in 0..worker_count {
            let worker_rx = Arc::clone(&shared_rx);
            let thread_name = format!("gittuit-async-{idx}");
            thread::Builder::new()
                .name(thread_name)
                .spawn(move || {
                    loop {
                        let message = {
                            let Ok(guard) = worker_rx.lock() else {
                                break;
                            };
                            guard.recv()
                        };
                        match message {
                            Ok(AsyncWorkerMessage::Run(job)) => job(),
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|err| anyhow!("failed to start async worker {idx}: {err}"))?;
        }
        Ok(Self {
            tx,
            latest_tokens: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn submit<F>(&self, job: F) -> std::result::Result<(), String>
    where
        F: FnOnce() + Send + 'static,
    {
        self.tx
            .try_send(AsyncWorkerMessage::Run(Box::new(job)))
            .map_err(format_async_queue_error)
    }

    fn submit_cancellable<F, C>(
        &self,
        key: &'static str,
        token: u64,
        on_cancel: C,
        job: F,
    ) -> std::result::Result<(), String>
    where
        F: FnOnce() + Send + 'static,
        C: FnOnce() + Send + 'static,
    {
        if let Ok(mut latest) = self.latest_tokens.lock() {
            latest.insert(key, token);
        }
        let latest_tokens = Arc::clone(&self.latest_tokens);
        self.submit(move || {
            let should_run = latest_tokens
                .lock()
                .ok()
                .and_then(|latest| latest.get(key).copied())
                == Some(token);
            if should_run {
                job();
            } else {
                on_cancel();
            }
        })
    }
}

fn format_async_queue_error(err: TrySendError<AsyncWorkerMessage>) -> String {
    match err {
        TrySendError::Full(_) => {
            "async queue is full; wait for background jobs to finish".to_string()
        }
        TrySendError::Disconnected(_) => "async queue is unavailable".to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AsyncJobLifecycle {
    Idle,
    Queued,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    Unstaged,
    Staged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryFocusPane {
    Commits,
    ChangedFiles,
    FileHistory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    RepoPicker,
    RepoBrowser,
    BranchPicker,
    RemoteBranchPicker,
    HistoryView,
    PullRequestView,
    TrackingStatusView,
    StashView,
    RepoView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    None,
    CommitSubject,
    CommitBody,
    AddRepoPath,
    NewBranchName,
    CreatePullRequestTitle,
    CreatePullRequestBody,
    ConfirmStashDrop,
    ConfirmDiscard,
    ConfirmPullRequestMerge,
}

enum AppAsyncEvent {
    JobState {
        key: &'static str,
        state: AsyncJobLifecycle,
    },
    JobCancelled {
        key: &'static str,
    },
    SnapshotReady {
        request_id: u64,
        snapshot: std::result::Result<RepoSnapshot, String>,
    },
    RepoPreviewReady {
        request_id: u64,
        key: String,
        preview: String,
    },
    HistoryDetailsReady {
        request_id: u64,
        commit_hash: String,
        short_hash: String,
        details: std::result::Result<GitCommitDetails, String>,
    },
    HistoryFileHistoryReady {
        request_id: u64,
        commit_hash: String,
        path: String,
        entries: std::result::Result<Vec<GitCommitEntry>, String>,
    },
    StashDetailsReady {
        request_id: u64,
        reference: String,
        details: std::result::Result<String, String>,
    },
    HistoryEntriesReady {
        request_id: u64,
        entries: std::result::Result<Vec<GitCommitEntry>, String>,
    },
    StashEntriesReady {
        request_id: u64,
        entries: std::result::Result<Vec<GitStashEntry>, String>,
    },
    PullRequestsReady {
        request_id: u64,
        filter: GitPullRequestFilter,
        entries: std::result::Result<Vec<GitPullRequestEntry>, String>,
    },
    PullRequestStatusReady {
        request_id: u64,
        pr_number: u64,
        summary: std::result::Result<GitPullRequestStatusSummary, String>,
    },
    TrackingSummaryReady {
        request_id: u64,
        summary: std::result::Result<GitTrackingCommitSummary, String>,
    },
    BranchEntriesReady {
        request_id: u64,
        entries: std::result::Result<Vec<GitBranchEntry>, String>,
    },
    RemoteBranchEntriesReady {
        request_id: u64,
        entries: std::result::Result<Vec<GitRemoteBranchEntry>, String>,
    },
    FullscreenDiffReady {
        request_id: u64,
        key: String,
        title: String,
        diff: std::result::Result<String, String>,
    },
    WriteOpFinished {
        op: AsyncWriteOp,
        result: std::result::Result<(), String>,
    },
}

#[derive(Debug, Clone)]
struct FullscreenDiffState {
    key: String,
    title: String,
    lines: Vec<String>,
    hunk_lines: Vec<usize>,
    scroll_y: usize,
    scroll_x: usize,
    loading: bool,
}

enum AsyncWriteOp {
    StageFile { path: String },
    UnstageFile { path: String },
    StageAll { total: usize },
    UnstageAll { total: usize },
    Fetch,
    Pull,
    Push,
    CherryPickContinue,
    CherryPickAbort,
    DiscardFile { path: String, is_untracked: bool },
    SwitchBranch { branch_name: String },
    CheckoutRemoteBranch { branch_name: String },
    CreateBranch { branch_name: String },
    CheckoutPullRequest { number: u64 },
    MergePullRequest { number: u64, method: GitPullRequestMergeMethod },
    CreatePullRequest,
    Commit,
    StashPush,
    StashApply { reference: String },
    StashPop { reference: String },
    StashDrop { reference: String },
    CheckoutDetached { short_hash: String },
    CherryPickCommit { short_hash: String },
    OpenPrInBrowser { number: u64 },
}

pub struct App {
    pub screen: Screen,
    pub input_mode: InputMode,
    pub registry: RepoRegistry,
    pub repo_picker_labels: Vec<String>,
    pub selected_repo: usize,
    pub browser_dir: PathBuf,
    pub browser_entries: Vec<BrowserEntry>,
    pub selected_browser: usize,
    pub show_hidden_browser_entries: bool,
    pub branch_entries: Vec<GitBranchEntry>,
    pub selected_branch: usize,
    pub remote_branch_entries: Vec<GitRemoteBranchEntry>,
    pub selected_remote_branch: usize,
    pub history_entries: Vec<GitCommitEntry>,
    pub selected_history: usize,
    pub pull_requests: Vec<GitPullRequestEntry>,
    pub selected_pr: usize,
    pub pr_filter: GitPullRequestFilter,
    pub repo_root: Option<PathBuf>,
    pub snapshot: RepoSnapshot,
    pub focus: FocusPane,
    unstaged_tree: ChangedFilesTree,
    staged_tree: ChangedFilesTree,
    pub selected_unstaged: usize,
    pub selected_staged: usize,
    pub status_message: String,
    pub input_buffer: String,
    input_cursor: usize,
    pub help_visible: bool,
    pub help_scroll: usize,
    last_fetch_at: Option<SystemTime>,
    tracking_summary: Option<GitTrackingCommitSummary>,
    pub stash_entries: Vec<GitStashEntry>,
    pub selected_stash: usize,
    stash_details: String,
    stash_details_for: Option<String>,
    history_details_visible: bool,
    history_focus: HistoryFocusPane,
    history_details: Option<GitCommitDetails>,
    history_details_for: Option<String>,
    history_tree: ChangedFilesTree,
    history_file_tree_selected: usize,
    history_file_history_entries: Vec<GitCommitEntry>,
    history_file_history_selected: usize,
    history_file_history_for_path: Option<String>,
    pr_status_summary: Option<GitPullRequestStatusSummary>,
    pr_status_for: Option<u64>,
    pr_status_refresh_deadline: Option<Instant>,
    pr_status_pending_for: Option<u64>,
    pending_discard: Option<PendingDiscard>,
    pending_pr_merge: Option<PendingPullRequestMerge>,
    pending_stash_drop: Option<String>,
    pending_pr_title: Option<String>,
    pending_commit_subject: Option<String>,
    runtime_log_path: Option<PathBuf>,
    repo_preview_key: Option<String>,
    repo_preview_text: String,
    repo_preview_refresh_deadline: Option<Instant>,
    async_scheduler: AsyncScheduler,
    async_tx: Sender<AppAsyncEvent>,
    async_rx: Receiver<AppAsyncEvent>,
    snapshot_request_seq: u64,
    snapshot_inflight: Option<u64>,
    snapshot_completion_message: Option<String>,
    repo_preview_request_seq: u64,
    repo_preview_inflight: Option<u64>,
    repo_preview_pending_key: Option<String>,
    history_details_request_seq: u64,
    history_details_inflight: Option<u64>,
    stash_details_request_seq: u64,
    stash_details_inflight: Option<u64>,
    history_entries_request_seq: u64,
    history_entries_inflight: Option<u64>,
    history_file_history_request_seq: u64,
    history_file_history_inflight: Option<u64>,
    stash_entries_request_seq: u64,
    stash_entries_inflight: Option<u64>,
    pull_requests_request_seq: u64,
    pull_requests_inflight: Option<u64>,
    pr_status_request_seq: u64,
    pr_status_inflight: Option<u64>,
    tracking_request_seq: u64,
    tracking_inflight: Option<u64>,
    branch_entries_request_seq: u64,
    branch_entries_inflight: Option<u64>,
    remote_branch_entries_request_seq: u64,
    remote_branch_entries_inflight: Option<u64>,
    fullscreen_diff: Option<FullscreenDiffState>,
    fullscreen_diff_request_seq: u64,
    fullscreen_diff_inflight: Option<u64>,
    fullscreen_diff_pending_key: Option<String>,
    async_job_lifecycle: HashMap<&'static str, AsyncJobLifecycle>,
    async_cancelled_jobs: u64,
    async_dispatch_failures: u64,
    last_async_error: Option<String>,
    write_inflight: bool,
}

#[derive(Debug, Clone)]
pub struct BrowserEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_git_root: bool,
}

#[derive(Debug, Clone)]
struct PendingDiscard {
    path: String,
    is_untracked: bool,
}

#[derive(Debug, Clone)]
struct PendingPullRequestMerge {
    number: u64,
    title: String,
    method: GitPullRequestMergeMethod,
}

impl App {
    pub fn new() -> Result<Self> {
        let registry = RepoRegistry::load()?;
        let (async_tx, async_rx) = mpsc::channel();
        let async_scheduler = AsyncScheduler::new(ASYNC_WORKER_COUNT, ASYNC_QUEUE_CAPACITY)?;
        let mut app = Self {
            screen: Screen::RepoPicker,
            input_mode: InputMode::None,
            registry,
            repo_picker_labels: Vec::new(),
            selected_repo: 0,
            browser_dir: dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")),
            browser_entries: Vec::new(),
            selected_browser: 0,
            show_hidden_browser_entries: false,
            branch_entries: Vec::new(),
            selected_branch: 0,
            remote_branch_entries: Vec::new(),
            selected_remote_branch: 0,
            history_entries: Vec::new(),
            selected_history: 0,
            pull_requests: Vec::new(),
            selected_pr: 0,
            pr_filter: GitPullRequestFilter::Open,
            repo_root: None,
            snapshot: RepoSnapshot::default(),
            focus: FocusPane::Unstaged,
            unstaged_tree: ChangedFilesTree::default(),
            staged_tree: ChangedFilesTree::default(),
            selected_unstaged: 0,
            selected_staged: 0,
            status_message: String::new(),
            input_buffer: String::new(),
            input_cursor: 0,
            help_visible: false,
            help_scroll: 0,
            last_fetch_at: None,
            tracking_summary: None,
            stash_entries: Vec::new(),
            selected_stash: 0,
            stash_details: String::new(),
            stash_details_for: None,
            history_details_visible: false,
            history_focus: HistoryFocusPane::Commits,
            history_details: None,
            history_details_for: None,
            history_tree: ChangedFilesTree::default(),
            history_file_tree_selected: 0,
            history_file_history_entries: Vec::new(),
            history_file_history_selected: 0,
            history_file_history_for_path: None,
            pr_status_summary: None,
            pr_status_for: None,
            pr_status_refresh_deadline: None,
            pr_status_pending_for: None,
            pending_discard: None,
            pending_pr_merge: None,
            pending_stash_drop: None,
            pending_pr_title: None,
            pending_commit_subject: None,
            runtime_log_path: None,
            repo_preview_key: None,
            repo_preview_text: "No unstaged diff output for selected file.".to_string(),
            repo_preview_refresh_deadline: None,
            async_scheduler,
            async_tx,
            async_rx,
            snapshot_request_seq: 0,
            snapshot_inflight: None,
            snapshot_completion_message: None,
            repo_preview_request_seq: 0,
            repo_preview_inflight: None,
            repo_preview_pending_key: None,
            history_details_request_seq: 0,
            history_details_inflight: None,
            stash_details_request_seq: 0,
            stash_details_inflight: None,
            history_entries_request_seq: 0,
            history_entries_inflight: None,
            history_file_history_request_seq: 0,
            history_file_history_inflight: None,
            stash_entries_request_seq: 0,
            stash_entries_inflight: None,
            pull_requests_request_seq: 0,
            pull_requests_inflight: None,
            pr_status_request_seq: 0,
            pr_status_inflight: None,
            tracking_request_seq: 0,
            tracking_inflight: None,
            branch_entries_request_seq: 0,
            branch_entries_inflight: None,
            remote_branch_entries_request_seq: 0,
            remote_branch_entries_inflight: None,
            fullscreen_diff: None,
            fullscreen_diff_request_seq: 0,
            fullscreen_diff_inflight: None,
            fullscreen_diff_pending_key: None,
            async_job_lifecycle: HashMap::new(),
            async_cancelled_jobs: 0,
            async_dispatch_failures: 0,
            last_async_error: None,
            write_inflight: false,
        };
        app.rebuild_repo_picker_labels();
        app.refresh_browser_entries()?;

        if app.registry.repos.is_empty() {
            app.status_message = "No repositories added. Press [a] to add a git repo root.".to_string();
            return Ok(app);
        }

        if app.registry.repos.len() == 1 {
            let open_result = app.open_selected_repo();
            if open_result.is_ok() {
                app.status_message =
                    "Auto-opened the only tracked repository. Press [b] to return to picker."
                        .to_string();
                return Ok(app);
            }
            app.status_message = "Failed to auto-open tracked repository. Use [Enter] to retry.".to_string();
            return Ok(app);
        }

        app.status_message = "Select a repository and press [Enter] to open.".to_string();
        Ok(app)
    }

    pub fn in_input_mode(&self) -> bool {
        self.input_mode != InputMode::None
    }

    pub fn tick(&mut self) {
        self.process_async_events();
        self.maybe_refresh_pr_status_summary();
        self.maybe_refresh_repo_preview_cache();
    }

    pub fn has_open_repo(&self) -> bool {
        self.repo_root.is_some()
    }

    pub fn is_repo_workspace_screen(&self) -> bool {
        matches!(
            self.screen,
            Screen::RepoView
                | Screen::BranchPicker
                | Screen::RemoteBranchPicker
                | Screen::HistoryView
                | Screen::PullRequestView
                | Screen::TrackingStatusView
                | Screen::StashView
        )
    }

    pub fn active_workspace_tab_index(&self) -> Option<u8> {
        match self.screen {
            Screen::RepoView => Some(1),
            Screen::BranchPicker | Screen::RemoteBranchPicker => Some(2),
            Screen::HistoryView => Some(3),
            Screen::TrackingStatusView => Some(4),
            Screen::StashView => Some(5),
            Screen::PullRequestView => Some(6),
            _ => None,
        }
    }

    pub fn switch_workspace_tab(&mut self, tab: u8) -> Result<()> {
        if !self.has_open_repo() {
            self.status_message = "Open a repository before using workspace tabs".to_string();
            return Ok(());
        }
        match tab {
            1 => {
                self.return_to_repo_view();
                self.status_message = "Status tab".to_string();
            }
            2 => self.enter_branch_picker()?,
            3 => self.enter_history_view()?,
            4 => self.enter_tracking_status_view()?,
            5 => self.enter_stash_view()?,
            6 => self.enter_pull_request_view()?,
            _ => {}
        }
        Ok(())
    }

    pub fn last_fetch_summary(&self) -> String {
        let Some(last_fetch) = self.last_fetch_at else {
            return "Last fetch: never".to_string();
        };
        let Ok(elapsed) = last_fetch.elapsed() else {
            return "Last fetch: just now".to_string();
        };
        let secs = elapsed.as_secs();
        if secs < 60 {
            return format!("Last fetch: {}s ago", secs);
        }
        if secs < 3600 {
            return format!("Last fetch: {}m ago", secs / 60);
        }
        if secs < 86_400 {
            return format!("Last fetch: {}h ago", secs / 3600);
        }
        format!("Last fetch: {}d ago", secs / 86_400)
    }

    pub fn toggle_help(&mut self) {
        self.help_visible = !self.help_visible;
        if self.help_visible {
            self.help_scroll = 0;
        }
    }

    pub fn close_help(&mut self) {
        self.help_visible = false;
        self.help_scroll = 0;
    }

    pub fn help_popup_title(&self) -> &'static str {
        match self.screen {
            Screen::RepoPicker => "Help: Main Menu",
            Screen::RepoBrowser => "Help: Repo Browser",
            Screen::BranchPicker => "Help: Local Branches",
            Screen::RemoteBranchPicker => "Help: Remote Branches",
            Screen::HistoryView => "Help: History",
            Screen::PullRequestView => "Help: Pull Requests",
            Screen::TrackingStatusView => "Help: Incoming/Outgoing",
            Screen::StashView => "Help: Stash",
            Screen::RepoView => "Help: Repository",
        }
    }

    pub fn help_popup_body(&self) -> String {
        self.help_lines().join("\n")
    }

    pub fn help_line_count(&self) -> usize {
        self.help_lines().len()
    }

    pub fn scroll_help_down(&mut self, amount: usize) {
        let max_scroll = self.help_line_count().saturating_sub(1);
        self.help_scroll = min(self.help_scroll.saturating_add(amount), max_scroll);
    }

    pub fn scroll_help_up(&mut self, amount: usize) {
        self.help_scroll = self.help_scroll.saturating_sub(amount);
    }

    pub fn scroll_help_to_top(&mut self) {
        self.help_scroll = 0;
    }

    pub fn scroll_help_to_bottom(&mut self) {
        self.help_scroll = self.help_line_count().saturating_sub(1);
    }

    fn help_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let mut section = |title: &str, items: &[String]| {
            lines.push(title.to_string());
            lines.extend(items.iter().cloned());
            lines.push(String::new());
        };

        section(
            "Global",
            &[
                "[?] close help".to_string(),
                "[q] quit".to_string(),
                "[r] refresh current view".to_string(),
                "[L] show log path".to_string(),
            ],
        );

        let nav_items = vec!["[j]/[k] move selection".to_string()];
        let workspace_tabs = vec!["[1]-[6] switch workspace tab".to_string()];

        match self.screen {
            Screen::RepoPicker => {
                section(
                    "Main Menu",
                    &[
                        nav_items[0].clone(),
                        "[Enter] open selected repository".to_string(),
                        "[a] add repository path".to_string(),
                        "[f] open repo browser".to_string(),
                        "[d] remove selected repository".to_string(),
                    ],
                );
            }
            Screen::RepoBrowser => {
                section(
                    "Repo Browser",
                    &[
                        nav_items[0].clone(),
                        "[Enter] open directory / add git repo".to_string(),
                        "[Backspace] go parent directory".to_string(),
                        "[.] toggle hidden dotfiles".to_string(),
                        "[b] back to main menu".to_string(),
                    ],
                );
            }
            Screen::BranchPicker => {
                section(
                    "Branches",
                    &[
                        nav_items[0].clone(),
                        "[Enter] switch selected branch".to_string(),
                        "[n] create and switch branch".to_string(),
                        "[G] open remote branches".to_string(),
                        "[b] back to status tab".to_string(),
                    ],
                );
                section("Workspace", &workspace_tabs);
            }
            Screen::RemoteBranchPicker => {
                section(
                    "Remote Branches",
                    &[
                        nav_items[0].clone(),
                        "[Enter] checkout tracking/local branch".to_string(),
                        "[g] open local branches".to_string(),
                        "[b] back to status tab".to_string(),
                    ],
                );
                section("Workspace", &workspace_tabs);
            }
            Screen::HistoryView => {
                section(
                    "History",
                    &[
                        nav_items[0].clone(),
                        "[Enter] toggle commit details".to_string(),
                        "[Left]/[Right] move focus/open diff".to_string(),
                        "[h] open file history from changed files".to_string(),
                        "[o] checkout selected commit (detached)".to_string(),
                        "[p] cherry-pick selected commit".to_string(),
                        "[b] back to status tab".to_string(),
                    ],
                );
                section("Workspace", &workspace_tabs);
            }
            Screen::PullRequestView => {
                section(
                    "Pull Requests",
                    &[
                        nav_items[0].clone(),
                        "[Enter]/[o] open PR in browser".to_string(),
                        "[c] checkout PR branch".to_string(),
                        "[m]/[s]/[R] merge/squash/rebase PR".to_string(),
                        "[n] create PR".to_string(),
                        "[f] cycle PR filter".to_string(),
                        "[b] back to status tab".to_string(),
                    ],
                );
                section("Workspace", &workspace_tabs);
            }
            Screen::TrackingStatusView => {
                section(
                    "Incoming/Outgoing",
                    &[
                        "[f]/[l]/[p] fetch/pull/push".to_string(),
                        "[b] back to status tab".to_string(),
                    ],
                );
                section("Workspace", &workspace_tabs);
            }
            Screen::StashView => {
                section(
                    "Stash",
                    &[
                        nav_items[0].clone(),
                        "[Enter] load stash details".to_string(),
                        "[s] stash current changes".to_string(),
                        "[a] apply selected stash".to_string(),
                        "[p] pop selected stash".to_string(),
                        "[d] drop selected stash (confirm)".to_string(),
                        "[b] back to status tab".to_string(),
                    ],
                );
                section("Workspace", &workspace_tabs);
            }
            Screen::RepoView => {
                let focus = match self.focus {
                    FocusPane::Unstaged => "unstaged",
                    FocusPane::Staged => "staged",
                };
                section(
                    "Status Tab",
                    &[
                        format!("[Tab] switch pane (current: {focus})"),
                        nav_items[0].clone(),
                        "[b] back to main menu".to_string(),
                    ],
                );
                section(
                    "Pane Actions",
                    &[
                        "[s]/[S] stage selected/all unstaged".to_string(),
                        "[u]/[U] unstage selected/all staged".to_string(),
                        "[x] discard selected unstaged (confirm)".to_string(),
                        "[c] commit staged changes".to_string(),
                        "[Right] expand folder / open file diff".to_string(),
                        "[Left] collapse folder".to_string(),
                    ],
                );
                section(
                    "Fullscreen Diff",
                    &[
                        "[Esc] close viewer".to_string(),
                        "[j]/[k]/[PgUp]/[PgDn]/[Home]/[End] scroll".to_string(),
                        "[Left]/[Right] or [h]/[l] horizontal scroll".to_string(),
                        "[n]/[p] next/prev hunk".to_string(),
                    ],
                );
                section(
                    "Repo Actions",
                    &[
                        "[f]/[l]/[p] fetch/pull/push".to_string(),
                        "[v]/[z] cherry-pick continue/abort".to_string(),
                    ],
                );
                section(
                    "Open Tabs",
                    &[
                        "[g]/[G] local/remote branches".to_string(),
                        "[h] history".to_string(),
                        "[i] incoming/outgoing".to_string(),
                        "[t] stash".to_string(),
                        "[P] pull requests".to_string(),
                    ],
                );
                section("Workspace", &workspace_tabs);
            }
        }

        if lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        lines
    }

    pub fn set_runtime_log_path(&mut self, path: PathBuf) {
        self.runtime_log_path = Some(path);
    }

    pub fn show_runtime_log_path_status(&mut self) {
        if let Some(path) = &self.runtime_log_path {
            self.status_message = format!("Log file: {}", display_path_for_ui(&path.to_string_lossy()));
            return;
        }

        match diagnostics::default_log_file_path() {
            Ok(path) => {
                self.status_message = format!(
                    "Logging is disabled. Launch with --log. Default log path: {}",
                    display_path_for_ui(&path.to_string_lossy())
                );
            }
            Err(err) => {
                self.status_message =
                    format!("Logging is disabled. Could not resolve default log path: {err}");
            }
        }
    }

    pub fn refresh(&mut self) -> Result<()> {
        match self.screen {
            Screen::RepoPicker => {
                self.registry = RepoRegistry::load()?;
                self.rebuild_repo_picker_labels();
                self.selected_repo = min(
                    self.selected_repo,
                    self.registry.repos.len().saturating_sub(1),
                );
                self.status_message = "Refreshed repository registry".to_string();
            }
            Screen::RepoBrowser => {
                self.refresh_browser_entries()?;
                self.status_message = format!("Browsing {}", self.browser_dir.display());
            }
            Screen::BranchPicker => {
                self.refresh_branch_entries()?;
                self.status_message = "Refreshed branch list".to_string();
            }
            Screen::RemoteBranchPicker => {
                self.refresh_remote_branch_entries()?;
                self.status_message = "Refreshed remote branch list".to_string();
            }
            Screen::HistoryView => {
                self.refresh_history_entries()?;
                self.set_async_running_status("Refreshing commit history");
            }
            Screen::PullRequestView => {
                match self.refresh_pull_requests() {
                    Ok(_) => {
                        self.status_message = format!(
                            "Refreshed pull requests ({})",
                            self.pull_request_filter_label()
                        );
                    }
                    Err(err) => {
                        self.status_message = format_gh_error_for_status(&err);
                    }
                }
            }
            Screen::TrackingStatusView => {
                self.refresh_tracking_status_summary()?;
                self.status_message = "Refreshed incoming/outgoing commit view".to_string();
            }
            Screen::StashView => {
                self.refresh_stash_entries()?;
                self.set_async_running_status("Refreshing stash list");
            }
            Screen::RepoView => {
                self.request_snapshot_refresh(Some("Refreshed git status".to_string()))?;
                self.set_async_running_status("Refreshing git status");
            }
        }
        Ok(())
    }

    pub fn cycle_focus(&mut self) {
        if self.screen != Screen::RepoView {
            return;
        }
        self.focus = match self.focus {
            FocusPane::Unstaged => FocusPane::Staged,
            FocusPane::Staged => FocusPane::Unstaged,
        };
        self.invalidate_repo_preview_cache();
        self.schedule_repo_preview_refresh();
    }

    pub fn move_next(&mut self) {
        match self.screen {
            Screen::RepoPicker => {
                if !self.registry.repos.is_empty() {
                    self.selected_repo = min(self.selected_repo + 1, self.registry.repos.len() - 1);
                }
            }
            Screen::RepoBrowser => {
                if !self.browser_entries.is_empty() {
                    self.selected_browser = min(self.selected_browser + 1, self.browser_entries.len() - 1);
                }
            }
            Screen::BranchPicker => {
                if !self.branch_entries.is_empty() {
                    self.selected_branch = min(self.selected_branch + 1, self.branch_entries.len() - 1);
                }
            }
            Screen::RemoteBranchPicker => {
                if !self.remote_branch_entries.is_empty() {
                    self.selected_remote_branch = min(
                        self.selected_remote_branch + 1,
                        self.remote_branch_entries.len() - 1,
                    );
                }
            }
            Screen::HistoryView => {
                if self.history_details_visible {
                    match self.history_focus {
                        HistoryFocusPane::Commits => {
                            if !self.history_entries.is_empty() {
                                self.selected_history =
                                    min(self.selected_history + 1, self.history_entries.len() - 1);
                                self.clear_history_details();
                                let _ = self.load_selected_commit_details();
                            }
                        }
                        HistoryFocusPane::ChangedFiles => {
                            let file_count = self.history_tree.len();
                            if file_count > 0 {
                                self.history_file_tree_selected =
                                    min(self.history_file_tree_selected + 1, file_count - 1);
                                self.clear_history_file_history();
                            }
                        }
                        HistoryFocusPane::FileHistory => {
                            if !self.history_file_history_entries.is_empty() {
                                self.history_file_history_selected = min(
                                    self.history_file_history_selected + 1,
                                    self.history_file_history_entries.len() - 1,
                                );
                            }
                        }
                    }
                } else if !self.history_entries.is_empty() {
                    self.selected_history = min(self.selected_history + 1, self.history_entries.len() - 1);
                    self.clear_history_details();
                }
            }
            Screen::PullRequestView => {
                if !self.pull_requests.is_empty() {
                    self.selected_pr = min(self.selected_pr + 1, self.pull_requests.len() - 1);
                    self.schedule_selected_pr_status_refresh();
                }
            }
            Screen::TrackingStatusView => {}
            Screen::StashView => {
                if !self.stash_entries.is_empty() {
                    self.selected_stash = min(self.selected_stash + 1, self.stash_entries.len() - 1);
                    self.clear_stash_details();
                }
            }
            Screen::RepoView => match self.focus {
                FocusPane::Unstaged => {
                    if self.unstaged_tree.len() > 0 {
                        self.selected_unstaged =
                            min(self.selected_unstaged + 1, self.unstaged_tree.len() - 1);
                        self.invalidate_repo_preview_cache();
                        self.schedule_repo_preview_refresh();
                    }
                }
                FocusPane::Staged => {
                    if self.staged_tree.len() > 0 {
                        self.selected_staged = min(self.selected_staged + 1, self.staged_tree.len() - 1);
                        self.invalidate_repo_preview_cache();
                        self.schedule_repo_preview_refresh();
                    }
                }
            },
        }
    }

    pub fn move_prev(&mut self) {
        match self.screen {
            Screen::RepoPicker => {
                self.selected_repo = self.selected_repo.saturating_sub(1);
            }
            Screen::RepoBrowser => {
                self.selected_browser = self.selected_browser.saturating_sub(1);
            }
            Screen::BranchPicker => {
                self.selected_branch = self.selected_branch.saturating_sub(1);
            }
            Screen::RemoteBranchPicker => {
                self.selected_remote_branch = self.selected_remote_branch.saturating_sub(1);
            }
            Screen::HistoryView => {
                if self.history_details_visible {
                    match self.history_focus {
                        HistoryFocusPane::Commits => {
                            self.selected_history = self.selected_history.saturating_sub(1);
                            self.clear_history_details();
                            let _ = self.load_selected_commit_details();
                        }
                        HistoryFocusPane::ChangedFiles => {
                            self.history_file_tree_selected =
                                self.history_file_tree_selected.saturating_sub(1);
                            self.clear_history_file_history();
                        }
                        HistoryFocusPane::FileHistory => {
                            self.history_file_history_selected =
                                self.history_file_history_selected.saturating_sub(1);
                        }
                    }
                } else {
                    self.selected_history = self.selected_history.saturating_sub(1);
                    self.clear_history_details();
                }
            }
            Screen::PullRequestView => {
                self.selected_pr = self.selected_pr.saturating_sub(1);
                self.schedule_selected_pr_status_refresh();
            }
            Screen::TrackingStatusView => {}
            Screen::StashView => {
                self.selected_stash = self.selected_stash.saturating_sub(1);
                self.clear_stash_details();
            }
            Screen::RepoView => match self.focus {
                FocusPane::Unstaged => {
                    self.selected_unstaged = self.selected_unstaged.saturating_sub(1);
                    self.invalidate_repo_preview_cache();
                    self.schedule_repo_preview_refresh();
                }
                FocusPane::Staged => {
                    self.selected_staged = self.selected_staged.saturating_sub(1);
                    self.invalidate_repo_preview_cache();
                    self.schedule_repo_preview_refresh();
                }
            },
        }
    }

    pub fn activate_focused_action(&mut self) -> Result<()> {
        match self.screen {
            Screen::RepoPicker => self.open_selected_repo(),
            Screen::RepoBrowser => self.browser_enter_selected(),
            Screen::BranchPicker => self.switch_selected_branch(),
            Screen::RemoteBranchPicker => self.checkout_selected_remote_branch(),
            Screen::HistoryView => self.toggle_history_details(),
            Screen::PullRequestView => self.open_selected_pr_in_browser(),
            Screen::TrackingStatusView => Ok(()),
            Screen::StashView => self.load_selected_stash_details(),
            Screen::RepoView => match self.focus {
                FocusPane::Unstaged => self.stage_selected(),
                FocusPane::Staged => self.unstage_selected(),
            },
        }
    }

    pub fn begin_add_repo_input(&mut self) {
        self.input_mode = InputMode::AddRepoPath;
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.status_message = "Enter repository root path containing .git".to_string();
    }

    pub fn enter_repo_browser(&mut self) -> Result<()> {
        self.screen = Screen::RepoBrowser;
        self.refresh_browser_entries()?;
        self.status_message = format!("Browsing {}", self.browser_dir.display());
        Ok(())
    }

    pub fn browser_go_parent(&mut self) -> Result<()> {
        if self.screen != Screen::RepoBrowser {
            return Ok(());
        }
        if let Some(parent) = self.browser_dir.parent() {
            self.browser_dir = parent.to_path_buf();
            self.refresh_browser_entries()?;
            self.status_message = format!("Browsing {}", self.browser_dir.display());
        } else {
            self.status_message = "Already at filesystem root".to_string();
        }
        Ok(())
    }

    pub fn toggle_browser_hidden_entries(&mut self) -> Result<()> {
        if self.screen != Screen::RepoBrowser {
            self.status_message = "Hidden file toggle is available in repo browser only".to_string();
            return Ok(());
        }
        self.show_hidden_browser_entries = !self.show_hidden_browser_entries;
        self.refresh_browser_entries()?;
        self.status_message = if self.show_hidden_browser_entries {
            "Repo browser: showing hidden entries".to_string()
        } else {
            "Repo browser: hiding hidden entries".to_string()
        };
        Ok(())
    }

    pub fn begin_commit_input(&mut self) {
        if self.screen != Screen::RepoView {
            self.status_message = "Open a repository before committing".to_string();
            return;
        }
        if self.snapshot.staged.is_empty() {
            self.status_message = "Stage files before committing".to_string();
            return;
        }
        let root = match self.current_repo_root() {
            Ok(path) => path.to_path_buf(),
            Err(err) => {
                self.status_message = format!("Unable to open repository for commit: {err}");
                return;
            }
        };
        match git::unresolved_conflict_files(&root) {
            Ok(conflicts) if !conflicts.is_empty() => {
                self.status_message = unresolved_conflicts_status(&conflicts);
                return;
            }
            Ok(_) => {}
            Err(err) => {
                self.status_message = format!("Unable to check merge conflicts: {err}");
                return;
            }
        }
        self.pending_commit_subject = None;
        self.input_mode = InputMode::CommitSubject;
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.status_message = "Enter commit subject and press [Enter]".to_string();
    }

    pub fn return_to_repo_view(&mut self) {
        self.screen = Screen::RepoView;
        self.input_mode = InputMode::None;
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.clear_history_details();
        self.clear_stash_details();
        self.clear_pr_status_summary();
        self.pending_stash_drop = None;
        self.pending_discard = None;
        self.pending_pr_merge = None;
        self.pending_pr_title = None;
        self.pending_commit_subject = None;
        self.schedule_repo_preview_refresh();
        self.close_fullscreen_diff();
        self.status_message = "Repository view".to_string();
    }


    pub fn cancel_input(&mut self) {
        let was_stash_drop_confirmation = self.input_mode == InputMode::ConfirmStashDrop;
        let was_discard_confirmation = self.input_mode == InputMode::ConfirmDiscard;
        let was_pr_merge_confirmation = self.input_mode == InputMode::ConfirmPullRequestMerge;
        self.input_mode = InputMode::None;
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.pending_stash_drop = None;
        self.pending_discard = None;
        self.pending_pr_merge = None;
        self.pending_pr_title = None;
        self.pending_commit_subject = None;
        self.status_message = if was_stash_drop_confirmation {
            "Stash drop cancelled".to_string()
        } else if was_discard_confirmation {
            "Discard cancelled".to_string()
        } else if was_pr_merge_confirmation {
            "PR merge cancelled".to_string()
        } else {
            "Input cancelled".to_string()
        };
    }

    pub fn push_input_char(&mut self, ch: char) {
        self.insert_input_char(ch);
    }

    pub fn pop_input_char(&mut self) {
        self.delete_input_char_before_cursor();
    }

    pub fn delete_input_char(&mut self) {
        let mut chars = self.input_buffer.chars().collect::<Vec<_>>();
        if self.input_cursor >= chars.len() {
            return;
        }
        chars.remove(self.input_cursor);
        self.input_buffer = chars.into_iter().collect();
    }

    pub fn move_input_cursor_left(&mut self) {
        self.input_cursor = self.input_cursor.saturating_sub(1);
    }

    pub fn move_input_cursor_right(&mut self) {
        let len = self.input_buffer.chars().count();
        self.input_cursor = min(self.input_cursor + 1, len);
    }

    pub fn move_input_cursor_home(&mut self) {
        self.input_cursor = 0;
    }

    pub fn move_input_cursor_end(&mut self) {
        self.input_cursor = self.input_buffer.chars().count();
    }

    pub fn move_input_cursor_up(&mut self) {
        let chars = self.input_buffer.chars().collect::<Vec<_>>();
        if chars.is_empty() || self.input_cursor == 0 {
            return;
        }
        let cursor = self.input_cursor.min(chars.len());
        let current_start = line_start_before_or_at(&chars, cursor);
        if current_start == 0 {
            self.input_cursor = 0;
            return;
        }

        let current_col = cursor.saturating_sub(current_start);
        let prev_end = current_start.saturating_sub(1);
        let prev_start = line_start_before_or_at(&chars, prev_end);
        let prev_len = prev_end.saturating_sub(prev_start);
        self.input_cursor = prev_start + min(current_col, prev_len);
    }

    pub fn move_input_cursor_down(&mut self) {
        let chars = self.input_buffer.chars().collect::<Vec<_>>();
        if chars.is_empty() {
            return;
        }
        let cursor = self.input_cursor.min(chars.len());
        let current_start = line_start_before_or_at(&chars, cursor);
        let current_end = line_end_from(&chars, current_start);
        if current_end >= chars.len() {
            self.input_cursor = chars.len();
            return;
        }

        let current_col = cursor.saturating_sub(current_start);
        let next_start = current_end + 1;
        let next_end = line_end_from(&chars, next_start);
        let next_len = next_end.saturating_sub(next_start);
        self.input_cursor = next_start + min(current_col, next_len);
    }

    fn insert_input_char(&mut self, ch: char) {
        let mut chars = self.input_buffer.chars().collect::<Vec<_>>();
        if self.input_cursor > chars.len() {
            self.input_cursor = chars.len();
        }
        chars.insert(self.input_cursor, ch);
        self.input_cursor += 1;
        self.input_buffer = chars.into_iter().collect();
    }

    fn delete_input_char_before_cursor(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let mut chars = self.input_buffer.chars().collect::<Vec<_>>();
        if self.input_cursor > chars.len() {
            self.input_cursor = chars.len();
        }
        chars.remove(self.input_cursor - 1);
        self.input_cursor -= 1;
        self.input_buffer = chars.into_iter().collect();
    }

    pub fn submit_input(&mut self) -> Result<()> {
        match self.input_mode {
            InputMode::None => Ok(()),
            InputMode::CommitSubject => self.commit_subject_from_input(),
            InputMode::CommitBody => self.commit_body_from_input(),
            InputMode::AddRepoPath => self.add_repo_from_input(),
            InputMode::NewBranchName => self.create_branch_from_input(),
            InputMode::CreatePullRequestTitle => self.create_pull_request_title_from_input(),
            InputMode::CreatePullRequestBody => self.create_pull_request_body_from_input(),
            InputMode::ConfirmStashDrop => Ok(()),
            InputMode::ConfirmDiscard => Ok(()),
            InputMode::ConfirmPullRequestMerge => Ok(()),
        }
    }

    pub fn autocomplete_input(&mut self) -> Result<()> {
        if self.input_mode == InputMode::AddRepoPath {
            self.autocomplete_repo_path_input()?;
        }
        Ok(())
    }

    pub fn stage_selected(&mut self) -> Result<()> {
        let Some(row) = self.current_unstaged_row() else {
            self.status_message = "No unstaged path selected".to_string();
            return Ok(());
        };
        let path = row.path.clone();
        let label = match row.kind {
            TreeRowKind::Directory => format!("Staging directory {}", path),
            TreeRowKind::File => format!("Staging {}", path),
        };
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            &label,
            AsyncWriteOp::StageFile { path: path.clone() },
            move || git::stage_file(&root, &path),
        );
        Ok(())
    }

    pub fn unstage_selected(&mut self) -> Result<()> {
        let Some(row) = self.current_staged_row() else {
            self.status_message = "No staged path selected".to_string();
            return Ok(());
        };
        let path = row.path.clone();
        let label = match row.kind {
            TreeRowKind::Directory => format!("Unstaging directory {}", path),
            TreeRowKind::File => format!("Unstaging {}", path),
        };
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            &label,
            AsyncWriteOp::UnstageFile { path: path.clone() },
            move || git::unstage_file(&root, &path),
        );
        Ok(())
    }

    pub fn stage_all_unstaged(&mut self) -> Result<()> {
        if self.snapshot.unstaged.is_empty() {
            self.status_message = "No unstaged files to stage".to_string();
            return Ok(());
        }
        let total = self.snapshot.unstaged.len();
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            "Staging all unstaged files",
            AsyncWriteOp::StageAll { total },
            move || git::stage_all(&root),
        );
        Ok(())
    }

    pub fn unstage_all_staged(&mut self) -> Result<()> {
        if self.snapshot.staged.is_empty() {
            self.status_message = "No staged files to unstage".to_string();
            return Ok(());
        }
        let total = self.snapshot.staged.len();
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            "Unstaging all staged files",
            AsyncWriteOp::UnstageAll { total },
            move || git::unstage_all(&root),
        );
        Ok(())
    }

    pub fn fetch_remotes(&mut self) -> Result<()> {
        if !matches!(self.screen, Screen::RepoView | Screen::TrackingStatusView) {
            self.status_message = "Open a repository before fetching".to_string();
            return Ok(());
        }
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op("Fetching remotes", AsyncWriteOp::Fetch, move || git::fetch(&root));
        Ok(())
    }

    pub fn pull_current_branch(&mut self) -> Result<()> {
        if !matches!(self.screen, Screen::RepoView | Screen::TrackingStatusView) {
            self.status_message = "Open a repository before pulling".to_string();
            return Ok(());
        }
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            "Pulling latest changes",
            AsyncWriteOp::Pull,
            move || git::pull(&root),
        );
        Ok(())
    }

    pub fn push_current_branch(&mut self) -> Result<()> {
        if !matches!(self.screen, Screen::RepoView | Screen::TrackingStatusView) {
            self.status_message = "Open a repository before pushing".to_string();
            return Ok(());
        }
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op("Pushing current branch", AsyncWriteOp::Push, move || {
            git::push(&root)
        });
        Ok(())
    }

    pub fn continue_cherry_pick(&mut self) -> Result<()> {
        if self.screen != Screen::RepoView {
            self.status_message = "Open a repository before continuing cherry-pick".to_string();
            return Ok(());
        }
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            "Continuing cherry-pick",
            AsyncWriteOp::CherryPickContinue,
            move || git::cherry_pick_continue(&root),
        );
        Ok(())
    }

    pub fn abort_cherry_pick(&mut self) -> Result<()> {
        if self.screen != Screen::RepoView {
            self.status_message = "Open a repository before aborting cherry-pick".to_string();
            return Ok(());
        }
        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            "Aborting cherry-pick",
            AsyncWriteOp::CherryPickAbort,
            move || git::cherry_pick_abort(&root),
        );
        Ok(())
    }

    pub fn discard_selected_unstaged(&mut self) -> Result<()> {
        let Some(entry) = self.current_unstaged_row() else {
            self.status_message = "No unstaged file selected".to_string();
            return Ok(());
        };
        if entry.kind == TreeRowKind::Directory {
            self.status_message = "Discard is file-only; select a file inside the folder".to_string();
            return Ok(());
        }
        self.pending_discard = Some(PendingDiscard {
            path: entry.path.clone(),
            is_untracked: entry.status.as_deref() == Some("??"),
        });
        self.input_mode = InputMode::ConfirmDiscard;
        self.status_message = "Confirm discard: [y] confirm, [n]/[Esc] cancel".to_string();
        Ok(())
    }

    pub fn confirm_discard_selected(&mut self) -> Result<()> {
        let Some(pending) = self.pending_discard.take() else {
            self.input_mode = InputMode::None;
            self.status_message = "No discard action to confirm".to_string();
            return Ok(());
        };

        let root = self.current_repo_root()?.to_path_buf();
        let pending_path = pending.path.clone();
        let pending_untracked = pending.is_untracked;
        self.input_mode = InputMode::None;
        self.request_write_op(
            &format!("Discarding {}", pending_path),
            AsyncWriteOp::DiscardFile {
                path: pending_path.clone(),
                is_untracked: pending_untracked,
            },
            move || git::discard_file(&root, &pending_path, pending_untracked),
        );
        Ok(())
    }

    pub fn open_selected_repo(&mut self) -> Result<()> {
        let Some(path) = self
            .registry
            .repos
            .get(self.selected_repo)
            .map(|repo| repo.path.clone())
        else {
            self.status_message = "No repository selected".to_string();
            return Ok(());
        };
        self.open_repo_path(PathBuf::from(path))
    }

    pub fn remove_selected_repo(&mut self) -> Result<()> {
        if self.registry.repos.is_empty() {
            self.status_message = "No repositories to remove".to_string();
            return Ok(());
        }
        let removed = self.registry.remove_index(self.selected_repo)?;
        self.selected_repo = self.selected_repo.saturating_sub(1);
        self.registry = RepoRegistry::load()?;
        self.rebuild_repo_picker_labels();
        if let Some(repo) = removed {
            self.status_message =
                format!("Removed repository {}", display_path_for_ui(&repo.path));
        }
        Ok(())
    }

    pub fn return_to_picker(&mut self) {
        self.screen = Screen::RepoPicker;
        self.repo_root = None;
        self.snapshot = RepoSnapshot::default();
        self.unstaged_tree.clear();
        self.staged_tree.clear();
        self.branch_entries.clear();
        self.selected_branch = 0;
        self.remote_branch_entries.clear();
        self.selected_remote_branch = 0;
        self.history_entries.clear();
        self.selected_history = 0;
        self.pull_requests.clear();
        self.selected_pr = 0;
        self.pr_filter = GitPullRequestFilter::Open;
        self.tracking_summary = None;
        self.stash_entries.clear();
        self.selected_stash = 0;
        self.clear_stash_details();
        self.last_fetch_at = None;
        self.input_mode = InputMode::None;
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.pending_stash_drop = None;
        self.pending_discard = None;
        self.pending_pr_merge = None;
        self.pending_pr_title = None;
        self.pending_commit_subject = None;
        self.clear_history_details();
        self.clear_history_file_history();
        self.history_details_visible = false;
        self.history_focus = HistoryFocusPane::Commits;
        self.clear_pr_status_summary();
        self.invalidate_repo_preview_cache();
        self.repo_preview_refresh_deadline = None;
        self.repo_preview_text = "No unstaged diff output for selected file.".to_string();
        self.snapshot_inflight = None;
        self.snapshot_completion_message = None;
        self.repo_preview_inflight = None;
        self.repo_preview_pending_key = None;
        self.history_details_inflight = None;
        self.stash_details_inflight = None;
        self.history_entries_inflight = None;
        self.stash_entries_inflight = None;
        self.pull_requests_inflight = None;
        self.pr_status_inflight = None;
        self.tracking_inflight = None;
        self.branch_entries_inflight = None;
        self.remote_branch_entries_inflight = None;
        self.history_file_history_inflight = None;
        self.fullscreen_diff_inflight = None;
        self.fullscreen_diff_pending_key = None;
        self.close_fullscreen_diff();
        self.write_inflight = false;
        self.status_message = "Main menu".to_string();
    }

    pub fn preview_text(&self) -> String {
        match self.screen {
            Screen::RepoPicker => {
                if let Some(repo) = self.registry.repos.get(self.selected_repo) {
                    format!(
                        "Selected repository:\n{}\n\n[Enter] open | [a] add path | [f] browse folders | [d] remove",
                        display_path_for_ui(&repo.path)
                    )
                } else {
                    "No repositories tracked.\nPress [a] to type a path or [f] to browse directories."
                        .to_string()
                }
            }
            Screen::RepoBrowser => {
                if let Some(entry) = self.browser_entries.get(self.selected_browser) {
                    let mut lines = vec![
                        format!("Current directory:\n{}", self.browser_dir.display()),
                        String::new(),
                        format!("Selected: {}", entry.path.display()),
                    ];
                    if entry.is_dir {
                        lines.push("Type: directory".to_string());
                    } else {
                        lines.push("Type: file (ignored for repo add)".to_string());
                    }
                    if entry.is_git_root {
                        lines.push("This directory is a git root (.git found).".to_string());
                        lines.push("Press [Enter] to add and open this repository.".to_string());
                    } else if entry.is_dir {
                        lines.push("Press [Enter] to enter this directory.".to_string());
                    }
                    lines.push(String::new());
                    lines.push(format!(
                        "Hidden entries: {} (toggle with [.])",
                        if self.show_hidden_browser_entries {
                            "shown"
                        } else {
                            "hidden"
                        }
                    ));
                    lines.join("\n")
                } else {
                    format!(
                        "Current directory:\n{}\n\nNo entries.\n\nHidden entries: {} (toggle with [.])",
                        self.browser_dir.display(),
                        if self.show_hidden_browser_entries { "shown" } else { "hidden" }
                    )
                }
            }
            Screen::BranchPicker => {
                if let Some(branch) = self.branch_entries.get(self.selected_branch) {
                    let marker = if branch.is_current { " (current)" } else { "" };
                    format!(
                        "Selected branch:\n{}{}\n\n[Enter] switch | [n] new branch | [b] back",
                        branch.name, marker
                    )
                } else {
                    "No local branches found.\nPress [b] to return.".to_string()
                }
            }
            Screen::RemoteBranchPicker => {
                if let Some(branch) = self.remote_branch_entries.get(self.selected_remote_branch) {
                    format!(
                        "Selected remote branch:\n{}\n\n[Enter] checkout tracking branch | [b] back",
                        branch.name
                    )
                } else {
                    "No remote branches found.\nRun [f] fetch in repo view, then retry.".to_string()
                }
            }
            Screen::HistoryView => self.history_preview_text(),
            Screen::PullRequestView => self.pull_request_preview_text(),
            Screen::TrackingStatusView => self.tracking_status_preview_text(),
            Screen::StashView => self.stash_preview_text(),
            Screen::RepoView => self.repo_preview_text.clone(),
        }
    }

    pub fn branch_summary(&self) -> String {
        if !self.has_open_repo() {
            return "No repository opened".to_string();
        }
        let mut summary = self.snapshot.branch.clone();
        if let Some(tracking) = &self.snapshot.tracking {
            summary.push_str(&format!(" -> {tracking}"));
        }
        if self.snapshot.ahead > 0 || self.snapshot.behind > 0 {
            summary.push_str(&format!(
                " [ahead {}, behind {}]",
                self.snapshot.ahead, self.snapshot.behind
            ));
        }
        summary
    }

    pub fn active_repo_label(&self) -> String {
        match self.screen {
            Screen::RepoBrowser => display_path_for_ui(&self.browser_dir.to_string_lossy()),
            _ => self
                .repo_root
                .as_ref()
                .map(|p| display_path_for_ui(&p.to_string_lossy()))
                .unwrap_or_else(|| "None".to_string()),
        }
    }

    pub fn format_path_for_ui(&self, raw: &str) -> String {
        display_path_for_ui(raw)
    }

    pub fn repo_picker_label(&self, index: usize) -> Option<&str> {
        self.repo_picker_labels.get(index).map(|label| label.as_str())
    }

    pub fn popup_title(&self) -> &'static str {
        match self.input_mode {
            InputMode::CommitSubject => "Commit subject ([Enter] next, [Esc] cancel)",
            InputMode::CommitBody => {
                "Commit body ([Enter] newline, [Up/Down] lines, [Ctrl+S]/[F2] submit, [Esc] cancel)"
            }
            InputMode::AddRepoPath => "Add repository path ([Enter] submit, [Esc] cancel)",
            InputMode::NewBranchName => "New branch name ([Enter] create/switch, [Esc] cancel)",
            InputMode::CreatePullRequestTitle => "Create PR: title ([Enter] next, [Esc] cancel)",
            InputMode::CreatePullRequestBody => "Create PR: body ([Enter] submit, [Esc] cancel)",
            InputMode::ConfirmStashDrop => "Confirm stash drop ([y] yes, [n]/[Esc] no)",
            InputMode::ConfirmDiscard => "Confirm discard ([y] yes, [n]/[Esc] no)",
            InputMode::ConfirmPullRequestMerge => "Confirm PR merge ([y] yes, [n]/[Esc] no)",
            InputMode::None => "",
        }
    }

    pub fn popup_input_text(&self) -> Option<&str> {
        match self.input_mode {
            InputMode::CommitSubject
            | InputMode::CommitBody
            | InputMode::AddRepoPath
            | InputMode::NewBranchName
            | InputMode::CreatePullRequestTitle
            | InputMode::CreatePullRequestBody => Some(&self.input_buffer),
            _ => None,
        }
    }

    pub fn popup_input_prefix(&self) -> Option<String> {
        match self.input_mode {
            InputMode::CommitBody => {
                let subject = self
                    .pending_commit_subject
                    .as_deref()
                    .unwrap_or("(missing subject)");
                Some(format!("Subject: {subject}\n\n"))
            }
            InputMode::CreatePullRequestBody => {
                let title = self.pending_pr_title.as_deref().unwrap_or("(missing title)");
                Some(format!("Title: {title}\n\n"))
            }
            InputMode::CommitSubject
            | InputMode::AddRepoPath
            | InputMode::NewBranchName
            | InputMode::CreatePullRequestTitle => Some(String::new()),
            _ => None,
        }
    }

    pub fn popup_input_cursor(&self) -> usize {
        self.input_cursor
    }

    pub fn popup_body(&self) -> String {
        match self.input_mode {
            InputMode::ConfirmStashDrop => {
                let Some(reference) = &self.pending_stash_drop else {
                    return "No stash selected to drop.".to_string();
                };
                format!(
                    "Drop stash {}?\n\nThis action cannot be undone.\n\n[y] confirm    [n]/[Esc] cancel",
                    reference
                )
            }
            InputMode::ConfirmDiscard => {
                let Some(pending) = &self.pending_discard else {
                    return "No file selected for discard.".to_string();
                };
                let action = if pending.is_untracked {
                    "delete this untracked file"
                } else {
                    "discard unstaged changes in this file"
                };
                format!(
                    "Are you sure you want to {}?\n\n{}\n\n[y] confirm    [n]/[Esc] cancel",
                    action, pending.path
                )
            }
            InputMode::ConfirmPullRequestMerge => {
                let Some(pending) = &self.pending_pr_merge else {
                    return "No pull request selected for merge.".to_string();
                };
                format!(
                    "Merge PR #{} with {} strategy?\n\n{}\n\n[y] confirm    [n]/[Esc] cancel",
                    pending.number,
                    pull_request_merge_method_label(pending.method),
                    pending.title
                )
            }
            InputMode::CommitBody => {
                if let Some(subject) = &self.pending_commit_subject {
                    format!("Subject: {}\n\n{}", subject, self.input_buffer)
                } else {
                    self.input_buffer.clone()
                }
            }
            InputMode::CreatePullRequestBody => {
                if let Some(title) = &self.pending_pr_title {
                    format!("Title: {}\n\n{}", title, self.input_buffer)
                } else {
                    self.input_buffer.clone()
                }
            }
            InputMode::CommitSubject
            | InputMode::AddRepoPath
            | InputMode::NewBranchName
            | InputMode::CreatePullRequestTitle => self.input_buffer.clone(),
            _ => self.input_buffer.clone(),
        }
    }

    pub fn input_mode_allows_multiline(&self) -> bool {
        matches!(self.input_mode, InputMode::CommitBody)
    }

    fn commit_subject_from_input(&mut self) -> Result<()> {
        let subject = self.input_buffer.trim();
        if subject.is_empty() {
            self.status_message = "Commit subject cannot be empty".to_string();
            return Ok(());
        }
        self.pending_commit_subject = Some(subject.to_string());
        self.input_mode = InputMode::CommitBody;
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.status_message = "Enter commit body (optional), [Ctrl+S] or [F2] to commit".to_string();
        Ok(())
    }

    fn commit_body_from_input(&mut self) -> Result<()> {
        let Some(subject) = self.pending_commit_subject.take() else {
            self.input_mode = InputMode::None;
            self.input_buffer.clear();
            self.input_cursor = 0;
            self.status_message = "Commit subject is missing; start commit again".to_string();
            return Ok(());
        };

        let body = self.input_buffer.trim_end().to_string();
        let body_option = if body.trim().is_empty() {
            None
        } else {
            Some(body)
        };
        let root = self.current_repo_root()?.to_path_buf();
        self.input_mode = InputMode::None;
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.request_write_op(
            "Creating commit",
            AsyncWriteOp::Commit,
            move || git::commit(&root, &subject, body_option.as_deref()),
        );
        Ok(())
    }

    fn add_repo_from_input(&mut self) -> Result<()> {
        let canonical = canonical_repo_path(self.input_buffer.trim())?;
        self.registry.add_repo(&canonical)?;
        self.registry = RepoRegistry::load()?;
        self.rebuild_repo_picker_labels();
        if let Some(idx) = self
            .registry
            .repos
            .iter()
            .position(|repo| repo.path == canonical.to_string_lossy())
        {
            self.selected_repo = idx;
        }
        self.input_mode = InputMode::None;
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.status_message = format!(
            "Added repository {}",
            display_path_for_ui(&canonical.to_string_lossy())
        );
        Ok(())
    }

    fn create_pull_request_title_from_input(&mut self) -> Result<()> {
        let title = self.input_buffer.trim();
        if title.is_empty() {
            self.status_message = "PR title cannot be empty".to_string();
            return Ok(());
        }
        self.pending_pr_title = Some(title.to_string());
        self.input_mode = InputMode::CreatePullRequestBody;
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.status_message = "Enter PR body (optional), then press [Enter]".to_string();
        Ok(())
    }

    fn create_pull_request_body_from_input(&mut self) -> Result<()> {
        let Some(title) = self.pending_pr_title.take() else {
            self.input_mode = InputMode::None;
            self.input_buffer.clear();
            self.input_cursor = 0;
            self.status_message = "PR title is missing; start PR creation again".to_string();
            return Ok(());
        };

        let body = self.input_buffer.trim().to_string();
        self.input_mode = InputMode::None;
        self.input_buffer.clear();
        self.input_cursor = 0;

        let root = self.current_repo_root()?.to_path_buf();
        self.request_write_op(
            "Creating pull request",
            AsyncWriteOp::CreatePullRequest,
            move || git::create_pull_request(&root, &title, &body),
        );
        Ok(())
    }

    fn autocomplete_repo_path_input(&mut self) -> Result<()> {
        let normalized = normalize_repo_path_input(&self.input_buffer);
        if normalized.is_empty() {
            self.status_message = "Type a path first, then press [Tab] to autocomplete.".to_string();
            return Ok(());
        }

        let normalized_path = PathBuf::from(&normalized);
        let (base_dir, fragment) = completion_base_and_fragment(&normalized_path, &normalized);
        if !base_dir.is_dir() {
            self.status_message = format!(
                "Autocomplete base directory does not exist: {}",
                base_dir.display()
            );
            return Ok(());
        }

        let mut candidates = fs::read_dir(&base_dir)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.to_lowercase().starts_with(&fragment.to_lowercase()) {
                    Some((name, entry.path().is_dir()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        if candidates.is_empty() {
            self.status_message = "No path matches found for autocomplete.".to_string();
            return Ok(());
        }

        candidates.sort_by(|a, b| a.0.cmp(&b.0));
        let names = candidates.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>();
        let common_prefix = common_prefix(&names);

        if names.len() > 1 && common_prefix.len() <= fragment.len() {
            let preview = names.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
            self.status_message = format!(
                "Multiple matches; keep typing to narrow autocomplete: {}",
                preview
            );
            return Ok(());
        }

        let completed_path = if common_prefix.len() > fragment.len() {
            base_dir.join(common_prefix)
        } else {
            base_dir.join(&candidates[0].0)
        };

        let mut completed = completed_path.to_string_lossy().to_string();
        let matched_dir = candidates
            .iter()
            .find(|(name, _)| *name == completed_path.file_name().unwrap_or_default().to_string_lossy())
            .map(|(_, is_dir)| *is_dir)
            .unwrap_or(false);
        if matched_dir && !completed.ends_with(std::path::MAIN_SEPARATOR) {
            completed.push(std::path::MAIN_SEPARATOR);
        }
        self.input_buffer = completed;
        self.input_cursor = self.input_buffer.chars().count();

        if names.len() == 1 {
            self.status_message = "Autocomplete applied (single match).".to_string();
        } else {
            let preview = names.into_iter().take(5).collect::<Vec<_>>().join(", ");
            self.status_message = format!("Autocomplete candidates: {}", preview);
        }
        Ok(())
    }

    fn open_repo_path(&mut self, path: PathBuf) -> Result<()> {
        self.repo_root = Some(path);
        self.snapshot = RepoSnapshot::default();
        self.refresh_last_fetch_from_git_metadata();
        self.screen = Screen::RepoView;
        self.focus = FocusPane::Unstaged;
        self.selected_unstaged = 0;
        self.selected_staged = 0;
        self.branch_entries.clear();
        self.selected_branch = 0;
        self.remote_branch_entries.clear();
        self.selected_remote_branch = 0;
        self.history_entries.clear();
        self.selected_history = 0;
        self.pull_requests.clear();
        self.selected_pr = 0;
        self.pr_filter = GitPullRequestFilter::Open;
        self.tracking_summary = None;
        self.stash_entries.clear();
        self.selected_stash = 0;
        self.input_mode = InputMode::None;
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.clear_history_details();
        self.clear_stash_details();
        self.pending_stash_drop = None;
        self.pending_discard = None;
        self.pending_pr_merge = None;
        self.pending_pr_title = None;
        self.pending_commit_subject = None;
        self.clear_pr_status_summary();
        self.snapshot_inflight = None;
        self.snapshot_completion_message = None;
        self.repo_preview_inflight = None;
        self.repo_preview_pending_key = None;
        self.history_details_inflight = None;
        self.stash_details_inflight = None;
        self.history_entries_inflight = None;
        self.stash_entries_inflight = None;
        self.pull_requests_inflight = None;
        self.pr_status_inflight = None;
        self.tracking_inflight = None;
        self.branch_entries_inflight = None;
        self.remote_branch_entries_inflight = None;
        self.write_inflight = false;
        self.invalidate_repo_preview_cache();
        self.schedule_repo_preview_refresh();
        self.request_snapshot_refresh(Some("Opened repository".to_string()))?;
        self.set_async_running_status("Opening repository");
        Ok(())
    }

    fn refresh_browser_entries(&mut self) -> Result<()> {
        let mut entries = fs::read_dir(&self.browser_dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = path.is_dir();
                let is_git_root = is_dir && path.join(".git").exists();
                BrowserEntry {
                    name,
                    path,
                    is_dir,
                    is_git_root,
                }
            })
            .filter(|entry| {
                if self.show_hidden_browser_entries {
                    return true;
                }
                !entry.name.starts_with('.')
            })
            .collect::<Vec<_>>();

        entries.sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            }
        });

        self.browser_entries = entries;
        self.selected_browser = min(self.selected_browser, self.browser_entries.len().saturating_sub(1));
        Ok(())
    }

    fn refresh_pull_requests(&mut self) -> Result<()> {
        self.clear_pr_status_summary();
        self.request_pull_requests_refresh()?;
        Ok(())
    }

    fn browser_enter_selected(&mut self) -> Result<()> {
        let Some(entry) = self.browser_entries.get(self.selected_browser).cloned() else {
            self.status_message = "No entry selected".to_string();
            return Ok(());
        };

        if entry.is_git_root {
            let canonical = self.registry.add_repo(&entry.path)?;
            self.registry = RepoRegistry::load()?;
            if let Some(idx) = self
                .registry
                .repos
                .iter()
                .position(|repo| repo.path == canonical.to_string_lossy())
            {
                self.selected_repo = idx;
            }
            self.open_repo_path(canonical)?;
            self.status_message = "Added and opened repository from browser".to_string();
            return Ok(());
        }

        if entry.is_dir {
            self.browser_dir = entry.path;
            self.selected_browser = 0;
            self.refresh_browser_entries()?;
            self.status_message = format!("Browsing {}", self.browser_dir.display());
            return Ok(());
        }

        self.status_message = "Selected entry is not a directory".to_string();
        Ok(())
    }

    fn current_repo_root(&self) -> Result<&PathBuf> {
        self.repo_root
            .as_ref()
            .ok_or_else(|| anyhow!("No repository is currently open"))
    }

    fn current_unstaged_row(&self) -> Option<&TreeRow> {
        self.unstaged_tree.row(self.selected_unstaged)
    }

    fn current_staged_row(&self) -> Option<&TreeRow> {
        self.staged_tree.row(self.selected_staged)
    }

    fn ensure_selection_bounds(&mut self) {
        self.rebuild_status_trees();
        if self.unstaged_tree.len() == 0 {
            self.selected_unstaged = 0;
        } else {
            self.selected_unstaged = min(self.selected_unstaged, self.unstaged_tree.len() - 1);
        }
        if self.staged_tree.len() == 0 {
            self.selected_staged = 0;
        } else {
            self.selected_staged = min(self.selected_staged, self.staged_tree.len() - 1);
        }
        self.invalidate_repo_preview_cache();
        self.schedule_repo_preview_refresh();
    }

    fn rebuild_status_trees(&mut self) {
        self.unstaged_tree.set_files(
            self.snapshot
                .unstaged
                .iter()
                .map(|entry| FileLeaf {
                    path: entry.path.clone(),
                    status: entry.status.clone(),
                })
                .collect(),
        );
        self.unstaged_tree.expand_all_dirs();

        self.staged_tree.set_files(
            self.snapshot
                .staged
                .iter()
                .map(|entry| FileLeaf {
                    path: entry.path.clone(),
                    status: entry.status.clone(),
                })
                .collect(),
        );
        self.staged_tree.expand_all_dirs();
    }

    fn pull_request_preview_text(&self) -> String {
        let Some(pr) = self.pull_requests.get(self.selected_pr) else {
            return format!(
                "No pull requests in '{}' filter.\n\n[o]/[Enter] open in browser | [c] checkout PR branch | [m]/[s]/[R] merge/squash/rebase | [n] create PR | [f] change filter",
                self.pull_request_filter_label()
            );
        };
        let draft_marker = if pr.is_draft { " (draft)" } else { "" };
        let status_block = match self.pr_status_for {
            Some(number) if number == pr.number => self
                .pr_status_summary
                .as_ref()
                .map(format_pr_status_summary)
                .unwrap_or_else(|| "PR status summary unavailable".to_string()),
            _ => "Loading PR status summary...".to_string(),
        };
        format!(
            "PR #{}{} [{}]\n{}\n\nAuthor: {}\nBranch: {} -> {}\nURL: {}\n{}\n\n[o]/[Enter] open in browser | [c] checkout PR branch | [m]/[s]/[R] merge/squash/rebase | [n] create PR | [f] change filter",
            pr.number,
            draft_marker,
            pr.state,
            pr.title,
            pr.author,
            pr.head_ref_name,
            pr.base_ref_name,
            pr.url,
            status_block
        )
    }

    fn refresh_selected_pr_status_summary(&mut self) {
        let Some(pr) = self.pull_requests.get(self.selected_pr) else {
            self.clear_pr_status_summary();
            return;
        };
        self.request_pr_status_refresh(pr.number);
    }

    fn clear_pr_status_summary(&mut self) {
        self.pr_status_for = None;
        self.pr_status_summary = None;
        self.pr_status_pending_for = None;
        self.pr_status_refresh_deadline = None;
        self.pr_status_inflight = None;
    }

    fn schedule_selected_pr_status_refresh(&mut self) {
        let Some(pr) = self.pull_requests.get(self.selected_pr) else {
            self.clear_pr_status_summary();
            return;
        };
        self.pr_status_pending_for = Some(pr.number);
        self.pr_status_refresh_deadline = Some(Instant::now() + Duration::from_millis(220));
    }

    fn maybe_refresh_pr_status_summary(&mut self) {
        if self.screen != Screen::PullRequestView || self.in_input_mode() || self.help_visible {
            return;
        }
        let Some(deadline) = self.pr_status_refresh_deadline else {
            return;
        };
        if Instant::now() < deadline {
            return;
        }
        let Some(expected_pr) = self.pr_status_pending_for else {
            self.pr_status_refresh_deadline = None;
            return;
        };
        let current_pr = self.pull_requests.get(self.selected_pr).map(|pr| pr.number);
        if current_pr != Some(expected_pr) {
            self.schedule_selected_pr_status_refresh();
            return;
        }
        self.refresh_selected_pr_status_summary();
    }

    fn process_async_events(&mut self) {
        while let Ok(event) = self.async_rx.try_recv() {
            match event {
                AppAsyncEvent::JobState { key, state } => {
                    self.async_job_lifecycle.insert(key, state);
                }
                AppAsyncEvent::JobCancelled { key } => {
                    self.async_cancelled_jobs = self.async_cancelled_jobs.saturating_add(1);
                    self.async_job_lifecycle.insert(key, AsyncJobLifecycle::Idle);
                }
                AppAsyncEvent::SnapshotReady {
                    request_id,
                    snapshot,
                } => {
                    if self.snapshot_inflight != Some(request_id) {
                        continue;
                    }
                    self.snapshot_inflight = None;
                    match snapshot {
                        Ok(snapshot) => {
                            self.snapshot = snapshot;
                            self.refresh_last_fetch_from_git_metadata();
                            self.ensure_selection_bounds();
                            self.invalidate_repo_preview_cache();
                            self.schedule_repo_preview_refresh();
                            if let Some(message) = self.snapshot_completion_message.take() {
                                self.status_message = message;
                            }
                        }
                        Err(err) => {
                            self.snapshot_completion_message = None;
                            self.set_async_error_status("Status refresh", &err);
                        }
                    }
                }
                AppAsyncEvent::RepoPreviewReady {
                    request_id,
                    key,
                    preview,
                } => {
                    if self.repo_preview_inflight != Some(request_id) {
                        continue;
                    }
                    self.repo_preview_inflight = None;
                    self.repo_preview_pending_key = None;
                    if self.current_repo_preview_key().as_deref() != Some(key.as_str()) {
                        continue;
                    }
                    self.repo_preview_key = Some(key);
                    self.repo_preview_text = preview;
                }
                AppAsyncEvent::HistoryDetailsReady {
                    request_id,
                    commit_hash,
                    short_hash,
                    details,
                } => {
                    if self.history_details_inflight != Some(request_id) {
                        continue;
                    }
                    self.history_details_inflight = None;
                    if self.history_entries.get(self.selected_history).map(|entry| entry.hash.as_str())
                        != Some(commit_hash.as_str())
                    {
                        continue;
                    }
                    match details {
                        Ok(details) => {
                            self.history_details = Some(details);
                            self.history_details_for = Some(commit_hash);
                            self.history_file_tree_selected = 0;
                            self.clear_history_file_history();
                            self.expand_all_history_dirs();
                            self.status_message = format!("Loaded commit details {}", short_hash);
                        }
                        Err(err) => {
                            self.set_async_error_status("Commit details", &err);
                        }
                    }
                }
                AppAsyncEvent::HistoryFileHistoryReady {
                    request_id,
                    commit_hash,
                    path,
                    entries,
                } => {
                    if self.history_file_history_inflight != Some(request_id) {
                        continue;
                    }
                    self.history_file_history_inflight = None;
                    if self.history_entries.get(self.selected_history).map(|entry| entry.hash.as_str())
                        != Some(commit_hash.as_str())
                    {
                        continue;
                    }
                    if self.history_file_history_for_path.as_deref() != Some(path.as_str()) {
                        continue;
                    }
                    match entries {
                        Ok(entries) => {
                            self.history_file_history_entries = entries;
                            self.history_file_history_selected = 0;
                            self.history_focus = HistoryFocusPane::FileHistory;
                            self.status_message = format!("Loaded history for {}", path);
                        }
                        Err(err) => self.set_async_error_status("File history", &err),
                    }
                }
                AppAsyncEvent::StashDetailsReady {
                    request_id,
                    reference,
                    details,
                } => {
                    if self.stash_details_inflight != Some(request_id) {
                        continue;
                    }
                    self.stash_details_inflight = None;
                    if self.stash_entries.get(self.selected_stash).map(|entry| entry.reference.as_str())
                        != Some(reference.as_str())
                    {
                        continue;
                    }
                    match details {
                        Ok(details) => {
                            self.stash_details = truncate_lines(details, 220);
                            self.stash_details_for = Some(reference.clone());
                            self.status_message = format!("Loaded stash details {}", reference);
                        }
                        Err(err) => {
                            self.set_async_error_status("Stash details", &err);
                        }
                    }
                }
                AppAsyncEvent::HistoryEntriesReady { request_id, entries } => {
                    if self.history_entries_inflight != Some(request_id) {
                        continue;
                    }
                    self.history_entries_inflight = None;
                    match entries {
                        Ok(entries) => {
                            self.history_entries = entries;
                            self.selected_history = min(
                                self.selected_history,
                                self.history_entries.len().saturating_sub(1),
                            );
                            self.clear_history_details();
                            if self.history_details_visible {
                                let _ = self.load_selected_commit_details();
                            }
                        }
                        Err(err) => {
                            self.set_async_error_status("History refresh", &err);
                        }
                    }
                }
                AppAsyncEvent::StashEntriesReady { request_id, entries } => {
                    if self.stash_entries_inflight != Some(request_id) {
                        continue;
                    }
                    self.stash_entries_inflight = None;
                    match entries {
                        Ok(entries) => {
                            self.stash_entries = entries;
                            self.selected_stash = min(
                                self.selected_stash,
                                self.stash_entries.len().saturating_sub(1),
                            );
                            self.clear_stash_details();
                        }
                        Err(err) => {
                            self.set_async_error_status("Stash refresh", &err);
                        }
                    }
                }
                AppAsyncEvent::PullRequestsReady {
                    request_id,
                    filter,
                    entries,
                } => {
                    if self.pull_requests_inflight != Some(request_id) {
                        continue;
                    }
                    self.pull_requests_inflight = None;
                    if self.pr_filter != filter {
                        continue;
                    }
                    match entries {
                        Ok(entries) => {
                            self.clear_pr_status_summary();
                            self.pull_requests = entries;
                            self.selected_pr = min(
                                self.selected_pr,
                                self.pull_requests.len().saturating_sub(1),
                            );
                            self.schedule_selected_pr_status_refresh();
                        }
                        Err(err) => {
                            self.status_message = format_gh_error_for_status(&anyhow!(err));
                        }
                    }
                }
                AppAsyncEvent::PullRequestStatusReady {
                    request_id,
                    pr_number,
                    summary,
                } => {
                    if self.pr_status_inflight != Some(request_id) {
                        continue;
                    }
                    self.pr_status_inflight = None;
                    if self.pull_requests.get(self.selected_pr).map(|pr| pr.number)
                        != Some(pr_number)
                    {
                        continue;
                    }
                    self.pr_status_for = Some(pr_number);
                    self.pr_status_summary = summary.ok();
                    self.pr_status_pending_for = None;
                    self.pr_status_refresh_deadline = None;
                }
                AppAsyncEvent::TrackingSummaryReady { request_id, summary } => {
                    if self.tracking_inflight != Some(request_id) {
                        continue;
                    }
                    self.tracking_inflight = None;
                    match summary {
                        Ok(summary) => self.tracking_summary = Some(summary),
                        Err(err) => {
                            self.tracking_summary = None;
                            self.set_async_error_status("Tracking refresh", &err);
                        }
                    }
                }
                AppAsyncEvent::BranchEntriesReady { request_id, entries } => {
                    if self.branch_entries_inflight != Some(request_id) {
                        continue;
                    }
                    self.branch_entries_inflight = None;
                    match entries {
                        Ok(entries) => {
                            self.branch_entries = entries;
                            self.selected_branch = self
                                .branch_entries
                                .iter()
                                .position(|entry| entry.is_current)
                                .unwrap_or(0);
                        }
                        Err(err) => self.set_async_error_status("Branch refresh", &err),
                    }
                }
                AppAsyncEvent::RemoteBranchEntriesReady { request_id, entries } => {
                    if self.remote_branch_entries_inflight != Some(request_id) {
                        continue;
                    }
                    self.remote_branch_entries_inflight = None;
                    match entries {
                        Ok(entries) => {
                            self.remote_branch_entries = entries;
                            self.selected_remote_branch = min(
                                self.selected_remote_branch,
                                self.remote_branch_entries.len().saturating_sub(1),
                            );
                        }
                        Err(err) => self.set_async_error_status("Remote branch refresh", &err),
                    }
                }
                AppAsyncEvent::FullscreenDiffReady {
                    request_id,
                    key,
                    title,
                    diff,
                } => {
                    if self.fullscreen_diff_inflight != Some(request_id) {
                        continue;
                    }
                    self.fullscreen_diff_inflight = None;
                    self.fullscreen_diff_pending_key = None;
                    if self.fullscreen_diff.as_ref().map(|state| state.key.as_str()) != Some(key.as_str()) {
                        continue;
                    }
                    match diff {
                        Ok(diff) => {
                            self.set_fullscreen_diff_content(&title, &key, &diff);
                            self.status_message = format!("Opened diff viewer: {}", title);
                        }
                        Err(err) => {
                            self.set_fullscreen_diff_content(
                                &title,
                                &key,
                                &format!("Unable to load diff:\n{err}"),
                            );
                            self.set_async_error_status("Diff viewer", &err);
                        }
                    }
                }
                AppAsyncEvent::WriteOpFinished { op, result } => {
                    self.write_inflight = false;
                    match (op, result) {
                        (AsyncWriteOp::StageFile { path }, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some(format!("Staged {}", path)));
                        }
                        (AsyncWriteOp::UnstageFile { path }, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some(format!("Unstaged {}", path)));
                        }
                        (AsyncWriteOp::StageAll { total }, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some(format!(
                                "Staged all unstaged files ({total})"
                            )));
                        }
                        (AsyncWriteOp::UnstageAll { total }, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some(format!(
                                "Unstaged all staged files ({total})"
                            )));
                        }
                        (AsyncWriteOp::Fetch, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some("Fetched remotes".to_string()));
                            if self.screen == Screen::TrackingStatusView {
                                let _ = self.refresh_tracking_status_summary();
                            }
                        }
                        (AsyncWriteOp::Pull, Ok(())) => {
                            let _ =
                                self.request_snapshot_refresh(Some("Pulled latest changes".to_string()));
                            if self.screen == Screen::TrackingStatusView {
                                let _ = self.refresh_tracking_status_summary();
                            }
                        }
                        (AsyncWriteOp::Push, Ok(())) => {
                            let _ =
                                self.request_snapshot_refresh(Some("Pushed current branch".to_string()));
                            if self.screen == Screen::TrackingStatusView {
                                let _ = self.refresh_tracking_status_summary();
                            }
                        }
                        (AsyncWriteOp::CherryPickContinue, Ok(())) => {
                            let _ = self
                                .request_snapshot_refresh(Some("Cherry-pick continued".to_string()));
                        }
                        (AsyncWriteOp::CherryPickAbort, Ok(())) => {
                            let _ =
                                self.request_snapshot_refresh(Some("Cherry-pick aborted".to_string()));
                        }
                        (
                            AsyncWriteOp::DiscardFile {
                                path,
                                is_untracked,
                            },
                            Ok(()),
                        ) => {
                            let message = if is_untracked {
                                format!("Removed untracked {}", path)
                            } else {
                                format!("Discarded unstaged changes in {}", path)
                            };
                            let _ = self.request_snapshot_refresh(Some(message));
                        }
                        (AsyncWriteOp::SwitchBranch { branch_name }, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some(format!(
                                "Switched to branch {}",
                                branch_name
                            )));
                            let _ = self.refresh_branch_entries();
                            self.return_to_repo_view();
                        }
                        (AsyncWriteOp::CheckoutRemoteBranch { branch_name }, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some(format!(
                                "Checked out remote branch {}",
                                branch_name
                            )));
                            let _ = self.refresh_remote_branch_entries();
                            self.return_to_repo_view();
                        }
                        (AsyncWriteOp::CreateBranch { branch_name }, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some(format!(
                                "Created and switched to branch {}",
                                branch_name
                            )));
                            let _ = self.refresh_branch_entries();
                            self.return_to_repo_view();
                        }
                        (AsyncWriteOp::CheckoutPullRequest { number }, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some(format!(
                                "Checked out PR #{}",
                                number
                            )));
                        }
                        (AsyncWriteOp::MergePullRequest { number, method }, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some(format!(
                                "Merged PR #{} with {}",
                                number,
                                pull_request_merge_method_label(method)
                            )));
                            let _ = self.refresh_pull_requests();
                        }
                        (AsyncWriteOp::CreatePullRequest, Ok(())) => {
                            let _ = self.refresh_pull_requests();
                            self.status_message = "Created pull request".to_string();
                        }
                        (AsyncWriteOp::Commit, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some("Commit created".to_string()));
                        }
                        (AsyncWriteOp::StashPush, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some("Stashed current changes".to_string()));
                            let _ = self.refresh_stash_entries();
                            self.clear_stash_details();
                        }
                        (AsyncWriteOp::StashApply { reference }, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some(format!("Applied {}", reference)));
                        }
                        (AsyncWriteOp::StashPop { reference }, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some(format!("Popped {}", reference)));
                            let _ = self.refresh_stash_entries();
                            self.clear_stash_details();
                        }
                        (AsyncWriteOp::StashDrop { reference }, Ok(())) => {
                            let _ = self.refresh_stash_entries();
                            self.clear_stash_details();
                            self.status_message = format!("Dropped {}", reference);
                        }
                        (AsyncWriteOp::CheckoutDetached { short_hash }, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some(format!(
                                "Checked out {} (detached HEAD)",
                                short_hash
                            )));
                            self.clear_history_details();
                        }
                        (AsyncWriteOp::CherryPickCommit { short_hash }, Ok(())) => {
                            let _ = self.request_snapshot_refresh(Some(format!(
                                "Cherry-picked {}",
                                short_hash
                            )));
                            let _ = self.refresh_history_entries();
                        }
                        (AsyncWriteOp::OpenPrInBrowser { number }, Ok(())) => {
                            self.status_message = format!("Opened PR #{} in browser", number);
                        }
                        (AsyncWriteOp::Commit, Err(err)) => {
                            self.status_message = err;
                        }
                        (AsyncWriteOp::StageFile { .. }, Err(err))
                        | (AsyncWriteOp::UnstageFile { .. }, Err(err))
                        | (AsyncWriteOp::StageAll { .. }, Err(err))
                        | (AsyncWriteOp::UnstageAll { .. }, Err(err))
                        | (AsyncWriteOp::Fetch, Err(err))
                        | (AsyncWriteOp::Pull, Err(err))
                        | (AsyncWriteOp::Push, Err(err))
                        | (AsyncWriteOp::CherryPickContinue, Err(err))
                        | (AsyncWriteOp::CherryPickAbort, Err(err))
                        | (AsyncWriteOp::DiscardFile { .. }, Err(err))
                        | (AsyncWriteOp::SwitchBranch { .. }, Err(err))
                        | (AsyncWriteOp::CheckoutRemoteBranch { .. }, Err(err))
                        | (AsyncWriteOp::CreateBranch { .. }, Err(err))
                        | (AsyncWriteOp::StashPush, Err(err))
                        | (AsyncWriteOp::StashApply { .. }, Err(err))
                        | (AsyncWriteOp::StashPop { .. }, Err(err))
                        | (AsyncWriteOp::StashDrop { .. }, Err(err))
                        | (AsyncWriteOp::CheckoutDetached { .. }, Err(err))
                        | (AsyncWriteOp::CherryPickCommit { .. }, Err(err)) => {
                            self.status_message = format!("Action failed: {err}");
                        }
                        (AsyncWriteOp::OpenPrInBrowser { .. }, Err(err)) => {
                            self.status_message = format_gh_error_for_status(&anyhow!(err));
                        }
                        (AsyncWriteOp::CheckoutPullRequest { .. }, Err(err))
                        | (AsyncWriteOp::MergePullRequest { .. }, Err(err))
                        | (AsyncWriteOp::CreatePullRequest, Err(err)) => {
                            self.status_message = format_gh_error_for_status(&anyhow!(err));
                        }
                    }
                }
            }
        }
    }

    fn set_async_running_status(&mut self, label: &str) {
        self.status_message = format!("{label}...");
    }

    fn set_async_error_status(&mut self, label: &str, err: &str) {
        self.last_async_error = Some(err.to_string());
        self.status_message = format!("{label} failed: {err}");
    }

    fn queue_async_task<F>(&mut self, key: &'static str, job: F) -> Result<()>
    where
        F: FnOnce() + Send + 'static,
    {
        self.async_job_lifecycle.insert(key, AsyncJobLifecycle::Queued);
        let tx = self.async_tx.clone();
        self.async_scheduler
            .submit(move || {
                let _ = tx.send(AppAsyncEvent::JobState {
                    key,
                    state: AsyncJobLifecycle::Running,
                });
                job();
                let _ = tx.send(AppAsyncEvent::JobState {
                    key,
                    state: AsyncJobLifecycle::Idle,
                });
            })
            .map_err(|err| {
                self.async_job_lifecycle.insert(key, AsyncJobLifecycle::Idle);
                self.async_dispatch_failures = self.async_dispatch_failures.saturating_add(1);
                self.last_async_error = Some(err.clone());
                anyhow!(err)
            })
    }

    fn queue_cancellable_async_task<F>(
        &mut self,
        key: &'static str,
        token: u64,
        job: F,
    ) -> Result<()>
    where
        F: FnOnce() + Send + 'static,
    {
        self.async_job_lifecycle.insert(key, AsyncJobLifecycle::Queued);
        let tx = self.async_tx.clone();
        let tx_cancel = self.async_tx.clone();
        self.async_scheduler
            .submit_cancellable(
                key,
                token,
                move || {
                    let _ = tx_cancel.send(AppAsyncEvent::JobCancelled { key });
                    let _ = tx_cancel.send(AppAsyncEvent::JobState {
                        key,
                        state: AsyncJobLifecycle::Idle,
                    });
                },
                move || {
                    let _ = tx.send(AppAsyncEvent::JobState {
                        key,
                        state: AsyncJobLifecycle::Running,
                    });
                    job();
                    let _ = tx.send(AppAsyncEvent::JobState {
                        key,
                        state: AsyncJobLifecycle::Idle,
                    });
                },
            )
            .map_err(|err| {
                self.async_job_lifecycle.insert(key, AsyncJobLifecycle::Idle);
                self.async_dispatch_failures = self.async_dispatch_failures.saturating_add(1);
                self.last_async_error = Some(err.clone());
                anyhow!(err)
            })
    }

    fn request_write_op<F>(&mut self, running_label: &str, op: AsyncWriteOp, job: F)
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        if self.write_inflight {
            self.status_message = "Another action is still running".to_string();
            return;
        }
        self.write_inflight = true;
        self.set_async_running_status(running_label);
        let tx = self.async_tx.clone();
        if let Err(err) = self.queue_async_task(JOB_WRITE, move || {
            let result = job().map_err(|err| err.to_string());
            let _ = tx.send(AppAsyncEvent::WriteOpFinished { op, result });
        }) {
            self.write_inflight = false;
            self.status_message = format!("Unable to queue action: {err}");
        }
    }

    fn request_snapshot_refresh(&mut self, completion_message: Option<String>) -> Result<()> {
        let root = self.current_repo_root()?.to_path_buf();
        self.snapshot_request_seq = self.snapshot_request_seq.saturating_add(1);
        let request_id = self.snapshot_request_seq;
        self.snapshot_inflight = Some(request_id);
        self.snapshot_completion_message = completion_message;

        let tx = self.async_tx.clone();
        if let Err(err) = self.queue_cancellable_async_task(JOB_SNAPSHOT, request_id, move || {
            let snapshot = git::snapshot(&root).map_err(|err| err.to_string());
            let _ = tx.send(AppAsyncEvent::SnapshotReady {
                request_id,
                snapshot,
            });
        }) {
            self.snapshot_inflight = None;
            self.snapshot_completion_message = None;
            return Err(err);
        }

        Ok(())
    }

    fn request_pull_requests_refresh(&mut self) -> Result<()> {
        let root = self.current_repo_root()?.to_path_buf();
        let filter = self.pr_filter;
        self.pull_requests_request_seq = self.pull_requests_request_seq.saturating_add(1);
        let request_id = self.pull_requests_request_seq;
        self.pull_requests_inflight = Some(request_id);
        let tx = self.async_tx.clone();
        if let Err(err) =
            self.queue_cancellable_async_task(JOB_PULL_REQUESTS, request_id, move || {
                let entries = git::pull_requests(&root, filter).map_err(|err| err.to_string());
                let _ = tx.send(AppAsyncEvent::PullRequestsReady {
                    request_id,
                    filter,
                    entries,
                });
            })
        {
            self.pull_requests_inflight = None;
            return Err(err);
        }
        Ok(())
    }

    fn request_pr_status_refresh(&mut self, pr_number: u64) {
        let Some(root) = self.repo_root.clone() else {
            return;
        };
        self.pr_status_request_seq = self.pr_status_request_seq.saturating_add(1);
        let request_id = self.pr_status_request_seq;
        self.pr_status_inflight = Some(request_id);
        let tx = self.async_tx.clone();
        if let Err(err) =
            self.queue_cancellable_async_task(JOB_PR_STATUS, request_id, move || {
                let summary = git::pull_request_status_summary(&root, pr_number)
                    .map_err(|err| err.to_string());
                let _ = tx.send(AppAsyncEvent::PullRequestStatusReady {
                    request_id,
                    pr_number,
                    summary,
                });
            })
        {
            self.pr_status_inflight = None;
            self.set_async_error_status("PR status refresh", &err.to_string());
        }
    }

    fn request_tracking_summary_refresh(&mut self) -> Result<()> {
        let root = self.current_repo_root()?.to_path_buf();
        self.tracking_request_seq = self.tracking_request_seq.saturating_add(1);
        let request_id = self.tracking_request_seq;
        self.tracking_inflight = Some(request_id);
        let tx = self.async_tx.clone();
        if let Err(err) = self.queue_cancellable_async_task(JOB_TRACKING, request_id, move || {
            let summary = git::tracking_commit_summary(&root, 30).map_err(|err| err.to_string());
            let _ = tx.send(AppAsyncEvent::TrackingSummaryReady { request_id, summary });
        }) {
            self.tracking_inflight = None;
            return Err(err);
        }
        Ok(())
    }

    fn request_branch_entries_refresh(&mut self) -> Result<()> {
        let root = self.current_repo_root()?.to_path_buf();
        self.branch_entries_request_seq = self.branch_entries_request_seq.saturating_add(1);
        let request_id = self.branch_entries_request_seq;
        self.branch_entries_inflight = Some(request_id);
        let tx = self.async_tx.clone();
        if let Err(err) =
            self.queue_cancellable_async_task(JOB_BRANCH_ENTRIES, request_id, move || {
                let entries = git::list_local_branches(&root).map_err(|err| err.to_string());
                let _ = tx.send(AppAsyncEvent::BranchEntriesReady { request_id, entries });
            })
        {
            self.branch_entries_inflight = None;
            return Err(err);
        }
        Ok(())
    }

    fn request_remote_branch_entries_refresh(&mut self) -> Result<()> {
        let root = self.current_repo_root()?.to_path_buf();
        self.remote_branch_entries_request_seq =
            self.remote_branch_entries_request_seq.saturating_add(1);
        let request_id = self.remote_branch_entries_request_seq;
        self.remote_branch_entries_inflight = Some(request_id);
        let tx = self.async_tx.clone();
        if let Err(err) = self.queue_cancellable_async_task(
            JOB_REMOTE_BRANCH_ENTRIES,
            request_id,
            move || {
                let entries = git::list_remote_branches(&root).map_err(|err| err.to_string());
                let _ = tx.send(AppAsyncEvent::RemoteBranchEntriesReady { request_id, entries });
            },
        ) {
            self.remote_branch_entries_inflight = None;
            return Err(err);
        }
        Ok(())
    }

    pub fn is_fullscreen_diff_visible(&self) -> bool {
        self.fullscreen_diff.is_some()
    }

    pub fn fullscreen_diff_title(&self) -> Option<&str> {
        self.fullscreen_diff.as_ref().map(|state| state.title.as_str())
    }

    pub fn fullscreen_diff_scroll_position(&self) -> Option<(usize, usize)> {
        self.fullscreen_diff
            .as_ref()
            .map(|state| (state.scroll_y, state.scroll_x))
    }

    pub fn fullscreen_diff_visible_lines(&self, width: usize, height: usize) -> Vec<String> {
        let Some(state) = self.fullscreen_diff.as_ref() else {
            return Vec::new();
        };
        if state.loading {
            return vec!["Loading diff...".to_string()];
        }
        let body_width = width.max(1);
        state
            .lines
            .iter()
            .skip(state.scroll_y)
            .take(height.max(1))
            .map(|line| {
                let chars = line.chars().collect::<Vec<_>>();
                let start = state.scroll_x.min(chars.len());
                chars
                    .iter()
                    .skip(start)
                    .take(body_width)
                    .collect::<String>()
            })
            .collect()
    }

    pub fn close_fullscreen_diff(&mut self) {
        self.fullscreen_diff = None;
        self.fullscreen_diff_inflight = None;
        self.fullscreen_diff_pending_key = None;
    }

    pub fn open_status_fullscreen_diff(&mut self) -> Result<()> {
        if self.screen != Screen::RepoView {
            return Ok(());
        }
        let Some(root) = self.repo_root.clone() else {
            return Ok(());
        };
        let (path, staged) = match self.focus {
            FocusPane::Unstaged => {
                let row = self
                    .current_unstaged_row()
                    .ok_or_else(|| anyhow!("No unstaged path selected"))?;
                if row.kind != TreeRowKind::File {
                    self.status_message = "Select a file to open diff preview".to_string();
                    return Ok(());
                }
                (row.path.clone(), false)
            }
            FocusPane::Staged => {
                let row = self
                    .current_staged_row()
                    .ok_or_else(|| anyhow!("No staged path selected"))?;
                if row.kind != TreeRowKind::File {
                    self.status_message = "Select a file to open diff preview".to_string();
                    return Ok(());
                }
                (row.path.clone(), true)
            }
        };
        let title = if staged {
            format!("Staged Diff: {path}")
        } else {
            format!("Unstaged Diff: {path}")
        };
        let key = format!("status:{}:{path}", if staged { "staged" } else { "unstaged" });
        self.request_fullscreen_diff(title, key, move || git::diff_for_file(&root, &path, staged))?;
        Ok(())
    }

    pub fn status_tree_focus_right(&mut self) -> Result<()> {
        if self.screen != Screen::RepoView {
            return Ok(());
        }
        let expanded = match self.focus {
            FocusPane::Unstaged => self.unstaged_tree.expand_selected_dir(self.selected_unstaged),
            FocusPane::Staged => self.staged_tree.expand_selected_dir(self.selected_staged),
        };
        if expanded {
            self.status_message = "Expanded folder".to_string();
            return Ok(());
        }
        self.open_status_fullscreen_diff()
    }

    pub fn status_tree_focus_left(&mut self) {
        if self.screen != Screen::RepoView {
            return;
        }
        let collapsed = match self.focus {
            FocusPane::Unstaged => {
                let selected_path = self
                    .unstaged_tree
                    .row_path(self.selected_unstaged)
                    .map(|path| path.to_string());
                let collapsed = self.unstaged_tree.collapse_selected_dir(self.selected_unstaged);
                if collapsed {
                    if let Some(path) = selected_path {
                        if let Some(new_idx) = self.unstaged_tree.find_row_index_by_path(&path) {
                            self.selected_unstaged = new_idx;
                        }
                    }
                }
                collapsed
            }
            FocusPane::Staged => {
                let selected_path = self
                    .staged_tree
                    .row_path(self.selected_staged)
                    .map(|path| path.to_string());
                let collapsed = self.staged_tree.collapse_selected_dir(self.selected_staged);
                if collapsed {
                    if let Some(path) = selected_path {
                        if let Some(new_idx) = self.staged_tree.find_row_index_by_path(&path) {
                            self.selected_staged = new_idx;
                        }
                    }
                }
                collapsed
            }
        };
        if collapsed {
            self.status_message = "Collapsed folder".to_string();
        }
    }

    pub fn unstaged_tree_rows(&self) -> &[TreeRow] {
        self.unstaged_tree.rows()
    }

    pub fn staged_tree_rows(&self) -> &[TreeRow] {
        self.staged_tree.rows()
    }

    pub fn fullscreen_diff_move_down(&mut self) {
        let Some(state) = self.fullscreen_diff.as_mut() else {
            return;
        };
        if state.lines.is_empty() {
            return;
        }
        state.scroll_y = min(state.scroll_y.saturating_add(1), state.lines.len().saturating_sub(1));
    }

    pub fn fullscreen_diff_move_up(&mut self) {
        if let Some(state) = self.fullscreen_diff.as_mut() {
            state.scroll_y = state.scroll_y.saturating_sub(1);
        }
    }

    pub fn fullscreen_diff_page_down(&mut self, page_height: usize) {
        let Some(state) = self.fullscreen_diff.as_mut() else {
            return;
        };
        if state.lines.is_empty() {
            return;
        }
        state.scroll_y = min(
            state.scroll_y.saturating_add(page_height.max(1)),
            state.lines.len().saturating_sub(1),
        );
    }

    pub fn fullscreen_diff_page_up(&mut self, page_height: usize) {
        if let Some(state) = self.fullscreen_diff.as_mut() {
            state.scroll_y = state.scroll_y.saturating_sub(page_height.max(1));
        }
    }

    pub fn fullscreen_diff_home(&mut self) {
        if let Some(state) = self.fullscreen_diff.as_mut() {
            state.scroll_y = 0;
        }
    }

    pub fn fullscreen_diff_end(&mut self) {
        let Some(state) = self.fullscreen_diff.as_mut() else {
            return;
        };
        if !state.lines.is_empty() {
            state.scroll_y = state.lines.len().saturating_sub(1);
        }
    }

    pub fn fullscreen_diff_scroll_left(&mut self) {
        if let Some(state) = self.fullscreen_diff.as_mut() {
            state.scroll_x = state.scroll_x.saturating_sub(4);
        }
    }

    pub fn fullscreen_diff_scroll_right(&mut self) {
        if let Some(state) = self.fullscreen_diff.as_mut() {
            state.scroll_x = state.scroll_x.saturating_add(4);
        }
    }

    pub fn fullscreen_diff_next_hunk(&mut self) {
        let Some(state) = self.fullscreen_diff.as_mut() else {
            return;
        };
        let target = state
            .hunk_lines
            .iter()
            .copied()
            .find(|line| *line > state.scroll_y);
        if let Some(target) = target {
            state.scroll_y = target;
        }
    }

    pub fn fullscreen_diff_prev_hunk(&mut self) {
        let Some(state) = self.fullscreen_diff.as_mut() else {
            return;
        };
        let target = state
            .hunk_lines
            .iter()
            .copied()
            .rev()
            .find(|line| *line < state.scroll_y);
        if let Some(target) = target {
            state.scroll_y = target;
        }
    }

    fn request_fullscreen_diff<F>(&mut self, title: String, key: String, load: F) -> Result<()>
    where
        F: FnOnce() -> Result<String> + Send + 'static,
    {
        self.fullscreen_diff = Some(FullscreenDiffState {
            key: key.clone(),
            title: title.clone(),
            lines: vec!["Loading diff...".to_string()],
            hunk_lines: Vec::new(),
            scroll_y: 0,
            scroll_x: 0,
            loading: true,
        });
        self.fullscreen_diff_request_seq = self.fullscreen_diff_request_seq.saturating_add(1);
        let request_id = self.fullscreen_diff_request_seq;
        self.fullscreen_diff_inflight = Some(request_id);
        self.fullscreen_diff_pending_key = Some(key.clone());
        let tx = self.async_tx.clone();
        if let Err(err) = self.queue_cancellable_async_task(JOB_FULLSCREEN_DIFF, request_id, move || {
            let diff = load().map_err(|err| err.to_string());
            let _ = tx.send(AppAsyncEvent::FullscreenDiffReady {
                request_id,
                key,
                title,
                diff,
            });
        }) {
            self.fullscreen_diff_inflight = None;
            self.fullscreen_diff_pending_key = None;
            self.close_fullscreen_diff();
            return Err(err);
        }
        Ok(())
    }

    fn set_fullscreen_diff_content(&mut self, title: &str, key: &str, diff: &str) {
        let lines = if diff.is_empty() {
            vec!["(No diff output)".to_string()]
        } else {
            diff.lines().map(|line| line.to_string()).collect::<Vec<_>>()
        };
        let hunk_lines = lines
            .iter()
            .enumerate()
            .filter_map(|(idx, line)| line.starts_with("@@").then_some(idx))
            .collect::<Vec<_>>();
        self.fullscreen_diff = Some(FullscreenDiffState {
            key: key.to_string(),
            title: title.to_string(),
            lines,
            hunk_lines,
            scroll_y: 0,
            scroll_x: 0,
            loading: false,
        });
    }

    fn request_repo_preview_refresh(&mut self) {
        if self.screen != Screen::RepoView {
            return;
        }
        let key = self.current_repo_preview_key();
        if self.repo_preview_key == key {
            return;
        }
        let Some(key) = key else {
            self.repo_preview_key = None;
            self.repo_preview_text = self.empty_repo_preview_text();
            self.repo_preview_inflight = None;
            self.repo_preview_pending_key = None;
            return;
        };

        if self.repo_preview_pending_key.as_deref() == Some(key.as_str())
            && self.repo_preview_inflight.is_some()
        {
            return;
        }

        let Some(root) = self.repo_root.clone() else {
            self.repo_preview_text = self.empty_repo_preview_text();
            return;
        };

        let (path, staged, empty_text) = match self.focus {
            FocusPane::Unstaged => self.current_unstaged_row().map_or_else(
                || (String::new(), false, self.empty_repo_preview_text()),
                |row| {
                    if row.kind == TreeRowKind::File {
                        (row.path.clone(), false, self.empty_repo_preview_text())
                    } else {
                        (String::new(), false, "Select a file to preview diff.".to_string())
                    }
                },
            ),
            FocusPane::Staged => self.current_staged_row().map_or_else(
                || (String::new(), true, self.empty_repo_preview_text()),
                |row| {
                    if row.kind == TreeRowKind::File {
                        (row.path.clone(), true, self.empty_repo_preview_text())
                    } else {
                        (String::new(), true, "Select a file to preview diff.".to_string())
                    }
                },
            ),
        };
        if path.is_empty() {
            self.repo_preview_key = None;
            self.repo_preview_text = empty_text;
            return;
        }

        self.repo_preview_request_seq = self.repo_preview_request_seq.saturating_add(1);
        let request_id = self.repo_preview_request_seq;
        self.repo_preview_inflight = Some(request_id);
        self.repo_preview_pending_key = Some(key.clone());
        self.repo_preview_text = "Loading preview...".to_string();

        let tx = self.async_tx.clone();
        if let Err(err) =
            self.queue_cancellable_async_task(JOB_REPO_PREVIEW, request_id, move || {
                let preview = git::diff_for_file(&root, &path, staged)
                    .map(|diff| truncate_lines(diff, 90))
                    .unwrap_or(empty_text);
                let _ = tx.send(AppAsyncEvent::RepoPreviewReady {
                    request_id,
                    key,
                    preview,
                });
            })
        {
            self.repo_preview_inflight = None;
            self.repo_preview_pending_key = None;
            self.set_async_error_status("Preview refresh", &err.to_string());
        }
    }

    fn schedule_repo_preview_refresh(&mut self) {
        if self.screen != Screen::RepoView {
            self.repo_preview_refresh_deadline = None;
            self.repo_preview_inflight = None;
            self.repo_preview_pending_key = None;
            return;
        }
        let key = self.current_repo_preview_key();
        if self.repo_preview_key == key {
            self.repo_preview_refresh_deadline = None;
            return;
        }

        if key.is_none() {
            self.repo_preview_text = self.empty_repo_preview_text();
            self.repo_preview_refresh_deadline = None;
            self.repo_preview_inflight = None;
            self.repo_preview_pending_key = None;
            return;
        }

        self.repo_preview_text = "Loading preview...".to_string();
        self.repo_preview_refresh_deadline = Some(Instant::now() + Duration::from_millis(110));
    }

    fn maybe_refresh_repo_preview_cache(&mut self) {
        if self.screen != Screen::RepoView || self.in_input_mode() || self.help_visible {
            return;
        }
        let Some(deadline) = self.repo_preview_refresh_deadline else {
            return;
        };
        if Instant::now() < deadline {
            return;
        }
        self.repo_preview_refresh_deadline = None;
        self.request_repo_preview_refresh();
    }

    fn invalidate_repo_preview_cache(&mut self) {
        self.repo_preview_key = None;
        self.repo_preview_inflight = None;
        self.repo_preview_pending_key = None;
    }

    fn current_repo_preview_key(&self) -> Option<String> {
        match self.focus {
            FocusPane::Unstaged => self.current_unstaged_row().and_then(|row| {
                (row.kind == TreeRowKind::File)
                    .then_some(format!("unstaged:{}:{}", self.selected_unstaged, row.path))
            }),
            FocusPane::Staged => self
                .current_staged_row()
                .and_then(|row| {
                    (row.kind == TreeRowKind::File)
                        .then_some(format!("staged:{}:{}", self.selected_staged, row.path))
                }),
        }
    }

    fn empty_repo_preview_text(&self) -> String {
        match self.focus {
            FocusPane::Unstaged => "No unstaged diff output for selected file.".to_string(),
            FocusPane::Staged => "No staged diff output for selected file.".to_string(),
        }
    }

    fn tracking_status_preview_text(&self) -> String {
        let Some(summary) = &self.tracking_summary else {
            return "Tracking comparison unavailable.\nEnsure the current branch has an upstream."
                .to_string();
        };

        let outgoing = if summary.outgoing.is_empty() {
            "  (none)".to_string()
        } else {
            summary
                .outgoing
                .iter()
                .take(20)
                .map(|entry| format!("  {} {}", entry.short_hash, entry.summary))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let incoming = if summary.incoming.is_empty() {
            "  (none)".to_string()
        } else {
            summary
                .incoming
                .iter()
                .take(20)
                .map(|entry| format!("  {} {}", entry.short_hash, entry.summary))
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            "Upstream: {}\n\nOutgoing ({}):\n{}\n\nIncoming ({}):\n{}\n\n[f] fetch  [l] pull  [p] push",
            summary.upstream,
            summary.outgoing.len(),
            outgoing,
            summary.incoming.len(),
            incoming
        )
    }

    fn stash_preview_text(&self) -> String {
        let Some(entry) = self.stash_entries.get(self.selected_stash) else {
            return "No stash entries.\nPress [s] to stash current changes from this view.".to_string();
        };
        if self.stash_details_for.as_deref() == Some(entry.reference.as_str()) && !self.stash_details.is_empty() {
            return self.stash_details.clone();
        }
        format!(
            "{}\n{}\n\n[Enter] details | [a] apply | [p] pop | [d] drop | [s] stash current changes",
            entry.reference, entry.message
        )
    }

    fn clear_stash_details(&mut self) {
        self.stash_details.clear();
        self.stash_details_for = None;
        self.stash_details_inflight = None;
    }

    fn refresh_last_fetch_from_git_metadata(&mut self) {
        let Some(repo_root) = self.repo_root.as_ref() else {
            self.last_fetch_at = None;
            return;
        };
        let Some(fetch_head) = fetch_head_path(repo_root) else {
            self.last_fetch_at = None;
            return;
        };
        let modified = fs::metadata(fetch_head)
            .and_then(|meta| meta.modified())
            .ok();
        self.last_fetch_at = modified;
    }

    fn rebuild_repo_picker_labels(&mut self) {
        self.repo_picker_labels = self
            .registry
            .repos
            .iter()
            .map(|repo| infer_repo_picker_label(&repo.path))
            .collect();
    }
}

fn truncate_lines(input: String, max_lines: usize) -> String {
    input.lines().take(max_lines).collect::<Vec<_>>().join("\n")
}

fn line_start_before_or_at(chars: &[char], cursor: usize) -> usize {
    let upto = cursor.min(chars.len());
    for idx in (0..upto).rev() {
        if chars[idx] == '\n' {
            return idx + 1;
        }
    }
    0
}

fn line_end_from(chars: &[char], start: usize) -> usize {
    let mut idx = start.min(chars.len());
    while idx < chars.len() && chars[idx] != '\n' {
        idx += 1;
    }
    idx
}

fn pull_request_merge_method_label(method: GitPullRequestMergeMethod) -> &'static str {
    match method {
        GitPullRequestMergeMethod::Merge => "merge",
        GitPullRequestMergeMethod::Squash => "squash",
        GitPullRequestMergeMethod::Rebase => "rebase",
    }
}

fn unresolved_conflicts_status(conflicts: &[String]) -> String {
    const MAX_LISTED: usize = 3;
    let total = conflicts.len();
    let listed = conflicts
        .iter()
        .take(MAX_LISTED)
        .map(|path| display_path_for_ui(path))
        .collect::<Vec<_>>();

    if listed.is_empty() {
        return "Cannot commit: unresolved merge conflicts detected. Resolve conflicts and stage files, then retry.".to_string();
    }

    let suffix = if total > MAX_LISTED {
        format!(" and {} more", total - MAX_LISTED)
    } else {
        String::new()
    };

    format!(
        "Cannot commit: unresolved merge conflicts in {} file(s): {}{}. Resolve conflicts and stage files, then retry.",
        total,
        listed.join(", "),
        suffix
    )
}

fn format_pr_status_summary(summary: &GitPullRequestStatusSummary) -> String {
    let merge_state = summary
        .merge_state_status
        .clone()
        .unwrap_or_else(|| "UNKNOWN".to_string());
    let review = summary
        .review_decision
        .clone()
        .unwrap_or_else(|| "UNKNOWN".to_string());
    format!(
        "Mergeability: {}\nReview decision: {}\nChecks: {}/{} passing, {} failing, {} pending",
        merge_state,
        review,
        summary.checks_passing,
        summary.checks_total,
        summary.checks_failing,
        summary.checks_pending
    )
}

fn format_gh_error_for_status(err: &anyhow::Error) -> String {
    let message = err.to_string();
    let lower = message.to_ascii_lowercase();

    if lower.contains("gh")
        && (lower.contains("not logged")
            || lower.contains("authentication")
            || lower.contains("auth token")
            || lower.contains("401")
            || lower.contains("forbidden")
            || lower.contains("please run gh auth login"))
    {
        return "GitHub CLI auth required: run `gh auth login`, then retry.".to_string();
    }

    if lower.contains("gh")
        && (lower.contains("not a git repository")
            || lower.contains("could not resolve to a repository")
            || lower.contains("no git remotes found"))
    {
        return "GitHub CLI could not resolve this repo. Ensure it has a GitHub remote and you have access."
            .to_string();
    }

    if lower.contains("gh") {
        return format!("GitHub CLI error: {}", message);
    }

    message
}

fn completion_base_and_fragment(path: &Path, raw: &str) -> (PathBuf, String) {
    if let Some(root) = windows_drive_root_from_input(raw) {
        return (root, String::new());
    }

    if path.is_dir() || raw.ends_with('\\') || raw.ends_with('/') {
        return (path.to_path_buf(), String::new());
    }
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            let fragment = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            (parent.to_path_buf(), fragment)
        }
        _ => {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            (home, raw.to_string())
        }
    }
}

fn windows_drive_root_from_input(raw: &str) -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }

    let bytes = raw.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive = raw.chars().next()?;
        return Some(PathBuf::from(format!("{drive}:\\",)));
    }

    None
}

fn common_prefix(values: &[String]) -> String {
    let Some(first) = values.first() else {
        return String::new();
    };
    let mut prefix = first.clone();
    for value in values.iter().skip(1) {
        while !value.starts_with(&prefix) && !prefix.is_empty() {
            prefix.pop();
        }
        if prefix.is_empty() {
            break;
        }
    }
    prefix
}

fn display_path_for_ui(raw: &str) -> String {
    if !cfg!(windows) {
        return raw.to_string();
    }

    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{}", rest);
    }
    if let Some(rest) = raw.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    raw.to_string()
}

fn infer_repo_picker_label(raw_path: &str) -> String {
    let repo_root = Path::new(raw_path);
    if let Some(origin_name) = origin_repo_name(repo_root) {
        return origin_name;
    }

    let display_path = display_path_for_ui(raw_path);
    Path::new(&display_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string())
        .unwrap_or(display_path)
}

fn origin_repo_name(repo_root: &Path) -> Option<String> {
    let config_path = git_config_path(repo_root)?;
    let content = fs::read_to_string(config_path).ok()?;

    let mut in_origin_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_origin_section = trimmed == r#"[remote "origin"]"#;
            continue;
        }
        if !in_origin_section {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=')
            && key.trim() == "url"
        {
            return repo_name_from_remote_url(value.trim());
        }
    }
    None
}

fn git_config_path(repo_root: &Path) -> Option<PathBuf> {
    let git_meta = repo_root.join(".git");
    if git_meta.is_dir() {
        return Some(git_meta.join("config"));
    }
    if git_meta.is_file() {
        let pointer = fs::read_to_string(&git_meta).ok()?;
        for line in pointer.lines() {
            let trimmed = line.trim();
            if let Some(path_part) = trimmed.strip_prefix("gitdir:") {
                let gitdir = path_part.trim();
                if gitdir.is_empty() {
                    return None;
                }
                let gitdir_path = Path::new(gitdir);
                let resolved = if gitdir_path.is_absolute() {
                    gitdir_path.to_path_buf()
                } else {
                    repo_root.join(gitdir_path)
                };
                return Some(resolved.join("config"));
            }
        }
    }
    None
}

fn repo_name_from_remote_url(url: &str) -> Option<String> {
    let no_trailing_slash = url.trim().trim_end_matches('/');
    if no_trailing_slash.is_empty() {
        return None;
    }
    let no_git_suffix = no_trailing_slash
        .strip_suffix(".git")
        .unwrap_or(no_trailing_slash);
    no_git_suffix
        .rsplit(['/', ':'])
        .find(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
}

fn fetch_head_path(repo_root: &Path) -> Option<PathBuf> {
    let config = git_config_path(repo_root)?;
    let git_dir = config.parent()?;
    Some(git_dir.join("FETCH_HEAD"))
}
