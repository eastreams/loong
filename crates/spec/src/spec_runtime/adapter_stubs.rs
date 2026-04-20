use super::*;

pub struct WebhookConnector;

#[async_trait]
impl CoreConnectorAdapter for WebhookConnector {
    fn name(&self) -> &str {
        "webhook"
    }

    async fn invoke_core(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(test_config) = command
            .payload
            .as_object()
            .and_then(|payload| payload.get("_loong_test"))
            .and_then(Value::as_object)
        {
            let delay_ms = test_config
                .get("delay_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if delay_ms > 0 {
                sleep(Duration::from_millis(delay_ms)).await;
            }
            let request_id = test_config
                .get("request_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned();
            let failures_before_success = test_config
                .get("failures_before_success")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            if !request_id.is_empty() && failures_before_success > 0 {
                let attempts_map =
                    WEBHOOK_TEST_RETRY_STATE.get_or_init(|| Mutex::new(BTreeMap::new()));
                let current_attempt = {
                    let mut guard = attempts_map.lock().map_err(|_err| {
                        ConnectorError::Execution("retry test state mutex poisoned".to_owned())
                    })?;
                    let entry = guard.entry(request_id.clone()).or_insert(0);
                    *entry = entry.saturating_add(1);
                    *entry
                };
                if current_attempt <= failures_before_success {
                    return Err(ConnectorError::Execution(format!(
                        "simulated transient failure for request_id={request_id}, attempt={current_attempt}, threshold={failures_before_success}"
                    )));
                }
            }
        }

        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "connector": "webhook",
                "operation": command.operation,
                "payload": command.payload,
            }),
        })
    }
}

pub struct CrmCoreConnector;

#[async_trait]
impl CoreConnectorAdapter for CrmCoreConnector {
    fn name(&self) -> &str {
        "http-core"
    }

    async fn invoke_core(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "tier": "core",
                "adapter": "http-core",
                "connector": command.connector_name,
                "operation": command.operation,
                "payload": command.payload,
            }),
        })
    }
}

pub struct CrmGrpcCoreConnector;

#[async_trait]
impl CoreConnectorAdapter for CrmGrpcCoreConnector {
    fn name(&self) -> &str {
        "grpc-core"
    }

    async fn invoke_core(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "tier": "core",
                "adapter": "grpc-core",
                "connector": command.connector_name,
                "operation": command.operation,
                "payload": command.payload,
            }),
        })
    }
}

pub struct ShieldedConnectorExtension;

#[async_trait]
impl kernel::ConnectorExtensionAdapter for ShieldedConnectorExtension {
    fn name(&self) -> &str {
        "shielded-bridge"
    }

    async fn invoke_extension(
        &self,
        command: ConnectorCommand,
        core: &(dyn CoreConnectorAdapter + Sync),
    ) -> Result<ConnectorOutcome, ConnectorError> {
        let probe = core
            .invoke_core(ConnectorCommand {
                connector_name: command.connector_name.clone(),
                operation: "probe".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            })
            .await?;
        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "tier": "extension",
                "extension": "shielded-bridge",
                "operation": command.operation,
                "core_probe": probe.payload,
                "payload": command.payload,
            }),
        })
    }
}

pub struct NativeCoreRuntime;

#[async_trait]
impl CoreRuntimeAdapter for NativeCoreRuntime {
    fn name(&self) -> &str {
        "native-core"
    }

    async fn execute_core(
        &self,
        request: RuntimeCoreRequest,
    ) -> Result<RuntimeCoreOutcome, kernel::RuntimePlaneError> {
        Ok(RuntimeCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "native-core",
                "action": request.action,
                "payload": request.payload,
            }),
        })
    }
}

pub struct FallbackCoreRuntime;

#[async_trait]
impl CoreRuntimeAdapter for FallbackCoreRuntime {
    fn name(&self) -> &str {
        "fallback-core"
    }

    async fn execute_core(
        &self,
        request: RuntimeCoreRequest,
    ) -> Result<RuntimeCoreOutcome, kernel::RuntimePlaneError> {
        Ok(RuntimeCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "fallback-core",
                "action": request.action,
                "payload": request.payload,
            }),
        })
    }
}

pub struct AcpBridgeRuntimeExtension;

#[async_trait]
impl RuntimeExtensionAdapter for AcpBridgeRuntimeExtension {
    fn name(&self) -> &str {
        "acp-bridge"
    }

    async fn execute_extension(
        &self,
        request: RuntimeExtensionRequest,
        core: &(dyn CoreRuntimeAdapter + Sync),
    ) -> Result<RuntimeExtensionOutcome, kernel::RuntimePlaneError> {
        let core_probe = core
            .execute_core(RuntimeCoreRequest {
                action: "probe".to_owned(),
                payload: json!({}),
            })
            .await?;
        Ok(RuntimeExtensionOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "extension": "acp-bridge",
                "action": request.action,
                "core_probe": core_probe.payload,
                "payload": request.payload,
            }),
        })
    }
}

// Local stubs: spec adapters do not execute real tools or memory.
fn stub_tool_core(request: ToolCoreRequest) -> Result<ToolCoreOutcome, String> {
    Ok(ToolCoreOutcome {
        status: "ok".to_string(),
        payload: json!({ "adapter": "core-tools", "tool": request.tool_name }),
    })
}

