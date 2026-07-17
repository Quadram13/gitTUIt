use std::{
    fs::{self, OpenOptions},
    path::PathBuf,
    process::Command,
};

use anyhow::{Result, anyhow};
use log::LevelFilter;
use simplelog::{ConfigBuilder, WriteLogger};

const DEFAULT_LOG_FILE_NAME: &str = "gitTUIt.log";
const REPOS_FILE_NAME: &str = "repos.json";

use crate::runtime_paths;

#[derive(Debug, Clone)]
pub struct RuntimeOptions {
    pub logging_enabled: bool,
    pub doctor_mode: bool,
    pub log_file_path: Option<PathBuf>,
    pub log_level: LevelFilter,
}

pub fn parse_runtime_options(args: impl IntoIterator<Item = String>) -> Result<RuntimeOptions> {
    let mut logging_enabled = false;
    let mut doctor_mode = false;
    let mut log_file_path = None;
    let mut log_level = LevelFilter::Info;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--log" => logging_enabled = true,
            "-l" => doctor_mode = true,
            "--log-file" => {
                let Some(path) = iter.next() else {
                    return Err(anyhow!("--log-file requires a path argument"));
                };
                logging_enabled = true;
                log_file_path = Some(PathBuf::from(path));
            }
            "--log-level" => {
                let Some(level) = iter.next() else {
                    return Err(anyhow!(
                        "--log-level requires one of: error, warn, info, debug, trace"
                    ));
                };
                logging_enabled = true;
                log_level = parse_level_filter(&level)?;
            }
            _ => {
                return Err(anyhow!(
                    "Unknown argument: {arg}. Supported flags: --log, --log-file <path>, --log-level <level>, -l"
                ));
            }
        }
    }

    Ok(RuntimeOptions {
        logging_enabled,
        doctor_mode,
        log_file_path,
        log_level,
    })
}

pub fn initialize_logging(options: &RuntimeOptions) -> Result<Option<PathBuf>> {
    if !options.logging_enabled {
        return Ok(None);
    }

    let log_path = resolve_log_file_path(options)?;
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new().create(true).append(true).open(&log_path)?;
    let mut config_builder = ConfigBuilder::new();
    let _ = config_builder.set_time_offset_to_local();
    let config = config_builder.build();
    WriteLogger::init(options.log_level, config, file)?;
    log::info!("Logging initialized");
    log::info!("Log file: {}", log_path.display());
    log::info!("Log level: {}", options.log_level);

    Ok(Some(log_path))
}

pub fn resolve_log_file_path(options: &RuntimeOptions) -> Result<PathBuf> {
    match &options.log_file_path {
        Some(path) => Ok(path.clone()),
        None => default_log_file_path(),
    }
}

pub fn doctor_report(options: &RuntimeOptions) -> Result<String> {
    let config_dir = runtime_paths::config_dir()?;
    let data_dir = runtime_paths::data_dir()?;
    let cache_dir = runtime_paths::cache_dir()?;
    let logs_dir = runtime_paths::logs_dir()?;
    let repos_file = config_dir.join(REPOS_FILE_NAME);
    let log_file = resolve_log_file_path(options)?;

    let git_version = command_version("git");
    let gh_version = command_version("gh");

    let report = format!(
        "gitTUIt diagnostics\n\
         ==================\n\
         Logging enabled: {}\n\
         Log level: {}\n\
         Log file path: {}\n\
         Logs dir: {}\n\
         Config dir: {}\n\
         Data dir: {}\n\
         Cache dir: {}\n\
         Repo registry path: {}\n\
         git version: {}\n\
         gh version: {}\n",
        options.logging_enabled,
        options.log_level,
        log_file.display(),
        logs_dir.display(),
        config_dir.display(),
        data_dir.display(),
        cache_dir.display(),
        repos_file.display(),
        git_version,
        gh_version
    );
    Ok(report)
}

fn parse_level_filter(raw: &str) -> Result<LevelFilter> {
    match raw.to_ascii_lowercase().as_str() {
        "error" => Ok(LevelFilter::Error),
        "warn" | "warning" => Ok(LevelFilter::Warn),
        "info" => Ok(LevelFilter::Info),
        "debug" => Ok(LevelFilter::Debug),
        "trace" => Ok(LevelFilter::Trace),
        _ => Err(anyhow!(
            "Invalid log level '{raw}'. Use one of: error, warn, info, debug, trace"
        )),
    }
}

pub fn default_log_file_path() -> Result<PathBuf> {
    Ok(runtime_paths::logs_dir()?.join(DEFAULT_LOG_FILE_NAME))
}

fn command_version(program: &str) -> String {
    let output = Command::new(program).arg("--version").output();
    let Ok(output) = output else {
        return "not found".to_string();
    };

    if !output.status.success() {
        return format!("failed (status: {})", output.status);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        "available (no output)".to_string()
    } else {
        stdout.lines().next().unwrap_or_default().to_string()
    }
}
