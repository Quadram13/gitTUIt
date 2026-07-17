use std::{env, path::PathBuf};

use anyhow::{Result, anyhow};

const APP_DIR_NAME: &str = "gitTUIt";
const APP_DIR_NAME_DEV: &str = "gitTUIt-dev";
const CONFIG_DIR_ENV_VAR: &str = "GITTUIT_CONFIG_DIR";

fn app_dir_name() -> &'static str {
    if cfg!(debug_assertions) {
        APP_DIR_NAME_DEV
    } else {
        APP_DIR_NAME
    }
}

pub fn config_dir() -> Result<PathBuf> {
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

    let base = dirs::config_dir().ok_or_else(|| anyhow!("Could not determine config directory"))?;
    Ok(base.join(app_dir_name()))
}

pub fn data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    Ok(base.join(app_dir_name()))
}

pub fn cache_dir() -> Result<PathBuf> {
    let base = dirs::cache_dir().ok_or_else(|| anyhow!("Could not determine cache directory"))?;
    Ok(base.join(app_dir_name()))
}

pub fn logs_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("logs"))
}
