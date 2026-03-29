use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
#[cfg(unix)]
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use super::wasm_artifact_file_identity;
use super::wasm_runtime_policy::{
    DEFAULT_WASM_MODULE_CACHE_CAPACITY, DEFAULT_WASM_MODULE_CACHE_MAX_BYTES,
    MAX_WASM_MODULE_CACHE_CAPACITY, MAX_WASM_MODULE_CACHE_MAX_BYTES,
    MIN_WASM_MODULE_CACHE_MAX_BYTES, default_wasm_signals_based_traps,
    parse_wasm_module_cache_capacity, parse_wasm_module_cache_max_bytes,
    parse_wasm_signals_based_traps,
};
use super::{
    BridgeRuntimePolicy, ConnectorCircuitBreakerPolicy, ConnectorProtocolContext, CoreToolRuntime,
    DynamicCatalogConnector, PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY,
    PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY, PluginActivationRuntimeContract,
    PluginRuntimeHealthResult, WasmModuleCache, activation_runtime_contract_checksum_hex,
    build_wasm_module_cache_key, compile_wasm_module, normalize_sha256_pin,
    plugin_activation_runtime_contract_json, process_stdio_runtime_evidence,
    provider_plugin_runtime_health_result, resolve_expected_wasm_sha256,
};
use kernel::{
    BridgeSupportMatrix, CoreConnectorAdapter, CoreToolAdapter, IntegrationCatalog,
    PluginBridgeKind, PluginCompatibilityMode, PluginCompatibilityShim,
    PluginCompatibilityShimSupport, PluginContractDialect, PluginSourceKind, ToolCoreOutcome,
    ToolCoreRequest,
};
use loongclaw_protocol::PROTOCOL_VERSION;
use serde_json::{Value, json};
use tokio::time::sleep;

const EMPTY_WASM_MODULE: [u8; 8] = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];

#[test]
fn parse_wasm_module_cache_capacity_defaults_for_missing_or_invalid_values() {
    assert_eq!(
        parse_wasm_module_cache_capacity(None),
        DEFAULT_WASM_MODULE_CACHE_CAPACITY
    );
    assert_eq!(
        parse_wasm_module_cache_capacity(Some("")),
        DEFAULT_WASM_MODULE_CACHE_CAPACITY
    );
    assert_eq!(
        parse_wasm_module_cache_capacity(Some("invalid")),
        DEFAULT_WASM_MODULE_CACHE_CAPACITY
    );
    assert_eq!(
        parse_wasm_module_cache_capacity(Some("0")),
        DEFAULT_WASM_MODULE_CACHE_CAPACITY
    );
}

#[test]
fn parse_wasm_module_cache_capacity_respects_positive_values_and_upper_bound() {
    assert_eq!(parse_wasm_module_cache_capacity(Some("1")), 1);
    assert_eq!(parse_wasm_module_cache_capacity(Some("128")), 128);

    let over_limit = format!("{}", MAX_WASM_MODULE_CACHE_CAPACITY + 1);
    assert_eq!(
        parse_wasm_module_cache_capacity(Some(over_limit.as_str())),
        MAX_WASM_MODULE_CACHE_CAPACITY
    );
}

#[test]
fn parse_wasm_module_cache_max_bytes_defaults_for_missing_or_invalid_values() {
    assert_eq!(
        parse_wasm_module_cache_max_bytes(None),
        DEFAULT_WASM_MODULE_CACHE_MAX_BYTES
    );
    assert_eq!(
        parse_wasm_module_cache_max_bytes(Some("")),
        DEFAULT_WASM_MODULE_CACHE_MAX_BYTES
    );
    assert_eq!(
        parse_wasm_module_cache_max_bytes(Some("invalid")),
        DEFAULT_WASM_MODULE_CACHE_MAX_BYTES
    );
    assert_eq!(
        parse_wasm_module_cache_max_bytes(Some("0")),
        DEFAULT_WASM_MODULE_CACHE_MAX_BYTES
    );
}

#[test]
fn parse_wasm_module_cache_max_bytes_respects_bounds() {
    assert_eq!(
        parse_wasm_module_cache_max_bytes(Some("1")),
        MIN_WASM_MODULE_CACHE_MAX_BYTES
    );
    assert_eq!(
        parse_wasm_module_cache_max_bytes(Some("1048576")),
        1_048_576
    );

    let over_limit = format!("{}", MAX_WASM_MODULE_CACHE_MAX_BYTES + 1);
    assert_eq!(
        parse_wasm_module_cache_max_bytes(Some(over_limit.as_str())),
        MAX_WASM_MODULE_CACHE_MAX_BYTES
    );
}

