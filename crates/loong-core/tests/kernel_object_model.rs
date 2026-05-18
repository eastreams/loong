use std::path::PathBuf;

use loong_core::{
    ApprovalState, ArtifactDurabilityClass, ChildBudgetPolicy, ExecutionArtifact,
    ExecutionArtifactKind, Session, SessionBudgetOverlay, SessionEvent, Task, TaskBudget,
    TaskEvent, TaskExecutionMode, TaskLifecycle, TurnStatus, WorkspaceContext,
};

fn workspace(branch: &str, worktree_suffix: &str) -> WorkspaceContext {
    let repo_root = PathBuf::from("/tmp/loong");
    let worktree_root = repo_root.join(".worktrees").join(worktree_suffix);
    WorkspaceContext::new(
        worktree_root.clone(),
        repo_root,
        worktree_root.clone(),
        worktree_root.join("crates/loong-core"),
        branch.to_owned(),
    )
}

#[test]
fn lifecycle_transitions_emit_fact_events() {
    let mut task = Task::new(
        "task-1",
        "land the executable kernel object model",
        workspace("feat/phase-2", "phase-2"),
        TaskBudget::default(),
        TaskExecutionMode::InteractiveAttached,
    );

    assert_eq!(task.lifecycle(), &TaskLifecycle::Queued);

    let running = task.transition_to(TaskLifecycle::Running).unwrap();
    assert!(matches!(
        running,
        TaskEvent::LifecycleChanged {
            from: TaskLifecycle::Queued,
            to: TaskLifecycle::Running,
            ..
        }
    ));

    task.begin_turn("turn-1", "advance model and tool execution")
        .unwrap();
    task.finish_current_turn(TurnStatus::Completed).unwrap();
    task.transition_to(TaskLifecycle::WaitingForApproval)
        .unwrap();
    task.transition_to(TaskLifecycle::Running).unwrap();
    task.transition_to(TaskLifecycle::Completed).unwrap();

    assert_eq!(task.current_turn().unwrap().turn_id, "turn-1");
    assert_eq!(task.current_turn().unwrap().status, TurnStatus::Completed);
    assert_eq!(task.lifecycle(), &TaskLifecycle::Completed);
    assert!(
        task.transition_to(TaskLifecycle::Running).is_err(),
        "terminal tasks must reject further lifecycle transitions"
    );

    let lifecycle_events = task
        .fact_events()
        .iter()
        .filter(|event| matches!(event, TaskEvent::LifecycleChanged { .. }))
        .count();
    assert_eq!(lifecycle_events, 4);
}

#[test]
fn workspace_binding_is_durable_and_subtasks_inherit_context() {
    let first_workspace = workspace("feat/phase-2", "phase-2-a");
    let second_workspace = workspace("feat/phase-2-next", "phase-2-b");
    let split_budget = TaskBudget {
        max_child_tasks: 1,
        max_tool_concurrency: 1,
        max_steps: Some(3),
        deadline_epoch_ms: Some(55),
        artifact_retention: Default::default(),
        event_retention: Default::default(),
    };

    let mut session = Session::new(
        "session-1",
        first_workspace.clone(),
        SessionBudgetOverlay::default(),
    );

    session
        .spawn_task(
            "task-root",
            "own the first workspace binding",
            TaskBudget::default(),
            TaskExecutionMode::InteractiveAttached,
        )
        .unwrap();
    assert_eq!(
        session.task("task-root").unwrap().workspace(),
        &first_workspace
    );

    session.switch_worktree(second_workspace.clone()).unwrap();
    let session_events = session.session_events();
    assert_eq!(session_events.len(), 2);
    assert!(matches!(
        &session_events[0],
        SessionEvent::SessionCreated {
            session_id,
            workspace,
        } if session_id == "session-1" && workspace == &first_workspace
    ));
    assert!(matches!(
        &session_events[1],
        SessionEvent::WorktreeSwitched {
            session_id,
            previous_workspace,
            next_workspace,
        } if session_id == "session-1"
            && previous_workspace == &first_workspace
            && next_workspace == &second_workspace
    ));
    session
        .spawn_task(
            "task-detached",
            "run detached against the switched worktree",
            TaskBudget::default(),
            TaskExecutionMode::DetachedBackground,
        )
        .unwrap();
    session
        .spawn_subtask(
            "task-detached",
            "task-child",
            "inherit the task worktree while splitting budget",
            TaskExecutionMode::DelegatedChild,
            ChildBudgetPolicy::Split {
                child_budget: split_budget.clone(),
            },
            None,
        )
        .unwrap();

    let child = session.task("task-child").unwrap();
    assert_eq!(child.workspace(), &second_workspace);
    assert_eq!(child.budget(), &split_budget);
    assert_eq!(child.parent_task_id.as_deref(), Some("task-detached"));
    assert_eq!(session.task("task-detached").unwrap().subtasks().len(), 1);
}

