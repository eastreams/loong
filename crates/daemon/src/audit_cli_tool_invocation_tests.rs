use loong_contracts::{Capability, InvocationOutcome};

use super::*;

#[test]
fn tool_invocation_event_exposes_filter_and_render_fields() {
    let kind = AuditEventKind::ToolInvocation {
        pack_id: "workspace-pack".to_owned(),
        path_display: "read".to_owned(),
        required_capabilities: vec![Capability::FilesystemRead],
        outcome: InvocationOutcome::Completed,
    };

    assert_eq!(audit_event_pack_id(&kind), Some("workspace-pack"));
    assert_eq!(audit_event_kind_label(&kind), "ToolInvocation");
    assert_eq!(triage_event_label(&kind), None);
    assert_eq!(
        format_audit_event_detail(&kind),
        "pack_id=workspace-pack path=read required_capabilities=[FilesystemRead] outcome=Completed"
    );
}
