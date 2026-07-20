use std::{collections::HashSet, fs, path::Path};

use serde::Deserialize;
use serde_json::Value;

const CONFIG_FILE_NAME: &str = "commit_config.json";
const REPO_CONFIG_DIR: &str = ".gittuit";
const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct CommitPolicy {
    pub all_types: Vec<String>,
    pub releasable_types: Vec<String>,
    pub guarded_prefixes: Vec<String>,
}

impl Default for CommitPolicy {
    fn default() -> Self {
        Self {
            all_types: vec![
                "feat".to_string(),
                "fix".to_string(),
                "docs".to_string(),
                "chore".to_string(),
                "refactor".to_string(),
                "test".to_string(),
                "build".to_string(),
                "ci".to_string(),
                "perf".to_string(),
                "revert".to_string(),
            ],
            releasable_types: vec!["feat".to_string(), "fix".to_string()],
            guarded_prefixes: vec!["src/".to_string()],
        }
    }
}

#[derive(Debug)]
pub struct CommitPolicyLoadResult {
    pub policy: CommitPolicy,
    pub warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CommitPolicyOverride {
    schema_version: Option<u32>,
    all_types: Option<Vec<String>>,
    releasable_types: Option<Vec<String>>,
    guarded_prefixes: Option<Vec<String>>,
}

pub fn load_effective_commit_policy(repo_root: Option<&Path>) -> CommitPolicyLoadResult {
    let mut policy = CommitPolicy::default();
    let mut warnings = Vec::new();

    if let Some(root) = repo_root {
        let repo_path = root.join(REPO_CONFIG_DIR).join(CONFIG_FILE_NAME);
        if let Some(override_cfg) = read_policy_override_file(&repo_path, "Repo config", &mut warnings)
        {
            match apply_override(&policy, override_cfg) {
                Ok(next) => policy = next,
                Err(err) => warnings.push(format!(
                    "Repo config at {} was ignored: {err}",
                    repo_path.display()
                )),
            }
        }
    }

    CommitPolicyLoadResult { policy, warnings }
}

fn read_policy_override_file(
    path: &Path,
    label: &str,
    warnings: &mut Vec<String>,
) -> Option<CommitPolicyOverride> {
    if !path.exists() {
        return None;
    }

    let raw = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) => {
            warnings.push(format!("{label} at {} could not be read: {err}", path.display()));
            return None;
        }
    };

    let parsed_value = match serde_json::from_str::<Value>(&raw) {
        Ok(value) => value,
        Err(err) => {
            warnings.push(format!("{label} at {} is not valid JSON: {err}", path.display()));
            return None;
        }
    };

    for key in unknown_field_names(&parsed_value) {
        warnings.push(format!(
            "{label} at {} contains unsupported field '{key}' (ignored).",
            path.display()
        ));
    }

    match serde_json::from_value::<CommitPolicyOverride>(parsed_value) {
        Ok(config) => Some(config),
        Err(err) => {
            warnings.push(format!("{label} at {} has invalid fields: {err}", path.display()));
            None
        }
    }
}

fn unknown_field_names(value: &Value) -> Vec<String> {
    let Some(map) = value.as_object() else {
        return Vec::new();
    };
    let supported = [
        "schema_version",
        "all_types",
        "releasable_types",
        "guarded_prefixes",
    ]
    .into_iter()
    .collect::<HashSet<_>>();
    map.keys()
        .filter(|key| !supported.contains(key.as_str()))
        .cloned()
        .collect()
}

fn apply_override(base: &CommitPolicy, override_cfg: CommitPolicyOverride) -> Result<CommitPolicy, String> {
    if let Some(version) = override_cfg.schema_version
        && version != SUPPORTED_SCHEMA_VERSION
    {
        return Err(format!(
            "schema_version {version} is not supported (expected {SUPPORTED_SCHEMA_VERSION})"
        ));
    }

    let mut next = base.clone();
    if let Some(list) = override_cfg.all_types {
        next.all_types = normalize_non_empty_list("all_types", list)?;
    }
    if let Some(list) = override_cfg.releasable_types {
        next.releasable_types = normalize_list(list);
    }
    if let Some(list) = override_cfg.guarded_prefixes {
        next.guarded_prefixes = normalize_list(list);
    }

    validate_policy(&next)?;
    Ok(next)
}

fn validate_policy(policy: &CommitPolicy) -> Result<(), String> {
    if policy.all_types.is_empty() {
        return Err("all_types must contain at least one value".to_string());
    }

    let all_types = policy
        .all_types
        .iter()
        .map(|value| value.as_str())
        .collect::<HashSet<_>>();
    for releasable in &policy.releasable_types {
        if !all_types.contains(releasable.as_str()) {
            return Err(format!(
                "releasable type '{releasable}' is missing from all_types"
            ));
        }
    }

    Ok(())
}

fn normalize_non_empty_list(field_name: &str, list: Vec<String>) -> Result<Vec<String>, String> {
    let normalized = normalize_list(list);
    if normalized.is_empty() {
        return Err(format!("{field_name} cannot be empty"));
    }
    Ok(normalized)
}

fn normalize_list(list: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    list.into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .filter(|item| seen.insert(item.clone()))
        .collect()
}