#[test]
fn session_rejects_cross_repo_worktree_switches_without_mutating_truth() {
    let first_workspace = workspace("feat/phase-2", "phase-2-a");
    let other_repo_workspace = WorkspaceContext::new(
        "/tmp/other/.worktrees/phase-2-b",
        "/tmp/other",
        "/tmp/other/.worktrees/phase-2-b",
        "/tmp/other/.worktrees/phase-2-b/crates/loong-core",
        "feat/other".to_owned(),
    );

    let mut session = Session::new(
        "session-2",
        first_workspace.clone(),
        SessionBudgetOverlay::default(),
    );

    let error = session.switch_worktree(other_repo_workspace).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("session workspace repo root mismatch"),
        "cross-repo switch should fail with a repo mismatch"
    );
    assert_eq!(session.workspace(), &first_workspace);
    assert_eq!(session.session_events().len(), 1);
    assert!(matches!(
        &session.session_events()[0],
        SessionEvent::SessionCreated {
            session_id,
            workspace,
        } if session_id == "session-2" && workspace == &first_workspace
    ));
}

#[test]
fn execution_artifacts_preserve_durability_contracts() {
    let mut task = Task::new(
        "task-2",
        "capture durable truth and projections separately",
        workspace("feat/artifacts", "artifacts"),
        TaskBudget::default(),
        TaskExecutionMode::DetachedBackground,
    );

    task.record_artifact(ExecutionArtifact::new(
        "patch-1",
        ExecutionArtifactKind::PatchEditRecord {
            paths: vec![PathBuf::from("crates/loong-core/src/task.rs")],
            summary: "add subtask lineage".to_owned(),
        },
    ));
    task.record_artifact(ExecutionArtifact::new(
        "approval-1",
        ExecutionArtifactKind::ApprovalCheckpoint {
            gate: "sandbox".to_owned(),
            state: ApprovalState::Approved,
            actor: Some("operator".to_owned()),
        },
    ));
    task.record_artifact(ExecutionArtifact::new(
        "assistant-1",
        ExecutionArtifactKind::AssistantTextOutput {
            text: "summarized completion note".to_owned(),
        },
    ));
    task.record_artifact(ExecutionArtifact::with_durability(
        "cache-1",
        ExecutionArtifactKind::GeneratedArtifactReference {
            location: PathBuf::from("/tmp/preview.txt"),
            description: "scratch preview".to_owned(),
        },
        ArtifactDurabilityClass::DiscardableCache,
    ));

    assert_eq!(task.artifacts().len(), 4);
    assert_eq!(task.artifacts().durable_truth().count(), 2);
    assert_eq!(task.artifacts().derived_projection().count(), 1);
    assert_eq!(task.artifacts().discardable_cache().count(), 1);

    let artifact_events = task
        .fact_events()
        .iter()
        .filter(|event| matches!(event, TaskEvent::ArtifactRecorded { .. }))
        .count();
    assert_eq!(artifact_events, 4);
}

