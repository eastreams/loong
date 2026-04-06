use std::ffi::{OsStr, OsString};

pub const HIGH_RISK_CHILD_PROCESS_ENV_VARS: &[&str] = &[
    "CC",
    "CXX",
    "CARGO_BUILD_RUSTC",
    "CMAKE_C_COMPILER",
    "CMAKE_CXX_COMPILER",
    "PIP_INDEX_URL",
    "PIP_EXTRA_INDEX_URL",
    "UV_INDEX_URL",
    "UV_EXTRA_INDEX_URL",
    "PYTHONPATH",
];

const ESSENTIAL_CHILD_PROCESS_ENV_VARS: &[&str] = &[
    "APPDATA",
    "COLORTERM",
    "COMSPEC",
    "HOME",
    "LANG",
    "LOCALAPPDATA",
    "LOGNAME",
    "NO_COLOR",
    "PATH",
    "PATHEXT",
    "PWD",
    "SHELL",
    "SYSTEMROOT",
    "TEMP",
    "TERM",
    "TMP",
    "TMPDIR",
    "USER",
    "USERPROFILE",
    "WINDIR",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
];

const ESSENTIAL_CHILD_PROCESS_ENV_PREFIXES: &[&str] = &["LC_"];

#[must_use]
pub fn child_process_env_var_is_allowed(name: &OsStr) -> bool {
    let rendered = name.to_string_lossy();

    let is_blocked = HIGH_RISK_CHILD_PROCESS_ENV_VARS
        .iter()
        .any(|blocked| rendered.eq_ignore_ascii_case(blocked));

    if is_blocked {
        return false;
    }

    let is_allowed = ESSENTIAL_CHILD_PROCESS_ENV_VARS
        .iter()
        .any(|allowed| rendered.eq_ignore_ascii_case(allowed));

    if is_allowed {
        return true;
    }

    let normalized = rendered.to_ascii_uppercase();

    ESSENTIAL_CHILD_PROCESS_ENV_PREFIXES
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
}

#[must_use]
pub fn sanitized_child_process_env() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| child_process_env_var_is_allowed(name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_process_env_blocks_high_risk_toolchain_and_index_overrides() {
        assert!(!child_process_env_var_is_allowed(OsStr::new(
            "UV_INDEX_URL"
        )));
        assert!(!child_process_env_var_is_allowed(OsStr::new(
            "PIP_EXTRA_INDEX_URL"
        )));
        assert!(!child_process_env_var_is_allowed(OsStr::new("PYTHONPATH")));
        assert!(!child_process_env_var_is_allowed(OsStr::new(
            "CMAKE_C_COMPILER"
        )));
    }

    #[test]
    fn child_process_env_keeps_essential_runtime_variables() {
        assert!(child_process_env_var_is_allowed(OsStr::new("PATH")));
        assert!(child_process_env_var_is_allowed(OsStr::new("HOME")));
        assert!(child_process_env_var_is_allowed(OsStr::new("TMPDIR")));
    }

    #[test]
    fn child_process_env_keeps_loongclaw_prefixed_variables() {
        assert!(child_process_env_var_is_allowed(OsStr::new("LC_TEST_FLAG")));
        assert!(child_process_env_var_is_allowed(OsStr::new("lc_test_flag")));
    }

    #[test]
    fn child_process_env_drops_unrecognized_variables() {
        assert!(!child_process_env_var_is_allowed(OsStr::new(
            "RANDOM_CUSTOM_FLAG"
        )));
    }
}
