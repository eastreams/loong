use std::collections::BTreeMap;

use super::acpx::AcpxCliProbeBackend;
use super::backend::AcpRuntimeBackend;
use crate::config::AcpConfig;
use crate::config::LoongConfig;

#[tokio::test]
async fn doctor_reports_invalid_acpx_backend_mcp_config() {
    let backend = AcpxCliProbeBackend;
    let config = LoongConfig {
        acp: AcpConfig {
            allow_mcp_server_injection: true,
            backends: crate::config::AcpBackendProfilesConfig {
                acpx: Some(crate::config::AcpxBackendConfig {
                    mcp_servers: BTreeMap::from([(
                        "".to_owned(),
                        crate::config::AcpxMcpServerConfig {
                            command: "uvx".to_owned(),
                            args: vec!["context7-mcp".to_owned()],
                            env: BTreeMap::new(),
                        },
                    )]),
                    ..crate::config::AcpxBackendConfig::default()
                }),
            },
            ..AcpConfig::default()
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
