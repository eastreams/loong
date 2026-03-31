use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
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
    BridgeRuntimePolicy, ConnectorProtocolContext, CoreToolRuntime, WasmModuleCache,
    build_wasm_module_cache_key, compile_wasm_module, normalize_sha256_pin,
    process_stdio_runtime_evidence, resolve_expected_wasm_sha256,
};
use kernel::{CoreToolAdapter, ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Value, json};

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

fn temp_wasm_fixture(prefix: &str, wat_source: &str) -> (PathBuf, PathBuf) {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{prefix}-{unique}"));
    fs::create_dir_all(&root).expect("create temp wasm root");
    let wasm_path = root.join("fixture.wasm");
    let wasm_bytes = wat::parse_str(wat_source).expect("compile wasm fixture");
    fs::write(&wasm_path, wasm_bytes).expect("write wasm fixture");
    (root, wasm_path)
}

fn test_wasm_channel(provider: &kernel::ProviderConfig) -> kernel::ChannelConfig {
    kernel::ChannelConfig {
        channel_id: "channel-wasm".to_owned(),
        endpoint: "local://fixture".to_owned(),
        provider_id: provider.provider_id.clone(),
        enabled: true,
        metadata: BTreeMap::new(),
    }
}

fn test_wasm_command(payload: Value) -> kernel::ConnectorCommand {
    kernel::ConnectorCommand {
        connector_name: "connector-x".to_owned(),
        operation: "invoke".to_owned(),
        required_capabilities: BTreeSet::from([kernel::Capability::InvokeConnector]),
        payload,
    }
}

fn test_wasm_runtime_policy(root: &Path) -> BridgeRuntimePolicy {
    BridgeRuntimePolicy {
        execute_wasm_component: true,
        wasm_allowed_path_prefixes: vec![root.to_path_buf()],
        wasm_fuel_limit: Some(200_000),
        ..BridgeRuntimePolicy::default()
    }
}

