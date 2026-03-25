use super::*;

#[tokio::test]
async fn execute_spec_allows_execution_with_clean_architecture_guard() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("loongclaw-guard-clean-{unique}"));
    fs::create_dir_all(&root).expect("create awareness root");
    fs::write(root.join("pack.md"), "# awareness\n").expect("write awareness file");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-guard-clean".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-guard-clean".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: Some(SelfAwarenessSpec {
            enabled: true,
            roots: vec![root.display().to_string()],
            plugin_roots: Vec::new(),
            proposed_mutations: vec!["examples/spec/runtime-extension.json".to_owned()],
            enforce_guard: true,
            immutable_core_paths: Vec::new(),
            mutable_extension_paths: Vec::new(),
        }),
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-guard-clean".to_owned(),
            objective: "run with clean guard".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "task");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert!(report.blocked_reason.is_none());
    assert!(report.self_awareness.is_some());
    assert!(
        report
            .architecture_guard
            .expect("guard report should be present")
            .denied_paths
            .is_empty()
    );
}

#[tokio::test]
async fn execute_spec_blocks_when_architecture_guard_detects_core_mutation() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("loongclaw-guard-{unique}"));
    fs::create_dir_all(&root).expect("create awareness root");
    fs::write(root.join("notes.md"), "# guard demo\n").expect("write awareness file");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-guard-block".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-guard".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: Some(SelfAwarenessSpec {
            enabled: true,
            roots: vec![root.display().to_string()],
            plugin_roots: Vec::new(),
            proposed_mutations: vec![
                "crates/kernel/src/kernel.rs".to_owned(),
                "examples/spec/runtime-extension.json".to_owned(),
            ],
            enforce_guard: true,
            immutable_core_paths: Vec::new(),
            mutable_extension_paths: Vec::new(),
        }),
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-guard".to_owned(),
            objective: "should not run".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert_eq!(report.outcome["status"], "blocked");
    assert!(report.blocked_reason.is_some());
    assert!(
        report
            .architecture_guard
            .expect("guard report should be present")
            .has_denials()
    );
}