#[test]
fn parse_wasm_signals_based_traps_defaults_to_platform_policy() {
    assert_eq!(
        parse_wasm_signals_based_traps(None),
        default_wasm_signals_based_traps()
    );
    assert_eq!(
        parse_wasm_signals_based_traps(Some("")),
        default_wasm_signals_based_traps()
    );
    assert_eq!(
        parse_wasm_signals_based_traps(Some("invalid-value")),
        default_wasm_signals_based_traps()
    );
}

#[test]
fn parse_wasm_signals_based_traps_accepts_boolean_aliases() {
    for raw in ["1", "true", "yes", "on", "enabled", "TRUE", " On "] {
        assert!(
            parse_wasm_signals_based_traps(Some(raw)),
            "expected true for {raw}"
        );
    }
    for raw in ["0", "false", "no", "off", "disabled", "FALSE", " Off "] {
        assert!(
            !parse_wasm_signals_based_traps(Some(raw)),
            "expected false for {raw}"
        );
    }
}

#[test]
fn normalize_sha256_pin_accepts_plain_or_prefixed_hex() {
    let expected = "ab".repeat(32);
    assert_eq!(
        normalize_sha256_pin(expected.as_str()).expect("plain digest should parse"),
        expected
    );
    assert_eq!(
        normalize_sha256_pin(format!("sha256:{expected}").as_str())
            .expect("prefixed digest should parse"),
        expected
    );
    assert_eq!(
        normalize_sha256_pin(format!("  SHA256:{expected}  ").as_str())
            .expect("prefix should be case-insensitive"),
        expected
    );
}

#[test]
fn normalize_sha256_pin_rejects_invalid_values() {
    assert!(normalize_sha256_pin("").is_err());
    assert!(normalize_sha256_pin("sha256:").is_err());
    assert!(normalize_sha256_pin("deadbeef").is_err());
    assert!(normalize_sha256_pin(&"z".repeat(64)).is_err());
}

fn provider_with_metadata(metadata: BTreeMap<String, String>) -> kernel::ProviderConfig {
    kernel::ProviderConfig {
        provider_id: "provider-x".to_owned(),
        connector_name: "connector-x".to_owned(),
        version: "1.0.0".to_owned(),
        metadata,
    }
}

fn openclaw_process_stdio_provider_with_command(
    command: &str,
    args: Vec<String>,
) -> kernel::ProviderConfig {
    let mut metadata = openclaw_process_stdio_metadata();

    metadata.insert("command".to_owned(), command.to_owned());
    if !args.is_empty() {
        let args_json = serde_json::to_string(&args).expect("encode process args");
        metadata.insert("args_json".to_owned(), args_json);
    }

    provider_with_metadata(metadata)
}

fn process_stdio_channel_for_provider(provider: &kernel::ProviderConfig) -> kernel::ChannelConfig {
    kernel::ChannelConfig {
        channel_id: "channel-process".to_owned(),
        provider_id: provider.provider_id.clone(),
        endpoint: "stdio://connector".to_owned(),
        enabled: true,
        metadata: BTreeMap::new(),
    }
}

fn process_stdio_request_id(
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    operation: &str,
) -> String {
    format!(
        "{}:{}:{operation}",
        provider.provider_id, channel.channel_id
    )
}

fn provider_runtime_health_from_catalog(
    catalog: &Arc<Mutex<IntegrationCatalog>>,
    provider_id: &str,
) -> PluginRuntimeHealthResult {
    let guard = catalog.lock().expect("catalog mutex poisoned");
    let provider = guard
        .provider(provider_id)
        .expect("provider should exist in catalog");
    let health = provider_plugin_runtime_health_result(&provider.metadata);

    health.expect("provider metadata should carry runtime health")
}

fn process_stdio_success_args(request_id: &str) -> Vec<String> {
    let response = json!({
        "method": "tools/call",
        "id": request_id,
        "payload": {
            "ok": true,
        },
        "version": PROTOCOL_VERSION,
    });
    let response_text = response.to_string();
    let script = format!("IFS= read -r _line; printf '%s\\n' '{response_text}'");

    vec!["-c".to_owned(), script]
}

