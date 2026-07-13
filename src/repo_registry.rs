use std::{
    env,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

const APP_DIR_NAME: &str = "gitTUIt";
const APP_DIR_NAME_DEV: &str = "gitTUIt-dev";
const REPOS_FILE_NAME: &str = "repos.json";
const CONFIG_DIR_ENV_VAR: &str = "GITTUIT_CONFIG_DIR";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepoRegistry {
    pub repos: Vec<TrackedRepo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedRepo {
    pub path: String,
}

impl RepoRegistry {
    pub fn load() -> Result<Self> {
        let file_path = registry_file_path()?;
        if !file_path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&file_path)
            .with_context(|| format!("Failed reading {}", file_path.display()))?;
        let parsed = serde_json::from_str::<RepoRegistry>(&content)
            .with_context(|| format!("Failed parsing {}", file_path.display()))?;
        Ok(parsed)
    }

    pub fn save(&self) -> Result<()> {
        let file_path = registry_file_path()?;
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed creating {}", parent.display()))?;
        }

        let serialized = serde_json::to_string_pretty(self).context("Failed serializing repo registry")?;
        fs::write(&file_path, serialized)
            .with_context(|| format!("Failed writing {}", file_path.display()))?;
        Ok(())
    }

    pub fn add_repo(&mut self, input_path: &Path) -> Result<PathBuf> {
        let canonical = input_path
            .canonicalize()
            .with_context(|| format!("Could not resolve path '{}'", input_path.display()))?;

        ensure_git_root(&canonical)?;

        let canonical_str = canonical.to_string_lossy().to_string();
        if self.repos.iter().any(|repo| repo.path == canonical_str) {
            return Ok(canonical);
        }

        self.repos.push(TrackedRepo { path: canonical_str });
        self.repos.sort_by(|a, b| a.path.cmp(&b.path));
        self.save()?;
        Ok(canonical)
    }

    pub fn remove_index(&mut self, index: usize) -> Result<Option<TrackedRepo>> {
        if index >= self.repos.len() {
            return Ok(None);
        }
        let removed = self.repos.remove(index);
        self.save()?;
        Ok(Some(removed))
    }
}

pub fn ensure_git_root(path: &Path) -> Result<()> {
    if !path.is_dir() {
        return Err(anyhow!("Path is not a directory: {}", path.display()));
    }
    let git_path = path.join(".git");
    if git_path.is_dir() || git_path.is_file() {
        return Ok(());
    }
    Err(anyhow!(
        "Directory is not a git root (.git missing at root): {}",
        path.display()
    ))
}

pub fn canonical_repo_path(input: &str) -> Result<PathBuf> {
    let normalized = normalize_repo_path_input(input);
    if normalized.is_empty() {
        return Err(anyhow!("Path cannot be empty"));
    }
    let candidate = PathBuf::from(normalized);
    let canonical = candidate
        .canonicalize()
        .with_context(|| format!("Could not resolve path '{}'", candidate.display()))?;
    ensure_git_root(&canonical)?;
    Ok(canonical)
}

pub fn normalize_repo_path_input(input: &str) -> String {
    let trimmed = input.trim();
    strip_matching_wrapping_quotes(trimmed).to_string()
}

fn registry_file_path() -> Result<PathBuf> {
    let base_dir = resolve_config_base_dir()?;
    Ok(base_dir.join(REPOS_FILE_NAME))
}

fn resolve_config_base_dir() -> Result<PathBuf> {
    if let Ok(override_dir) = env::var(CONFIG_DIR_ENV_VAR) {
        let trimmed = override_dir.trim();
        if trimmed.is_empty() {
            return Err(anyhow!(
                "{} is set but empty. Provide a valid directory path.",
                CONFIG_DIR_ENV_VAR
            ));
        }
        return Ok(PathBuf::from(trimmed));
    }

    let config_dir = dirs::config_dir().ok_or_else(|| anyhow!("Could not determine config directory"))?;
    let app_dir = if cfg!(debug_assertions) {
        APP_DIR_NAME_DEV
    } else {
        APP_DIR_NAME
    };
    Ok(config_dir.join(app_dir))
}

fn strip_matching_wrapping_quotes(input: &str) -> &str {
    let bytes = input.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &input[1..input.len() - 1];
        }
    }
    input
}
