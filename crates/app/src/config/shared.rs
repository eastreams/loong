use std::{
    env,
    path::{Path, PathBuf},
};

pub(super) const DEFAULT_CONFIG_FILE: &str = "config.toml";
pub(super) const DEFAULT_SQLITE_FILE: &str = "memory.sqlite3";

pub(super) fn default_loongclaw_home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".loongclaw")
}

pub(super) fn expand_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
    }
    if let Some(stripped) = trimmed.strip_prefix("~/") {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(stripped);
    }
    Path::new(trimmed).to_path_buf()
}

#[cfg(feature = "channel-feishu")]
pub(super) fn read_secret_prefer_inline(
    inline: Option<&str>,
    env_key: Option<&str>,
) -> Option<String> {
    if let Some(raw) = inline {
        let value = raw.trim();
        if !value.is_empty() {
            return Some(value.to_owned());
        }
    }
    if let Some(key) = env_key {
        let value = env::var(key).ok()?;
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    None
}
