mod branches;
mod history;
mod prs;
mod stash;

use std::{ffi::OsString, path::Path, process::Command, time::Instant};

use anyhow::{Context, Result, anyhow};
use log::{debug, error};

pub use branches::{
    BranchEntry, RemoteBranchEntry, checkout_remote_tracking_branch, create_and_switch_branch,
    list_local_branches, list_remote_branches, switch_branch,
};
pub use history::{
    CommitEntry, TrackingCommitSummary, cherry_pick, cherry_pick_abort, cherry_pick_continue,
    checkout_detached, commit_details, commit_history, tracking_commit_summary,
};
pub use prs::{
    PullRequestEntry, PullRequestFilter, PullRequestMergeMethod, PullRequestStatusSummary,
    checkout_pr, create_pull_request, merge_pull_request, open_pr_in_browser,
    pull_request_status_summary, pull_requests,
};
pub use stash::{
    StashEntry, list_stashes, stash_apply, stash_drop, stash_pop, stash_push, stash_show,
};

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone, Default)]
pub struct RepoSnapshot {
    pub branch: String,
    pub tracking: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub unstaged: Vec<FileEntry>,
    pub staged: Vec<FileEntry>,
}

pub fn snapshot(repo_root: &Path) -> Result<RepoSnapshot> {
    let raw = run_command(
        "git",
        ["status", "--porcelain=1", "--branch"],
        repo_root,
    )?;
    parse_status(&raw)
}

pub fn stage_file(repo_root: &Path, path: &str) -> Result<()> {
    run_command("git", ["add", "--", path], repo_root).map(|_| ())
}

pub fn stage_all(repo_root: &Path) -> Result<()> {
    run_command("git", ["add", "-A", "--", "."], repo_root).map(|_| ())
}

pub fn unstage_file(repo_root: &Path, path: &str) -> Result<()> {
    let restore_result = run_command("git", ["restore", "--staged", "--", path], repo_root);
    if restore_result.is_ok() {
        return Ok(());
    }
    run_command("git", ["reset", "HEAD", "--", path], repo_root).map(|_| ())
}

pub fn unstage_all(repo_root: &Path) -> Result<()> {
    let restore_result = run_command("git", ["restore", "--staged", "."], repo_root);
    if restore_result.is_ok() {
        return Ok(());
    }
    run_command("git", ["reset", "HEAD", "--", "."], repo_root).map(|_| ())
}

pub fn discard_file(repo_root: &Path, path: &str, is_untracked: bool) -> Result<()> {
    if is_untracked {
        return run_command("git", ["clean", "-f", "--", path], repo_root).map(|_| ());
    }

    let restore_result = run_command("git", ["restore", "--worktree", "--", path], repo_root);
    if restore_result.is_ok() {
        return Ok(());
    }
    run_command("git", ["checkout", "--", path], repo_root).map(|_| ())
}

pub fn commit(repo_root: &Path, message: &str) -> Result<()> {
    run_command("git", ["commit", "-m", message], repo_root).map(|_| ())
}

pub fn fetch(repo_root: &Path) -> Result<()> {
    run_command("git", ["fetch", "--all", "--prune"], repo_root).map(|_| ())
}

pub fn pull(repo_root: &Path) -> Result<()> {
    run_command("git", ["pull", "--ff-only"], repo_root).map(|_| ())
}

pub fn push(repo_root: &Path) -> Result<()> {
    let push_result = run_command("git", ["push"], repo_root);
    if push_result.is_ok() {
        return Ok(());
    }
    run_command("git", ["push", "-u", "origin", "HEAD"], repo_root).map(|_| ())
}

pub fn diff_for_file(repo_root: &Path, path: &str, staged: bool) -> Result<String> {
    if staged {
        run_command("git", ["diff", "--staged", "--", path], repo_root)
    } else {
        run_command("git", ["diff", "--", path], repo_root)
    }
}

