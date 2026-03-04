use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    contracts::Capability,
    errors::IntegrationError,
    integration::{AutoProvisionRequest, ChannelConfig, IntegrationCatalog, ProviderConfig},
    pack::VerticalPackManifest,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub plugin_id: String,
    pub provider_id: String,
    pub connector_name: String,
    pub channel_id: Option<String>,
    pub endpoint: Option<String>,
    pub capabilities: BTreeSet<Capability>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub path: String,
    pub language: String,
    pub manifest: PluginManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginScanReport {
    pub scanned_files: usize,
    pub matched_plugins: usize,
    pub descriptors: Vec<PluginDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginAbsorbReport {
    pub absorbed_plugins: usize,
    pub provider_upserts: usize,
    pub channel_upserts: usize,
    pub connectors_added_to_pack: BTreeSet<String>,
    pub capabilities_added_to_pack: BTreeSet<Capability>,
}

#[derive(Debug, Default)]
pub struct PluginScanner;

impl PluginScanner {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    pub fn scan_path<P: AsRef<Path>>(&self, root: P) -> Result<PluginScanReport, IntegrationError> {
        let root = root.as_ref();
        if !root.exists() {
            return Err(IntegrationError::PluginScanRootNotFound(
                root.display().to_string(),
            ));
        }

        let mut report = PluginScanReport::default();
        let mut files = Vec::new();
        collect_files(root, &mut files)?;
        files.sort();

        for path in files {
            report.scanned_files = report.scanned_files.saturating_add(1);
            let bytes = fs::read(&path).map_err(|error| IntegrationError::PluginFileRead {
                path: path.display().to_string(),
                reason: error.to_string(),
            })?;
            let content = match String::from_utf8(bytes) {
                Ok(content) => content,
                Err(_) => continue,
            };

            if let Some(manifest) = parse_manifest_block(&content, &path)? {
                report.matched_plugins = report.matched_plugins.saturating_add(1);
                report.descriptors.push(PluginDescriptor {
                    path: path.display().to_string(),
                    language: detect_language(&path),
                    manifest,
                });
            }
        }

        Ok(report)
    }

    pub fn absorb(
        &self,
        catalog: &mut IntegrationCatalog,
        pack: &mut VerticalPackManifest,
        report: &PluginScanReport,
    ) -> PluginAbsorbReport {
        let mut absorbed = PluginAbsorbReport::default();

        for descriptor in &report.descriptors {
            let manifest = &descriptor.manifest;

            catalog.upsert_provider(ProviderConfig {
                provider_id: manifest.provider_id.clone(),
                connector_name: manifest.connector_name.clone(),
                version: manifest
                    .metadata
                    .get("version")
                    .cloned()
                    .unwrap_or_else(|| "0.1.0".to_owned()),
                metadata: manifest.metadata.clone(),
            });
            absorbed.provider_upserts = absorbed.provider_upserts.saturating_add(1);

            if let Some(channel_id) = &manifest.channel_id {
                catalog.upsert_channel(ChannelConfig {
                    channel_id: channel_id.clone(),
                    provider_id: manifest.provider_id.clone(),
                    endpoint: manifest.endpoint.clone().unwrap_or_else(|| {
                        format!("https://{}.local/{channel_id}/invoke", manifest.provider_id)
                    }),
                    enabled: true,
                    metadata: BTreeMap::from([(
                        "source_plugin".to_owned(),
                        manifest.plugin_id.clone(),
                    )]),
                });
                absorbed.channel_upserts = absorbed.channel_upserts.saturating_add(1);
            }

            if pack
                .allowed_connectors
                .insert(manifest.connector_name.clone())
            {
                absorbed
                    .connectors_added_to_pack
                    .insert(manifest.connector_name.clone());
            }

            if pack
                .granted_capabilities
                .insert(Capability::InvokeConnector)
            {
                absorbed
                    .capabilities_added_to_pack
                    .insert(Capability::InvokeConnector);
            }

            for capability in &manifest.capabilities {
                if pack.granted_capabilities.insert(*capability) {
                    absorbed.capabilities_added_to_pack.insert(*capability);
                }
            }

            absorbed.absorbed_plugins = absorbed.absorbed_plugins.saturating_add(1);
        }

        absorbed
    }

    #[must_use]
    pub fn to_auto_provision_requests(
        &self,
        report: &PluginScanReport,
    ) -> Vec<AutoProvisionRequest> {
        report
            .descriptors
            .iter()
            .map(|descriptor| AutoProvisionRequest {
                provider_id: descriptor.manifest.provider_id.clone(),
                channel_id: descriptor
                    .manifest
                    .channel_id
                    .clone()
                    .unwrap_or_else(|| format!("{}-default", descriptor.manifest.provider_id)),
                connector_name: Some(descriptor.manifest.connector_name.clone()),
                endpoint: descriptor.manifest.endpoint.clone(),
                required_capabilities: descriptor.manifest.capabilities.clone(),
            })
            .collect()
    }
}

fn collect_files(path: &Path, acc: &mut Vec<PathBuf>) -> Result<(), IntegrationError> {
    let metadata = fs::metadata(path).map_err(|error| IntegrationError::PluginFileRead {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;

    if metadata.is_file() {
        acc.push(path.to_path_buf());
        return Ok(());
    }

    for entry in fs::read_dir(path).map_err(|error| IntegrationError::PluginFileRead {
        path: path.display().to_string(),
        reason: error.to_string(),
    })? {
        let entry = entry.map_err(|error| IntegrationError::PluginFileRead {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
        let child = entry.path();
        if child.is_dir() {
            if should_skip_dir(&child) {
                continue;
            }
            collect_files(&child, acc)?;
        } else if child.is_file() {
            acc.push(child);
        }
    }
    Ok(())
}

fn should_skip_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".git" | "target" | "node_modules" | ".venv" | ".idea" | ".codex")
    )
}

fn parse_manifest_block(
    content: &str,
    path: &Path,
) -> Result<Option<PluginManifest>, IntegrationError> {
    const START: &str = "CHUMOS_PLUGIN_START";
    const END: &str = "CHUMOS_PLUGIN_END";

    let Some(start_idx) = content.find(START) else {
        return Ok(None);
    };
    let Some(end_idx) = content[start_idx..].find(END).map(|idx| start_idx + idx) else {
        return Err(IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: "missing CHUMOS_PLUGIN_END".to_owned(),
        });
    };

    let block = &content[start_idx + START.len()..end_idx];
    let cleaned = block
        .lines()
        .map(clean_manifest_line)
        .collect::<Vec<_>>()
        .join("\n");

    let manifest: PluginManifest = serde_json::from_str(cleaned.trim()).map_err(|error| {
        IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: error.to_string(),
        }
    })?;

    Ok(Some(manifest))
}

