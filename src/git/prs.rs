use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use super::run_command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullRequestFilter {
    Open,
    Draft,
    Merged,
}

#[derive(Debug, Clone)]
pub struct PullRequestEntry {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub is_draft: bool,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub author: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullRequestMergeMethod {
    Merge,
    Squash,
    Rebase,
}

#[derive(Debug, Clone)]
pub struct PullRequestStatusSummary {
    pub merge_state_status: Option<String>,
    pub review_decision: Option<String>,
    pub checks_total: usize,
    pub checks_passing: usize,
    pub checks_failing: usize,
    pub checks_pending: usize,
}

pub fn pull_requests(repo_root: &Path, filter: PullRequestFilter) -> Result<Vec<PullRequestEntry>> {
    let state = match filter {
        PullRequestFilter::Merged => "merged",
        PullRequestFilter::Open | PullRequestFilter::Draft => "open",
    };
    let raw = run_command(
        "gh",
        [
            "pr",
            "list",
            "--state",
            state,
            "--limit",
            "100",
            "--json",
            "number,title,state,isDraft,headRefName,baseRefName,url,author",
        ],
        repo_root,
    )?;
    let api_entries = serde_json::from_str::<Vec<GhPullRequest>>(&raw)
        .context("Failed to parse `gh pr list` output")?;

    let mut entries = api_entries
        .into_iter()
        .filter(|entry| match filter {
            PullRequestFilter::Open => !entry.is_draft,
            PullRequestFilter::Draft => entry.is_draft,
            PullRequestFilter::Merged => true,
        })
        .map(|entry| PullRequestEntry {
            number: entry.number,
            title: entry.title,
            state: entry.state,
            is_draft: entry.is_draft,
            head_ref_name: entry.head_ref_name,
            base_ref_name: entry.base_ref_name,
            author: entry.author.login,
            url: entry.url,
        })
        .collect::<Vec<_>>();

    entries.sort_by(|a, b| b.number.cmp(&a.number));
    Ok(entries)
}

pub fn open_pr_in_browser(repo_root: &Path, pr_number: u64) -> Result<()> {
    run_command(
        "gh",
        ["pr", "view", &pr_number.to_string(), "--web"],
        repo_root,
    )
    .map(|_| ())
}

pub fn checkout_pr(repo_root: &Path, pr_number: u64) -> Result<()> {
    run_command("gh", ["pr", "checkout", &pr_number.to_string()], repo_root).map(|_| ())
}

pub fn merge_pull_request(
    repo_root: &Path,
    pr_number: u64,
    method: PullRequestMergeMethod,
) -> Result<()> {
    let method_flag = match method {
        PullRequestMergeMethod::Merge => "--merge",
        PullRequestMergeMethod::Squash => "--squash",
        PullRequestMergeMethod::Rebase => "--rebase",
    };
    run_command(
        "gh",
        ["pr", "merge", &pr_number.to_string(), method_flag],
        repo_root,
    )
    .map(|_| ())
}

pub fn create_pull_request(repo_root: &Path, title: &str, body: &str) -> Result<()> {
    run_command(
        "gh",
        ["pr", "create", "--title", title, "--body", body],
        repo_root,
    )
    .map(|_| ())
}

pub fn pull_request_status_summary(
    repo_root: &Path,
    pr_number: u64,
) -> Result<PullRequestStatusSummary> {
    let raw = run_command(
        "gh",
        [
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "mergeStateStatus,reviewDecision,statusCheckRollup",
        ],
        repo_root,
    )?;
    parse_pull_request_status_summary(&raw)
}

fn parse_pull_request_status_summary(raw: &str) -> Result<PullRequestStatusSummary> {
    let value =
        serde_json::from_str::<Value>(raw).context("Failed to parse `gh pr view` output")?;

    let merge_state_status = value
        .get("mergeStateStatus")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let review_decision = value
        .get("reviewDecision")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let mut checks_total = 0usize;
    let mut checks_passing = 0usize;
    let mut checks_failing = 0usize;
    let mut checks_pending = 0usize;

    if let Some(entries) = value.get("statusCheckRollup").and_then(Value::as_array) {
        for entry in entries {
            let raw_state = entry
                .get("state")
                .and_then(Value::as_str)
                .or_else(|| entry.get("status").and_then(Value::as_str))
                .or_else(|| entry.get("conclusion").and_then(Value::as_str))
                .unwrap_or("")
                .to_ascii_uppercase();
            if raw_state.is_empty() {
                continue;
            }

            checks_total += 1;
            if matches!(
                raw_state.as_str(),
                "SUCCESS" | "EXPECTED" | "NEUTRAL" | "SKIPPED"
            ) {
                checks_passing += 1;
            } else if matches!(
                raw_state.as_str(),
                "FAILURE"
                    | "ERROR"
                    | "CANCELLED"
                    | "TIMED_OUT"
                    | "ACTION_REQUIRED"
                    | "STALE"
                    | "STARTUP_FAILURE"
            ) {
                checks_failing += 1;
            } else {
                checks_pending += 1;
            }
        }
    }

    Ok(PullRequestStatusSummary {
        merge_state_status,
        review_decision,
        checks_total,
        checks_passing,
        checks_failing,
        checks_pending,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPullRequest {
    number: u64,
    title: String,
    state: String,
    is_draft: bool,
    head_ref_name: String,
    base_ref_name: String,
    url: String,
    author: GhUser,
}

#[derive(Debug, Deserialize)]
struct GhUser {
    login: String,
}
