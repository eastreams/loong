use loong_contracts::{
    AuthorizationActionSnapshot, AuthorizationAttempt, AuthorizationAttemptEvent,
    AuthorizationAttemptId, AuthorizationEvidence, AuthorizationScope, AuthorizationSubject,
    Capability,
};

use super::*;

#[test]
fn authorization_denial_exposes_filter_triage_and_render_fields() {
    let kind = AuditEventKind::Authorization {
        evidence: AuthorizationEvidence {
            subject: AuthorizationSubject {
                actor_id: "actor:test".to_owned(),
                scope: AuthorizationScope::Session {
                    session_id: "session:test".to_owned(),
                },
            },
            action: AuthorizationActionSnapshot {
                kind: "fs.read".to_owned(),
                operation: "read".to_owned(),
                resource: Some("/workspace/note.txt".to_owned()),
                required_capabilities: vec![Capability::FilesystemRead],
            },
            attempt: AuthorizationAttempt::Started {
                id: AuthorizationAttemptId(7),
                event: AuthorizationAttemptEvent::CapabilityDenied {
                    capability: Capability::FilesystemRead,
                },
            },
        },
    };

    assert_eq!(audit_event_pack_id(&kind), None);
    assert_eq!(audit_event_kind_label(&kind), "Authorization");
    assert_eq!(triage_event_label(&kind), Some("authorization_denied"));
    assert_eq!(
        parse_audit_event_kind_filter("authorization"),
        Ok("Authorization".to_owned())
    );
    assert_eq!(
        parse_audit_triage_label_filter("authorization_denied"),
        Ok("authorization_denied".to_owned())
    );
    assert_eq!(
        triage_event_hint(&kind),
        Some(
            "grant the required capability or adjust the policy or authority for the requested action"
                .to_owned()
        )
    );
    assert!(format_audit_event_detail(&kind).contains("action_kind=fs.read operation=read"));
    assert!(triage_event_summary(&kind).is_some());
}

#[test]
fn authorization_failure_has_a_distinct_triage_label() {
    let kind = AuditEventKind::Authorization {
        evidence: AuthorizationEvidence {
            subject: AuthorizationSubject {
                actor_id: "actor:test".to_owned(),
                scope: AuthorizationScope::Session {
                    session_id: "session:test".to_owned(),
                },
            },
            action: AuthorizationActionSnapshot {
                kind: "fs.read".to_owned(),
                operation: "read".to_owned(),
                resource: None,
                required_capabilities: Vec::new(),
            },
            attempt: AuthorizationAttempt::StartFailed,
        },
    };

    assert_eq!(triage_event_label(&kind), Some("authorization_failed"));
    assert_eq!(
        parse_audit_triage_label_filter("authorization_failed"),
        Ok("authorization_failed".to_owned())
    );
    assert_eq!(
        triage_event_hint(&kind),
        Some(
            "restore the authorization identity, permission, and audit backend before retrying"
                .to_owned()
        )
    );
}

#[test]
fn legacy_token_authorization_exposes_pack_and_token_filters() {
    let kind = AuditEventKind::Authorization {
        evidence: AuthorizationEvidence {
            subject: AuthorizationSubject {
                actor_id: "legacy-agent".to_owned(),
                scope: AuthorizationScope::LegacyToken {
                    boundary: "spec".to_owned(),
                    pack_id: "legacy-pack".to_owned(),
                    token_id: "legacy-token".to_owned(),
                },
            },
            action: AuthorizationActionSnapshot {
                kind: "action.legacy".to_owned(),
                operation: "tool".to_owned(),
                resource: None,
                required_capabilities: Vec::new(),
            },
            attempt: AuthorizationAttempt::Started {
                id: AuthorizationAttemptId(9),
                event: AuthorizationAttemptEvent::CapabilityDenied {
                    capability: Capability::InvokeTool,
                },
            },
        },
    };

    assert_eq!(audit_event_pack_id(&kind), Some("legacy-pack"));
    assert_eq!(audit_event_token_id(&kind), Some("legacy-token"));
}