fn openclaw_attested_runtime_contract() -> PluginActivationRuntimeContract {
    PluginActivationRuntimeContract {
        plugin_id: "weather-sdk".to_owned(),
        source_path: "/tmp/weather-sdk/dist/index.js".to_owned(),
        source_kind: PluginSourceKind::PackageManifest,
        dialect: PluginContractDialect::OpenClawModernManifest,
        dialect_version: Some("openclaw.plugin.json".to_owned()),
        compatibility_mode: PluginCompatibilityMode::OpenClawModern,
        compatibility_shim: Some(PluginCompatibilityShim {
            shim_id: "openclaw-modern-compat".to_owned(),
            family: "openclaw-modern-compat".to_owned(),
        }),
        bridge_kind: PluginBridgeKind::ProcessStdio,
        adapter_family: "openclaw-modern-compat".to_owned(),
        entrypoint_hint: "stdin/stdout::invoke".to_owned(),
        source_language: "javascript".to_owned(),
        compatibility: None,
    }
}

fn openclaw_attested_runtime_contract_json() -> String {
    plugin_activation_runtime_contract_json(&openclaw_attested_runtime_contract())
        .expect("encode activation contract")
}

fn openclaw_attested_runtime_contract_checksum() -> String {
    activation_runtime_contract_checksum_hex(openclaw_attested_runtime_contract_json().as_bytes())
}

fn openclaw_process_stdio_metadata() -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::from([
        ("plugin_id".to_owned(), "weather-sdk".to_owned()),
        (
            "plugin_source_path".to_owned(),
            "/tmp/weather-sdk/dist/index.js".to_owned(),
        ),
        (
            "plugin_source_kind".to_owned(),
            "package_manifest".to_owned(),
        ),
        (
            "plugin_dialect".to_owned(),
            "openclaw_modern_manifest".to_owned(),
        ),
        (
            "plugin_dialect_version".to_owned(),
            "openclaw.plugin.json".to_owned(),
        ),
        (
            "plugin_compatibility_mode".to_owned(),
            "openclaw_modern".to_owned(),
        ),
        (
            "plugin_compatibility_shim_id".to_owned(),
            "openclaw-modern-compat".to_owned(),
        ),
        (
            "plugin_compatibility_shim_family".to_owned(),
            "openclaw-modern-compat".to_owned(),
        ),
        ("bridge_kind".to_owned(), "process_stdio".to_owned()),
        (
            "adapter_family".to_owned(),
            "openclaw-modern-compat".to_owned(),
        ),
        ("source_language".to_owned(), "javascript".to_owned()),
    ]);
    let raw_contract = openclaw_attested_runtime_contract_json();
    metadata.insert(
        PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY.to_owned(),
        raw_contract.clone(),
    );
    metadata.insert(
        PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY.to_owned(),
        activation_runtime_contract_checksum_hex(raw_contract.as_bytes()),
    );
    metadata
}

fn openclaw_runtime_matrix(supported_source_languages: &[&str]) -> BridgeSupportMatrix {
    let profile = PluginCompatibilityShimSupport {
        shim: PluginCompatibilityShim {
            shim_id: "openclaw-modern-compat".to_owned(),
            family: "openclaw-modern-compat".to_owned(),
        },
        version: Some("openclaw-modern@1".to_owned()),
        supported_dialects: BTreeSet::from([PluginContractDialect::OpenClawModernManifest]),
        supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
        supported_adapter_families: BTreeSet::new(),
        supported_source_languages: supported_source_languages
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
    }
    .normalized();

    BridgeSupportMatrix {
        supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
        supported_adapter_families: BTreeSet::new(),
        supported_compatibility_modes: BTreeSet::from([
            PluginCompatibilityMode::Native,
            PluginCompatibilityMode::OpenClawModern,
        ]),
        supported_compatibility_shims: BTreeSet::from([profile.shim.clone()]),
        supported_compatibility_shim_profiles: BTreeMap::from([(profile.shim.clone(), profile)]),
    }
}

