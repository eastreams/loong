use std::fs;

use serde_json::json;

use crate::session::store::SessionStoreConfig;

use super::{
    AcquireWorkUnitLeaseRequest, AddWorkUnitDependencyRequest, AppendWorkUnitNoteRequest,
    ArchiveWorkUnitRequest, AssignWorkUnitRequest, CompleteWorkUnitRequest, NewWorkUnitRecord,
    RemoveWorkUnitDependencyRequest, StartWorkUnitLeaseRequest, UpdateWorkUnitRequest,
    WORK_UNIT_ASSIGNED_EVENT_KIND, WORK_UNIT_DEPENDENCY_ADDED_EVENT_KIND,
    WORK_UNIT_DEPENDENCY_REMOVED_EVENT_KIND, WORK_UNIT_NOTE_ADDED_EVENT_KIND,
    WORK_UNIT_UPDATED_EVENT_KIND, WorkUnitCompletionDisposition, WorkUnitHeartbeatRequest,
    WorkUnitListQuery, WorkUnitRepository,
};
use loong_contracts::{
    WorkSourceKind, WorkUnitKind, WorkUnitPriority, WorkUnitRetryPolicy, WorkUnitSourceRef,
    WorkUnitStatus,
};

fn isolated_memory_config(test_name: &str) -> SessionStoreConfig {
    let base = std::env::temp_dir().join(format!(
        "loong-work-unit-repository-{test_name}-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&base);
    let db_path = base.join("memory.sqlite3");
    let _ = fs::remove_file(&db_path);
    SessionStoreConfig {
        sqlite_path: Some(db_path),
        runtime_config: None,
    }
}

fn sample_source_ref() -> WorkUnitSourceRef {
    WorkUnitSourceRef {
        source_kind: WorkSourceKind::Discord,
        project_id: Some("loong-ai/server".to_owned()),
        channel_id: Some("feature".to_owned()),
        thread_id: Some("thread-42".to_owned()),
        message_id: Some("msg-7".to_owned()),
        external_ref: Some("feature-thread".to_owned()),
        source_url: Some("https://discord.example/feature/thread-42".to_owned()),
    }
}

fn sample_work_unit(status: WorkUnitStatus) -> NewWorkUnitRecord {
    NewWorkUnitRecord {
        work_unit_id: Some("wu-test".to_owned()),
        kind: WorkUnitKind::Feature,
        title: "Durable runtime foundation".to_owned(),
        description: "Implement the first durable work-unit runtime slice".to_owned(),
        source_ref: sample_source_ref(),
        status,
        priority: WorkUnitPriority::High,
        retry_policy: WorkUnitRetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 1_000,
            max_backoff_ms: 8_000,
        },
        parent_work_unit_id: None,
        next_run_at_ms: Some(1_000),
    }
}

#[test]
fn create_work_unit_round_trips_snapshot_fields() {
    let config = isolated_memory_config("create-roundtrip");
    let repository = WorkUnitRepository::new(&config).expect("repository");
    let created = repository
        .create_work_unit(sample_work_unit(WorkUnitStatus::Ready), Some("operator"))
        .expect("create work unit");

    assert_eq!(created.work_unit.work_unit_id, "wu-test");
    assert_eq!(created.work_unit.status, WorkUnitStatus::Ready);
    assert_eq!(created.work_unit.priority, WorkUnitPriority::High);
    assert_eq!(
        created.work_unit.source_ref.source_kind,
        WorkSourceKind::Discord
    );
    assert_eq!(created.work_unit.attempt_count, 0);
    assert_eq!(created.work_unit.assigned_to, None);
    assert!(created.work_unit.blocks_work_unit_ids.is_empty());
    assert!(created.work_unit.blocked_by_work_unit_ids.is_empty());
    assert!(created.lease.is_none());

    let events = repository
        .list_work_unit_events("wu-test", 10)
        .expect("list work unit events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_kind, "work_unit_created");
}

#[test]
fn update_work_unit_mutates_editable_fields_and_records_event() {
    let config = isolated_memory_config("update-fields");
    let repository = WorkUnitRepository::new(&config).expect("repository");
    repository
        .create_work_unit(sample_work_unit(WorkUnitStatus::Triaged), Some("operator"))
        .expect("create work unit");

    let updated = repository
        .update_work_unit(UpdateWorkUnitRequest {
            work_unit_id: "wu-test".to_owned(),
            title: Some("Durable runtime foundation v2".to_owned()),
            description: Some("Refine orchestration surface".to_owned()),
            status: Some(WorkUnitStatus::WaitingReview),
            priority: Some(WorkUnitPriority::Critical),
            next_run_at_ms: Some(2_500),
            blocking_reason: Some("waiting for design review".to_owned()),
            clear_blocking_reason: false,
            actor: Some("planner".to_owned()),
            now_ms: Some(2_000),
        })
        .expect("update work unit")
        .expect("updated snapshot");
    assert_eq!(updated.work_unit.title, "Durable runtime foundation v2");
    assert_eq!(
        updated.work_unit.description,
        "Refine orchestration surface"
    );
    assert_eq!(updated.work_unit.status, WorkUnitStatus::WaitingReview);
    assert_eq!(updated.work_unit.priority, WorkUnitPriority::Critical);
    assert_eq!(updated.work_unit.next_run_at_ms, 2_500);
    assert_eq!(
        updated.work_unit.blocking_reason.as_deref(),
        Some("waiting for design review")
    );

    let ready = repository
        .update_work_unit(UpdateWorkUnitRequest {
            work_unit_id: "wu-test".to_owned(),
            title: None,
            description: None,
            status: Some(WorkUnitStatus::Ready),
            priority: None,
            next_run_at_ms: None,
            blocking_reason: None,
            clear_blocking_reason: true,
            actor: Some("planner".to_owned()),
            now_ms: Some(2_100),
        })
        .expect("clear blocking reason")
        .expect("ready snapshot");
    assert_eq!(ready.work_unit.status, WorkUnitStatus::Ready);
    assert_eq!(ready.work_unit.blocking_reason, None);

    let events = repository
        .list_work_unit_events("wu-test", 10)
        .expect("list work unit events");
    assert!(
        events
            .iter()
            .any(|event| event.event_kind == WORK_UNIT_UPDATED_EVENT_KIND),
        "expected work-unit update event"
    );
}

#[test]
fn update_work_unit_rejects_runtime_owned_status_transition() {
    let config = isolated_memory_config("update-runtime-owned-status");
    let repository = WorkUnitRepository::new(&config).expect("repository");
    repository
        .create_work_unit(sample_work_unit(WorkUnitStatus::Ready), Some("operator"))
        .expect("create work unit");
    repository
        .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
            owner: "worker-a".to_owned(),
            ttl_ms: 5_000,
            actor: Some("scheduler".to_owned()),
            now_ms: Some(1_000),
        })
        .expect("acquire lease")
        .expect("leased snapshot");

    let error = repository
        .update_work_unit(UpdateWorkUnitRequest {
            work_unit_id: "wu-test".to_owned(),
            title: None,
            description: None,
            status: Some(WorkUnitStatus::WaitingReview),
            priority: None,
            next_run_at_ms: None,
            blocking_reason: None,
            clear_blocking_reason: false,
            actor: Some("planner".to_owned()),
            now_ms: Some(1_100),
        })
        .expect_err("runtime-owned status transition should be rejected");

    assert!(error.contains("cannot change status from `leased`"));
}

