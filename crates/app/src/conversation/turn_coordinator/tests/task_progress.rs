use super::*;

#[cfg(feature = "memory-sqlite")]
fn task_progress_test_checkpoint(
    result_kind: TurnCheckpointResultKind,
    runs_after_turn: bool,
    attempts_context_compaction: bool,
) -> TurnCheckpointSnapshot {
    TurnCheckpointSnapshot {
        identity: None,
        preparation: TurnPreparationSnapshot {
            lane: ExecutionLane::Fast,
            max_tool_steps: 1,
            raw_tool_output_requested: false,
            context_message_count: 1,
            context_fingerprint_sha256: "ctx".to_owned(),
            estimated_tokens: None,
        },
        request: TurnCheckpointRequest::Continue { tool_intents: 1 },
        lane: Some(TurnLaneExecutionSnapshot {
            lane: ExecutionLane::Fast,
            had_tool_intents: true,
            tool_request_summary: None,
            raw_tool_output_requested: false,
            result_kind,
            safe_lane_terminal_route: None,
        }),
        reply: None,
        finalization: TurnFinalizationCheckpoint::PersistReply {
            persistence_mode: ReplyPersistenceMode::Success,
            runs_after_turn,
            attempts_context_compaction,
        },
    }
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn checkpoint_waits_for_external_resolution_on_needs_approval() {
    let checkpoint =
        task_progress_test_checkpoint(TurnCheckpointResultKind::NeedsApproval, false, false);

    assert!(checkpoint_waits_for_external_resolution(&checkpoint));
    assert!(!checkpoint_requires_verification_phase(&checkpoint));
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn checkpoint_requires_verification_phase_for_post_turn_work() {
    let checkpoint = task_progress_test_checkpoint(TurnCheckpointResultKind::FinalText, true, true);

    assert!(!checkpoint_waits_for_external_resolution(&checkpoint));
    assert!(checkpoint_requires_verification_phase(&checkpoint));
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn waiting_task_progress_record_uses_waiting_status_and_approval_gate_handle() {
    let record = waiting_task_progress_record(
        &LoongConfig::default(),
        "session-approval",
        "await approval",
    );

    assert_eq!(record.status, TaskProgressStatus::Waiting);
    assert_eq!(
        record.verification_state,
        Some(TaskVerificationState::Pending)
    );
    assert_eq!(record.active_handles.len(), 1);
    assert_eq!(record.active_handles[0].handle_kind, "approval_gate");
    assert_eq!(
        record
            .resume_recipe
            .as_ref()
            .map(|value| value.recommended_tool.as_str()),
        Some("task_status")
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn verifying_task_progress_record_uses_verifying_status_and_finalization_handle() {
    let record =
        verifying_task_progress_record(&LoongConfig::default(), "session-verify", "finalize");

    assert_eq!(record.status, TaskProgressStatus::Verifying);
    assert_eq!(
        record.verification_state,
        Some(TaskVerificationState::Pending)
    );
    assert_eq!(record.active_handles.len(), 1);
    assert_eq!(record.active_handles[0].handle_kind, "turn_finalization");
    assert_eq!(
        record
            .resume_recipe
            .as_ref()
            .map(|value| value.recommended_tool.as_str()),
        Some("task_status")
    );
}
