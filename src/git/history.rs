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

#[derive(Debug, Clone)]
pub struct CommitChangedFile {
    pub status: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct CommitDetails {
    pub hash: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: String,
    pub subject: String,
    pub body: String,
    pub files: Vec<CommitChangedFile>,
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

pub fn tracking_commit_summary(
    repo_root: &Path,
    max_count: usize,
) -> Result<TrackingCommitSummary> {
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

pub fn commit_details_structured(repo_root: &Path, commit_hash: &str) -> Result<CommitDetails> {
    let metadata_raw = run_command(
        "git",
        [
            "show",
            "-s",
            "--date=iso-strict",
            "--pretty=format:%H%x1f%an%x1f%ae%x1f%ad%x1f%s",
            commit_hash,
        ],
        repo_root,
    )?;
    let body = run_command(
        "git",
        ["show", "-s", "--pretty=format:%b", commit_hash],
        repo_root,
    )?;
    let files_raw = run_command(
        "git",
        ["show", "--name-status", "--format=", commit_hash],
        repo_root,
    )?;

    parse_commit_details(&metadata_raw, &body, &files_raw)
}

pub fn commit_file_diff(repo_root: &Path, commit_hash: &str, path: &str) -> Result<String> {
    run_command(
        "git",
        [
            "show",
            "--color=never",
            "--format=",
            "--patch",
            "--find-renames",
            commit_hash,
            "--",
            path,
        ],
        repo_root,
    )
}

pub fn file_history(repo_root: &Path, path: &str, max_count: usize) -> Result<Vec<CommitEntry>> {
    let count = max_count.max(1).to_string();
    let raw = run_command(
        "git",
        [
            "log",
            "--max-count",
            &count,
            "--date=relative",
            "--pretty=format:%H%x1f%h%x1f%s%x1f%an%x1f%ar",
            "--",
            path,
        ],
        repo_root,
    )?;
    Ok(parse_commit_history(&raw))
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

fn parse_commit_details(metadata_raw: &str, body: &str, files_raw: &str) -> Result<CommitDetails> {
    let line = metadata_raw
        .lines()
        .next()
        .ok_or_else(|| anyhow!("No commit metadata found"))?;
    let mut parts = line.split('\u{1f}');
    let hash = parts.next().unwrap_or_default().trim().to_string();
    let author_name = parts.next().unwrap_or_default().trim().to_string();
    let author_email = parts.next().unwrap_or_default().trim().to_string();
    let authored_at = parts.next().unwrap_or_default().trim().to_string();
    let subject = parts.next().unwrap_or_default().trim().to_string();
    if hash.is_empty() {
        return Err(anyhow!("Commit hash was empty"));
    }

    let files = parse_changed_files(files_raw);
    Ok(CommitDetails {
        hash,
        author_name,
        author_email,
        authored_at,
        subject,
        body: body.trim_end().to_string(),
        files,
    })
}

fn parse_changed_files(raw: &str) -> Vec<CommitChangedFile> {
    raw.lines()
        .filter_map(|line| {
            if line.trim().is_empty() {
                return None;
            }
            let mut parts = line.split('\t');
            let status = parts.next()?.trim().to_string();
            let first_path = parts.next().unwrap_or("").trim();
            let second_path = parts.next().unwrap_or("").trim();
            let path = if !second_path.is_empty() {
                second_path.to_string()
            } else {
                first_path.to_string()
            };
            if path.is_empty() {
                return None;
            }
            Some(CommitChangedFile { status, path })
        })
        .collect()
}
