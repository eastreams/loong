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

fn scan_diagnostic<'a>(
    report: &'a PluginScanReport,
    code: PluginDiagnosticCode,
    plugin_id: &str,
) -> Option<&'a PluginDiagnosticFinding> {
    report
        .diagnostic_findings
        .iter()
        .find(|finding| finding.code == code && finding.plugin_id.as_deref() == Some(plugin_id))
}

#[test]
fn scanner_finds_manifest_in_rust_and_python_files() {
    let root = unique_tmp_dir("loong-plugin-scan");
    fs::create_dir_all(&root).expect("create temp root");

    let rust_file = root.join("openrouter.rs");
    fs::write(
        &rust_file,
        r#"
// LOONG_PLUGIN_START
// {
//   "plugin_id": "openrouter-rs",
//   "provider_id": "openrouter",
//   "connector_name": "openrouter",
//   "channel_id": "primary",
//   "endpoint": "https://openrouter.ai/api/v1/chat/completions",
//   "capabilities": ["InvokeConnector", "ObserveTelemetry"],
//   "metadata": {"version":"0.2.0","lang":"rust"}
// }
// LOONG_PLUGIN_END
"#,
    )
    .expect("write rust plugin");

    let py_file = root.join("slack_plugin.py");
    fs::write(
        &py_file,
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "slack-py",
#   "provider_id": "slack",
#   "connector_name": "slack",
#   "channel_id": "alerts",
#   "endpoint": "https://hooks.slack.com/services/aaa/bbb/ccc",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"version":"1.1.0","lang":"python"}
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write python plugin");

    let scanner = PluginScanner::new();
    let report = scanner.scan_path(&root).expect("scan should succeed");
    assert_eq!(report.matched_plugins, 2);
    assert!(
        report
            .descriptors
            .iter()
            .any(|descriptor| descriptor.manifest.provider_id == "openrouter")
    );
    assert!(
        report
            .descriptors
            .iter()
            .all(|descriptor| descriptor.source_kind == PluginSourceKind::EmbeddedSource)
    );
    assert!(
        report
            .descriptors
            .iter()
            .all(|descriptor| descriptor.package_manifest_path.is_none())
    );
    assert!(
        report.descriptors.iter().all(|descriptor| matches!(
            descriptor.manifest.trust_tier,
            PluginTrustTier::Unverified
        ))
    );
    assert!(
        report
            .descriptors
            .iter()
            .any(|descriptor| descriptor.manifest.provider_id == "slack")
    );
    assert_eq!(
        report
            .diagnostic_findings
            .iter()
            .filter(|finding| finding.code == PluginDiagnosticCode::EmbeddedSourceLegacyContract)
            .count(),
        2
    );
    assert_eq!(
        report
            .diagnostic_findings
            .iter()
            .filter(|finding| finding.code == PluginDiagnosticCode::LegacyMetadataVersion)
            .count(),
        2
    );
}

#[test]
fn scanner_finds_package_manifest_file() {
    let root = unique_tmp_dir("loong-plugin-package-manifest");
    fs::create_dir_all(&root).expect("create temp root");

    let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &manifest_file,
        r#"
{
  "api_version": "v1alpha1",
  "plugin_id": "tavily-search",
  "version": "0.3.0",
  "provider_id": "tavily",
  "connector_name": "tavily-http",
  "endpoint": "https://api.tavily.com/search",
  "capabilities": ["InvokeConnector"],
  "trust_tier": "verified-community",
  "metadata": {
    "bridge_kind": "http_json",
    "adapter_family": "web-search"
  },
  "summary": "Manifest-discovered Tavily package",
  "tags": ["search", "provider"],
  "setup": {
    "mode": "metadata_only",
    "surface": " web_search ",
    "required_env_vars": ["TAVILY_API_KEY", " ", "TAVILY_API_KEY"],
    "recommended_env_vars": ["TEAM_TAVILY_KEY"],
    "required_config_keys": ["tools.web_search.default_provider"],
    "default_env_var": " TAVILY_API_KEY ",
    "docs_urls": ["https://docs.example.com/tavily", "https://docs.example.com/tavily"],
    "remediation": " set a Tavily credential before enabling search "
  }
}
"#,
    )
    .expect("write package manifest");

    let scanner = PluginScanner::new();
    let report = scanner.scan_path(&root).expect("scan should succeed");

    assert_eq!(report.scanned_files, 1);
    assert_eq!(report.matched_plugins, 1);
    assert_eq!(report.descriptors.len(), 1);
    assert_eq!(
        report.descriptors[0].path,
        manifest_file.display().to_string()
    );
    assert_eq!(report.descriptors[0].language, "manifest");
    assert_eq!(
        report.descriptors[0].manifest.api_version.as_deref(),
        Some(CURRENT_PLUGIN_MANIFEST_API_VERSION)
    );
    assert_eq!(
        report.descriptors[0].manifest.version.as_deref(),
        Some("0.3.0")
    );
    assert_eq!(report.descriptors[0].manifest.plugin_id, "tavily-search");
    assert_eq!(report.descriptors[0].manifest.provider_id, "tavily");
    assert_eq!(
        report.descriptors[0]
            .manifest
            .metadata
            .get("version")
            .map(String::as_str),
        Some("0.3.0")
    );
    assert_eq!(
        report.descriptors[0].source_kind,
        PluginSourceKind::PackageManifest
    );
    assert_eq!(
        report.descriptors[0].package_root,
        root.display().to_string()
    );
    assert_eq!(
        report.descriptors[0].package_manifest_path,
        Some(manifest_file.display().to_string())
    );
    assert_eq!(
        report.descriptors[0].manifest.trust_tier,
        PluginTrustTier::VerifiedCommunity
    );
    assert_eq!(
        report.descriptors[0].manifest.setup,
        Some(PluginSetup {
            mode: PluginSetupMode::MetadataOnly,
            surface: Some("web_search".to_owned()),
            required_env_vars: vec!["TAVILY_API_KEY".to_owned()],
            recommended_env_vars: vec!["TEAM_TAVILY_KEY".to_owned()],
            required_config_keys: vec!["tools.web_search.default_provider".to_owned()],
            default_env_var: Some("TAVILY_API_KEY".to_owned()),
            docs_urls: vec!["https://docs.example.com/tavily".to_owned()],
            remediation: Some("set a Tavily credential before enabling search".to_owned()),
        })
    );
}

