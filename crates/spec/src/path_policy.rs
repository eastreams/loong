use std::{
    fs,
    path::{Path, PathBuf},
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

#[cfg(test)]
mod tests {
    use super::{normalize_path_for_policy, resolve_plugin_relative_path};
    use std::{
        env, fs,
        path::{Path, PathBuf},
    };

    #[test]
    fn resolve_plugin_relative_path_joins_relative_artifacts_to_source_parent() {
        let resolved = resolve_plugin_relative_path(
            "/tmp/plugins/manifest/plugin.json",
            "artifacts/component.wasm",
        );

        assert_eq!(
            resolved,
            PathBuf::from("/tmp/plugins/manifest/artifacts/component.wasm")
        );
    }

    #[test]
    fn resolve_plugin_relative_path_preserves_absolute_artifacts() {
        let absolute = PathBuf::from("/tmp/plugins/component.wasm");
        let resolved = resolve_plugin_relative_path(
            "/tmp/plugins/manifest/plugin.json",
            absolute
                .to_str()
                .expect("absolute test path should be valid utf-8"),
        );

        assert_eq!(resolved, absolute);
    }

    #[test]
    fn normalize_path_for_policy_canonicalizes_existing_paths() {
        let cargo_toml = Path::new("Cargo.toml");
        let normalized = normalize_path_for_policy(cargo_toml);
        let canonical = fs::canonicalize(cargo_toml).expect("Cargo.toml should exist");

        assert_eq!(normalized, canonical);
    }

    #[test]
    fn normalize_path_for_policy_absolutizes_missing_relative_paths_from_cwd() {
        let relative = PathBuf::from(format!(
            ".omx/tmp/path-policy-missing-{}-spec",
            std::process::id()
        ));
        let normalized = normalize_path_for_policy(&relative);
        let cwd = env::current_dir().expect("cwd should resolve");

        assert_eq!(normalized, cwd.join(relative));
    }
}