#[tokio::test]
async fn bridge_execution_payload_surfaces_plugin_compatibility_context() {
    let provider = provider_with_metadata(openclaw_process_stdio_metadata());
    let channel = kernel::ChannelConfig {
        channel_id: "channel-compat".to_owned(),
        provider_id: provider.provider_id.clone(),
        endpoint: "stdio://compat".to_owned(),
        enabled: true,
        metadata: BTreeMap::new(),
    };
    let command = kernel::ConnectorCommand {
        connector_name: provider.connector_name.clone(),
        operation: "invoke".to_owned(),
        required_capabilities: BTreeSet::new(),
        payload: json!({"city": "shanghai"}),
    };
    let runtime_policy = BridgeRuntimePolicy {
        compatibility_matrix: openclaw_runtime_matrix(&["javascript"]),
        ..BridgeRuntimePolicy::default()
    };

    let execution =
        super::bridge_execution_payload(&provider, &channel, &command, &runtime_policy).await;

    assert_eq!(execution["status"], json!("planned"));
    assert_eq!(
        execution["plugin_compatibility"]["dialect"],
        json!("openclaw_modern_manifest")
    );
    assert_eq!(
        execution["plugin_compatibility"]["dialect_version"],
        json!("openclaw.plugin.json")
    );
    assert_eq!(
        execution["plugin_compatibility"]["mode"],
        json!("openclaw_modern")
    );
    assert_eq!(
        execution["plugin_compatibility"]["shim"]["shim_id"],
        json!("openclaw-modern-compat")
    );
    assert_eq!(
        execution["plugin_compatibility"]["shim"]["family"],
        json!("openclaw-modern-compat")
    );
    assert_eq!(
        execution["plugin_compatibility"]["shim_support"]["version"],
        json!("openclaw-modern@1")
    );
    assert_eq!(
        execution["plugin_compatibility"]["shim_support"]["supported_dialects"][0],
        json!("openclaw_modern_manifest")
    );
    assert_eq!(
        execution["plugin_compatibility"]["runtime_projection"]["source_language"],
        json!("javascript")
    );
    assert_eq!(
        execution["plugin_compatibility"]["runtime_guard"]["status"],
        json!("passed")
    );
    assert_eq!(
        execution["plugin_compatibility"]["activation_contract"]["source_kind"],
        json!("package_manifest")
    );
    assert_eq!(
        execution["plugin_compatibility"]["activation_contract_checksum"],
        json!(openclaw_attested_runtime_contract_checksum())
    );
    assert_eq!(
        execution["plugin_compatibility"]["runtime_guard"]["activation_contract_verified"],
        json!(true)
    );
    assert_eq!(
        execution["plugin_compatibility"]["runtime_guard"]["activation_contract_integrity"],
        json!("verified")
    );
}