fn clean_manifest_line(line: &str) -> String {
    let trimmed = line.trim_start();
    for prefix in ["//", "#", "--", ";", "/*", "*", "*/"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim_start().to_owned();
        }
    }
    trimmed.to_owned()
}

fn detect_language(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .unwrap_or_else(|| "unknown".to_owned())
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

    fn sample_pack() -> VerticalPackManifest {
        VerticalPackManifest {
            pack_id: "sample-pack".to_owned(),
            domain: "engineering".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: crate::contracts::ExecutionRoute {
                harness_kind: crate::contracts::HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn scanner_finds_manifest_in_rust_and_python_files() {
        let root = unique_tmp_dir("chumos-plugin-scan");
        fs::create_dir_all(&root).expect("create temp root");

        let rust_file = root.join("openrouter.rs");
        fs::write(
            &rust_file,
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "openrouter-rs",
//   "provider_id": "openrouter",
//   "connector_name": "openrouter",
//   "channel_id": "primary",
//   "endpoint": "https://openrouter.ai/api/v1/chat/completions",
//   "capabilities": ["InvokeConnector", "ObserveTelemetry"],
//   "metadata": {"version":"0.2.0","lang":"rust"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write rust plugin");

        let py_file = root.join("slack_plugin.py");
        fs::write(
            &py_file,
            r#"
# CHUMOS_PLUGIN_START
# {
#   "plugin_id": "slack-py",
#   "provider_id": "slack",
#   "connector_name": "slack",
#   "channel_id": "alerts",
#   "endpoint": "https://hooks.slack.com/services/aaa/bbb/ccc",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"version":"1.1.0","lang":"python"}
# }
# CHUMOS_PLUGIN_END
"#,
        )
        .expect("write python plugin");

        let scanner = PluginScanner::new();
        let report = scanner.scan_path(&root).expect("scan should succeed");
        assert_eq!(report.matched_plugins, 2);
        assert!(report
            .descriptors
            .iter()
            .any(|descriptor| descriptor.manifest.provider_id == "openrouter"));
        assert!(report
            .descriptors
            .iter()
            .any(|descriptor| descriptor.manifest.provider_id == "slack"));
    }

    #[test]
    fn scanner_absorbs_plugins_into_catalog_and_pack() {
        let report = PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            descriptors: vec![PluginDescriptor {
                path: "/tmp/openai.rs".to_owned(),
                language: "rs".to_owned(),
                manifest: PluginManifest {
                    plugin_id: "openai-rs".to_owned(),
                    provider_id: "openai".to_owned(),
                    connector_name: "openai".to_owned(),
                    channel_id: Some("chat-main".to_owned()),
                    endpoint: Some("https://api.openai.com/v1/chat/completions".to_owned()),
                    capabilities: BTreeSet::from([
                        Capability::InvokeConnector,
                        Capability::ObserveTelemetry,
                    ]),
                    metadata: BTreeMap::from([("version".to_owned(), "1.3.0".to_owned())]),
                },
            }],
        };

        let mut catalog = IntegrationCatalog::new();
        let mut pack = sample_pack();
        let scanner = PluginScanner::new();

        let absorb = scanner.absorb(&mut catalog, &mut pack, &report);
        assert_eq!(absorb.absorbed_plugins, 1);
        assert_eq!(absorb.provider_upserts, 1);
        assert_eq!(absorb.channel_upserts, 1);
        assert!(catalog.provider("openai").is_some());
        assert!(catalog.channel("chat-main").is_some());
        assert!(pack.allowed_connectors.contains("openai"));
        assert!(pack
            .granted_capabilities
            .contains(&Capability::InvokeConnector));
    }

    #[test]
    fn scanner_skips_non_utf8_files_instead_of_failing() {
        let root = unique_tmp_dir("chumos-plugin-binary");
        fs::create_dir_all(&root).expect("create temp root");
        let binary = root.join("compiled.bin");
        fs::write(&binary, [0xff_u8, 0xfe, 0x00, 0x81]).expect("write binary file");

        let scanner = PluginScanner::new();
        let report = scanner
            .scan_path(&root)
            .expect("binary files should be skipped, not fail");
        assert_eq!(report.scanned_files, 1);
        assert_eq!(report.matched_plugins, 0);
    }
}
