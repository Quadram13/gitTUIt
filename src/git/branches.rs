use std::path::Path;

use anyhow::{Result, anyhow};

use super::run_command;

#[derive(Debug, Clone)]
pub struct BranchEntry {
    pub name: String,
    pub is_current: bool,
}

#[derive(Debug, Clone)]
pub struct RemoteBranchEntry {
    pub name: String,
}

pub fn list_local_branches(repo_root: &Path) -> Result<Vec<BranchEntry>> {
    let raw = run_command("git", ["branch", "--list"], repo_root)?;
    Ok(parse_local_branches(&raw))
}

pub fn list_remote_branches(repo_root: &Path) -> Result<Vec<RemoteBranchEntry>> {
    let raw = run_command(
        "git",
        ["branch", "--remotes", "--format=%(refname:short)"],
        repo_root,
    )?;
    Ok(parse_remote_branches(&raw))
}

pub fn switch_branch(repo_root: &Path, branch: &str) -> Result<()> {
    let switch_result = run_command("git", ["switch", branch], repo_root);
    if switch_result.is_ok() {
        return Ok(());
    }
    run_command("git", ["checkout", branch], repo_root).map(|_| ())
}

pub fn create_and_switch_branch(repo_root: &Path, branch: &str) -> Result<()> {
    let switch_result = run_command("git", ["switch", "-c", branch], repo_root);
    if switch_result.is_ok() {
        return Ok(());
    }
    run_command("git", ["checkout", "-b", branch], repo_root).map(|_| ())
}

pub fn checkout_remote_tracking_branch(repo_root: &Path, remote_branch: &str) -> Result<String> {
    if remote_branch.trim().is_empty() {
        return Err(anyhow!("Remote branch name cannot be empty"));
    }

    let local_branch = remote_branch
        .split_once('/')
        .map(|(_, local)| local.to_string())
        .unwrap_or_else(|| remote_branch.to_string());

    if run_command("git", ["switch", "--track", remote_branch], repo_root).is_ok()
        || run_command("git", ["checkout", "--track", remote_branch], repo_root).is_ok()
        || run_command("git", ["switch", &local_branch], repo_root).is_ok()
    {
        return Ok(local_branch);
    }

    run_command("git", ["checkout", &local_branch], repo_root)?;
    Ok(local_branch)
}

fn parse_local_branches(raw: &str) -> Vec<BranchEntry> {
    raw.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Some(rest) = trimmed.strip_prefix('*') {
                let name = rest.trim().to_string();
                if name.is_empty() {
                    return None;
                }
                return Some(BranchEntry {
                    name,
                    is_current: true,
                });
            }
            Some(BranchEntry {
                name: trimmed.to_string(),
                is_current: false,
            })
        })
        .collect()
}

fn parse_remote_branches(raw: &str) -> Vec<RemoteBranchEntry> {
    let mut entries = raw
        .lines()
        .filter_map(|line| {
            let name = line.trim();
            if name.is_empty() || name.ends_with("/HEAD") || name.contains("->") {
                return None;
            }
            Some(RemoteBranchEntry {
                name: name.to_string(),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}