#[test]
fn acquire_start_heartbeat_complete_flow_updates_snapshot_and_events() {
    let config = isolated_memory_config("lease-flow");
    let repository = WorkUnitRepository::new(&config).expect("repository");
    repository
        .create_work_unit(sample_work_unit(WorkUnitStatus::Ready), Some("operator"))
        .expect("create work unit");

    let leased = repository
        .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
            owner: "worker-a".to_owned(),
            ttl_ms: 5_000,
            actor: Some("scheduler".to_owned()),
            now_ms: Some(2_000),
        })
        .expect("acquire lease")
        .expect("leased work unit");
    assert_eq!(leased.work_unit.status, WorkUnitStatus::Leased);
    assert_eq!(leased.work_unit.attempt_count, 1);
    assert_eq!(leased.lease.as_ref().expect("lease").owner, "worker-a");

    let running = repository
        .mark_leased_running(StartWorkUnitLeaseRequest {
            work_unit_id: "wu-test".to_owned(),
            owner: "worker-a".to_owned(),
            actor: Some("worker-a".to_owned()),
            now_ms: Some(2_500),
        })
        .expect("mark running")
        .expect("running snapshot");
    assert_eq!(running.work_unit.status, WorkUnitStatus::Running);

    let heartbeat = repository
        .heartbeat_lease(WorkUnitHeartbeatRequest {
            work_unit_id: "wu-test".to_owned(),
            owner: "worker-a".to_owned(),
            ttl_ms: 7_000,
            actor: Some("worker-a".to_owned()),
            now_ms: Some(3_000),
        })
        .expect("heartbeat")
        .expect("heartbeat snapshot");
    let heartbeat_lease = heartbeat.lease.expect("lease after heartbeat");
    assert_eq!(heartbeat_lease.expires_at_ms, 10_000);

    let completed = repository
        .complete_work_unit(CompleteWorkUnitRequest {
            work_unit_id: "wu-test".to_owned(),
            owner: "worker-a".to_owned(),
            disposition: WorkUnitCompletionDisposition::Completed,
            actor: Some("worker-a".to_owned()),
            now_ms: Some(4_000),
            next_run_at_ms: None,
            result_payload_json: Some(json!({"summary": "done"})),
            error: None,
        })
        .expect("complete work unit")
        .expect("completed snapshot");
    assert_eq!(completed.work_unit.status, WorkUnitStatus::Completed);
    assert!(completed.lease.is_none());
    assert_eq!(
        completed.work_unit.result_payload_json,
        Some(json!({"summary": "done"}))
    );

    let events = repository
        .list_work_unit_events("wu-test", 10)
        .expect("list events");
    let event_kinds = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect::<Vec<_>>();
    assert!(event_kinds.contains(&"work_unit_created"));
    assert!(event_kinds.contains(&"work_unit_leased"));
    assert!(event_kinds.contains(&"work_unit_started"));
    assert!(event_kinds.contains(&"work_unit_heartbeat"));
    assert!(event_kinds.contains(&"work_unit_completed"));
}