#[tokio::test]
async fn bridge_execution_payload_blocks_when_compatibility_shim_profile_drifts_at_runtime() {
    let provider = provider_with_metadata(openclaw_process_stdio_metadata());
    let channel = kernel::ChannelConfig {
        channel_id: "channel-compat".to_owned(),
        provider_id: provider.provider_id.clone(),
        endpoint: "stdio://compat".to_owned(),
        enabled: true,
        metadata: BTreeMap::new(),
    };
    let command = kernel::ConnectorCommand {
        connector_name: provider.connector_name.clone(),
        operation: "invoke".to_owned(),
        required_capabilities: BTreeSet::new(),
        payload: json!({"city": "shanghai"}),
    };
    let runtime_policy = BridgeRuntimePolicy {
        compatibility_matrix: openclaw_runtime_matrix(&["python"]),
        ..BridgeRuntimePolicy::default()
    };

    let execution =
        super::bridge_execution_payload(&provider, &channel, &command, &runtime_policy).await;

    assert_eq!(execution["status"], json!("blocked"));
    assert_eq!(execution["block_class"], json!("compatibility_contract"));
    assert!(
        execution["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("source language `javascript`"))
    );
    assert_eq!(
        execution["plugin_compatibility"]["runtime_guard"]["status"],
        json!("blocked")
    );
    assert_eq!(
        execution["plugin_compatibility"]["shim_support_mismatch_reasons"][0],
        json!("source language `javascript`")
    );
    assert_eq!(
        execution["plugin_compatibility"]["runtime_guard"]["activation_contract_attested"],
        json!(true)
    );
    assert_eq!(
        execution["plugin_compatibility"]["runtime_guard"]["activation_contract_verified"],
        json!(true)
    );
}

#[tokio::test]
async fn bridge_execution_payload_blocks_when_activation_contract_checksum_drifts_at_runtime() {
    let mut metadata = openclaw_process_stdio_metadata();
    metadata.insert(
        PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY.to_owned(),
        "deadbeefdeadbeef".to_owned(),
    );
    let provider = provider_with_metadata(metadata);
    let channel = kernel::ChannelConfig {
        channel_id: "channel-compat".to_owned(),
        provider_id: provider.provider_id.clone(),
        endpoint: "stdio://compat".to_owned(),
        enabled: true,
        metadata: BTreeMap::new(),
    };
    let command = kernel::ConnectorCommand {
        connector_name: provider.connector_name.clone(),
        operation: "invoke".to_owned(),
        required_capabilities: BTreeSet::new(),
        payload: json!({"city": "shanghai"}),
    };
    let runtime_policy = BridgeRuntimePolicy {
        compatibility_matrix: openclaw_runtime_matrix(&["javascript"]),
        ..BridgeRuntimePolicy::default()
    };

    let execution =
        super::bridge_execution_payload(&provider, &channel, &command, &runtime_policy).await;

    assert_eq!(execution["status"], json!("blocked"));
    assert_eq!(execution["block_class"], json!("compatibility_contract"));
    assert!(
        execution["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("checksum mismatch"))
    );
    assert_eq!(
        execution["plugin_compatibility"]["activation_contract"],
        Value::Null
    );
    assert_eq!(
        execution["plugin_compatibility"]["activation_contract_checksum"],
        json!("deadbeefdeadbeef")
    );
    assert_eq!(
        execution["plugin_compatibility"]["runtime_guard"]["activation_contract_attested"],
        json!(true)
    );
    assert_eq!(
        execution["plugin_compatibility"]["runtime_guard"]["activation_contract_verified"],
        json!(false)
    );
    assert_eq!(
        execution["plugin_compatibility"]["runtime_guard"]["activation_contract_integrity"],
        json!("invalid")
    );
    assert_eq!(
        execution["plugin_compatibility"]["runtime_guard"]["activation_contract_computed_checksum"],
        json!(openclaw_attested_runtime_contract_checksum())
    );
}

#[tokio::test]
async fn dynamic_catalog_connector_rejects_compatibility_projection_drift_after_registration() {
    let mut metadata = openclaw_process_stdio_metadata();
    metadata.insert(
        "plugin_compatibility_shim_id".to_owned(),
        "openclaw-legacy-compat".to_owned(),
    );
    metadata.insert(
        "plugin_compatibility_shim_family".to_owned(),
        "openclaw-legacy-compat".to_owned(),
    );
    let provider = provider_with_metadata(metadata);
    let channel = kernel::ChannelConfig {
        channel_id: "channel-compat".to_owned(),
        provider_id: provider.provider_id.clone(),
        endpoint: "stdio://compat".to_owned(),
        enabled: true,
        metadata: BTreeMap::new(),
    };
    let mut catalog = IntegrationCatalog::new();
    catalog.upsert_provider(provider.clone());
    catalog.upsert_channel(channel.clone());
    let connector = DynamicCatalogConnector::new(
        provider.connector_name.clone(),
        provider.provider_id.clone(),
        Arc::new(Mutex::new(catalog)),
        BridgeRuntimePolicy {
            compatibility_matrix: openclaw_runtime_matrix(&["javascript"]),
            ..BridgeRuntimePolicy::default()
        },
    );

    let error = connector
        .invoke_core(kernel::ConnectorCommand {
            connector_name: provider.connector_name.clone(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({"channel_id":"channel-compat","city":"shanghai"}),
        })
        .await
        .expect_err("compatibility projection drift must block execution");

    assert!(
        error
            .to_string()
            .contains("plugin activation contract drifted after registration")
    );
}

#[tokio::test]
async fn dynamic_catalog_connector_circuit_breaker_isolates_repeated_bridge_failures() {
    let provider = openclaw_process_stdio_provider_with_command("false", Vec::new());
    let channel = process_stdio_channel_for_provider(&provider);
    let request_id = process_stdio_request_id(&provider, &channel, "invoke");
    let recovery_args = process_stdio_success_args(&request_id);
    let recovery_provider = openclaw_process_stdio_provider_with_command("sh", recovery_args);
    let mut catalog = IntegrationCatalog::new();

    catalog.upsert_provider(provider.clone());
    catalog.upsert_channel(channel.clone());

    let connector = DynamicCatalogConnector::new(
        provider.connector_name.clone(),
        provider.provider_id.clone(),
        Arc::new(Mutex::new(catalog)),
        BridgeRuntimePolicy {
            execute_process_stdio: true,
            allowed_process_commands: BTreeSet::from(["false".to_owned(), "sh".to_owned()]),
            bridge_circuit_breaker: ConnectorCircuitBreakerPolicy {
                enabled: true,
                failure_threshold: 2,
                cooldown_ms: 25,
                half_open_max_calls: 1,
                success_threshold: 1,
            },
            enforce_execution_success: true,
            compatibility_matrix: openclaw_runtime_matrix(&["javascript"]),
            ..BridgeRuntimePolicy::default()
        },
    );

    let command = kernel::ConnectorCommand {
        connector_name: provider.connector_name.clone(),
        operation: "invoke".to_owned(),
        required_capabilities: BTreeSet::new(),
        payload: json!({"channel_id": channel.channel_id}),
    };

    let first_error = connector
        .invoke_core(command.clone())
        .await
        .expect_err("first failing bridge call should error");
    let first_error_text = first_error.to_string();
    assert!(first_error_text.contains("bridge_circuit_phase_before=closed"));
    assert!(first_error_text.contains("bridge_circuit_phase_after=closed"));
    let first_health =
        provider_runtime_health_from_catalog(&connector.catalog, &provider.provider_id);
    assert_eq!(first_health.status, "degraded");
    assert_eq!(first_health.circuit_phase, "closed");
    assert_eq!(first_health.consecutive_failures, 1);
    assert!(first_health.last_failure_reason.is_some());

    let second_error = connector
        .invoke_core(command.clone())
        .await
        .expect_err("second failing bridge call should open circuit");
    let second_error_text = second_error.to_string();
    assert!(second_error_text.contains("bridge_circuit_phase_before=closed"));
    assert!(second_error_text.contains("bridge_circuit_phase_after=open"));
    let second_health =
        provider_runtime_health_from_catalog(&connector.catalog, &provider.provider_id);
    assert_eq!(second_health.status, "quarantined");
    assert_eq!(second_health.circuit_phase, "open");
    assert_eq!(second_health.consecutive_failures, 2);

    {
        let mut catalog = connector.catalog.lock().expect("catalog mutex poisoned");

        catalog.upsert_provider(recovery_provider);
    }

    let open_error = connector
        .invoke_core(command.clone())
        .await
        .expect_err("open circuit should short-circuit before re-execution");
    let open_error_text = open_error.to_string();
    assert!(open_error_text.contains("circuit-open"));
    let open_health =
        provider_runtime_health_from_catalog(&connector.catalog, &provider.provider_id);
    assert_eq!(open_health.status, "quarantined");
    assert_eq!(open_health.circuit_phase, "open");
    assert!(
        open_health
            .last_failure_reason
            .as_deref()
            .map(|reason| reason.contains("circuit-open"))
            .unwrap_or(false)
    );

    sleep(Duration::from_millis(30)).await;

    let recovered = connector
        .invoke_core(command)
        .await
        .expect("half-open recovery call should succeed");
    assert_eq!(
        recovered.payload["bridge_execution"]["status"],
        json!("executed")
    );
    assert_eq!(
        recovered.payload["bridge_execution"]["circuit_breaker"]["phase_before"],
        json!("half_open")
    );
    assert_eq!(
        recovered.payload["bridge_execution"]["circuit_breaker"]["phase_after"],
        json!("closed")
    );
    let recovered_health =
        provider_runtime_health_from_catalog(&connector.catalog, &provider.provider_id);
    assert_eq!(recovered_health.status, "healthy");
    assert_eq!(recovered_health.circuit_phase, "closed");
    assert_eq!(recovered_health.consecutive_failures, 0);
    assert!(recovered_health.last_failure_reason.is_none());
}

#[test]
fn resolve_expected_wasm_sha256_rejects_conflicting_metadata_pins() {
    let provider = provider_with_metadata(BTreeMap::from([
        ("plugin_id".to_owned(), "plugin-a".to_owned()),
        ("component_sha256".to_owned(), "aa".repeat(32)),
        ("component_sha256_pin".to_owned(), "bb".repeat(32)),
    ]));
    let policy = BridgeRuntimePolicy::default();
    let error = resolve_expected_wasm_sha256(&provider, &policy)
        .expect_err("conflicting metadata pins should be rejected");
    assert!(error.contains("conflicting wasm sha256 pins"));
}

#[test]
fn resolve_expected_wasm_sha256_rejects_metadata_and_policy_conflict() {
    let provider = provider_with_metadata(BTreeMap::from([
        ("plugin_id".to_owned(), "plugin-a".to_owned()),
        ("component_sha256".to_owned(), "aa".repeat(32)),
    ]));
    let mut policy = BridgeRuntimePolicy::default();
    policy
        .wasm_required_sha256_by_plugin
        .insert("plugin-a".to_owned(), "bb".repeat(32));

    let error = resolve_expected_wasm_sha256(&provider, &policy)
        .expect_err("metadata/policy conflict should be rejected");
    assert!(error.contains("between provider metadata"));
}

#[test]
fn process_stdio_runtime_evidence_reports_balanced_execution_tier() {
    let provider = provider_with_metadata(BTreeMap::new());
    let channel = kernel::ChannelConfig {
        channel_id: "channel-x".to_owned(),
        endpoint: "stdio://connector".to_owned(),
        provider_id: provider.provider_id.clone(),
        enabled: true,
        metadata: BTreeMap::new(),
    };
    let command = kernel::ConnectorCommand {
        connector_name: "connector-x".to_owned(),
        operation: "call".to_owned(),
        required_capabilities: BTreeSet::new(),
        payload: json!({}),
    };
    let mut context =
        ConnectorProtocolContext::from_connector_command(&provider, &channel, &command);
    super::authorize_connector_protocol_context(&mut context)
        .expect("protocol context should authorize");

    let runtime = process_stdio_runtime_evidence(
        &context,
        BridgeRuntimePolicy {
            execute_process_stdio: true,
            allowed_process_commands: BTreeSet::from(["demo-connector".to_owned()]),
            ..BridgeRuntimePolicy::default()
        }
        .process_stdio_execution_security_tier(),
        "demo-connector",
        &["--serve".to_owned()],
        5_000,
        super::ProcessStdioRuntimeEvidenceKind::BaseOnly,
    );

    assert_eq!(runtime["execution_tier"], json!("balanced"));
}

#[test]
fn execute_wasm_component_bridge_reports_restricted_execution_tier() {
    let unique = format!(
        "loongclaw-wasm-tier-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    );
    let root = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&root).expect("create temp wasm root");
    let wasm_path = root.join("fixture.wasm");
    std::fs::write(&wasm_path, EMPTY_WASM_MODULE).expect("write wasm fixture");

    let provider = provider_with_metadata(BTreeMap::from([
        ("component".to_owned(), wasm_path.display().to_string()),
        ("plugin_id".to_owned(), "plugin-a".to_owned()),
    ]));
    let channel = kernel::ChannelConfig {
        channel_id: "channel-wasm".to_owned(),
        endpoint: "local://fixture".to_owned(),
        provider_id: provider.provider_id.clone(),
        enabled: true,
        metadata: BTreeMap::new(),
    };
    let command = kernel::ConnectorCommand {
        connector_name: "connector-x".to_owned(),
        operation: "call".to_owned(),
        required_capabilities: BTreeSet::new(),
        payload: json!({}),
    };
    let runtime_policy = BridgeRuntimePolicy {
        execute_wasm_component: true,
        wasm_allowed_path_prefixes: vec![root.clone()],
        ..BridgeRuntimePolicy::default()
    };

    let execution = super::execute_wasm_component_bridge(
        json!({"status": "planned"}),
        &provider,
        &channel,
        &command,
        &runtime_policy,
    );

    assert_eq!(execution["runtime"]["execution_tier"], json!("restricted"));
    let _ = std::fs::remove_file(&wasm_path);
    let _ = std::fs::remove_dir(&root);
}

#[test]
fn execute_wasm_component_bridge_reports_runtime_on_artifact_resolution_failure() {
    let provider = provider_with_metadata(BTreeMap::new());
    let channel = kernel::ChannelConfig {
        channel_id: "channel-wasm".to_owned(),
        endpoint: "local://fixture".to_owned(),
        provider_id: provider.provider_id.clone(),
        enabled: true,
        metadata: BTreeMap::new(),
    };
    let command = kernel::ConnectorCommand {
        connector_name: "connector-x".to_owned(),
        operation: "call".to_owned(),
        required_capabilities: BTreeSet::new(),
        payload: json!({}),
    };
    let runtime_policy = BridgeRuntimePolicy {
        execute_wasm_component: true,
        ..BridgeRuntimePolicy::default()
    };

    let execution = super::execute_wasm_component_bridge(
        json!({"status": "planned"}),
        &provider,
        &channel,
        &command,
        &runtime_policy,
    );

    assert_eq!(execution["status"], json!("blocked"));
    assert_eq!(
        execution["reason"],
        json!("wasm_component execution requires component artifact path")
    );
    assert_eq!(execution["runtime"]["executor"], json!("wasmtime_module"));
    assert_eq!(execution["runtime"]["execution_tier"], json!("restricted"));
}

#[test]
fn wasm_module_cache_key_distinguishes_expected_sha256_pin() {
    let path = Path::new("/tmp/pin-test.wasm");
    let pin_a = "aa".repeat(32);
    let pin_b = "bb".repeat(32);
    let key_a = build_wasm_module_cache_key(path, 8, Some(1), None, Some(pin_a), false);
    let key_b = build_wasm_module_cache_key(path, 8, Some(1), None, Some(pin_b), false);
    assert_ne!(key_a, key_b);
}

#[test]
fn wasm_module_cache_evicts_lru_entries_when_byte_budget_exceeded() {
    let compiled = Arc::new(
        compile_wasm_module(&EMPTY_WASM_MODULE, false, None)
            .expect("empty wasm module should compile"),
    );
    let mut cache = WasmModuleCache::default();
    let key_a =
        build_wasm_module_cache_key(Path::new("/tmp/a.wasm"), 6, Some(1), None, None, false);
    let key_b =
        build_wasm_module_cache_key(Path::new("/tmp/b.wasm"), 6, Some(2), None, None, false);

    let first = cache.insert(key_a.clone(), compiled.clone(), 6, 8, 10);
    assert!(first.inserted);
    assert_eq!(first.evicted_entries, 0);
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.total_module_bytes(), 6);

    let second = cache.insert(key_b.clone(), compiled, 6, 8, 10);
    assert!(second.inserted);
    assert_eq!(second.evicted_entries, 1);
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.total_module_bytes(), 6);
    assert!(cache.get(&key_a).is_none());
    assert!(cache.get(&key_b).is_some());
}

#[test]
fn wasm_module_cache_skips_single_module_larger_than_byte_budget() {
    let compiled = Arc::new(
        compile_wasm_module(&EMPTY_WASM_MODULE, false, None)
            .expect("empty wasm module should compile"),
    );
    let mut cache = WasmModuleCache::default();
    let baseline =
        build_wasm_module_cache_key(Path::new("/tmp/base.wasm"), 4, Some(1), None, None, false);
    let oversized = build_wasm_module_cache_key(
        Path::new("/tmp/oversized.wasm"),
        11,
        Some(2),
        None,
        None,
        false,
    );

    let baseline_insert = cache.insert(baseline.clone(), compiled.clone(), 4, 8, 10);
    assert!(baseline_insert.inserted);

    let oversized_insert = cache.insert(oversized.clone(), compiled, 11, 8, 10);
    assert!(!oversized_insert.inserted);
    assert_eq!(oversized_insert.evicted_entries, 0);
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.total_module_bytes(), 4);
    assert!(cache.get(&baseline).is_some());
    assert!(cache.get(&oversized).is_none());
}

