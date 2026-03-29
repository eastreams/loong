use super::*;

#[test]
fn parse_bridge_kind_label_distinguishes_acp_bridge_and_runtime() {
    assert_eq!(
        parse_bridge_kind_label("acp"),
        Some(PluginBridgeKind::AcpBridge)
    );
    assert_eq!(
        parse_bridge_kind_label("acp_bridge"),
        Some(PluginBridgeKind::AcpBridge)
    );
    assert_eq!(
        parse_bridge_kind_label("acpx"),
        Some(PluginBridgeKind::AcpRuntime)
    );
    assert_eq!(
        parse_bridge_kind_label("acp_runtime"),
        Some(PluginBridgeKind::AcpRuntime)
    );
}

#[test]
fn default_bridge_defaults_keep_acp_surfaces_distinct() {
    assert_eq!(
        default_bridge_adapter_family(PluginBridgeKind::AcpBridge),
        "acp-bridge-adapter"
    );
    assert_eq!(
        default_bridge_adapter_family(PluginBridgeKind::AcpRuntime),
        "acp-runtime-adapter"
    );
    assert_eq!(
        default_bridge_entrypoint(PluginBridgeKind::AcpBridge, "https://example.test"),
        "acp::bridge"
    );
    assert_eq!(
        default_bridge_entrypoint(PluginBridgeKind::AcpRuntime, "https://example.test"),
        "acp::turn"
    );
}