#[test]
fn retry_completion_schedules_backoff_and_exhaustion_becomes_failed_terminal() {
    let config = isolated_memory_config("retry-completion");
    let repository = WorkUnitRepository::new(&config).expect("repository");
    repository
        .create_work_unit(sample_work_unit(WorkUnitStatus::Ready), Some("operator"))
        .expect("create work unit");

    let first_lease = repository
        .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
            owner: "worker-a".to_owned(),
            ttl_ms: 5_000,
            actor: None,
            now_ms: Some(2_000),
        })
        .expect("first lease")
        .expect("leased snapshot");
    assert_eq!(first_lease.work_unit.attempt_count, 1);

    let first_retry = repository
        .complete_work_unit(CompleteWorkUnitRequest {
            work_unit_id: "wu-test".to_owned(),
            owner: "worker-a".to_owned(),
            disposition: WorkUnitCompletionDisposition::RetryPending,
            actor: Some("worker-a".to_owned()),
            now_ms: Some(4_000),
            next_run_at_ms: None,
            result_payload_json: None,
            error: Some("transient".to_owned()),
        })
        .expect("first retry")
        .expect("retry snapshot");
    assert_eq!(first_retry.work_unit.status, WorkUnitStatus::RetryPending);
    assert_eq!(first_retry.work_unit.next_run_at_ms, 5_000);
    assert_eq!(
        first_retry.work_unit.last_error.as_deref(),
        Some("transient")
    );

    let second_lease = repository
        .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
            owner: "worker-b".to_owned(),
            ttl_ms: 5_000,
            actor: None,
            now_ms: Some(5_000),
        })
        .expect("second lease")
        .expect("second leased snapshot");
    assert_eq!(second_lease.work_unit.attempt_count, 2);

    let second_retry = repository
        .complete_work_unit(CompleteWorkUnitRequest {
            work_unit_id: "wu-test".to_owned(),
            owner: "worker-b".to_owned(),
            disposition: WorkUnitCompletionDisposition::RetryPending,
            actor: None,
            now_ms: Some(6_000),
            next_run_at_ms: None,
            result_payload_json: None,
            error: Some("still transient".to_owned()),
        })
        .expect("second retry")
        .expect("second retry snapshot");
    assert_eq!(second_retry.work_unit.next_run_at_ms, 8_000);

    let third_lease = repository
        .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
            owner: "worker-c".to_owned(),
            ttl_ms: 5_000,
            actor: None,
            now_ms: Some(8_000),
        })
        .expect("third lease")
        .expect("third leased snapshot");
    assert_eq!(third_lease.work_unit.attempt_count, 3);

    let exhausted = repository
        .complete_work_unit(CompleteWorkUnitRequest {
            work_unit_id: "wu-test".to_owned(),
            owner: "worker-c".to_owned(),
            disposition: WorkUnitCompletionDisposition::RetryPending,
            actor: None,
            now_ms: Some(9_000),
            next_run_at_ms: None,
            result_payload_json: None,
            error: Some("retry budget exhausted".to_owned()),
        })
        .expect("retry exhaustion")
        .expect("failed terminal snapshot");
    assert_eq!(exhausted.work_unit.status, WorkUnitStatus::FailedTerminal);
}

