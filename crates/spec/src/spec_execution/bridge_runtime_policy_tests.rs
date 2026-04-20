use std::collections::BTreeSet;

use super::*;

#[test]
fn bridge_runtime_policy_honors_raw_circuit_breaker_override_when_scan_is_disabled() {
    let mut spec = RunnerSpec::template();
    let (mut bridge, _source) = load_bundled_bridge_support_policy("openclaw-ecosystem-balanced")
        .expect("bundled bridge support should resolve");
    let mut security_scan = SecurityScanSpec {
        enabled: false,
        ..SecurityScanSpec::default()
    };
    security_scan.runtime.bridge_circuit_breaker.enabled = false;
    bridge.security_scan = Some(security_scan);
    spec.bridge_support = Some(bridge);

    let policy = bridge_runtime_policy(&spec, None).expect("bridge runtime policy should build");

    assert!(!policy.bridge_circuit_breaker.enabled);
}

#[test]
fn bridge_runtime_policy_rejects_invalid_circuit_breaker_override() {
    let mut spec = RunnerSpec::template();
    let (mut bridge, _source) = load_bundled_bridge_support_policy("openclaw-ecosystem-balanced")
        .expect("bundled bridge support should resolve");
    let mut security_scan = SecurityScanSpec {
        enabled: false,
        ..SecurityScanSpec::default()
    };
    security_scan
        .runtime
        .bridge_circuit_breaker
        .failure_threshold = 0;
    bridge.security_scan = Some(security_scan);
    spec.bridge_support = Some(bridge);

    let error = bridge_runtime_policy(&spec, None)
        .expect_err("invalid bridge circuit breaker policy should fail");

    assert!(error.contains("failure_threshold"));
}

#[test]
fn bridge_runtime_policy_normalizes_guest_readable_config_keys() {
    let mut spec = RunnerSpec::template();
    let (mut bridge, _source) = load_bundled_bridge_support_policy("openclaw-ecosystem-balanced")
        .expect("bundled bridge support should resolve");
    let mut security_scan = SecurityScanSpec {
        enabled: true,
        ..SecurityScanSpec::default()
    };
    security_scan.runtime.guest_readable_config_keys = vec![
        " provider.region ".to_owned(),
        "channel.mode".to_owned(),
        "provider.region".to_owned(),
    ];
    bridge.security_scan = Some(security_scan);
    spec.bridge_support = Some(bridge);
    let security_scan = spec
        .bridge_support
        .as_ref()
        .and_then(|bridge| bridge.security_scan.as_ref());

    let policy =
        bridge_runtime_policy(&spec, security_scan).expect("bridge runtime policy should build");

    assert_eq!(
        policy.wasm_guest_readable_config_keys,
        BTreeSet::from(["channel.mode".to_owned(), "provider.region".to_owned(),])
    );
}

#[test]
fn bridge_runtime_policy_rejects_invalid_guest_readable_config_key_namespace() {
    let mut spec = RunnerSpec::template();
    let (mut bridge, _source) = load_bundled_bridge_support_policy("openclaw-ecosystem-balanced")
        .expect("bundled bridge support should resolve");
    let mut security_scan = SecurityScanSpec {
        enabled: true,
        ..SecurityScanSpec::default()
    };
    security_scan.runtime.guest_readable_config_keys = vec!["region".to_owned()];
    bridge.security_scan = Some(security_scan);
    spec.bridge_support = Some(bridge);
    let security_scan = spec
        .bridge_support
        .as_ref()
        .and_then(|bridge| bridge.security_scan.as_ref());

    let error = bridge_runtime_policy(&spec, security_scan)
        .expect_err("invalid guest-readable config key should fail");

    assert!(error.contains("guest_readable_config_keys"));
    assert!(error.contains("region"));
    assert!(error.contains(WASM_GUEST_CONFIG_PROVIDER_PREFIX));
    assert!(error.contains(WASM_GUEST_CONFIG_CHANNEL_PREFIX));
}

#[test]
fn bridge_runtime_policy_rejects_guest_readable_config_key_with_inner_whitespace() {
    let mut spec = RunnerSpec::template();
    let (mut bridge, _source) = load_bundled_bridge_support_policy("openclaw-ecosystem-balanced")
        .expect("bundled bridge support should resolve");
    let mut security_scan = SecurityScanSpec {
        enabled: true,
        ..SecurityScanSpec::default()
    };
    security_scan.runtime.guest_readable_config_keys = vec!["provider.region name".to_owned()];
    bridge.security_scan = Some(security_scan);
    spec.bridge_support = Some(bridge);
    let security_scan = spec
        .bridge_support
        .as_ref()
        .and_then(|bridge| bridge.security_scan.as_ref());

    let error = bridge_runtime_policy(&spec, security_scan)
        .expect_err("guest-readable config key with inner whitespace should fail");

    assert!(error.contains("provider.region name"));
    assert!(error.contains("guest_readable_config_keys"));
}

#[tokio::test]
async fn execute_spec_rejects_invalid_guest_readable_config_keys_before_plugin_scan() {
    let mut spec = RunnerSpec::template();
    let (mut bridge, _source) = load_bundled_bridge_support_policy("openclaw-ecosystem-balanced")
        .expect("bundled bridge support should resolve");
    let mut security_scan = SecurityScanSpec {
        enabled: true,
        ..SecurityScanSpec::default()
    };
    security_scan.runtime.guest_readable_config_keys = vec!["region".to_owned()];
    bridge.security_scan = Some(security_scan);
    spec.bridge_support = Some(bridge);
    spec.plugin_scan = Some(PluginScanSpec {
        enabled: true,
        roots: vec!["/definitely/missing/loong-plugin-root".to_owned()],
    });

    let report = execute_spec(&spec, false).await;
    let blocked_reason = report
        .blocked_reason
        .expect("invalid bridge runtime policy should block the spec");

    assert_eq!(report.operation_kind, "blocked");
    assert!(blocked_reason.contains("bridge runtime policy is invalid"));
    assert!(blocked_reason.contains("guest_readable_config_keys"));
    assert!(!blocked_reason.contains("plugin scan failed"));
}
