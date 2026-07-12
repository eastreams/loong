use std::collections::BTreeSet;

use loong_contracts::Capability;
use loong_core::policy::engine::PolicyEngine;
use loong_kernel::PolicyPipeline;
use loong_runtime::tool_plane::{ToolInvocationAction, ToolPath};
use serde_json::json;

use super::ToolInvocationAllowPolicy;
use crate::context::{AppContextFactory, bootstrap_test_kernel_context};

#[tokio::test]
async fn app_policy_allows_registered_tool_invocation_after_capability_gate() {
    let kernel_context =
        bootstrap_test_kernel_context("test-agent", 60).expect("bootstrap context");
    let execution_context = kernel_context
        .memory_core_execution_context()
        .expect("build execution context");
    let mut policy = PolicyPipeline::<AppContextFactory>::new();
    policy.push_policy(ToolInvocationAllowPolicy);

    let grant = policy
        .grant(
            &execution_context,
            ToolInvocationAction::new(
                ToolPath::from("read"),
                BTreeSet::from([Capability::InvokeTool]),
                json!({ "path": "notes.txt" }),
            ),
        )
        .await;

    assert!(grant.is_ok());
}

#[cfg(feature = "tool-file")]
#[test]
fn builtin_tool_plane_exposes_registered_file_paths() {
    let paths = super::app_tool_plane().registered_paths();

    assert!(paths.contains(&ToolPath::from("read")));
    assert!(paths.contains(&ToolPath::from("write")));
    assert!(paths.contains(&ToolPath::from("glob.search")));
    assert!(paths.contains(&ToolPath::from("content.search")));
}