#[test]
fn session_shared_artifacts_preserve_durability_contracts() {
    let mut session = Session::new(
        "session-artifacts",
        workspace("feat/session-artifacts", "session-artifacts"),
        SessionBudgetOverlay::default(),
    );

    session.record_artifact(ExecutionArtifact::new(
        "session-patch-1",
        ExecutionArtifactKind::PatchEditRecord {
            paths: vec![PathBuf::from("crates/loong-core/src/session.rs")],
            summary: "add shared session artifact storage".to_owned(),
        },
    ));
    session.record_artifact(ExecutionArtifact::new(
        "session-assistant-1",
        ExecutionArtifactKind::AssistantTextOutput {
            text: "retained session summary".to_owned(),
        },
    ));
    session.record_artifact(ExecutionArtifact::with_durability(
        "session-cache-1",
        ExecutionArtifactKind::GeneratedArtifactReference {
            location: PathBuf::from("/tmp/session-preview.txt"),
            description: "scratch session preview".to_owned(),
        },
        ArtifactDurabilityClass::DiscardableCache,
    ));

    assert_eq!(session.artifacts().len(), 3);
    assert_eq!(session.artifacts().durable_truth().count(), 1);
    assert_eq!(session.artifacts().derived_projection().count(), 1);
    assert_eq!(session.artifacts().discardable_cache().count(), 1);

    let artifact_events = session
        .session_events()
        .iter()
        .filter(|event| matches!(event, SessionEvent::ArtifactRecorded { .. }))
        .count();
    assert_eq!(artifact_events, 3);
}

#[test]
fn session_overlay_budget_caps_parallel_root_tasks() {
    let overlay = SessionBudgetOverlay {
        max_parallel_tasks: 1,
        ..SessionBudgetOverlay::default()
    };
    let mut session = Session::new(
        "session-budget",
        workspace("feat/budget", "budget"),
        overlay,
    );

    session
        .spawn_task(
            "task-1",
            "first active task",
            TaskBudget::default(),
            TaskExecutionMode::InteractiveAttached,
        )
        .expect("first task should fit overlay budget");

    let error = session
        .spawn_task(
            "task-2",
            "second active task",
            TaskBudget::default(),
            TaskExecutionMode::InteractiveAttached,
        )
        .expect_err("parallel root task limit should be enforced");
    assert!(
        error
            .to_string()
            .contains("exceeded max_parallel_tasks limit 1"),
        "unexpected error: {error}"
    );

    let task = session.task_mut("task-1").expect("task-1 present");
    task.transition_to(TaskLifecycle::Running)
        .expect("running transition should succeed");
    task.transition_to(TaskLifecycle::Completed)
        .expect("terminal transition should succeed");
    session
        .spawn_task(
            "task-2",
            "second task after completion",
            TaskBudget::default(),
            TaskExecutionMode::InteractiveAttached,
        )
        .expect("completed root tasks should free overlay capacity");
}

#[test]
fn task_budget_caps_parallel_child_tasks_per_parent() {
    let mut session = Session::new(
        "session-child-budget",
        workspace("feat/child-budget", "child-budget"),
        SessionBudgetOverlay::default(),
    );
    let parent_budget = TaskBudget {
        max_child_tasks: 1,
        ..TaskBudget::default()
    };
    session
        .spawn_task(
            "parent",
            "parent task",
            parent_budget,
            TaskExecutionMode::InteractiveAttached,
        )
        .expect("parent task should spawn");

    session
        .spawn_subtask(
            "parent",
            "child-1",
            "first child",
            TaskExecutionMode::DelegatedChild,
            ChildBudgetPolicy::Inherit,
            None,
        )
        .expect("first child should fit task budget");

    let error = session
        .spawn_subtask(
            "parent",
            "child-2",
            "second child",
            TaskExecutionMode::DelegatedChild,
            ChildBudgetPolicy::Inherit,
            None,
        )
        .expect_err("parent child-task limit should be enforced");
    assert!(
        error
            .to_string()
            .contains("exceeded max_child_tasks limit 1"),
        "unexpected error: {error}"
    );

    let child = session.task_mut("child-1").expect("child-1 present");
    child
        .transition_to(TaskLifecycle::Running)
        .expect("running transition should succeed");
    child
        .transition_to(TaskLifecycle::Completed)
        .expect("terminal transition should succeed");
    session
        .spawn_subtask(
            "parent",
            "child-2",
            "second child after completion",
            TaskExecutionMode::DelegatedChild,
            ChildBudgetPolicy::Inherit,
            None,
        )
        .expect("completed child tasks should free task-local capacity");
}

