use std::collections::BTreeMap;

use super::acpx::AcpxCliProbeBackend;
use super::backend::AcpRuntimeBackend;
use crate::config::AcpConfig;
use crate::config::LoongConfig;

#[tokio::test]
async fn doctor_reports_invalid_mcp_registry_config() {
    let backend = AcpxCliProbeBackend;
    let config = LoongConfig {
        acp: AcpConfig {
            allow_mcp_server_injection: true,
            ..AcpConfig::default()
        },
        mcp: crate::mcp::McpConfig {
            servers: BTreeMap::from([(
                "".to_owned(),
                crate::mcp::McpServerConfig {
                    transport: crate::mcp::McpServerTransportConfig::Stdio {
                        command: "uvx".to_owned(),
                        args: vec!["context7-mcp".to_owned()],
                        env: BTreeMap::new(),
                        cwd: None,
                    },
                    enabled: true,
                    required: false,
                    startup_timeout_ms: None,
                    tool_timeout_ms: None,
                    enabled_tools: Vec::new(),
                    disabled_tools: Vec::new(),
                },
            )]),
        },
        ..LoongConfig::default()
    };

    let report = backend
        .doctor(&config)
        .await
        .expect("doctor should not error")
        .expect("doctor report");

    assert!(!report.healthy);
    assert_eq!(
        report.diagnostics.get("status"),
        Some(&"invalid_config".to_owned())
    );
    assert!(
        report
            .diagnostics
            .get("error")
            .is_some_and(|error| error.contains("must not be empty"))
    );
}