#[test]
fn scanner_requires_api_version_for_package_manifest() {
    let root = unique_tmp_dir("loong-plugin-package-api-required");
    fs::create_dir_all(&root).expect("create temp root");

    let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &manifest_file,
        r#"
{
  "version": "1.0.0",
  "plugin_id": "missing-api-version",
  "provider_id": "missing-api-version",
  "connector_name": "missing-api-version",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
    )
    .expect("write package manifest");

    let error = PluginScanner::new()
        .scan_path(&root)
        .expect_err("package manifests must declare api_version");

    let rendered = error.to_string();
    assert!(rendered.contains("api_version"));
    assert!(rendered.contains("package manifest"));
}

#[test]
fn scanner_requires_top_level_version_for_package_manifest() {
    let root = unique_tmp_dir("loong-plugin-package-version-required");
    fs::create_dir_all(&root).expect("create temp root");

    let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &manifest_file,
        r#"
{
  "api_version": "v1alpha1",
  "plugin_id": "missing-version",
  "provider_id": "missing-version",
  "connector_name": "missing-version",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
    )
    .expect("write package manifest");

    let error = PluginScanner::new()
        .scan_path(&root)
        .expect_err("package manifests must declare top-level version");

    let rendered = error.to_string();
    assert!(rendered.contains("top-level version"));
    assert!(rendered.contains("package manifest"));
}

#[test]
fn scanner_rejects_legacy_version_metadata_in_package_manifest() {
    let root = unique_tmp_dir("loong-plugin-package-legacy-version");
    fs::create_dir_all(&root).expect("create temp root");

    let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &manifest_file,
        r#"
{
  "api_version": "v1alpha1",
  "version": "1.2.3",
  "plugin_id": "legacy-version-metadata",
  "provider_id": "legacy-version-metadata",
  "connector_name": "legacy-version-metadata",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json",
    "version": "1.2.3"
  }
}
"#,
    )
    .expect("write package manifest");

    let error = PluginScanner::new()
        .scan_path(&root)
        .expect_err("package manifests should reject metadata.version");

    let rendered = error.to_string();
    assert!(rendered.contains("metadata.version"));
    assert!(rendered.contains("top-level `version`"));
}

#[test]
fn scanner_rejects_reserved_metadata_namespace_in_package_manifest() {
    let root = unique_tmp_dir("loong-plugin-package-reserved-metadata");
    fs::create_dir_all(&root).expect("create temp root");

    let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &manifest_file,
        r#"
{
  "api_version": "v1alpha1",
  "version": "1.2.3",
  "plugin_id": "reserved-metadata",
  "provider_id": "reserved-metadata",
  "connector_name": "reserved-metadata",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json",
    "plugin_version": "1.2.3"
  }
}
"#,
    )
    .expect("write package manifest");

    let error = PluginScanner::new()
        .scan_path(&root)
        .expect_err("package manifests should reject reserved metadata namespace");

    let rendered = error.to_string();
    assert!(rendered.contains("plugin_version"));
    assert!(rendered.contains("reserved"));
}

#[test]
fn scanner_rejects_invalid_top_level_plugin_version() {
    let root = unique_tmp_dir("loong-plugin-invalid-version");
    fs::create_dir_all(&root).expect("create temp root");

    let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &manifest_file,
        r#"
{
  "api_version": "v1alpha1",
  "version": "not-a-semver",
  "plugin_id": "bad-version",
  "provider_id": "bad-version",
  "connector_name": "bad-version",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
    )
    .expect("write package manifest");

    let error = PluginScanner::new()
        .scan_path(&root)
        .expect_err("invalid plugin version should fail parse");

    let rendered = error.to_string();
    assert!(rendered.contains("invalid semver"));
    assert!(rendered.contains("not-a-semver"));
}

#[test]
fn scanner_rejects_conflicting_top_level_and_metadata_version_in_source_manifest() {
    let root = unique_tmp_dir("loong-plugin-source-version-conflict");
    fs::create_dir_all(&root).expect("create temp root");

    let source_file = root.join("plugin.py");
    fs::write(
        &source_file,
        r#"
# LOONG_PLUGIN_START
# {
#   "version": "1.2.3",
#   "plugin_id": "source-version-conflict",
#   "provider_id": "source-version-conflict",
#   "connector_name": "source-version-conflict",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind": "http_json",
#     "version": "9.9.9"
#   }
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write source manifest");

    let error = PluginScanner::new()
        .scan_path(&root)
        .expect_err("source manifests should reject conflicting version truth");

    let rendered = error.to_string();
    assert!(rendered.contains("plugin version conflict"));
    assert!(rendered.contains("1.2.3"));
    assert!(rendered.contains("9.9.9"));
}

#[test]
fn scanner_rejects_unknown_package_manifest_fields() {
    let root = unique_tmp_dir("loong-plugin-unknown-package-field");
    fs::create_dir_all(&root).expect("create temp root");

    let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &manifest_file,
        r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "unknown-field",
  "provider_id": "unknown-field",
  "connector_name": "unknown-field",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  },
  "slot_claim": []
}
"#,
    )
    .expect("write package manifest");

    let error = PluginScanner::new()
        .scan_path(&root)
        .expect_err("unknown package manifest fields should fail parse");

    let rendered = error.to_string();
    assert!(rendered.contains("unknown field"));
    assert!(rendered.contains("slot_claim"));
}

