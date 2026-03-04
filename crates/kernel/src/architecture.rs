use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitecturePathDecision {
    AllowedMutable,
    DeniedImmutable,
    DeniedUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitecturePathReport {
    pub path: String,
    pub decision: ArchitecturePathDecision,
    pub matched_prefix: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ArchitectureGuardReport {
    pub total_paths: usize,
    pub allowed_paths: Vec<String>,
    pub denied_paths: Vec<String>,
    pub unknown_paths: Vec<String>,
    pub reports: Vec<ArchitecturePathReport>,
}

impl ArchitectureGuardReport {
    #[must_use]
    pub fn has_denials(&self) -> bool {
        !self.denied_paths.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureBoundaryPolicy {
    pub immutable_prefixes: BTreeSet<String>,
    pub mutable_prefixes: BTreeSet<String>,
}

impl Default for ArchitectureBoundaryPolicy {
    fn default() -> Self {
        Self {
            immutable_prefixes: BTreeSet::from([
                "crates/kernel/src/contracts.rs".to_owned(),
                "crates/kernel/src/errors.rs".to_owned(),
                "crates/kernel/src/harness.rs".to_owned(),
                "crates/kernel/src/kernel.rs".to_owned(),
                "crates/kernel/src/policy.rs".to_owned(),
            ]),
            mutable_prefixes: BTreeSet::from([
                "README.md".to_owned(),
                "crates/daemon/src/".to_owned(),
                "crates/kernel/src/audit.rs".to_owned(),
                "crates/kernel/src/architecture.rs".to_owned(),
                "crates/kernel/src/awareness.rs".to_owned(),
                "crates/kernel/src/connector.rs".to_owned(),
                "crates/kernel/src/integration.rs".to_owned(),
                "crates/kernel/src/memory.rs".to_owned(),
                "crates/kernel/src/plugin.rs".to_owned(),
                "crates/kernel/src/plugin_ir.rs".to_owned(),
                "crates/kernel/src/policy_ext.rs".to_owned(),
                "crates/kernel/src/runtime.rs".to_owned(),
                "crates/kernel/src/tests.rs".to_owned(),
                "crates/kernel/src/tool.rs".to_owned(),
                "docs/".to_owned(),
                "examples/".to_owned(),
            ]),
        }
    }
}

impl ArchitectureBoundaryPolicy {
    #[must_use]
    pub fn evaluate_paths<S: AsRef<str>>(&self, paths: &[S]) -> ArchitectureGuardReport {
        let mut report = ArchitectureGuardReport::default();

        let normalized_immutable: Vec<String> = self
            .immutable_prefixes
            .iter()
            .map(|prefix| normalize(prefix))
            .collect();
        let normalized_mutable: Vec<String> = self
            .mutable_prefixes
            .iter()
            .map(|prefix| normalize(prefix))
            .collect();

        for path in paths {
            let normalized = normalize(path.as_ref());
            report.total_paths = report.total_paths.saturating_add(1);

            if let Some(prefix) = longest_prefix_match(&normalized, &normalized_immutable) {
                report.denied_paths.push(normalized.clone());
                report.reports.push(ArchitecturePathReport {
                    path: normalized,
                    decision: ArchitecturePathDecision::DeniedImmutable,
                    matched_prefix: Some(prefix.clone()),
                    reason: format!("path is protected by immutable core boundary: {prefix}"),
                });
                continue;
            }

            if let Some(prefix) = longest_prefix_match(&normalized, &normalized_mutable) {
                report.allowed_paths.push(normalized.clone());
                report.reports.push(ArchitecturePathReport {
                    path: normalized,
                    decision: ArchitecturePathDecision::AllowedMutable,
                    matched_prefix: Some(prefix.clone()),
                    reason: format!("path is inside mutable extension boundary: {prefix}"),
                });
                continue;
            }

            report.denied_paths.push(normalized.clone());
            report.unknown_paths.push(normalized.clone());
            report.reports.push(ArchitecturePathReport {
                path: normalized,
                decision: ArchitecturePathDecision::DeniedUnknown,
                matched_prefix: None,
                reason: "path is outside declared mutable boundaries".to_owned(),
            });
        }

        report
    }
}

fn longest_prefix_match<'a>(path: &str, prefixes: &'a [String]) -> Option<&'a String> {
    prefixes
        .iter()
        .filter(|prefix| prefix_matches(path, prefix))
        .max_by_key(|prefix| prefix.len())
}

fn prefix_matches(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return false;
    }
    if path == prefix {
        return true;
    }

    if let Some(trimmed) = prefix.strip_suffix('/') {
        return path == trimmed || path.starts_with(prefix);
    }

    let with_slash = format!("{prefix}/");
    path.starts_with(&with_slash)
}

fn normalize(path: &str) -> String {
    let replaced = path.trim().replace('\\', "/");
    let without_prefix = replaced
        .strip_prefix("./")
        .map_or(replaced.as_str(), |value| value);
    without_prefix.trim_start_matches('/').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architecture_guard_denies_immutable_core_mutations() {
        let policy = ArchitectureBoundaryPolicy::default();
        let paths = [
            "crates/kernel/src/kernel.rs",
            "crates/kernel/src/contracts.rs",
            "examples/spec/runtime-extension.json",
        ];

        let report = policy.evaluate_paths(&paths);
        assert_eq!(report.total_paths, 3);
        assert!(report
            .denied_paths
            .contains(&"crates/kernel/src/kernel.rs".to_owned()));
        assert!(report
            .denied_paths
            .contains(&"crates/kernel/src/contracts.rs".to_owned()));
        assert!(report
            .allowed_paths
            .contains(&"examples/spec/runtime-extension.json".to_owned()));
        assert!(report.has_denials());
    }

    #[test]
    fn architecture_guard_denies_unknown_paths_by_default() {
        let policy = ArchitectureBoundaryPolicy::default();
        let report = policy.evaluate_paths(&["scripts/internal/unsafe.sh"]);

        assert_eq!(report.total_paths, 1);
        assert!(report
            .denied_paths
            .contains(&"scripts/internal/unsafe.sh".to_owned()));
        assert!(report
            .unknown_paths
            .contains(&"scripts/internal/unsafe.sh".to_owned()));
    }

    #[test]
    fn architecture_guard_allows_extension_mutations() {
        let policy = ArchitectureBoundaryPolicy::default();
        let report = policy.evaluate_paths(&[
            "./crates/daemon/src/main.rs",
            "docs/layered-kernel-design.md",
            "examples/spec/plugin-scan-hotplug.json",
        ]);

        assert_eq!(report.denied_paths.len(), 0);
        assert_eq!(report.allowed_paths.len(), 3);
        assert!(!report.has_denials());
    }
}
