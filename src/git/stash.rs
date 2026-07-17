use std::path::Path;

use anyhow::Result;

use super::run_command;

#[derive(Debug, Clone)]
pub struct StashEntry {
    pub reference: String,
    pub message: String,
}

pub fn list_stashes(repo_root: &Path) -> Result<Vec<StashEntry>> {
    let raw = run_command("git", ["stash", "list", "--format=%gd%x1f%gs"], repo_root)?;
    Ok(parse_stashes(&raw))
}

pub fn stash_push(repo_root: &Path) -> Result<()> {
    run_command("git", ["stash", "push", "-u"], repo_root).map(|_| ())
}

pub fn stash_apply(repo_root: &Path, reference: &str) -> Result<()> {
    run_command("git", ["stash", "apply", reference], repo_root).map(|_| ())
}

pub fn stash_pop(repo_root: &Path, reference: &str) -> Result<()> {
    run_command("git", ["stash", "pop", reference], repo_root).map(|_| ())
}

pub fn stash_drop(repo_root: &Path, reference: &str) -> Result<()> {
    run_command("git", ["stash", "drop", reference], repo_root).map(|_| ())
}

pub fn stash_show(repo_root: &Path, reference: &str) -> Result<String> {
    run_command(
        "git",
        [
            "stash",
            "show",
            "--patch",
            "--stat",
            "--color=never",
            reference,
        ],
        repo_root,
    )
}

fn parse_stashes(raw: &str) -> Vec<StashEntry> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            let reference = parts.next()?.trim().to_string();
            let message = parts.next()?.trim().to_string();
            if reference.is_empty() {
                return None;
            }
            Some(StashEntry { reference, message })
        })
        .collect()
}
