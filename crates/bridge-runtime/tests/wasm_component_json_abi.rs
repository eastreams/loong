use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use loong_bridge_runtime::{BridgeExecutionPolicy, execute_wasm_component_bridge_call};
use loong_contracts::Capability;
use loong_kernel::{ChannelConfig, ConnectorCommand, ProviderConfig};
use serde_json::{Value, json};

#[tokio::test]
async fn execute_wasm_component_bridge_call_runs_wat_json_abi_module() -> Result<(), Box<dyn Error>>
{
    let (root, module_path) = write_demo_wat_module()?;
    let provider = ProviderConfig {
        provider_id: "wasm-provider".to_owned(),
        connector_name: "wasm-connector".to_owned(),
        version: "0.1.0".to_owned(),
        metadata: BTreeMap::from([("package_root".to_owned(), root.display().to_string())]),
    };
    let channel = ChannelConfig {
        channel_id: "weixin".to_owned(),
        provider_id: "wasm-provider".to_owned(),
        endpoint: module_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("plugin.wat")
            .to_owned(),
        enabled: true,
        metadata: BTreeMap::new(),
    };
    let command = ConnectorCommand {
        connector_name: "wasm-connector".to_owned(),
        operation: "send_message".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        payload: json!({"message":"hello wasm"}),
    };
    let runtime_policy = BridgeExecutionPolicy {
        execute_process_stdio: false,
        execute_http_json: false,
        execute_wasm_component: true,
        allowed_process_commands: BTreeSet::new(),
    };

    let result = execute_wasm_component_bridge_call(&provider, &channel, &command, &runtime_policy)
        .await
        .map_err(|failure| std::io::Error::other(failure.reason))?;

    ensure_value_eq(
        "response_payload.ok",
        result.response_payload.get("ok"),
        Some(&Value::Bool(true)),
    )?;
    ensure_str_eq(
        "response_payload.via",
        result.response_payload.get("via").and_then(Value::as_str),
        Some("wasm"),
    )?;
    ensure_str_eq(
        "runtime_evidence.executor",
        result
            .runtime_evidence
            .get("executor")
            .and_then(Value::as_str),
        Some("wasm_component_wasmtime_json_abi"),
    )?;
    ensure_str_eq(
        "runtime_evidence.abi",
        result.runtime_evidence.get("abi").and_then(Value::as_str),
        Some("loong_json_abi_v1"),
    )?;

    let _ = fs::remove_dir_all(root);
    Ok(())
}

fn write_demo_wat_module() -> Result<(PathBuf, PathBuf), Box<dyn Error>> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::temp_dir().join(format!("loong-wasm-json-abi-test-{timestamp}"));
    fs::create_dir_all(&root)?;
    let module_path = root.join("plugin.wat");
    fs::write(&module_path, demo_wat_module())?;

    Ok((root, module_path))
}

fn ensure_value_eq(
    label: &str,
    actual: Option<&Value>,
    expected: Option<&Value>,
) -> Result<(), Box<dyn Error>> {
    if actual == expected {
        return Ok(());
    }

    Err(std::io::Error::other(format!(
        "{label} mismatch: actual={actual:?}, expected={expected:?}"
    ))
    .into())
}

fn ensure_str_eq(
    label: &str,
    actual: Option<&str>,
    expected: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    if actual == expected {
        return Ok(());
    }

    Err(std::io::Error::other(format!(
        "{label} mismatch: actual={actual:?}, expected={expected:?}"
    ))
    .into())
}

fn demo_wat_module() -> &'static str {
    r#"
(module
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 2048))
  (data (i32.const 1024) "{\"payload\":{\"ok\":true,\"via\":\"wasm\",\"messages\":[],\"demo_message\":\"handled by Loong demo WASM plugin\"}}")
  (func (export "loong_alloc") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $len)))
    (local.get $ptr)
  )
  (func (export "loong_free") (param $ptr i32) (param $len i32))
  (func (export "loong_invoke") (param $ptr i32) (param $len i32) (result i64)
    (i64.or
      (i64.shl (i64.extend_i32_u (i32.const 1024)) (i64.const 32))
      (i64.extend_i32_u (i32.const 101))
    )
  )
)
"#
}
