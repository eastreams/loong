#[cfg(feature = "memory-sqlite")]
pub mod recovery;

#[cfg(feature = "memory-sqlite")]
pub mod repository;

pub(crate) const DELEGATE_CANCEL_REQUESTED_EVENT_KIND: &str = "delegate_cancel_requested";
pub(crate) const DELEGATE_CANCELLED_EVENT_KIND: &str = "delegate_cancelled";
pub(crate) const DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED: &str = "operator_requested";
pub(crate) const DELEGATE_CANCELLED_ERROR_PREFIX: &str = "delegate_cancelled:";

pub(crate) fn delegate_cancelled_error(reason: &str) -> String {
    format!(
        "{DELEGATE_CANCELLED_ERROR_PREFIX} {}",
        reason.trim().trim_matches(':')
    )
}

pub(crate) fn parse_delegate_cancelled_reason(error: &str) -> Option<String> {
    error
        .strip_prefix(DELEGATE_CANCELLED_ERROR_PREFIX)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(all(test, feature = "memory-sqlite"))]
mod recovery_tests {
    use serde_json::json;

    use crate::session::repository::{
        FinalizeSessionTerminalRequest, SessionEventRecord, SessionState,
    };

    use super::recovery::{
        build_async_spawn_failure_recovery_payload, build_terminal_finalize_recovery_payload,
        observe_missing_recovery, RECOVERY_EVENT_KIND,
        RECOVERY_KIND_ASYNC_SPAWN_FAILURE_PERSIST_FAILED,
        RECOVERY_KIND_TERMINAL_FINALIZE_PERSIST_FAILED, RECOVERY_KIND_UNKNOWN,
        RECOVERY_SOURCE_EVENT, RECOVERY_SOURCE_NONE,
    };

    #[test]
    fn build_terminal_finalize_recovery_payload_uses_shared_schema() {
        let payload = build_terminal_finalize_recovery_payload(
            &FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({"turn_count": 1}),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({"final_output": "done"}),
            },
            "delegate_terminal_finalize_failed: database busy",
        );

        assert_eq!(
            payload["recovery_kind"],
            RECOVERY_KIND_TERMINAL_FINALIZE_PERSIST_FAILED
        );
        assert_eq!(payload["recovered_state"], "failed");
        assert_eq!(
            payload["attempted_terminal_event_kind"],
            "delegate_completed"
        );
        assert_eq!(payload["attempted_outcome_status"], "ok");
    }

    #[test]
    fn observe_missing_recovery_prefers_structured_event() {
        let recovery = observe_missing_recovery(
            &[SessionEventRecord {
                id: 1,
                session_id: "child-session".to_owned(),
                event_kind: RECOVERY_EVENT_KIND.to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                payload_json: build_async_spawn_failure_recovery_payload(
                    Some("child"),
                    "spawn unavailable",
                    "delegate_async_spawn_failure_persist_failed: sqlite_busy; original spawn error: spawn unavailable",
                ),
                ts: 42,
            }],
            Some("delegate_terminal_finalize_failed: ignored because event wins"),
        );

        assert_eq!(recovery.source, RECOVERY_SOURCE_EVENT);
        assert_eq!(
            recovery.kind,
            RECOVERY_KIND_ASYNC_SPAWN_FAILURE_PERSIST_FAILED
        );
        assert_eq!(recovery.event_kind, RECOVERY_EVENT_KIND);
        assert_eq!(
            recovery.original_error.as_deref(),
            Some("spawn unavailable")
        );
    }

    #[test]
    fn observe_missing_recovery_returns_unknown_none_when_no_metadata_exists() {
        let recovery = observe_missing_recovery(&[], None);

        assert_eq!(recovery.source, RECOVERY_SOURCE_NONE);
        assert_eq!(recovery.kind, RECOVERY_KIND_UNKNOWN);
        assert!(recovery.event_kind.is_empty());
        assert_eq!(recovery.recovery_error, None);
    }

    #[test]
    fn observe_missing_recovery_infers_running_async_overdue_from_last_error() {
        let recovery = observe_missing_recovery(
            &[],
            Some(
                "delegate_async_running_overdue_marked_failed: running delegate child exceeded timeout after 91s (threshold 30s)",
            ),
        );

        assert_eq!(recovery.source, "last_error");
        assert_eq!(recovery.kind, "running_async_overdue_marked_failed");
        assert!(recovery.event_kind.is_empty());
        assert_eq!(
            recovery.recovery_error.as_deref(),
            Some(
                "delegate_async_running_overdue_marked_failed: running delegate child exceeded timeout after 91s (threshold 30s)",
            )
        );
    }
}
