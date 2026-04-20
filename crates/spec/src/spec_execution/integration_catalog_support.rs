use super::*;

pub(crate) fn fnv1a64_hex(bytes: &[u8]) -> String {
    activation_runtime_contract_checksum_hex(bytes)
}

pub fn hex_lower(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}

pub(super) fn default_integration_catalog() -> IntegrationCatalog {
    let mut catalog = IntegrationCatalog::new();
    for (provider_id, connector, version, class) in [
        ("openai", "openai", "1.0.0", "llm"),
        ("anthropic", "anthropic", "1.0.0", "llm"),
        ("github", "github", "1.0.0", "devops"),
        ("slack", "slack", "1.0.0", "messaging"),
        ("notion", "notion", "1.0.0", "workspace"),
    ] {
        catalog.register_template(kernel::ProviderTemplate {
            provider_id: provider_id.to_owned(),
            default_connector_name: connector.to_owned(),
            default_version: version.to_owned(),
            metadata: BTreeMap::from([("class".to_owned(), class.to_owned())]),
        });
    }
    catalog
}

pub(super) fn register_dynamic_catalog_connectors(
    kernel: &mut LoongKernel<StaticPolicyEngine>,
    catalog: Arc<Mutex<IntegrationCatalog>>,
    bridge_runtime_policy: BridgeRuntimePolicy,
) {
    let snapshot = {
        let guard = match catalog.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        guard.providers()
    };

    for provider in snapshot {
        let connector = DynamicCatalogConnector::new(
            provider.connector_name,
            provider.provider_id,
            catalog.clone(),
            bridge_runtime_policy.clone(),
        );

        kernel.register_core_connector_adapter(connector);
    }
}

pub(super) fn snapshot_runtime_integration_catalog(
    catalog: &Arc<Mutex<IntegrationCatalog>>,
) -> Result<IntegrationCatalog, String> {
    let guard = catalog
        .lock()
        .map_err(|_err| "integration catalog mutex poisoned".to_owned())?;
    let snapshot = guard.clone();

    Ok(snapshot)
}

pub(super) fn operation_connector_name(operation: &OperationSpec) -> Option<String> {
    #[allow(clippy::wildcard_enum_match_arm)]
    match operation {
        OperationSpec::ConnectorLegacy { connector_name, .. }
        | OperationSpec::ConnectorCore { connector_name, .. }
        | OperationSpec::ConnectorExtension { connector_name, .. } => Some(connector_name.clone()),
        OperationSpec::ProgrammaticToolCall { steps, .. } => {
            steps.iter().find_map(|step| match step {
                ProgrammaticStep::ConnectorCall { connector_name, .. } => {
                    Some(connector_name.clone())
                }
                ProgrammaticStep::ConnectorBatch { calls, .. } => {
                    calls.first().map(|call| call.connector_name.clone())
                }
                ProgrammaticStep::SetLiteral { .. }
                | ProgrammaticStep::JsonPointer { .. }
                | ProgrammaticStep::Conditional { .. } => None,
            })
        }
        _ => None,
    }
}
