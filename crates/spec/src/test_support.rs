use std::collections::{BTreeMap, BTreeSet};

use kernel::{Capability, ExecutionRoute, HarnessKind, VerticalPackManifest};

use crate::{OperationSpec, RunnerSpec};

pub fn make_runner_spec(operation: OperationSpec) -> RunnerSpec {
    RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-native-tool-check".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-native-tool-check".to_owned(),
        ttl_s: 60,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        plugin_setup_readiness: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        operation,
    }
}