#[test]
fn recover_expired_leases_moves_units_to_retry_pending_or_failed_terminal() {
    let config = isolated_memory_config("recover-expired");
    let repository = WorkUnitRepository::new(&config).expect("repository");

    let first = NewWorkUnitRecord {
        work_unit_id: Some("wu-first".to_owned()),
        retry_policy: WorkUnitRetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 1_000,
            max_backoff_ms: 4_000,
        },
        ..sample_work_unit(WorkUnitStatus::Ready)
    };
    let second = NewWorkUnitRecord {
        work_unit_id: Some("wu-second".to_owned()),
        retry_policy: WorkUnitRetryPolicy {
            max_attempts: 1,
            initial_backoff_ms: 1_000,
            max_backoff_ms: 4_000,
        },
        ..sample_work_unit(WorkUnitStatus::Ready)
    };

    repository
        .create_work_unit(first, None)
        .expect("create first work unit");
    repository
        .create_work_unit(second, None)
        .expect("create second work unit");

    repository
        .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
            owner: "worker-a".to_owned(),
            ttl_ms: 2_000,
            actor: None,
            now_ms: Some(1_000),
        })
        .expect("lease first work unit")
        .expect("first leased");
    repository
        .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
            owner: "worker-b".to_owned(),
            ttl_ms: 2_000,
            actor: None,
            now_ms: Some(1_100),
        })
        .expect("lease second work unit")
        .expect("second leased");

    let recovered = repository
        .recover_expired_leases(Some("recovery-scan"), Some(5_000))
        .expect("recover expired leases");
    assert_eq!(recovered.len(), 2);

    let first_snapshot = repository
        .load_work_unit_snapshot("wu-first")
        .expect("load first snapshot")
        .expect("first snapshot");
    assert_eq!(
        first_snapshot.work_unit.status,
        WorkUnitStatus::RetryPending
    );
    assert_eq!(first_snapshot.work_unit.next_run_at_ms, 6_000);

    let second_snapshot = repository
        .load_work_unit_snapshot("wu-second")
        .expect("load second snapshot")
        .expect("second snapshot");
    assert_eq!(
        second_snapshot.work_unit.status,
        WorkUnitStatus::FailedTerminal
    );
}

