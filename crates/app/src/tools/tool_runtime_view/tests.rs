use super::*;

#[cfg(feature = "tool-file")]
#[test]
fn delegate_projection_uses_registered_paths_and_contract_allowlist() {
    let config = crate::config::LoongConfig::default();
    let runtime = crate::runtime::bootstrap_runtime_with_config(&config)
        .expect("builtin runtime should construct");
    let contract = ConstrainedSubagentContractView {
        child_tool_allowlist: vec!["read".to_owned()],
        ..ConstrainedSubagentContractView::default()
    };

    let view = runtime_delegate_child_tool_view(&runtime, &config.tools, Some(&contract));

    assert!(view.contains("read"));
    assert!(!view.contains("write"));
    assert!(!view.contains("edit"));
}
