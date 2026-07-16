use loong_contracts::{
    ActionExecutionEvent, AuthorizationScope, AuthorizationSubject, Capabilities, Capability,
    GrantId,
};

use super::*;

#[test]
fn action_execution_event_exposes_filter_and_render_fields() {
    let grant_id = GrantId::new();
    let kind = AuditEventKind::ActionExecution {
        grant_id,
        event: ActionExecutionEvent::Completed,
    };

    assert_eq!(audit_event_pack_id(&kind), None);
    assert_eq!(audit_event_kind_label(&kind), "ActionExecution");
    assert_eq!(triage_event_label(&kind), None);
    assert_eq!(
        format_audit_event_detail(&kind),
        format!("grant_id={grant_id} event=Completed")
    );
}

#[test]
fn tool_capability_override_rejection_has_no_legacy_pack_and_renders_authority_delta() {
    let requested = Capabilities::from([Capability::FilesystemWrite]);
    let declared = Capabilities::from([Capability::FilesystemRead]);
    let kind = AuditEventKind::ToolCapabilityOverrideRejected {
        subject: AuthorizationSubject {
            actor_id: "actor:test".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "session:test".to_owned(),
            },
        },
        path_display: "read".to_owned(),
        requested: requested.clone(),
        declared: declared.clone(),
    };

    assert_eq!(audit_event_pack_id(&kind), None);
    assert_eq!(
        audit_event_kind_label(&kind),
        "ToolCapabilityOverrideRejected"
    );
    assert_eq!(
        format_audit_event_detail(&kind),
        format!(
            "path=read capability_override_rejected requested={requested:?} declared={declared:?}"
        )
    );
}
