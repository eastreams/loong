fn non_empty_env_var(name: &str) -> Option<std::ffi::OsString> {
    let value = std::env::var_os(name);
    let value_ref = value.as_deref();
    let value_is_non_empty = value_ref.is_some_and(|candidate| !candidate.is_empty());

    if value_is_non_empty {
        return value;
    }

    None
}

/// Copies deprecated `LOONGCLAW_*` env vars into their `LOONG_*` replacements
/// and emits a deprecation warning. No-op when the new name is already set.
pub fn make_env_compatible() {
    const MIGRATIONS: &[(&str, &str)] = &[("LOONG_HOME", "LOONGCLAW_HOME")];

    for &(new_name, old_name) in MIGRATIONS {
        let old_value = non_empty_env_var(old_name);
        let new_value = non_empty_env_var(new_name);
        let new_is_unset = new_value.is_none();

        if !new_is_unset {
            continue;
        }

        let Some(old_value) = old_value else {
            continue;
        };

        // SAFETY: single-threaded — called before tokio runtime and parse_cli.
        #[allow(unsafe_code, clippy::disallowed_methods)]
        unsafe {
            std::env::set_var(new_name, &old_value);
        }
        tracing::warn!("{old_name} is deprecated. Set {new_name} instead.");
    }
}

#[cfg(test)]
mod tests {
    use super::make_env_compatible;
    use crate::test_support::ScopedEnv;

    fn loong_home() -> Option<std::path::PathBuf> {
        std::env::var_os("LOONG_HOME").map(std::path::PathBuf::from)
    }

    #[test]
    fn migrates_deprecated_env_when_new_is_unset() {
        let mut env = ScopedEnv::new();
        let value = std::env::temp_dir().join("loong-compat-old-only");
        env.set("LOONGCLAW_HOME", &value);
        env.remove("LOONG_HOME");

        make_env_compatible();

        assert_eq!(loong_home(), Some(value));
    }

    #[test]
    fn does_not_overwrite_new_env_when_both_set() {
        let mut env = ScopedEnv::new();
        let new_value = std::env::temp_dir().join("loong-compat-new");
        let old_value = std::env::temp_dir().join("loong-compat-old");
        env.set("LOONG_HOME", &new_value);
        env.set("LOONGCLAW_HOME", &old_value);

        make_env_compatible();

        assert_eq!(loong_home(), Some(new_value));
    }

    #[test]
    fn migrates_deprecated_env_when_new_is_empty() {
        let mut env = ScopedEnv::new();
        let old_value = std::env::temp_dir().join("loong-compat-old-when-new-empty");
        env.set("LOONGCLAW_HOME", &old_value);
        env.set("LOONG_HOME", "");

        make_env_compatible();

        assert_eq!(loong_home(), Some(old_value));
    }

    #[test]
    fn ignores_empty_deprecated_env_value() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_HOME", "");
        env.remove("LOONG_HOME");

        make_env_compatible();

        assert!(loong_home().is_none());
    }

    #[test]
    fn no_op_when_only_new_env_is_set() {
        let mut env = ScopedEnv::new();
        let value = std::env::temp_dir().join("loong-compat-new-only");
        env.set("LOONG_HOME", &value);
        env.remove("LOONGCLAW_HOME");

        make_env_compatible();

        assert_eq!(loong_home(), Some(value));
    }

    #[test]
    fn no_op_when_neither_env_is_set() {
        let mut env = ScopedEnv::new();
        env.remove("LOONG_HOME");
        env.remove("LOONGCLAW_HOME");

        make_env_compatible();

        assert!(loong_home().is_none());
    }
}