#[test]
fn execute_wasm_component_bridge_exchanges_request_output_and_logs() {
    let wat_source = r#"
            (module
              (import "loongclaw" "input_len" (func $input_len (result i32)))
              (import "loongclaw" "read_input" (func $read_input (param i32 i32) (result i32)))
              (import "loongclaw" "write_output" (func $write_output (param i32 i32) (result i32)))
              (import "loongclaw" "log" (func $log (param i32 i32) (result i32)))
              (func (export "run") (result i32)
                (local $input_len i32)
                i32.const 0
                i32.const 5
                call $log
                drop
                call $input_len
                local.set $input_len
                i32.const 32
                local.get $input_len
                call $read_input
                drop
                i32.const 32
                local.get $input_len
                call $write_output
                drop
                i32.const 0)
              (memory (export "memory") 1)
              (data (i32.const 0) "hello"))
        "#;
    let (root, wasm_path) = temp_wasm_fixture("loongclaw-wasm-host-abi-ok", wat_source);
    let component_path = wasm_path.display().to_string();
    let provider =
        provider_with_metadata(BTreeMap::from([("component".to_owned(), component_path)]));
    let channel = test_wasm_channel(&provider);
    let command = test_wasm_command(json!({
        "input": "ping",
        "nested": {
            "ok": true,
        },
    }));
    let runtime_policy = test_wasm_runtime_policy(&root);
    let expected_request = super::wasm_bridge_request_payload(&provider, &channel, &command);

    let execution = super::execute_wasm_component_bridge(
        json!({"status": "planned"}),
        &provider,
        &channel,
        &command,
        &runtime_policy,
    );

    assert_eq!(execution["status"], json!("executed"));
    assert_eq!(execution["runtime"]["host_abi_enabled"], json!(true));
    assert_eq!(
        execution["runtime"]["entrypoint_signature"],
        json!("() -> i32")
    );
    assert_eq!(execution["runtime"]["guest_exit_code"], json!(0));
    assert_eq!(execution["runtime"]["guest_logs"], json!(["hello"]));
    assert_eq!(execution["runtime"]["guest_logs_truncated"], json!(false));
    assert_eq!(execution["runtime"]["output_json"], expected_request);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn execute_wasm_component_bridge_preserves_legacy_unit_signature() {
    let wat_source = r#"(module (func (export "run")))"#;
    let (root, wasm_path) = temp_wasm_fixture("loongclaw-wasm-legacy-ok", wat_source);
    let component_path = wasm_path.display().to_string();
    let provider =
        provider_with_metadata(BTreeMap::from([("component".to_owned(), component_path)]));
    let channel = test_wasm_channel(&provider);
    let command = test_wasm_command(json!({"input":"ping"}));
    let runtime_policy = test_wasm_runtime_policy(&root);

    let execution = super::execute_wasm_component_bridge(
        json!({"status": "planned"}),
        &provider,
        &channel,
        &command,
        &runtime_policy,
    );

    assert_eq!(execution["status"], json!("executed"));
    assert_eq!(execution["runtime"]["host_abi_enabled"], json!(false));
    assert_eq!(
        execution["runtime"]["entrypoint_signature"],
        json!("() -> ()")
    );
    assert_eq!(execution["runtime"]["guest_exit_code"], Value::Null);
    assert_eq!(execution["runtime"]["guest_logs"], json!([]));
    assert_eq!(execution["runtime"]["output_json"], Value::Null);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn execute_wasm_component_bridge_allows_scalar_host_abi_imports_without_memory() {
    let wat_source = r#"
            (module
              (import "loongclaw" "input_len" (func $input_len (result i32)))
              (func (export "run") (result i32)
                call $input_len
                drop
                i32.const 0))
        "#;
    let (root, wasm_path) = temp_wasm_fixture("loongclaw-wasm-host-abi-no-memory", wat_source);
    let component_path = wasm_path.display().to_string();
    let provider =
        provider_with_metadata(BTreeMap::from([("component".to_owned(), component_path)]));
    let channel = test_wasm_channel(&provider);
    let command = test_wasm_command(json!({"input":"ping"}));
    let runtime_policy = test_wasm_runtime_policy(&root);

    let execution = super::execute_wasm_component_bridge(
        json!({"status": "planned"}),
        &provider,
        &channel,
        &command,
        &runtime_policy,
    );

    assert_eq!(execution["status"], json!("executed"));
    assert_eq!(execution["runtime"]["guest_exit_code"], json!(0));
    assert_eq!(execution["runtime"]["host_abi_enabled"], json!(true));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn execute_wasm_component_bridge_fails_when_memory_backed_host_abi_is_missing_memory() {
    let wat_source = r#"
            (module
              (import "loongclaw" "read_input" (func $read_input (param i32 i32) (result i32)))
              (func (export "run") (result i32)
                i32.const 0
                i32.const 0
                call $read_input
                drop
                i32.const 0))
        "#;
    let (root, wasm_path) = temp_wasm_fixture("loongclaw-wasm-host-abi-no-memory", wat_source);
    let component_path = wasm_path.display().to_string();
    let provider =
        provider_with_metadata(BTreeMap::from([("component".to_owned(), component_path)]));
    let channel = test_wasm_channel(&provider);
    let command = test_wasm_command(json!({"input":"ping"}));
    let runtime_policy = test_wasm_runtime_policy(&root);

    let execution = super::execute_wasm_component_bridge(
        json!({"status": "planned"}),
        &provider,
        &channel,
        &command,
        &runtime_policy,
    );

    assert_eq!(execution["status"], json!("failed"));
    assert_eq!(
        execution["reason"],
        json!("wasm host ABI requires exported memory")
    );
    assert_eq!(execution["runtime"]["host_abi_enabled"], json!(true));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn execute_wasm_component_bridge_times_out_during_instantiation_start_function() {
    let wat_source = r#"
            (module
              (func $start
                (loop
                  br 0))
              (func (export "run") (result i32)
                i32.const 0)
              (start $start))
        "#;
    let (root, wasm_path) = temp_wasm_fixture("loongclaw-wasm-instantiate-timeout", wat_source);
    let component_path = wasm_path.display().to_string();
    let provider =
        provider_with_metadata(BTreeMap::from([("component".to_owned(), component_path)]));
    let channel = test_wasm_channel(&provider);
    let command = test_wasm_command(json!({"input":"ping"}));
    let mut runtime_policy = test_wasm_runtime_policy(&root);
    runtime_policy.wasm_fuel_limit = None;
    runtime_policy.wasm_timeout_ms = Some(50);

    let execution = super::execute_wasm_component_bridge(
        json!({"status": "planned"}),
        &provider,
        &channel,
        &command,
        &runtime_policy,
    );

    assert_eq!(execution["status"], json!("failed"));
    assert_eq!(
        execution["reason"],
        json!("wasm execution timed out after 50ms")
    );
    assert_eq!(execution["runtime"]["timeout_ms"], json!(50));
    assert_eq!(execution["runtime"]["timeout_triggered"], json!(true));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn execute_wasm_component_bridge_fails_when_guest_output_is_not_json() {
    let wat_source = r#"
            (module
              (import "loongclaw" "write_output" (func $write_output (param i32 i32) (result i32)))
              (func (export "run") (result i32)
                i32.const 0
                i32.const 8
                call $write_output
                drop
                i32.const 0)
              (memory (export "memory") 1)
              (data (i32.const 0) "not-json"))
        "#;
    let (root, wasm_path) = temp_wasm_fixture("loongclaw-wasm-host-abi-bad-output", wat_source);
    let component_path = wasm_path.display().to_string();
    let provider =
        provider_with_metadata(BTreeMap::from([("component".to_owned(), component_path)]));
    let channel = test_wasm_channel(&provider);
    let command = test_wasm_command(json!({"input":"ping"}));
    let runtime_policy = test_wasm_runtime_policy(&root);

    let execution = super::execute_wasm_component_bridge(
        json!({"status": "planned"}),
        &provider,
        &channel,
        &command,
        &runtime_policy,
    );

    assert_eq!(execution["status"], json!("failed"));
    assert!(
        execution["reason"]
            .as_str()
            .expect("reason should be a string")
            .contains("wasm guest output is not valid JSON")
    );
    assert_eq!(execution["runtime"]["output_text"], json!("not-json"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn execute_wasm_component_bridge_respects_configured_output_limit() {
    let wat_source = r#"
            (module
              (import "loongclaw" "write_output" (func $write_output (param i32 i32) (result i32)))
              (func (export "run") (result i32)
                i32.const 0
                i32.const 8
                call $write_output
                drop
                i32.const 0)
              (memory (export "memory") 1)
              (data (i32.const 0) "not-json"))
        "#;
    let (root, wasm_path) = temp_wasm_fixture("loongclaw-wasm-host-abi-output-limit", wat_source);
    let component_path = wasm_path.display().to_string();
    let provider =
        provider_with_metadata(BTreeMap::from([("component".to_owned(), component_path)]));
    let channel = test_wasm_channel(&provider);
    let command = test_wasm_command(json!({"input":"ping"}));
    let mut runtime_policy = test_wasm_runtime_policy(&root);
    runtime_policy.wasm_max_output_bytes = Some(4);

    let execution = super::execute_wasm_component_bridge(
        json!({"status": "planned"}),
        &provider,
        &channel,
        &command,
        &runtime_policy,
    );

    assert_eq!(execution["status"], json!("failed"));
    assert_eq!(
        execution["reason"],
        json!("wasm guest output exceeds host ABI limit of 4 bytes")
    );
    assert_eq!(execution["runtime"]["max_output_bytes"], json!(4));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn execute_wasm_component_bridge_reports_guest_abort_reason() {
    let wat_source = r#"
            (module
              (import "loongclaw" "abort" (func $abort (param i32)))
              (func (export "run") (result i32)
                i32.const 7
                call $abort
                i32.const 0)
              (memory (export "memory") 1))
        "#;
    let (root, wasm_path) = temp_wasm_fixture("loongclaw-wasm-host-abi-abort", wat_source);
    let component_path = wasm_path.display().to_string();
    let provider =
        provider_with_metadata(BTreeMap::from([("component".to_owned(), component_path)]));
    let channel = test_wasm_channel(&provider);
    let command = test_wasm_command(json!({"input":"ping"}));
    let runtime_policy = test_wasm_runtime_policy(&root);

    let execution = super::execute_wasm_component_bridge(
        json!({"status": "planned"}),
        &provider,
        &channel,
        &command,
        &runtime_policy,
    );

    assert_eq!(execution["status"], json!("failed"));
    assert_eq!(execution["reason"], json!("wasm guest aborted with code 7"));
    assert_eq!(execution["runtime"]["host_abi_enabled"], json!(true));
    assert_eq!(
        execution["runtime"]["entrypoint_signature"],
        json!("() -> i32")
    );
    let _ = fs::remove_dir_all(root);
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
    let key_a = build_wasm_module_cache_key(path, 8, Some(1), None, Some(pin_a), false, false);
    let key_b = build_wasm_module_cache_key(path, 8, Some(1), None, Some(pin_b), false, false);
    assert_ne!(key_a, key_b);
}

#[test]
fn wasm_module_cache_key_distinguishes_epoch_interruption_configuration() {
    let path = Path::new("/tmp/epoch-interrupt-test.wasm");
    let key_without_epoch = build_wasm_module_cache_key(path, 8, Some(1), None, None, false, false);
    let key_with_epoch = build_wasm_module_cache_key(path, 8, Some(1), None, None, false, true);
    assert_ne!(key_without_epoch, key_with_epoch);
}

#[test]
fn wasm_module_cache_evicts_lru_entries_when_byte_budget_exceeded() {
    let compiled = Arc::new(
        compile_wasm_module(&EMPTY_WASM_MODULE, false, false, None)
            .expect("empty wasm module should compile"),
    );
    let mut cache = WasmModuleCache::default();
    let key_a = build_wasm_module_cache_key(
        Path::new("/tmp/a.wasm"),
        6,
        Some(1),
        None,
        None,
        false,
        false,
    );
    let key_b = build_wasm_module_cache_key(
        Path::new("/tmp/b.wasm"),
        6,
        Some(2),
        None,
        None,
        false,
        false,
    );

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
        compile_wasm_module(&EMPTY_WASM_MODULE, false, false, None)
            .expect("empty wasm module should compile"),
    );
    let mut cache = WasmModuleCache::default();
    let baseline = build_wasm_module_cache_key(
        Path::new("/tmp/base.wasm"),
        4,
        Some(1),
        None,
        None,
        false,
        false,
    );
    let oversized = build_wasm_module_cache_key(
        Path::new("/tmp/oversized.wasm"),
        11,
        Some(2),
        None,
        None,
        false,
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