#[cfg(unix)]
#[test]
fn wasm_artifact_file_identity_distinguishes_different_files() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let base = std::env::temp_dir().join(format!("loongclaw-wasm-file-identity-{unique}"));
    fs::create_dir_all(&base).expect("create temp dir");
    let file_a = base.join("a.wasm");
    let file_b = base.join("b.wasm");
    fs::write(&file_a, b"(module)").expect("write file a");
    fs::write(&file_b, b"(module)").expect("write file b");

    let metadata_a = fs::metadata(&file_a).expect("metadata file a");
    let metadata_b = fs::metadata(&file_b).expect("metadata file b");
    let identity_a =
        wasm_artifact_file_identity(&metadata_a).expect("file identity for file a exists");
    let identity_b =
        wasm_artifact_file_identity(&metadata_b).expect("file identity for file b exists");

    assert_ne!(identity_a, identity_b);
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn core_tool_runtime_claw_migrate_without_native_executor_fails_closed() {
    let error = CoreToolRuntime::default()
        .execute_core_tool(ToolCoreRequest {
            tool_name: "claw.migrate".to_owned(),
            payload: json!({"mode": "plan"}),
        })
        .await
        .expect_err("native-only tool execution should fail without an injected executor");

    assert!(error.to_string().contains("native tool executor"));
}