fn run_command<I, S>(program: &str, args: I, cwd: &Path) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let args_vec: Vec<OsString> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .collect();
    let args_display = args_vec
        .iter()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let redacted_args_display = redact_sensitive_text(&args_display);
    debug!(
        "Running command in {}: {} {}",
        cwd.display(),
        program,
        redacted_args_display
    );

    let started = Instant::now();
    let output = Command::new(program)
        .args(&args_vec)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("Failed to execute {program}"))?;
    let elapsed_ms = started.elapsed().as_millis();

    if output.status.success() {
        debug!(
            "Command succeeded in {}ms (status={}): {} {}",
            elapsed_ms,
            output.status,
            program,
            redacted_args_display
        );
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let redacted_stderr = redact_sensitive_text(&stderr);
        error!(
            "Command failed in {}ms (status={}): {} {} | stderr: {}",
            elapsed_ms,
            output.status,
            program,
            redacted_args_display,
            redacted_stderr.trim()
        );
        Err(anyhow!(
            "{program} failed with status {}: {}",
            output.status,
            redacted_stderr
        ))
    }
}

fn redact_sensitive_text(input: &str) -> String {
    let mut output = input.to_string();
    for marker in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"] {
        output = redact_token_after_marker(&output, marker);
    }

    let mut sanitized_lines = Vec::new();
    for line in output.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("authorization:")
            || lower.contains("token:")
            || lower.contains("password:")
            || lower.contains("passwd:")
            || lower.contains("secret:")
        {
            sanitized_lines.push("[REDACTED: sensitive line]".to_string());
        } else {
            sanitized_lines.push(line.to_string());
        }
    }
    sanitized_lines.join("\n")
}

fn redact_token_after_marker(input: &str, marker: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut idx = 0usize;
    while let Some(found) = input[idx..].find(marker) {
        let start = idx + found;
        output.push_str(&input[idx..start]);
        output.push_str(marker);
        output.push_str("[REDACTED]");

        let token_start = start + marker.len();
        let token_len = input[token_start..]
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '_' | '-'))
            .count();
        idx = token_start + token_len;
    }
    output.push_str(&input[idx..]);
    output
}

fn parse_status(raw: &str) -> Result<RepoSnapshot> {
    let mut snapshot = RepoSnapshot::default();
    let mut first = true;

    for line in raw.lines() {
        if first && line.starts_with("## ") {
            parse_branch_line(line, &mut snapshot);
            first = false;
            continue;
        }
        first = false;
        if line.len() < 3 {
            continue;
        }

        let mut chars = line.chars();
        let x = chars.next().unwrap_or(' ');
        let y = chars.next().unwrap_or(' ');
        let path_part = line.get(3..).unwrap_or("").trim();
        if path_part.is_empty() {
            continue;
        }
        let normalized = normalize_status_path(path_part);

        let status_text = format!("{x}{y}");
        let is_untracked = x == '?' && y == '?';
        let has_staged = x != ' ' && x != '?';
        let has_unstaged = y != ' ' || is_untracked;

        if has_staged {
            snapshot.staged.push(FileEntry {
                path: normalized.clone(),
                status: status_text.clone(),
            });
        }
        if has_unstaged {
            snapshot.unstaged.push(FileEntry {
                path: normalized,
                status: status_text,
            });
        }
    }

    Ok(snapshot)
}

fn parse_branch_line(line: &str, snapshot: &mut RepoSnapshot) {
    let body = line.trim_start_matches("## ").trim();
    if let Some((branch_part, rest)) = body.split_once("...") {
        snapshot.branch = branch_part.trim().to_string();
        if let Some((tracking, counts)) = rest.split_once(' ') {
            snapshot.tracking = Some(tracking.trim().to_string());
            parse_ahead_behind(counts, snapshot);
        } else {
            snapshot.tracking = Some(rest.trim().to_string());
        }
    } else if let Some((branch, counts)) = body.split_once(' ') {
        snapshot.branch = branch.trim().to_string();
        parse_ahead_behind(counts, snapshot);
    } else {
        snapshot.branch = body.to_string();
    }
}

fn parse_ahead_behind(part: &str, snapshot: &mut RepoSnapshot) {
    let cleaned = part.trim().trim_start_matches('[').trim_end_matches(']');
    for section in cleaned.split(',') {
        let item = section.trim();
        if let Some(value) = item.strip_prefix("ahead ") {
            snapshot.ahead = value.trim().parse().unwrap_or(0);
        }
        if let Some(value) = item.strip_prefix("behind ") {
            snapshot.behind = value.trim().parse().unwrap_or(0);
        }
    }
}

fn normalize_status_path(path: &str) -> String {
    if let Some((_, new_path)) = path.split_once(" -> ") {
        new_path.trim().to_string()
    } else {
        path.to_string()
    }
}