fn maybe_execute_native_tool(
    request: &ToolCoreRequest,
    native_tool_executor: Option<crate::NativeToolExecutor>,
) -> Option<Result<ToolCoreOutcome, String>> {
    if let Some(executor) = native_tool_executor
        && let Some(result) = executor(request.clone())
    {
        return Some(result);
    }
    if crate::tool_name_requires_native_tool_executor(request.tool_name.as_str()) {
        return Some(Err(format!(
            "native tool executor required for tool `{}`",
            request.tool_name
        )));
    }
    None
}

fn stub_memory_core(request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
    Ok(MemoryCoreOutcome {
        status: "ok".to_string(),
        payload: json!({ "adapter": "kv-core", "operation": request.operation }),
    })
}

#[derive(Clone, Copy, Default)]
pub struct CoreToolRuntime {
    native_tool_executor: Option<crate::NativeToolExecutor>,
}

impl CoreToolRuntime {
    pub const fn new(native_tool_executor: Option<crate::NativeToolExecutor>) -> Self {
        Self {
            native_tool_executor,
        }
    }
}

#[async_trait]
impl CoreToolAdapter for CoreToolRuntime {
    fn name(&self) -> &str {
        "core-tools"
    }

    async fn execute_core_tool(
        &self,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, kernel::ToolPlaneError> {
        if let Some(result) = maybe_execute_native_tool(&request, self.native_tool_executor) {
            return result.map_err(kernel::ToolPlaneError::Execution);
        }
        stub_tool_core(request).map_err(kernel::ToolPlaneError::Execution)
    }
}

pub struct SqlAnalyticsToolExtension;

#[async_trait]
impl ToolExtensionAdapter for SqlAnalyticsToolExtension {
    fn name(&self) -> &str {
        "sql-analytics"
    }

    async fn execute_tool_extension(
        &self,
        request: ToolExtensionRequest,
        core: &(dyn CoreToolAdapter + Sync),
    ) -> Result<ToolExtensionOutcome, kernel::ToolPlaneError> {
        let core_probe = core
            .execute_core_tool(ToolCoreRequest {
                tool_name: "schema_probe".to_owned(),
                payload: json!({}),
            })
            .await?;
        Ok(ToolExtensionOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "extension": "sql-analytics",
                "action": request.extension_action,
                "core_probe": core_probe.payload,
                "payload": request.payload,
            }),
        })
    }
}

pub struct ClawMigrationToolExtension;

#[async_trait]
impl ToolExtensionAdapter for ClawMigrationToolExtension {
    fn name(&self) -> &str {
        "claw-migration"
    }

    async fn execute_tool_extension(
        &self,
        request: ToolExtensionRequest,
        core: &(dyn CoreToolAdapter + Sync),
    ) -> Result<ToolExtensionOutcome, kernel::ToolPlaneError> {
        let mut payload = request.payload.clone();
        if payload.get("mode").is_none()
            && let Some(object) = payload.as_object_mut()
        {
            object.insert(
                "mode".to_owned(),
                Value::String(request.extension_action.clone()),
            );
        }

        let core_outcome = core
            .execute_core_tool(ToolCoreRequest {
                tool_name: "config.import".to_owned(),
                payload,
            })
            .await?;
        let mut response = serde_json::Map::new();
        response.insert(
            "extension".to_owned(),
            Value::String("claw-migration".to_owned()),
        );
        response.insert(
            "action".to_owned(),
            Value::String(request.extension_action.clone()),
        );
        response.insert("core_outcome".to_owned(), core_outcome.payload.clone());
        if let Some(core_object) = core_outcome.payload.as_object() {
            for (key, value) in core_object {
                response.entry(key.clone()).or_insert_with(|| value.clone());
            }
        } else {
            response.insert("result".to_owned(), core_outcome.payload);
        }
        Ok(ToolExtensionOutcome {
            status: "ok".to_owned(),
            payload: Value::Object(response),
        })
    }
}

pub struct KvCoreMemory;

#[async_trait]
impl CoreMemoryAdapter for KvCoreMemory {
    fn name(&self) -> &str {
        "kv-core"
    }

    async fn execute_core_memory(
        &self,
        request: MemoryCoreRequest,
    ) -> Result<MemoryCoreOutcome, kernel::MemoryPlaneError> {
        stub_memory_core(request).map_err(kernel::MemoryPlaneError::Execution)
    }
}

pub struct VectorIndexMemoryExtension;

#[async_trait]
impl MemoryExtensionAdapter for VectorIndexMemoryExtension {
    fn name(&self) -> &str {
        "vector-index"
    }

    async fn execute_memory_extension(
        &self,
        request: MemoryExtensionRequest,
        core: &(dyn CoreMemoryAdapter + Sync),
    ) -> Result<MemoryExtensionOutcome, kernel::MemoryPlaneError> {
        let core_probe = core
            .execute_core_memory(MemoryCoreRequest {
                operation: "probe".to_owned(),
                payload: json!({}),
            })
            .await?;
        Ok(MemoryExtensionOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "extension": "vector-index",
                "operation": request.operation,
                "core_probe": core_probe.payload,
                "payload": request.payload,
            }),
        })
    }
}
