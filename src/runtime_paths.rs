use std::{env, path::PathBuf};

use anyhow::{Result, anyhow};

const APP_DIR_NAME: &str = "gitTUIt";
const APP_DIR_NAME_DEV: &str = "gitTUIt-dev";
const CONFIG_DIR_ENV_VAR: &str = "GITTUIT_CONFIG_DIR";
const RUNTIME_ROOT_ENV_VAR: &str = "GITTUIT_RUNTIME_ROOT";
const DEV_RUNTIME_DIR_NAME: &str = ".gittuit-runtime";

fn app_dir_name() -> &'static str {
    if cfg!(debug_assertions) {
        APP_DIR_NAME_DEV
    } else {
        APP_DIR_NAME
    }
}

fn runtime_root_override() -> Result<Option<PathBuf>> {
    let Ok(override_root) = env::var(RUNTIME_ROOT_ENV_VAR) else {
        return Ok(None);
    };
    let trimmed = override_root.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "{} is set but empty. Provide a valid directory path.",
            RUNTIME_ROOT_ENV_VAR
        ));
    }
    Ok(Some(PathBuf::from(trimmed)))
}

fn dev_runtime_root() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }

    let mut cursor = env::current_dir().ok()?;
    loop {
        let is_repo_root = cursor.join(".git").exists();
        let has_cargo_toml = cursor.join("Cargo.toml").is_file();
        if is_repo_root && has_cargo_toml {
            return Some(cursor.join(DEV_RUNTIME_DIR_NAME));
        }

        let Some(parent) = cursor.parent() else {
            return None;
        };
        cursor = parent.to_path_buf();
    }
}

pub fn config_dir() -> Result<PathBuf> {
    if let Some(root) = runtime_root_override()? {
        return Ok(root.join("config"));
    }

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

    if let Some(root) = dev_runtime_root() {
        return Ok(root.join("config"));
    }

    let base = dirs::config_dir().ok_or_else(|| anyhow!("Could not determine config directory"))?;
    Ok(base.join(app_dir_name()))
}

pub fn data_dir() -> Result<PathBuf> {
    if let Some(root) = runtime_root_override()? {
        return Ok(root.join("data"));
    }

    if let Some(root) = dev_runtime_root() {
        return Ok(root.join("data"));
    }

    let base = dirs::data_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    Ok(base.join(app_dir_name()))
}

pub fn cache_dir() -> Result<PathBuf> {
    if let Some(root) = runtime_root_override()? {
        return Ok(root.join("cache"));
    }

    if let Some(root) = dev_runtime_root() {
        return Ok(root.join("cache"));
    }

    let base = dirs::cache_dir().ok_or_else(|| anyhow!("Could not determine cache directory"))?;
    Ok(base.join(app_dir_name()))
}

pub fn logs_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("logs"))
}