#[test]
fn archive_work_unit_requires_terminal_status() {
    let config = isolated_memory_config("archive-work-unit");
    let repository = WorkUnitRepository::new(&config).expect("repository");
    repository
        .create_work_unit(sample_work_unit(WorkUnitStatus::Ready), None)
        .expect("create work unit");

    let archived_before_terminal = repository
        .archive_work_unit(ArchiveWorkUnitRequest {
            work_unit_id: "wu-test".to_owned(),
            actor: Some("operator".to_owned()),
            now_ms: Some(2_000),
        })
        .expect("archive before terminal should not error");
    assert!(archived_before_terminal.is_none());

    repository
        .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
            owner: "worker-a".to_owned(),
            ttl_ms: 5_000,
            actor: None,
            now_ms: Some(2_100),
        })
        .expect("lease for archive flow")
        .expect("leased snapshot");
    repository
        .complete_work_unit(CompleteWorkUnitRequest {
            work_unit_id: "wu-test".to_owned(),
            owner: "worker-a".to_owned(),
            disposition: WorkUnitCompletionDisposition::Cancelled,
            actor: None,
            now_ms: Some(2_200),
            next_run_at_ms: None,
            result_payload_json: None,
            error: Some("operator cancelled".to_owned()),
        })
        .expect("cancel work unit")
        .expect("cancelled snapshot");

    let archived = repository
        .archive_work_unit(ArchiveWorkUnitRequest {
            work_unit_id: "wu-test".to_owned(),
            actor: Some("operator".to_owned()),
            now_ms: Some(2_300),
        })
        .expect("archive terminal work unit")
        .expect("archived snapshot");
    assert_eq!(archived.work_unit.status, WorkUnitStatus::Archived);
    assert_eq!(archived.work_unit.archived_at_ms, Some(2_300));
}

#[test]
fn runtime_health_reports_counts_and_expired_leases() {
    let config = isolated_memory_config("runtime-health");
    let repository = WorkUnitRepository::new(&config).expect("repository");

    let ready = NewWorkUnitRecord {
        work_unit_id: Some("wu-ready".to_owned()),
        ..sample_work_unit(WorkUnitStatus::Ready)
    };
    let blocked = NewWorkUnitRecord {
        work_unit_id: Some("wu-blocked".to_owned()),
        ..sample_work_unit(WorkUnitStatus::WaitingReview)
    };

    repository
        .create_work_unit(ready, None)
        .expect("create ready");
    repository
        .create_work_unit(blocked, None)
        .expect("create blocked");

    repository
        .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
            owner: "worker-a".to_owned(),
            ttl_ms: 1_000,
            actor: None,
            now_ms: Some(1_000),
        })
        .expect("lease ready")
        .expect("leased snapshot");

    let health = repository
        .load_runtime_health(Some(5_000))
        .expect("load runtime health");
    assert_eq!(health.total_count, 2);
    assert_eq!(health.ready_count, 0);
    assert_eq!(health.leased_count, 1);
    assert_eq!(health.blocked_count, 1);
    assert_eq!(health.expired_lease_count, 1);
}

