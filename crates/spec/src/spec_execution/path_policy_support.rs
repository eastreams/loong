use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use crate::CliResult;
use crate::spec_runtime::{
    WASM_GUEST_CONFIG_CHANNEL_PREFIX, WASM_GUEST_CONFIG_PROVIDER_PREFIX,
    wasm_guest_config_key_is_supported,
};

pub fn resolve_plugin_relative_path(source_path: &str, artifact: &str) -> PathBuf {
    let candidate = PathBuf::from(artifact);
    if candidate.is_absolute() {
        return candidate;
    }

    let source = Path::new(source_path);
    if let Some(parent) = source.parent() {
        parent.join(candidate)
    } else {
        candidate
    }
}

pub(crate) fn normalize_allowed_path_prefixes(prefixes: &[String]) -> Vec<PathBuf> {
    prefixes
        .iter()
        .map(|prefix| normalize_path_for_policy(&PathBuf::from(prefix)))
        .collect()
}

pub(crate) fn validate_wasm_guest_readable_config_keys(keys: &BTreeSet<String>) -> CliResult<()> {
    for key in keys {
        let key_is_supported = wasm_guest_config_key_is_supported(key.as_str());
        if key_is_supported {
            continue;
        }

        return Err(format!(
            "invalid security scan runtime.guest_readable_config_keys entry `{key}`: entries must use `{WASM_GUEST_CONFIG_PROVIDER_PREFIX}<metadata_key>` or `{WASM_GUEST_CONFIG_CHANNEL_PREFIX}<metadata_key>`"
        ));
    }

    Ok(())
}

pub fn normalize_path_for_policy(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }

    if path.is_absolute() {
        return path.to_path_buf();
    }

    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}
