use super::*;
use crate::spec_runtime::BootstrapSpec;

#[test]
fn bootstrap_policy_maps_distinct_acp_bridge_and_runtime_flags() {
    let mut spec = RunnerSpec::template();
    spec.bootstrap = Some(BootstrapSpec {
        enabled: true,
        allow_http_json_auto_apply: None,
        allow_process_stdio_auto_apply: None,
        allow_native_ffi_auto_apply: None,
        allow_wasm_component_auto_apply: None,
        allow_mcp_server_auto_apply: None,
        allow_acp_bridge_auto_apply: Some(true),
        allow_acp_runtime_auto_apply: Some(false),
        block_unverified_high_risk_auto_apply: Some(true),
        enforce_ready_execution: None,
        max_tasks: None,
    });

    let policy = bootstrap_policy(&spec).expect("bootstrap policy should resolve");
    assert!(policy.allow_acp_bridge_auto_apply);
    assert!(!policy.allow_acp_runtime_auto_apply);
    assert!(policy.block_unverified_high_risk_auto_apply);
}
