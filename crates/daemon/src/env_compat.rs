/// Rebrand cleanup removed deprecated env aliases, so startup no longer needs
/// any pre-parse process environment rewrites.
pub fn make_env_compatible() {}

#[cfg(test)]
mod tests {
    use super::make_env_compatible;
    use crate::test_support::ScopedEnv;

    fn loong_home() -> Option<std::path::PathBuf> {
        std::env::var_os("LOONG_HOME").map(std::path::PathBuf::from)
    }

    #[test]
    fn make_env_compatible_is_no_op_for_canonical_env() {
        let mut env = ScopedEnv::new();
        let value = std::env::temp_dir().join("loong-home-canonical");
        env.set("LOONG_HOME", &value);

        make_env_compatible();

        assert_eq!(loong_home(), Some(value));
    }

    #[test]
    fn make_env_compatible_leaves_missing_env_missing() {
        let mut env = ScopedEnv::new();
        env.remove("LOONG_HOME");

        make_env_compatible();

        assert!(loong_home().is_none());
    }
}
