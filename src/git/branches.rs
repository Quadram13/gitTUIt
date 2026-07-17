use std::path::Path;

use anyhow::{Result, anyhow};
use asyncgit::sync::{self, BranchDetails, RepoPath};

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
    let repo_path = repo_path_from_root(repo_root);
    let mut entries = sync::get_branches_info(&repo_path, true)?
        .into_iter()
        .filter_map(|branch| match branch.details {
            BranchDetails::Local(local) => Some(BranchEntry {
                name: branch.name,
                is_current: local.is_head,
            }),
            BranchDetails::Remote(_) => None,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

pub fn list_remote_branches(repo_root: &Path) -> Result<Vec<RemoteBranchEntry>> {
    let repo_path = repo_path_from_root(repo_root);
    let mut entries = sync::get_branches_info(&repo_path, false)?
        .into_iter()
        .filter_map(|branch| {
            if branch.name.is_empty()
                || branch.name.ends_with("/HEAD")
                || branch.name.contains("->")
            {
                return None;
            }
            Some(RemoteBranchEntry { name: branch.name })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

pub fn switch_branch(repo_root: &Path, branch: &str) -> Result<()> {
    let repo_path = repo_path_from_root(repo_root);
    sync::checkout_branch(&repo_path, branch)?;
    Ok(())
}

pub fn create_and_switch_branch(repo_root: &Path, branch: &str) -> Result<()> {
    let repo_path = repo_path_from_root(repo_root);
    sync::create_branch(&repo_path, branch)?;
    Ok(())
}

pub fn checkout_remote_tracking_branch(repo_root: &Path, remote_branch: &str) -> Result<String> {
    if remote_branch.trim().is_empty() {
        return Err(anyhow!("Remote branch name cannot be empty"));
    }

    let repo_path = repo_path_from_root(repo_root);
    let remote_branch_info = sync::get_branches_info(&repo_path, false)?
        .into_iter()
        .find(|branch| branch.name == remote_branch || branch.reference == remote_branch)
        .ok_or_else(|| anyhow!("Remote branch not found: {remote_branch}"))?;

    sync::branch::checkout_remote_branch(&repo_path, &remote_branch_info)?;

    let local_branch = remote_branch_info
        .name
        .split_once('/')
        .map(|(_, local)| local.to_string())
        .unwrap_or_else(|| remote_branch_info.name);
    Ok(local_branch.trim_start_matches('/').to_string())
}

fn repo_path_from_root(repo_root: &Path) -> RepoPath {
    repo_root.to_path_buf().into()
}