#[test]
fn scanner_prefers_package_manifest_over_embedded_source_manifest() {
    let root = unique_tmp_dir("loong-plugin-precedence");
    let package_root = root.join("pkg");
    fs::create_dir_all(&package_root).expect("create temp root");

    let manifest_file = package_root.join(PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &manifest_file,
        r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "package-plugin",
  "provider_id": "package-provider",
  "connector_name": "package-connector",
  "channel_id": "package-channel",
  "endpoint": "https://package.example/invoke",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
    )
    .expect("write package manifest");

    let source_file = package_root.join("plugin.py");
    fs::write(
        &source_file,
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "package-plugin",
#   "provider_id": "package-provider",
#   "connector_name": "package-connector",
#   "channel_id": "package-channel",
#   "endpoint": "https://package.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write source plugin");

    let scanner = PluginScanner::new();
    let report = scanner.scan_path(&root).expect("scan should succeed");

    assert_eq!(report.scanned_files, 2);
    assert_eq!(report.matched_plugins, 1);
    assert_eq!(report.descriptors.len(), 1);
    assert_eq!(
        report.descriptors[0].path,
        manifest_file.display().to_string()
    );
    assert_eq!(
        report.descriptors[0].source_kind,
        PluginSourceKind::PackageManifest
    );
    assert_eq!(
        report.descriptors[0].package_root,
        package_root.display().to_string()
    );
    assert_eq!(
        report.descriptors[0].package_manifest_path,
        Some(manifest_file.display().to_string())
    );
    assert_eq!(report.descriptors[0].manifest.plugin_id, "package-plugin");
    assert_eq!(
        report.descriptors[0].manifest.provider_id,
        "package-provider"
    );
    let finding = scan_diagnostic(
        &report,
        PluginDiagnosticCode::ShadowedEmbeddedSource,
        "package-plugin",
    )
    .expect("shadowed embedded source finding");
    assert_eq!(finding.phase, PluginDiagnosticPhase::Scan);
    assert!(!finding.blocking);
}

#[test]
fn scanner_fails_when_package_manifest_conflicts_with_source_manifest() {
    let root = unique_tmp_dir("loong-plugin-conflict");
    let package_root = root.join("pkg");
    fs::create_dir_all(&package_root).expect("create temp root");

    let manifest_file = package_root.join(PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &manifest_file,
        r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "package-plugin",
  "provider_id": "package-provider",
  "connector_name": "package-connector",
  "channel_id": "package-channel",
  "endpoint": "https://package.example/invoke",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
    )
    .expect("write package manifest");

    let source_file = package_root.join("plugin.py");
    fs::write(
        &source_file,
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "package-plugin",
#   "provider_id": "source-provider",
#   "connector_name": "package-connector",
#   "channel_id": "package-channel",
#   "endpoint": "https://package.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write source plugin");

    let scanner = PluginScanner::new();
    let error = scanner
        .scan_path(&root)
        .expect_err("conflicting manifests should fail");

    assert_eq!(
        error,
        IntegrationError::PluginManifestConflict {
            package_manifest_path: manifest_file.display().to_string(),
            source_path: source_file.display().to_string(),
            field: "provider_id".to_owned(),
            package_value: "\"package-provider\"".to_owned(),
            source_value: "\"source-provider\"".to_owned(),
        }
    );
}

#[test]
fn scanner_uses_nearest_package_manifest_for_nested_package_roots() {
    let root = unique_tmp_dir("loong-plugin-nested-package-root");
    let outer_root = root.join("outer");
    let inner_root = outer_root.join("inner");
    fs::create_dir_all(&inner_root).expect("create nested root");

    let outer_manifest_file = outer_root.join(PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &outer_manifest_file,
        r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "outer-plugin",
  "provider_id": "outer-provider",
  "connector_name": "outer-connector",
  "channel_id": "outer-channel",
  "endpoint": "https://outer.example/invoke",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
    )
    .expect("write outer package manifest");

    let inner_manifest_file = inner_root.join(PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &inner_manifest_file,
        r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "inner-plugin",
  "provider_id": "inner-provider",
  "connector_name": "inner-connector",
  "channel_id": "inner-channel",
  "endpoint": "https://inner.example/invoke",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
    )
    .expect("write inner package manifest");

    let source_file = inner_root.join("plugin.py");
    fs::write(
        &source_file,
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "inner-plugin",
#   "provider_id": "inner-provider",
#   "connector_name": "inner-connector",
#   "channel_id": "inner-channel",
#   "endpoint": "https://inner.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write nested source plugin");

    let scanner = PluginScanner::new();
    let report = scanner.scan_path(&root).expect("scan should succeed");

    assert_eq!(report.matched_plugins, 2);
    assert_eq!(report.descriptors.len(), 2);
    assert!(
        report
            .descriptors
            .iter()
            .any(|descriptor| descriptor.path == outer_manifest_file.display().to_string())
    );
    assert!(
        report
            .descriptors
            .iter()
            .any(|descriptor| descriptor.path == inner_manifest_file.display().to_string())
    );
}

#[test]
fn scanner_allows_source_only_optional_fields_under_package_manifest() {
    let root = unique_tmp_dir("loong-plugin-optional-source-fields");
    let package_root = root.join("pkg");
    fs::create_dir_all(&package_root).expect("create temp root");

    let manifest_file = package_root.join(PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &manifest_file,
        r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "package-plugin",
  "provider_id": "package-provider",
  "connector_name": "package-connector",
  "channel_id": "package-channel",
  "endpoint": "https://package.example/invoke",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
    )
    .expect("write package manifest");

    let source_file = package_root.join("plugin.py");
    fs::write(
        &source_file,
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "package-plugin",
#   "provider_id": "package-provider",
#   "connector_name": "package-connector",
#   "channel_id": "package-channel",
#   "endpoint": "https://package.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","legacy_source":"true"},
#   "summary": "legacy source summary",
#   "tags": ["legacy", "source"],
#   "input_examples": [{"query":"hello"}]
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write source plugin");

    let scanner = PluginScanner::new();
    let report = scanner.scan_path(&root).expect("scan should succeed");

    assert_eq!(report.scanned_files, 2);
    assert_eq!(report.matched_plugins, 1);
    assert_eq!(report.descriptors.len(), 1);
    assert_eq!(
        report.descriptors[0].path,
        manifest_file.display().to_string()
    );
    assert_eq!(report.descriptors[0].manifest.summary, None);
    assert!(report.descriptors[0].manifest.tags.is_empty());
    assert!(report.descriptors[0].manifest.input_examples.is_empty());
    assert!(
        !report.descriptors[0]
            .manifest
            .metadata
            .contains_key("legacy_source")
    );
    assert_eq!(
        report.descriptors[0].manifest.provider_id,
        "package-provider"
    );
    assert_eq!(report.descriptors[0].language, "manifest");
    let finding = scan_diagnostic(
        &report,
        PluginDiagnosticCode::ShadowedEmbeddedSource,
        "package-plugin",
    )
    .expect("shadowed embedded source finding");
    assert_eq!(finding.phase, PluginDiagnosticPhase::Scan);
    assert!(!finding.blocking);
}

#[test]
fn scanner_falls_back_to_embedded_source_manifest_without_package_manifest() {
    let root = unique_tmp_dir("loong-plugin-source-fallback");
    let package_root = root.join("pkg");
    fs::create_dir_all(&package_root).expect("create temp root");

    let source_file = package_root.join("plugin.py");
    fs::write(
        &source_file,
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "source-plugin",
#   "provider_id": "source-provider",
#   "connector_name": "source-connector",
#   "channel_id": "source-channel",
#   "endpoint": "https://source.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"process_stdio"},
#   "setup": {
#     "surface": "channel",
#     "required_env_vars": ["SOURCE_TOKEN"],
#     "default_env_var": "SOURCE_TOKEN"
#   }
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write source plugin");

    let scanner = PluginScanner::new();
    let report = scanner.scan_path(&root).expect("scan should succeed");

    assert_eq!(report.scanned_files, 1);
    assert_eq!(report.matched_plugins, 1);
    assert_eq!(report.descriptors.len(), 1);
    assert_eq!(
        report.descriptors[0].path,
        source_file.display().to_string()
    );
    assert_eq!(
        report.descriptors[0].source_kind,
        PluginSourceKind::EmbeddedSource
    );
    assert_eq!(
        report.descriptors[0].package_root,
        package_root.display().to_string()
    );
    assert_eq!(report.descriptors[0].package_manifest_path, None);
    assert_eq!(report.descriptors[0].language, "py");
    assert_eq!(report.descriptors[0].manifest.plugin_id, "source-plugin");
    assert_eq!(
        report.descriptors[0].manifest.provider_id,
        "source-provider"
    );
    let finding = scan_diagnostic(
        &report,
        PluginDiagnosticCode::EmbeddedSourceLegacyContract,
        "source-plugin",
    )
    .expect("embedded source legacy finding");
    assert_eq!(finding.phase, PluginDiagnosticPhase::Scan);
    assert!(!finding.blocking);
    assert_eq!(
        report.descriptors[0].manifest.setup,
        Some(PluginSetup {
            mode: PluginSetupMode::MetadataOnly,
            surface: Some("channel".to_owned()),
            required_env_vars: vec!["SOURCE_TOKEN".to_owned()],
            recommended_env_vars: Vec::new(),
            required_config_keys: Vec::new(),
            default_env_var: Some("SOURCE_TOKEN".to_owned()),
            docs_urls: Vec::new(),
            remediation: None,
        })
    );
}

#[test]
fn scanner_treats_empty_metadata_only_setup_as_absent() {
    let root = unique_tmp_dir("loong-plugin-empty-setup");
    let package_root = root.join("pkg");
    fs::create_dir_all(&package_root).expect("create temp root");

    let manifest_file = package_root.join(PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &manifest_file,
        r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "package-plugin",
  "provider_id": "package-provider",
  "connector_name": "package-connector",
  "channel_id": "package-channel",
  "endpoint": "https://package.example/invoke",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
    )
    .expect("write package manifest");

    let source_file = package_root.join("plugin.py");
    fs::write(
        &source_file,
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "package-plugin",
#   "provider_id": "package-provider",
#   "connector_name": "package-connector",
#   "channel_id": "package-channel",
#   "endpoint": "https://package.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"},
#   "setup": {}
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write source plugin");

    let scanner = PluginScanner::new();
    let report = scanner.scan_path(&root).expect("scan should succeed");

    assert_eq!(report.scanned_files, 2);
    assert_eq!(report.matched_plugins, 1);
    assert_eq!(report.descriptors.len(), 1);
    assert_eq!(report.descriptors[0].manifest.setup, None);
}

#[test]
fn scanner_recognizes_openclaw_modern_manifest_through_explicit_compatibility_boundary() {
    let root = unique_tmp_dir("loong-openclaw-modern");
    let package_root = root.join("pkg");
    fs::create_dir_all(package_root.join("dist")).expect("create temp root");

    let package_manifest = package_root.join(OPENCLAW_PACKAGE_MANIFEST_FILE_NAME);
    fs::write(
        &package_manifest,
        r#"
{
  "id": "search-sdk",
  "name": "Search SDK",
  "description": "OpenClaw search integration",
  "version": "1.2.3",
  "kind": "provider",
  "providers": ["web_search"],
  "channels": ["search"],
  "skills": ["search"],
  "configSchema": {}
}
"#,
    )
    .expect("write openclaw manifest");

    let package_json = package_root.join(PACKAGE_JSON_FILE_NAME);
    fs::write(
        &package_json,
        r#"
{
  "name": "@acme/search-provider",
  "version": "1.2.3",
  "description": "Search provider package",
  "openclaw": {
    "extensions": ["dist/index.js"],
    "setupEntry": "dist/setup.js",
    "channel": {
      "id": "search",
      "label": "Search",
      "aliases": ["web-search"]
    }
  }
}
"#,
    )
    .expect("write package.json");
    fs::write(package_root.join("dist/index.js"), "export {};\n").expect("write entry");
    fs::write(package_root.join("dist/setup.js"), "export {};\n").expect("write setup");

    let report = PluginScanner::new()
        .scan_path(&root)
        .expect("scan should succeed");

    assert_eq!(report.matched_plugins, 1);
    assert_eq!(report.descriptors.len(), 1);
    assert_eq!(
        report.descriptors[0].dialect,
        PluginContractDialect::OpenClawModernManifest
    );
    assert_eq!(
        report.descriptors[0].compatibility_mode,
        PluginCompatibilityMode::OpenClawModern
    );
    assert_eq!(report.descriptors[0].language, "js");
    assert_eq!(report.descriptors[0].manifest.plugin_id, "search-sdk");
    assert_eq!(
        report.descriptors[0].package_manifest_path,
        Some(package_manifest.display().to_string())
    );
    assert_eq!(
        report.descriptors[0].path,
        package_root.join("dist/index.js").display().to_string()
    );
    let foreign = scan_diagnostic(
        &report,
        PluginDiagnosticCode::ForeignDialectContract,
        "search-sdk",
    )
    .expect("foreign dialect diagnostic");
    assert_eq!(foreign.phase, PluginDiagnosticPhase::Scan);
    assert!(!foreign.blocking);
    assert_eq!(
        report.descriptors[0]
            .manifest
            .metadata
            .get("adapter_family")
            .map(String::as_str),
        Some(OPENCLAW_MODERN_COMPATIBILITY_ADAPTER_FAMILY)
    );
}

#[test]
fn scanner_recognizes_openclaw_legacy_package_metadata_without_promoting_it_to_native() {
    let root = unique_tmp_dir("loong-openclaw-legacy");
    let package_root = root.join("pkg");
    fs::create_dir_all(package_root.join("dist")).expect("create temp root");

    let package_json = package_root.join(PACKAGE_JSON_FILE_NAME);
    fs::write(
        &package_json,
        r#"
{
  "name": "@acme/search-provider",
  "version": "0.9.0",
  "description": "Legacy OpenClaw package",
  "openclaw": {
    "extensions": ["dist/index.js"],
    "setupEntry": "dist/setup.js"
  }
}
"#,
    )
    .expect("write legacy package.json");
    fs::write(package_root.join("dist/index.js"), "export {};\n").expect("write entry");
    fs::write(package_root.join("dist/setup.js"), "export {};\n").expect("write setup");

    let report = PluginScanner::new()
        .scan_path(&root)
        .expect("scan should succeed");

    assert_eq!(report.matched_plugins, 1);
    assert_eq!(report.descriptors.len(), 1);
    assert_eq!(
        report.descriptors[0].dialect,
        PluginContractDialect::OpenClawLegacyPackage
    );
    assert_eq!(
        report.descriptors[0].compatibility_mode,
        PluginCompatibilityMode::OpenClawLegacy
    );
    assert_eq!(report.descriptors[0].manifest.plugin_id, "search");
    assert_eq!(
        report.descriptors[0].package_manifest_path,
        Some(package_json.display().to_string())
    );
    let foreign = scan_diagnostic(
        &report,
        PluginDiagnosticCode::ForeignDialectContract,
        "search",
    )
    .expect("foreign dialect diagnostic");
    assert_eq!(foreign.phase, PluginDiagnosticPhase::Scan);
    let legacy = scan_diagnostic(
        &report,
        PluginDiagnosticCode::LegacyOpenClawContract,
        "search",
    )
    .expect("legacy openclaw diagnostic");
    assert_eq!(legacy.phase, PluginDiagnosticPhase::Scan);
    assert_eq!(
        report.descriptors[0]
            .manifest
            .metadata
            .get("adapter_family")
            .map(String::as_str),
        Some(OPENCLAW_LEGACY_COMPATIBILITY_ADAPTER_FAMILY)
    );
}

#[test]
fn scanner_absorbs_plugins_into_catalog_and_pack() {
    let report = PluginScanReport {
        scanned_files: 1,
        matched_plugins: 1,
        diagnostic_findings: Vec::new(),
        descriptors: vec![PluginDescriptor {
            path: "/tmp/openai.rs".to_owned(),
            source_kind: PluginSourceKind::EmbeddedSource,
            dialect: PluginContractDialect::LoongEmbeddedSource,
            dialect_version: None,
            compatibility_mode: PluginCompatibilityMode::Native,
            package_root: "/tmp".to_owned(),
            package_manifest_path: None,
            language: "rs".to_owned(),
            manifest: PluginManifest {
                api_version: None,
                version: Some("1.3.0".to_owned()),
                plugin_id: "openai-rs".to_owned(),
                provider_id: "openai".to_owned(),
                connector_name: "openai".to_owned(),
                channel_id: Some("chat-main".to_owned()),
                endpoint: Some("https://api.openai.com/v1/chat/completions".to_owned()),
                capabilities: BTreeSet::from([
                    Capability::InvokeConnector,
                    Capability::ObserveTelemetry,
                ]),
                trust_tier: PluginTrustTier::Official,
                metadata: BTreeMap::from([("version".to_owned(), "1.3.0".to_owned())]),
                summary: None,
                tags: Vec::new(),
                input_examples: Vec::new(),
                output_examples: Vec::new(),
                defer_loading: false,
                setup: None,
                slot_claims: Vec::new(),
                compatibility: None,
            },
        }],
    };

    let mut catalog = IntegrationCatalog::new();
    let mut pack = sample_pack();
    let scanner = PluginScanner::new();

    let absorb = scanner
        .absorb(&mut catalog, &mut pack, &report)
        .expect("absorb should succeed");
    assert_eq!(absorb.absorbed_plugins, 1);
    assert_eq!(absorb.provider_upserts, 1);
    assert_eq!(absorb.channel_upserts, 1);
    assert!(catalog.provider("openai").is_some());
    assert!(catalog.channel("chat-main").is_some());
    assert!(pack.allowed_connectors.contains("openai"));
    assert!(
        pack.granted_capabilities
            .contains(&Capability::InvokeConnector)
    );
}

#[test]
fn absorb_rejects_conflicting_exclusive_slot_claims() {
    let report = PluginScanReport {
        scanned_files: 2,
        matched_plugins: 2,
        diagnostic_findings: Vec::new(),
        descriptors: vec![
            PluginDescriptor {
                path: "/tmp/search-a.py".to_owned(),
                source_kind: PluginSourceKind::EmbeddedSource,
                dialect: PluginContractDialect::LoongEmbeddedSource,
                dialect_version: None,
                compatibility_mode: PluginCompatibilityMode::Native,
                package_root: "/tmp".to_owned(),
                package_manifest_path: None,
                language: "py".to_owned(),
                manifest: PluginManifest {
                    api_version: None,
                    version: None,
                    plugin_id: "search-a".to_owned(),
                    provider_id: "search-a".to_owned(),
                    connector_name: "search-a".to_owned(),
                    channel_id: None,
                    endpoint: None,
                    capabilities: BTreeSet::from([Capability::InvokeConnector]),
                    trust_tier: PluginTrustTier::Unverified,
                    metadata: BTreeMap::new(),
                    summary: None,
                    tags: Vec::new(),
                    input_examples: Vec::new(),
                    output_examples: Vec::new(),
                    defer_loading: false,
                    setup: None,
                    slot_claims: vec![PluginSlotClaim {
                        slot: "provider:web_search".to_owned(),
                        key: "tavily".to_owned(),
                        mode: PluginSlotMode::Exclusive,
                    }],
                    compatibility: None,
                },
            },
            PluginDescriptor {
                path: "/tmp/search-b.py".to_owned(),
                source_kind: PluginSourceKind::EmbeddedSource,
                dialect: PluginContractDialect::LoongEmbeddedSource,
                dialect_version: None,
                compatibility_mode: PluginCompatibilityMode::Native,
                package_root: "/tmp".to_owned(),
                package_manifest_path: None,
                language: "py".to_owned(),
                manifest: PluginManifest {
                    api_version: None,
                    version: None,
                    plugin_id: "search-b".to_owned(),
                    provider_id: "search-b".to_owned(),
                    connector_name: "search-b".to_owned(),
                    channel_id: None,
                    endpoint: None,
                    capabilities: BTreeSet::from([Capability::InvokeConnector]),
                    trust_tier: PluginTrustTier::Unverified,
                    metadata: BTreeMap::new(),
                    summary: None,
                    tags: Vec::new(),
                    input_examples: Vec::new(),
                    output_examples: Vec::new(),
                    defer_loading: false,
                    setup: None,
                    slot_claims: vec![PluginSlotClaim {
                        slot: "provider:web_search".to_owned(),
                        key: "tavily".to_owned(),
                        mode: PluginSlotMode::Exclusive,
                    }],
                    compatibility: None,
                },
            },
        ],
    };

    let mut catalog = IntegrationCatalog::new();
    let mut pack = sample_pack();

    let error = PluginScanner::new()
        .absorb(&mut catalog, &mut pack, &report)
        .expect_err("conflicting exclusive slot claims should fail");

    let rendered = error.to_string();
    assert!(rendered.contains("slot claim conflict"));
    assert!(catalog.provider("search-a").is_none());
    assert!(catalog.provider("search-b").is_none());
}

#[test]
fn absorb_allows_shared_and_advisory_slot_claims_and_projects_metadata() {
    let current_host_version_req = format!(">={}", env!("CARGO_PKG_VERSION"));
    let report = PluginScanReport {
        scanned_files: 2,
        matched_plugins: 2,
        diagnostic_findings: Vec::new(),
        descriptors: vec![
            PluginDescriptor {
                path: "/tmp/search-shared.py".to_owned(),
                source_kind: PluginSourceKind::EmbeddedSource,
                dialect: PluginContractDialect::LoongEmbeddedSource,
                dialect_version: None,
                compatibility_mode: PluginCompatibilityMode::Native,
                package_root: "/tmp".to_owned(),
                package_manifest_path: None,
                language: "py".to_owned(),
                manifest: PluginManifest {
                    api_version: None,
                    version: Some("1.0.0".to_owned()),
                    plugin_id: "search-shared".to_owned(),
                    provider_id: "search-shared".to_owned(),
                    connector_name: "search-shared".to_owned(),
                    channel_id: None,
                    endpoint: None,
                    capabilities: BTreeSet::from([Capability::InvokeConnector]),
                    trust_tier: PluginTrustTier::Unverified,
                    metadata: BTreeMap::new(),
                    summary: None,
                    tags: Vec::new(),
                    input_examples: Vec::new(),
                    output_examples: Vec::new(),
                    defer_loading: false,
                    setup: None,
                    slot_claims: vec![PluginSlotClaim {
                        slot: "tool:search".to_owned(),
                        key: "web".to_owned(),
                        mode: PluginSlotMode::Shared,
                    }],
                    compatibility: Some(PluginCompatibility {
                        host_api: Some(CURRENT_PLUGIN_HOST_API.to_owned()),
                        host_version_req: Some(current_host_version_req.clone()),
                    }),
                },
            },
            PluginDescriptor {
                path: "/tmp/search-advisory.py".to_owned(),
                source_kind: PluginSourceKind::EmbeddedSource,
                dialect: PluginContractDialect::LoongEmbeddedSource,
                dialect_version: None,
                compatibility_mode: PluginCompatibilityMode::Native,
                package_root: "/tmp".to_owned(),
                package_manifest_path: None,
                language: "py".to_owned(),
                manifest: PluginManifest {
                    api_version: None,
                    version: None,
                    plugin_id: "search-advisory".to_owned(),
                    provider_id: "search-advisory".to_owned(),
                    connector_name: "search-advisory".to_owned(),
                    channel_id: None,
                    endpoint: None,
                    capabilities: BTreeSet::from([Capability::InvokeConnector]),
                    trust_tier: PluginTrustTier::Unverified,
                    metadata: BTreeMap::new(),
                    summary: None,
                    tags: Vec::new(),
                    input_examples: Vec::new(),
                    output_examples: Vec::new(),
                    defer_loading: false,
                    setup: None,
                    slot_claims: vec![PluginSlotClaim {
                        slot: "tool:search".to_owned(),
                        key: "web".to_owned(),
                        mode: PluginSlotMode::Advisory,
                    }],
                    compatibility: None,
                },
            },
        ],
    };

    let mut catalog = IntegrationCatalog::new();
    let mut pack = sample_pack();

    let absorb = PluginScanner::new()
        .absorb(&mut catalog, &mut pack, &report)
        .expect("shared and advisory slot claims should coexist");

    assert_eq!(absorb.absorbed_plugins, 2);
    let shared_provider = catalog
        .provider("search-shared")
        .expect("shared provider should be registered");
    assert_eq!(
        shared_provider
            .metadata
            .get(PLUGIN_SLOT_CLAIMS_METADATA_KEY)
            .map(String::as_str),
        Some("[{\"slot\":\"tool:search\",\"key\":\"web\",\"mode\":\"shared\"}]")
    );
    assert_eq!(
        shared_provider
            .metadata
            .get(PLUGIN_COMPATIBILITY_HOST_API_METADATA_KEY)
            .map(String::as_str),
        Some(CURRENT_PLUGIN_HOST_API)
    );
    assert_eq!(
        shared_provider
            .metadata
            .get(PLUGIN_COMPATIBILITY_HOST_VERSION_REQ_METADATA_KEY)
            .map(String::as_str),
        Some(current_host_version_req.as_str())
    );
}

#[test]
fn absorb_rejects_incompatible_host_api() {
    let report = PluginScanReport {
        scanned_files: 1,
        matched_plugins: 1,
        diagnostic_findings: Vec::new(),
        descriptors: vec![PluginDescriptor {
            path: "/tmp/incompatible-host.py".to_owned(),
            source_kind: PluginSourceKind::EmbeddedSource,
            dialect: PluginContractDialect::LoongEmbeddedSource,
            dialect_version: None,
            compatibility_mode: PluginCompatibilityMode::Native,
            package_root: "/tmp".to_owned(),
            package_manifest_path: None,
            language: "py".to_owned(),
            manifest: PluginManifest {
                api_version: None,
                version: None,
                plugin_id: "incompatible-host".to_owned(),
                provider_id: "incompatible-host".to_owned(),
                connector_name: "incompatible-host".to_owned(),
                channel_id: None,
                endpoint: None,
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier: PluginTrustTier::Unverified,
                metadata: BTreeMap::new(),
                summary: None,
                tags: Vec::new(),
                input_examples: Vec::new(),
                output_examples: Vec::new(),
                defer_loading: false,
                setup: None,
                slot_claims: Vec::new(),
                compatibility: Some(PluginCompatibility {
                    host_api: Some("loong-plugin/v999".to_owned()),
                    host_version_req: None,
                }),
            },
        }],
    };

    let mut catalog = IntegrationCatalog::new();
    let mut pack = sample_pack();

    let error = PluginScanner::new()
        .absorb(&mut catalog, &mut pack, &report)
        .expect_err("incompatible host api should fail closed");

    let rendered = error.to_string();
    assert!(rendered.contains("compatibility.host_api"));
    assert!(rendered.contains(CURRENT_PLUGIN_HOST_API));
    assert!(catalog.provider("incompatible-host").is_none());
}

#[test]
fn absorb_rejects_invalid_host_version_requirement() {
    let report = PluginScanReport {
        scanned_files: 1,
        matched_plugins: 1,
        diagnostic_findings: Vec::new(),
        descriptors: vec![PluginDescriptor {
            path: "/tmp/invalid-version.py".to_owned(),
            source_kind: PluginSourceKind::EmbeddedSource,
            dialect: PluginContractDialect::LoongEmbeddedSource,
            dialect_version: None,
            compatibility_mode: PluginCompatibilityMode::Native,
            package_root: "/tmp".to_owned(),
            package_manifest_path: None,
            language: "py".to_owned(),
            manifest: PluginManifest {
                api_version: None,
                version: None,
                plugin_id: "invalid-version".to_owned(),
                provider_id: "invalid-version".to_owned(),
                connector_name: "invalid-version".to_owned(),
                channel_id: None,
                endpoint: None,
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier: PluginTrustTier::Unverified,
                metadata: BTreeMap::new(),
                summary: None,
                tags: Vec::new(),
                input_examples: Vec::new(),
                output_examples: Vec::new(),
                defer_loading: false,
                setup: None,
                slot_claims: Vec::new(),
                compatibility: Some(PluginCompatibility {
                    host_api: Some(CURRENT_PLUGIN_HOST_API.to_owned()),
                    host_version_req: Some("not-a-semver-req".to_owned()),
                }),
            },
        }],
    };

    let mut catalog = IntegrationCatalog::new();
    let mut pack = sample_pack();

    let error = PluginScanner::new()
        .absorb(&mut catalog, &mut pack, &report)
        .expect_err("invalid host version requirement should fail closed");

    let rendered = error.to_string();
    assert!(rendered.contains("compatibility.host_version_req"));
    assert!(rendered.contains("invalid"));
    assert!(catalog.provider("invalid-version").is_none());
}

#[test]
fn scanner_skips_non_utf8_files_instead_of_failing() {
    let root = unique_tmp_dir("loong-plugin-binary");
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

#[test]
fn absorb_rolls_back_catalog_and_pack_on_validation_failure() {
    // First descriptor is valid, second has an empty provider_id which
    // triggers validation failure. The rollback must undo the first
    // descriptor's mutations so catalog and pack remain unchanged.
    let report = PluginScanReport {
        scanned_files: 2,
        matched_plugins: 2,
        diagnostic_findings: Vec::new(),
        descriptors: vec![
            PluginDescriptor {
                path: "/tmp/good.rs".to_owned(),
                source_kind: PluginSourceKind::EmbeddedSource,
                dialect: PluginContractDialect::LoongEmbeddedSource,
                dialect_version: None,
                compatibility_mode: PluginCompatibilityMode::Native,
                package_root: "/tmp".to_owned(),
                package_manifest_path: None,
                language: "rs".to_owned(),
                manifest: PluginManifest {
                    api_version: None,
                    version: Some("1.0.0".to_owned()),
                    plugin_id: "good-plugin".to_owned(),
                    provider_id: "good-provider".to_owned(),
                    connector_name: "good-connector".to_owned(),
                    channel_id: Some("good-channel".to_owned()),
                    endpoint: Some("https://good.local/invoke".to_owned()),
                    capabilities: BTreeSet::from([Capability::InvokeConnector]),
                    trust_tier: PluginTrustTier::VerifiedCommunity,
                    metadata: BTreeMap::from([("version".to_owned(), "1.0.0".to_owned())]),
                    summary: None,
                    tags: Vec::new(),
                    input_examples: Vec::new(),
                    output_examples: Vec::new(),
                    defer_loading: false,
                    setup: None,
                    slot_claims: Vec::new(),
                    compatibility: None,
                },
            },
            PluginDescriptor {
                path: "/tmp/bad.rs".to_owned(),
                source_kind: PluginSourceKind::EmbeddedSource,
                dialect: PluginContractDialect::LoongEmbeddedSource,
                dialect_version: None,
                compatibility_mode: PluginCompatibilityMode::Native,
                package_root: "/tmp".to_owned(),
                package_manifest_path: None,
                language: "rs".to_owned(),
                manifest: PluginManifest {
                    api_version: None,
                    version: None,
                    plugin_id: "bad-plugin".to_owned(),
                    provider_id: String::new(), // empty — triggers validation error
                    connector_name: "bad-connector".to_owned(),
                    channel_id: None,
                    endpoint: None,
                    capabilities: BTreeSet::new(),
                    trust_tier: PluginTrustTier::Unverified,
                    metadata: BTreeMap::new(),
                    summary: None,
                    tags: Vec::new(),
                    input_examples: Vec::new(),
                    output_examples: Vec::new(),
                    defer_loading: false,
                    setup: None,
                    slot_claims: Vec::new(),
                    compatibility: None,
                },
            },
        ],
    };

    let mut catalog = IntegrationCatalog::new();
    let mut pack = sample_pack();
    let scanner = PluginScanner::new();

    let catalog_before = catalog.clone();
    let pack_before = pack.clone();

    let result = scanner.absorb(&mut catalog, &mut pack, &report);
    assert!(result.is_err(), "absorb should fail on empty provider_id");

    // Verify rollback: catalog and pack are identical to their pre-absorb state.
    assert_eq!(catalog, catalog_before, "catalog must be rolled back");
    assert_eq!(pack, pack_before, "pack must be rolled back");
}

#[test]
fn format_plugin_provenance_summary_prefers_package_manifest_context() {
    let summary = format_plugin_provenance_summary(
        PluginSourceKind::EmbeddedSource,
        "/tmp/pkg/plugin.py",
        Some("/tmp/pkg/loong.plugin.json"),
    );

    assert_eq!(
        summary,
        "embedded_source:/tmp/pkg/plugin.py (package_manifest:/tmp/pkg/loong.plugin.json)"
    );
}