fn test_native_tool_executor(request: ToolCoreRequest) -> Option<Result<ToolCoreOutcome, String>> {
    if request.tool_name != "claw.migrate" {
        return None;
    }
    Some(Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "native-tools",
            "tool": request.tool_name,
        }),
    }))
}

#[tokio::test]
async fn core_tool_runtime_uses_explicit_native_executor_when_present() {
    let outcome = CoreToolRuntime::new(Some(test_native_tool_executor))
        .execute_core_tool(ToolCoreRequest {
            tool_name: "claw.migrate".to_owned(),
            payload: json!({"mode": "plan"}),
        })
        .await
        .expect("native tool execution should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["adapter"], "native-tools");
    assert_eq!(outcome.payload["tool"], "claw.migrate");
}

fn declining_native_tool_executor(
    request: ToolCoreRequest,
) -> Option<Result<ToolCoreOutcome, String>> {
    if request.tool_name == "claw.migrate" {
        return None;
    }
    Some(Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "native-tools",
            "tool": request.tool_name,
        }),
    }))
}

#[tokio::test]
async fn core_tool_runtime_claw_migrate_fails_closed_when_executor_declines_request() {
    let error = CoreToolRuntime::new(Some(declining_native_tool_executor))
        .execute_core_tool(ToolCoreRequest {
            tool_name: "claw.migrate".to_owned(),
            payload: json!({"mode": "plan"}),
        })
        .await
        .expect_err("native-only tool execution should fail closed when executor declines");

    assert!(error.to_string().contains("native tool executor"));
}