#[test]
fn list_work_units_filters_archived_entries_by_default() {
    let config = isolated_memory_config("list-filter");
    let repository = WorkUnitRepository::new(&config).expect("repository");

    let active = NewWorkUnitRecord {
        work_unit_id: Some("wu-active".to_owned()),
        ..sample_work_unit(WorkUnitStatus::Ready)
    };
    let archived = NewWorkUnitRecord {
        work_unit_id: Some("wu-archived".to_owned()),
        ..sample_work_unit(WorkUnitStatus::Ready)
    };

    repository
        .create_work_unit(active, None)
        .expect("create active");
    repository
        .create_work_unit(archived, None)
        .expect("create archived candidate");
    repository
        .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
            owner: "worker-a".to_owned(),
            ttl_ms: 5_000,
            actor: None,
            now_ms: Some(1_000),
        })
        .expect("lease active")
        .expect("leased active");
    repository
        .complete_work_unit(CompleteWorkUnitRequest {
            work_unit_id: "wu-active".to_owned(),
            owner: "worker-a".to_owned(),
            disposition: WorkUnitCompletionDisposition::Completed,
            actor: None,
            now_ms: Some(2_000),
            next_run_at_ms: None,
            result_payload_json: None,
            error: None,
        })
        .expect("complete active")
        .expect("completed active");
    repository
        .archive_work_unit(ArchiveWorkUnitRequest {
            work_unit_id: "wu-active".to_owned(),
            actor: None,
            now_ms: Some(3_000),
        })
        .expect("archive active")
        .expect("archived active");

    let visible = repository
        .list_work_units(WorkUnitListQuery::default())
        .expect("list visible work units");
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].work_unit.work_unit_id, "wu-archived");

    let with_archived = repository
        .list_work_units(WorkUnitListQuery {
            include_archived: true,
            ..WorkUnitListQuery::default()
        })
        .expect("list work units including archived");
    assert_eq!(with_archived.len(), 2);
}

#[test]
fn load_runtime_health_returns_zero_counts_for_empty_repository() {
    let config = isolated_memory_config("runtime-health-empty");
    let repository = WorkUnitRepository::new(&config).expect("repository");

    let health = repository
        .load_runtime_health(Some(5_000))
        .expect("load runtime health");

    assert_eq!(health.total_count, 0);
    assert_eq!(health.ready_count, 0);
    assert_eq!(health.leased_count, 0);
    assert_eq!(health.running_count, 0);
    assert_eq!(health.blocked_count, 0);
    assert_eq!(health.retry_pending_count, 0);
    assert_eq!(health.terminal_count, 0);
    assert_eq!(health.archived_count, 0);
    assert_eq!(health.expired_lease_count, 0);
}

