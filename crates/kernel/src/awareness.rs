use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    architecture::{ArchitectureBoundaryPolicy, ArchitectureGuardReport},
    errors::IntegrationError,
    plugin::{PluginScanReport, PluginScanner},
    plugin_ir::{
        BridgeSupportMatrix, PluginActivationInventoryEntry, PluginActivationPlan, PluginIR,
        PluginSetupReadinessContext, PluginTranslationReport, PluginTranslator,
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodebaseAwarenessConfig {
    pub roots: Vec<String>,
    pub plugin_roots: Vec<String>,
    pub proposed_mutations: Vec<String>,
    pub architecture_policy: ArchitectureBoundaryPolicy,
}

impl Default for CodebaseAwarenessConfig {
    fn default() -> Self {
        Self {
            roots: vec![".".to_owned()],
            plugin_roots: Vec::new(),
            proposed_mutations: Vec::new(),
            architecture_policy: ArchitectureBoundaryPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CodebaseAwarenessSnapshot {
    pub scanned_roots: Vec<String>,
    pub scanned_files: usize,
    pub language_distribution: BTreeMap<String, usize>,
    pub deterministic_fingerprint: String,
    pub plugin_scan_reports: Vec<PluginScanReport>,
    pub plugin_translation_reports: Vec<PluginTranslationReport>,
    pub plugin_activation_reports: Vec<PluginActivationPlan>,
    pub plugin_inventory: Vec<PluginIR>,
    pub plugin_activation_inventory: Vec<PluginActivationInventoryEntry>,
    pub architecture_guard: ArchitectureGuardReport,
}

#[derive(Debug, Clone)]
struct FileFact {
    path: String,
    size_bytes: u64,
    language: String,
}

#[derive(Debug, Default)]
pub struct CodebaseAwarenessEngine {
    plugin_scanner: PluginScanner,
    plugin_translator: PluginTranslator,
}

impl CodebaseAwarenessEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            plugin_scanner: PluginScanner::new(),
            plugin_translator: PluginTranslator::new(),
        }
    }

    pub fn snapshot(
        &self,
        config: &CodebaseAwarenessConfig,
    ) -> Result<CodebaseAwarenessSnapshot, IntegrationError> {
        let roots = if config.roots.is_empty() {
            vec![".".to_owned()]
        } else {
            config.roots.clone()
        };

        let mut file_facts = Vec::new();
        for root in &roots {
            let root_path = PathBuf::from(root);
            if !root_path.exists() {
                return Err(IntegrationError::AwarenessRootNotFound(root.to_owned()));
            }
            collect_file_facts(&root_path, &mut file_facts)?;
        }

        file_facts.sort_by(|left, right| left.path.cmp(&right.path));

        let mut language_distribution = BTreeMap::new();
        for fact in &file_facts {
            *language_distribution
                .entry(fact.language.clone())
                .or_insert(0) += 1;
        }

        let plugin_roots = if config.plugin_roots.is_empty() {
            roots.clone()
        } else {
            config.plugin_roots.clone()
        };

        let mut plugin_scan_reports = Vec::new();
        let mut plugin_translation_reports = Vec::new();
        let mut plugin_activation_reports = Vec::new();
        let mut plugin_inventory = Vec::new();
        let mut plugin_activation_inventory = Vec::new();
        let bridge_matrix = BridgeSupportMatrix::default();
        let setup_readiness_context = PluginSetupReadinessContext::default();

        for root in &plugin_roots {
            let report = self.plugin_scanner.scan_path(root)?;
            let translation = self.plugin_translator.translate_scan_report(&report);
            let activation = self.plugin_translator.plan_activation(
                &translation,
                &bridge_matrix,
                &setup_readiness_context,
            );
            plugin_inventory.extend(translation.entries.iter().cloned());
            plugin_activation_inventory.extend(activation.inventory_entries(&translation));
            plugin_scan_reports.push(report);
            plugin_translation_reports.push(translation);
            plugin_activation_reports.push(activation);
        }

        let architecture_guard = config
            .architecture_policy
            .evaluate_paths(&config.proposed_mutations);

        Ok(CodebaseAwarenessSnapshot {
            scanned_roots: roots,
            scanned_files: file_facts.len(),
            language_distribution,
            deterministic_fingerprint: fingerprint(&file_facts),
            plugin_scan_reports,
            plugin_translation_reports,
            plugin_activation_reports,
            plugin_inventory,
            plugin_activation_inventory,
            architecture_guard,
        })
    }
}

fn collect_file_facts(path: &Path, acc: &mut Vec<FileFact>) -> Result<(), IntegrationError> {
    let metadata = fs::metadata(path).map_err(|error| IntegrationError::AwarenessFileRead {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;

    if metadata.is_file() {
        let normalized_path = normalize_path(path);
        acc.push(FileFact {
            language: detect_language(path),
            path: normalized_path,
            size_bytes: metadata.len(),
        });
        return Ok(());
    }

    for entry in fs::read_dir(path).map_err(|error| IntegrationError::AwarenessFileRead {
        path: path.display().to_string(),
        reason: error.to_string(),
    })? {
        let entry = entry.map_err(|error| IntegrationError::AwarenessFileRead {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
        let child = entry.path();

        if child.is_dir() {
            if should_skip_dir(&child) {
                continue;
            }
            collect_file_facts(&child, acc)?;
        } else if child.is_file() {
            let child_metadata =
                fs::metadata(&child).map_err(|error| IntegrationError::AwarenessFileRead {
                    path: child.display().to_string(),
                    reason: error.to_string(),
                })?;
            acc.push(FileFact {
                language: detect_language(&child),
                path: normalize_path(&child),
                size_bytes: child_metadata.len(),
            });
        }
    }

    Ok(())
}

fn detect_language(path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_else(|| "unknown".to_owned());

    match extension.as_str() {
        "rs" => "rust".to_owned(),
        "py" => "python".to_owned(),
        "go" => "go".to_owned(),
        "js" => "javascript".to_owned(),
        "ts" => "typescript".to_owned(),
        "toml" => "toml".to_owned(),
        "md" => "markdown".to_owned(),
        "json" => "json".to_owned(),
        "yaml" | "yml" => "yaml".to_owned(),
        other => other.to_owned(),
    }
}

fn normalize_path(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_owned()
}

fn should_skip_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".git" | "target" | "node_modules" | ".venv" | ".idea" | ".codex")
    )
}

fn fingerprint(file_facts: &[FileFact]) -> String {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    let mut hash = OFFSET_BASIS;
    for fact in file_facts {
        let row = format!("{}:{}:{}\n", fact.path, fact.size_bytes, fact.language);
        for byte in row.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_tmp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        std::env::temp_dir().join(format!("{}-{}", prefix, nanos))
    }

    #[test]
    fn awareness_snapshot_captures_languages_plugins_and_guard() {
        let root = unique_tmp_dir("loong-awareness");
        fs::create_dir_all(&root).expect("create temp root");

        fs::write(root.join("runtime.rs"), "pub fn run() {}\n").expect("write rust file");
        fs::write(root.join("agent.py"), "print('hello')\n").expect("write python file");
        fs::write(
            root.join("plugin.rs"),
            r#"
// LOONG_PLUGIN_START
// {
//   "plugin_id": "openrouter-rs",
//   "provider_id": "openrouter",
//   "connector_name": "openrouter",
//   "channel_id": "primary",
//   "endpoint": "https://openrouter.ai/api/v1/chat/completions",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"version":"0.5.0"}
// }
// LOONG_PLUGIN_END
"#,
        )
        .expect("write plugin file");

        let engine = CodebaseAwarenessEngine::new();
        let snapshot = engine
            .snapshot(&CodebaseAwarenessConfig {
                roots: vec![root.display().to_string()],
                plugin_roots: vec![root.display().to_string()],
                proposed_mutations: vec!["examples/spec/runtime-extension.json".to_owned()],
                architecture_policy: ArchitectureBoundaryPolicy::default(),
            })
            .expect("awareness snapshot should succeed");

        assert_eq!(snapshot.scanned_roots.len(), 1);
        assert_eq!(snapshot.plugin_inventory.len(), 1);
        assert_eq!(snapshot.plugin_activation_reports.len(), 1);
        assert_eq!(snapshot.plugin_activation_inventory.len(), 1);
        assert_eq!(
            snapshot.plugin_activation_inventory[0]
                .activation_status
                .map(|status| status.as_str().to_owned()),
            Some("ready".to_owned())
        );
        assert!(
            snapshot
                .language_distribution
                .get("rust")
                .copied()
                .unwrap_or(0)
                >= 1
        );
        assert!(
            snapshot
                .language_distribution
                .get("python")
                .copied()
                .unwrap_or(0)
                >= 1
        );
        assert!(!snapshot.architecture_guard.has_denials());
        assert!(!snapshot.deterministic_fingerprint.is_empty());
    }

    #[test]
    fn awareness_snapshot_detects_guard_violations() {
        let root = unique_tmp_dir("loong-awareness-guard");
        fs::create_dir_all(&root).expect("create temp root");
        fs::write(root.join("main.rs"), "fn main() {}\n").expect("write rust file");

        let engine = CodebaseAwarenessEngine::new();
        let snapshot = engine
            .snapshot(&CodebaseAwarenessConfig {
                roots: vec![root.display().to_string()],
                plugin_roots: Vec::new(),
                proposed_mutations: vec!["crates/kernel/src/kernel.rs".to_owned()],
                architecture_policy: ArchitectureBoundaryPolicy::default(),
            })
            .expect("awareness snapshot should succeed");

        assert!(snapshot.architecture_guard.has_denials());
        assert!(
            snapshot
                .architecture_guard
                .denied_paths
                .contains(&"crates/kernel/src/kernel.rs".to_owned())
        );
    }

    #[test]
    fn awareness_snapshot_skips_target_directory_noise() {
        let root = unique_tmp_dir("loong-awareness-skip");
        fs::create_dir_all(root.join("target")).expect("create target directory");
        fs::write(root.join("target").join("build.bin"), [0_u8, 159, 146, 150])
            .expect("write binary");
        fs::write(root.join("lib.rs"), "pub fn stable() {}\n").expect("write rust file");

        let engine = CodebaseAwarenessEngine::new();
        let snapshot = engine
            .snapshot(&CodebaseAwarenessConfig {
                roots: vec![root.display().to_string()],
                plugin_roots: vec![root.display().to_string()],
                proposed_mutations: Vec::new(),
                architecture_policy: ArchitectureBoundaryPolicy::default(),
            })
            .expect("awareness snapshot should succeed");

        assert_eq!(snapshot.scanned_files, 1);
        assert_eq!(snapshot.language_distribution.get("rust").copied(), Some(1));
    }

    #[test]
    fn awareness_snapshot_projects_plugin_activation_inventory_with_slot_conflicts() {
        let root = unique_tmp_dir("loong-awareness-slots");
        fs::create_dir_all(&root).expect("create temp root");

        fs::write(
            root.join("first.py"),
            r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "search-a",
#   "provider_id": "search-a",
#   "connector_name": "search-a",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/a",
#   "capabilities": ["InvokeConnector"],
#   "slot_claims": [{"slot":"provider:web_search","key":"default","mode":"exclusive"}],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONG_PLUGIN_END
"#,
        )
        .expect("write first plugin");
        fs::write(
            root.join("second.py"),
            r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "search-b",
#   "provider_id": "search-b",
#   "connector_name": "search-b",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/b",
#   "capabilities": ["InvokeConnector"],
#   "slot_claims": [{"slot":"provider:web_search","key":"default","mode":"exclusive"}],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONG_PLUGIN_END
"#,
        )
        .expect("write second plugin");

        let engine = CodebaseAwarenessEngine::new();
        let snapshot = engine
            .snapshot(&CodebaseAwarenessConfig {
                roots: vec![root.display().to_string()],
                plugin_roots: vec![root.display().to_string()],
                proposed_mutations: Vec::new(),
                architecture_policy: ArchitectureBoundaryPolicy::default(),
            })
            .expect("awareness snapshot should succeed");

        assert_eq!(snapshot.plugin_activation_reports.len(), 1);
        assert_eq!(snapshot.plugin_activation_reports[0].blocked_plugins, 2);
        assert_eq!(snapshot.plugin_activation_inventory.len(), 2);
        assert!(snapshot.plugin_activation_inventory.iter().all(|entry| {
            entry
                .activation_status
                .is_some_and(|status| status.as_str() == "blocked_slot_claim_conflict")
        }));
    }
}