#[test]
fn session_overlay_budget_caps_parallel_child_tasks_across_parents() {
    let overlay = SessionBudgetOverlay {
        max_parallel_child_tasks: 1,
        ..SessionBudgetOverlay::default()
    };
    let mut session = Session::new(
        "session-overlay-child-budget",
        workspace("feat/session-child-budget", "session-child-budget"),
        overlay,
    );
    let parent_budget = TaskBudget {
        max_child_tasks: 4,
        ..TaskBudget::default()
    };
    session
        .spawn_task(
            "parent-1",
            "first parent",
            parent_budget.clone(),
            TaskExecutionMode::InteractiveAttached,
        )
        .expect("first parent should spawn");
    session
        .spawn_task(
            "parent-2",
            "second parent",
            parent_budget,
            TaskExecutionMode::InteractiveAttached,
        )
        .expect("second parent should spawn");

    session
        .spawn_subtask(
            "parent-1",
            "child-1",
            "first child",
            TaskExecutionMode::DelegatedChild,
            ChildBudgetPolicy::Inherit,
            None,
        )
        .expect("first child should fit session overlay");

    let error = session
        .spawn_subtask(
            "parent-2",
            "child-2",
            "second child",
            TaskExecutionMode::DelegatedChild,
            ChildBudgetPolicy::Inherit,
            None,
        )
        .expect_err("session-wide child overlay limit should be enforced");
    assert!(
        error
            .to_string()
            .contains("exceeded max_parallel_child_tasks limit 1"),
        "unexpected error: {error}"
    );

    let child = session.task_mut("child-1").expect("child-1 present");
    child
        .transition_to(TaskLifecycle::Running)
        .expect("running transition should succeed");
    child
        .transition_to(TaskLifecycle::Completed)
        .expect("terminal transition should succeed");
    session
        .spawn_subtask(
            "parent-2",
            "child-2",
            "second child after completion",
            TaskExecutionMode::DelegatedChild,
            ChildBudgetPolicy::Inherit,
            None,
        )
        .expect("completed child tasks should free session overlay capacity");
}