#[test]
fn assignment_dependency_and_note_actions_round_trip_through_snapshot() {
    let config = isolated_memory_config("assignment-dependency-note");
    let repository = WorkUnitRepository::new(&config).expect("repository");
    let blocker = NewWorkUnitRecord {
        work_unit_id: Some("wu-blocker".to_owned()),
        title: "Blocker".to_owned(),
        description: "Complete prerequisite".to_owned(),
        priority: WorkUnitPriority::Low,
        ..sample_work_unit(WorkUnitStatus::Ready)
    };
    let blocked = sample_work_unit(WorkUnitStatus::Ready);

    repository
        .create_work_unit(blocker, Some("operator"))
        .expect("create blocker work unit");
    repository
        .create_work_unit(blocked, Some("operator"))
        .expect("create blocked work unit");

    let assigned = repository
        .assign_work_unit(AssignWorkUnitRequest {
            work_unit_id: "wu-test".to_owned(),
            assigned_to: Some("designer".to_owned()),
            actor: Some("operator".to_owned()),
            now_ms: Some(1_200),
        })
        .expect("assign work unit")
        .expect("assigned snapshot");
    assert_eq!(assigned.work_unit.assigned_to.as_deref(), Some("designer"));

    let dependency_added = repository
        .add_dependency(AddWorkUnitDependencyRequest {
            blocking_work_unit_id: "wu-blocker".to_owned(),
            blocked_work_unit_id: "wu-test".to_owned(),
            actor: Some("operator".to_owned()),
            now_ms: Some(1_300),
        })
        .expect("add dependency")
        .expect("dependency snapshot");
    assert_eq!(
        dependency_added.work_unit.blocked_by_work_unit_ids,
        vec!["wu-blocker".to_owned()]
    );

    let note = repository
        .append_note(AppendWorkUnitNoteRequest {
            work_unit_id: "wu-test".to_owned(),
            actor: Some("operator".to_owned()),
            note: "needs design review".to_owned(),
            now_ms: Some(1_350),
        })
        .expect("append note")
        .expect("note event");
    assert_eq!(note.event_kind, WORK_UNIT_NOTE_ADDED_EVENT_KIND);

    let leased = repository
        .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
            owner: "worker-a".to_owned(),
            ttl_ms: 5_000,
            actor: Some("scheduler".to_owned()),
            now_ms: Some(1_400),
        })
        .expect("acquire lease")
        .expect("leased snapshot");
    assert_eq!(leased.work_unit.work_unit_id, "wu-blocker");

    let dependency_removed = repository
        .remove_dependency(RemoveWorkUnitDependencyRequest {
            blocking_work_unit_id: "wu-blocker".to_owned(),
            blocked_work_unit_id: "wu-test".to_owned(),
            actor: Some("operator".to_owned()),
            now_ms: Some(1_500),
        })
        .expect("remove dependency")
        .expect("dependency removed snapshot");
    assert!(
        dependency_removed
            .work_unit
            .blocked_by_work_unit_ids
            .is_empty()
    );

    let snapshot = repository
        .load_work_unit_snapshot("wu-test")
        .expect("load work unit snapshot")
        .expect("work unit snapshot");
    assert_eq!(snapshot.work_unit.assigned_to.as_deref(), Some("designer"));
    assert!(snapshot.work_unit.blocked_by_work_unit_ids.is_empty());

    let blocker_snapshot = repository
        .load_work_unit_snapshot("wu-blocker")
        .expect("load blocker snapshot")
        .expect("blocker snapshot");
    assert!(
        blocker_snapshot.work_unit.blocks_work_unit_ids.is_empty(),
        "dependency removal should clear blocker-side relation view"
    );

    let events = repository
        .list_work_unit_events("wu-test", 20)
        .expect("list work unit events");
    let event_kinds = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect::<Vec<_>>();
    assert!(event_kinds.contains(&WORK_UNIT_ASSIGNED_EVENT_KIND));
    assert!(event_kinds.contains(&WORK_UNIT_DEPENDENCY_ADDED_EVENT_KIND));
    assert!(event_kinds.contains(&WORK_UNIT_DEPENDENCY_REMOVED_EVENT_KIND));
    assert!(event_kinds.contains(&WORK_UNIT_NOTE_ADDED_EVENT_KIND));
}

#[test]
fn dependency_cycle_is_rejected_before_persisting_edge() {
    let config = isolated_memory_config("dependency-cycle");
    let repository = WorkUnitRepository::new(&config).expect("repository");
    let first = NewWorkUnitRecord {
        work_unit_id: Some("wu-first".to_owned()),
        title: "First".to_owned(),
        description: "First work unit".to_owned(),
        ..sample_work_unit(WorkUnitStatus::Ready)
    };
    let second = NewWorkUnitRecord {
        work_unit_id: Some("wu-second".to_owned()),
        title: "Second".to_owned(),
        description: "Second work unit".to_owned(),
        ..sample_work_unit(WorkUnitStatus::Ready)
    };

    repository
        .create_work_unit(first, Some("operator"))
        .expect("create first work unit");
    repository
        .create_work_unit(second, Some("operator"))
        .expect("create second work unit");
    repository
        .add_dependency(AddWorkUnitDependencyRequest {
            blocking_work_unit_id: "wu-first".to_owned(),
            blocked_work_unit_id: "wu-second".to_owned(),
            actor: Some("operator".to_owned()),
            now_ms: Some(1_000),
        })
        .expect("add first dependency");

    let error = repository
        .add_dependency(AddWorkUnitDependencyRequest {
            blocking_work_unit_id: "wu-second".to_owned(),
            blocked_work_unit_id: "wu-first".to_owned(),
            actor: Some("operator".to_owned()),
            now_ms: Some(1_100),
        })
        .expect_err("dependency cycle should be rejected");

    assert!(error.contains("would create a cycle"));
}
