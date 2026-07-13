use std::path::Path;

use anyhow::{Result, anyhow};

use super::run_command;

#[derive(Debug, Clone)]
pub struct CommitEntry {
    pub hash: String,
    pub short_hash: String,
    pub summary: String,
    pub author: String,
    pub relative_time: String,
}

#[derive(Debug, Clone)]
pub struct CommitSummaryEntry {
    pub short_hash: String,
    pub summary: String,
}

#[derive(Debug, Clone)]
pub struct TrackingCommitSummary {
    pub upstream: String,
    pub outgoing: Vec<CommitSummaryEntry>,
    pub incoming: Vec<CommitSummaryEntry>,
}

pub fn commit_history(repo_root: &Path, max_count: usize) -> Result<Vec<CommitEntry>> {
    let count = max_count.max(1).to_string();
    let raw = run_command(
        "git",
        [
            "log",
            "--max-count",
            &count,
            "--date=relative",
            "--pretty=format:%H%x1f%h%x1f%s%x1f%an%x1f%ar",
        ],
        repo_root,
    )?;
    Ok(parse_commit_history(&raw))
}

pub fn tracking_commit_summary(repo_root: &Path, max_count: usize) -> Result<TrackingCommitSummary> {
    let upstream = run_command(
        "git",
        ["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
        repo_root,
    )?
    .trim()
    .to_string();
    if upstream.is_empty() {
        return Err(anyhow!(
            "No upstream configured for current branch (publish/push with upstream first)"
        ));
    }

    let count = max_count.max(1).to_string();
    let outgoing_raw = run_command(
        "git",
        [
            "log",
            "--max-count",
            &count,
            "--pretty=format:%h%x1f%s",
            "@{u}..HEAD",
        ],
        repo_root,
    )?;
    let incoming_raw = run_command(
        "git",
        [
            "log",
            "--max-count",
            &count,
            "--pretty=format:%h%x1f%s",
            "HEAD..@{u}",
        ],
        repo_root,
    )?;

    Ok(TrackingCommitSummary {
        upstream,
        outgoing: parse_commit_summaries(&outgoing_raw),
        incoming: parse_commit_summaries(&incoming_raw),
    })
}

pub fn commit_details(repo_root: &Path, commit_hash: &str) -> Result<String> {
    run_command(
        "git",
        [
            "show",
            "--color=never",
            "--decorate",
            "--stat",
            "--patch",
            "--find-renames",
            commit_hash,
        ],
        repo_root,
    )
}

pub fn checkout_detached(repo_root: &Path, commit_hash: &str) -> Result<()> {
    let switch_result = run_command("git", ["switch", "--detach", commit_hash], repo_root);
    if switch_result.is_ok() {
        return Ok(());
    }
    run_command("git", ["checkout", "--detach", commit_hash], repo_root).map(|_| ())
}

pub fn cherry_pick(repo_root: &Path, commit_hash: &str) -> Result<()> {
    run_command("git", ["cherry-pick", commit_hash], repo_root).map(|_| ())
}

pub fn cherry_pick_continue(repo_root: &Path) -> Result<()> {
    run_command("git", ["cherry-pick", "--continue"], repo_root).map(|_| ())
}

pub fn cherry_pick_abort(repo_root: &Path) -> Result<()> {
    run_command("git", ["cherry-pick", "--abort"], repo_root).map(|_| ())
}

fn parse_commit_history(raw: &str) -> Vec<CommitEntry> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            let hash = parts.next()?.trim().to_string();
            let short_hash = parts.next()?.trim().to_string();
            let summary = parts.next()?.trim().to_string();
            let author = parts.next()?.trim().to_string();
            let relative_time = parts.next()?.trim().to_string();

            if hash.is_empty() || short_hash.is_empty() {
                return None;
            }

            Some(CommitEntry {
                hash,
                short_hash,
                summary,
                author,
                relative_time,
            })
        })
        .collect()
}

fn parse_commit_summaries(raw: &str) -> Vec<CommitSummaryEntry> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            let short_hash = parts.next()?.trim().to_string();
            let summary = parts.next()?.trim().to_string();
            if short_hash.is_empty() {
                return None;
            }
            Some(CommitSummaryEntry {
                short_hash,
                summary,
            })
        })
        .collect()
}