#[test]
fn task_budget_caps_artifact_retention_to_latest_records() {
    let budget = TaskBudget {
        artifact_retention: loong_core::RetentionBudget {
            max_records: Some(2),
        },
        ..TaskBudget::default()
    };
    let mut task = Task::new(
        "task-retention-artifacts",
        "retain only the latest artifacts",
        workspace("feat/retention", "retention-artifacts"),
        budget,
        TaskExecutionMode::DetachedBackground,
    );

    task.record_artifact(ExecutionArtifact::new(
        "artifact-1",
        ExecutionArtifactKind::AssistantTextOutput {
            text: "first".to_owned(),
        },
    ));
    task.record_artifact(ExecutionArtifact::new(
        "artifact-2",
        ExecutionArtifactKind::AssistantTextOutput {
            text: "second".to_owned(),
        },
    ));
    task.record_artifact(ExecutionArtifact::new(
        "artifact-3",
        ExecutionArtifactKind::AssistantTextOutput {
            text: "third".to_owned(),
        },
    ));

    let artifact_ids = task
        .artifacts()
        .iter()
        .map(|artifact| artifact.artifact_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(artifact_ids, vec!["artifact-2", "artifact-3"]);
}

#[test]
fn task_budget_caps_event_retention_to_latest_records() {
    let budget = TaskBudget {
        event_retention: loong_core::RetentionBudget {
            max_records: Some(3),
        },
        ..TaskBudget::default()
    };
    let mut task = Task::new(
        "task-retention-events",
        "retain only the latest events",
        workspace("feat/retention", "retention-events"),
        budget,
        TaskExecutionMode::InteractiveAttached,
    );

    task.transition_to(TaskLifecycle::Running)
        .expect("running transition should succeed");
    task.begin_turn("turn-1", "retained turn").unwrap();
    task.record_artifact(ExecutionArtifact::new(
        "artifact-1",
        ExecutionArtifactKind::AssistantTextOutput {
            text: "retained".to_owned(),
        },
    ));

    let retained_kinds = task
        .fact_events()
        .iter()
        .map(|event| match event {
            TaskEvent::TaskCreated { .. } => "task_created",
            TaskEvent::WorkspaceBound { .. } => "workspace_bound",
            TaskEvent::LifecycleChanged { .. } => "lifecycle_changed",
            TaskEvent::TurnUpdated { .. } => "turn_updated",
            TaskEvent::ArtifactRecorded { .. } => "artifact_recorded",
            TaskEvent::SubtaskRegistered { .. } => "subtask_registered",
        })
        .collect::<Vec<_>>();
    assert_eq!(
        retained_kinds,
        vec!["lifecycle_changed", "turn_updated", "artifact_recorded"]
    );
}

#[test]
fn session_overlay_budget_caps_event_retention_to_latest_records() {
    let first_workspace = workspace("feat/session-retention", "session-retention-a");
    let second_workspace = workspace("feat/session-retention", "session-retention-b");
    let third_workspace = workspace("feat/session-retention", "session-retention-c");
    let overlay = SessionBudgetOverlay {
        max_total_events: Some(2),
        ..SessionBudgetOverlay::default()
    };
    let mut session = Session::new("session-retention", first_workspace.clone(), overlay);

    session
        .switch_worktree(second_workspace.clone())
        .expect("first switch should succeed");
    session
        .switch_worktree(third_workspace.clone())
        .expect("second switch should succeed");

    let retained_events = session.session_events();
    assert_eq!(retained_events.len(), 2);
    assert!(matches!(
        &retained_events[0],
        SessionEvent::WorktreeSwitched {
            session_id,
            previous_workspace,
            next_workspace,
        } if session_id == "session-retention"
            && previous_workspace == &first_workspace
            && next_workspace == &second_workspace
    ));
    assert!(matches!(
        &retained_events[1],
        SessionEvent::WorktreeSwitched {
            session_id,
            previous_workspace,
            next_workspace,
        } if session_id == "session-retention"
            && previous_workspace == &second_workspace
            && next_workspace == &third_workspace
    ));
}

#[test]
fn session_overlay_budget_caps_artifact_retention_to_latest_records() {
    let overlay = SessionBudgetOverlay {
        max_total_artifacts: Some(2),
        ..SessionBudgetOverlay::default()
    };
    let mut session = Session::new(
        "session-artifact-retention",
        workspace("feat/session-retention", "session-retention-artifacts"),
        overlay,
    );

    session.record_artifact(ExecutionArtifact::new(
        "artifact-1",
        ExecutionArtifactKind::AssistantTextOutput {
            text: "first".to_owned(),
        },
    ));
    session.record_artifact(ExecutionArtifact::new(
        "artifact-2",
        ExecutionArtifactKind::AssistantTextOutput {
            text: "second".to_owned(),
        },
    ));
    session.record_artifact(ExecutionArtifact::new(
        "artifact-3",
        ExecutionArtifactKind::AssistantTextOutput {
            text: "third".to_owned(),
        },
    ));

    let artifact_ids = session
        .artifacts()
        .iter()
        .map(|artifact| artifact.artifact_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(artifact_ids, vec!["artifact-2", "artifact-3"]);
}
