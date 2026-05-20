use serde_json::json;

use super::{
    SessionsCommandExecution, build_session_heal_plan, render_session_heal_plan_lines,
    render_session_inspection_lines, render_sessions_cli_text,
};

#[test]
fn render_session_inspection_lines_includes_diagnostics_summaries() {
    let detail = json!({
        "session": {
            "session_id": "session-1",
            "kind": "root",
            "state": "running",
            "parent_session_id": null,
            "label": "Root",
            "turn_count": 3,
            "last_turn_at": 123,
            "last_error": "rate_limited"
        },
        "workflow": {},
        "terminal_outcome_state": "not_terminal",
        "terminal_outcome": null,
        "recovery": null,
        "recent_events": [],
        "diagnostics": {
            "latest_provider_failover": {
                "reason": "rate_limited",
                "model": "gpt-4o",
                "stage": "status_failure",
                "request_id": "req-123"
            },
            "recommended_action": {
                "tool_name": "session_wait",
                "kind": "follow_resume_recipe",
                "source": "task_progress_resume_recipe"
            }
        }
    });

    let lines = render_session_inspection_lines(&detail).expect("render lines");

    assert!(
        lines.iter().any(|line| {
            line == "latest_provider_failover: reason=rate_limited model=gpt-4o stage=status_failure request_id=req-123"
        }),
        "expected latest_provider_failover summary, got: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| {
            line == "recommended_action: tool=session_wait kind=follow_resume_recipe source=task_progress_resume_recipe"
        }),
        "expected recommended_action summary, got: {lines:#?}"
    );
}

#[test]
fn build_session_heal_plan_marks_manual_checkpoint_recovery_as_observe_only() {
    let detail = json!({
        "diagnostics": {
            "attention_hints": ["checkpoint attention"]
        },
        "turn_checkpoint": {
            "summary": {
                "requires_recovery": true
            }
        }
    });

    let plan =
        build_session_heal_plan("/tmp/loong.toml", "ops-root", "session-1", &detail)
            .expect("build heal plan");

    assert_eq!(plan.actions.len(), 1);
    assert_eq!(plan.actions[0].tool_name, "session_status");
    assert!(!plan.actions[0].can_apply);
    assert_eq!(plan.actions[0].kind, "turn_checkpoint_manual_review");
    assert_eq!(plan.attention_hints, vec!["checkpoint attention".to_owned()]);
}

#[test]
fn build_session_heal_plan_adds_turn_checkpoint_repair_action_when_runtime_repair_is_possible() {
    let detail = json!({
        "diagnostics": {
            "attention_hints": ["checkpoint attention"]
        },
        "turn_checkpoint": {
            "summary": {
                "requires_recovery": true,
                "latest_identity_present": true,
                "latest_runs_after_turn": true,
                "latest_attempts_context_compaction": true,
                "latest_after_turn": "completed",
                "latest_compaction": "failed",
                "session_state": "finalization_failed",
                "checkpoint_durable": true,
                "reply_durable": true
            }
        }
    });

    let plan =
        build_session_heal_plan("/tmp/loong.toml", "ops-root", "session-1", &detail)
            .expect("build heal plan");

    assert_eq!(plan.actions.len(), 1);
    assert_eq!(plan.actions[0].tool_name, "turn_checkpoint_repair");
    assert!(plan.actions[0].can_apply);
    assert_eq!(plan.actions[0].kind, "run_compaction");
}

#[test]
fn build_session_heal_plan_preserves_task_surface_commands_for_resume_recipes() {
    let detail = json!({
        "task_progress": {
            "task_id": "task-123"
        },
        "diagnostics": {
            "recommended_action": {
                "tool_name": "task_status",
                "kind": "follow_resume_recipe",
                "source": "task_progress_resume_recipe",
                "session_id": "task-owner",
                "requires_mutation": false
            },
            "attention_hints": []
        }
    });

    let plan =
        build_session_heal_plan("/tmp/loong.toml", "ops-root", "session-1", &detail)
            .expect("build heal plan");

    assert_eq!(plan.actions.len(), 1);
    assert_eq!(plan.actions[0].tool_name, "task_status");
    assert_eq!(
        plan.actions[0].command,
        "loong tasks status --config '/tmp/loong.toml' --session 'ops-root' 'task-123'"
    );
    assert!(!plan.actions[0].can_apply);
}

#[test]
fn render_session_heal_plan_lines_surface_actions_and_hints() {
    let plan = json!({
        "action_count": 1,
        "applyable_count": 1,
        "attention_count": 1,
        "actions": [{
            "id": "recommended:session_recover",
            "tool_name": "session_recover",
            "kind": "queued_async_overdue_marked_failed",
            "source": "session_recover_plan",
            "can_apply": true
        }],
        "attention_hints": ["provider_failover_present reason=rate_limited"]
    });

    let lines = render_session_heal_plan_lines(&plan).expect("render heal plan lines");

    assert!(
        lines.iter().any(|line| {
            line.contains("actions=1")
                && line.contains("applyable=1")
                && line.contains("attention_hints=1")
        }),
        "expected heal plan summary, got: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| {
            line.contains("recommended:session_recover")
                && line.contains("tool=session_recover")
                && line.contains("apply=yes")
        }),
        "expected action line, got: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("hint provider_failover_present")),
        "expected attention hint line, got: {lines:#?}"
    );
}

#[test]
fn render_session_inspection_lines_includes_tree_summary() {
    let detail = json!({
        "session": {
            "session_id": "root-session",
            "kind": "root",
            "parent_session_id": null,
            "label": "Root",
            "state": "ready",
            "turn_count": 2,
            "last_turn_at": 123,
            "last_error": null,
        },
        "workflow": {
            "workflow_id": "root-session",
        },
        "tree": {
            "head_count": 2,
            "active_path_count": 3,
            "artifact_count": 2,
            "active_head_name": "active",
            "artifact_counts": {
                "checkpoint": 1,
                "branch_summary": 1,
            },
            "heads": [
                {"head_name": "active"},
                {"head_name": "thread/alpha"},
            ],
        },
        "recent_events": [],
    });

    let lines = render_session_inspection_lines(&detail).expect("render session inspection");

    assert!(lines.iter().any(|line| line == "tree_head_count: 2"));
    assert!(lines.iter().any(|line| line == "tree_active_head: active"));
    assert!(
        lines
            .iter()
            .any(|line| line == "tree_head_names: active, thread/alpha")
    );
    assert!(lines.iter().any(|line| line == "tree_artifact_count: 2"));
}

#[test]
fn render_session_inspection_lines_includes_branch_summary_counts() {
    let detail = json!({
        "session": {
            "session_id": "root-session",
            "kind": "root",
            "parent_session_id": null,
            "label": "Root",
            "state": "ready",
            "turn_count": 1,
            "last_turn_at": null,
            "last_error": null,
        },
        "workflow": {
            "workflow_id": "root-session",
        },
        "tree": {
            "head_count": 1,
            "active_path_count": 2,
            "artifact_count": 3,
            "active_head_name": "active",
            "artifact_counts": {
                "checkpoint": 1,
                "branch_summary": 2,
            },
            "heads": [
                {"head_name": "active"},
            ],
        },
        "recent_events": [],
    });

    let lines = render_session_inspection_lines(&detail).expect("render session inspection");

    assert!(lines.iter().any(|line| line == "tree_checkpoint_count: 1"));
    assert!(
        lines
            .iter()
            .any(|line| line == "tree_branch_summary_count: 2")
    );
}

#[test]
fn render_sessions_path_text_includes_head_and_nodes() {
    let execution = SessionsCommandExecution {
        resolved_config_path: "/tmp/loong.toml".to_owned(),
        current_session_id: "root-session".to_owned(),
        payload: json!({
            "command": "path",
            "detail": {
                "session_id": "root-session",
                "head_name": "thread/alpha",
                "path": [
                    {
                        "node_id": "session-root:root-session",
                        "kind": "root",
                        "role": null,
                        "content": null,
                    },
                    {
                        "node_id": "session-turn:root-session:1",
                        "kind": "user_turn",
                        "role": "user",
                        "content": "hello",
                    }
                ]
            }
        }),
    };

    let rendered = render_sessions_cli_text(&execution).expect("rendered path");

    assert!(rendered.contains("path for `root-session` head `thread/alpha` (2)"));
    assert!(rendered.contains("session-turn:root-session:1"));
    assert!(rendered.contains("content=hello"));
}

#[test]
fn render_sessions_tree_mutation_text_includes_artifact_summary() {
    let execution = SessionsCommandExecution {
        resolved_config_path: "/tmp/loong.toml".to_owned(),
        current_session_id: "root-session".to_owned(),
        payload: json!({
            "command": "branch-summary",
            "detail": {
                "session_id": "root-session",
                "artifact": {
                    "artifact_id": "branch-summary:root-session:1:thread_alpha",
                    "kind": "branch_summary",
                    "summary_text": "alpha summary"
                }
            }
        }),
    };

    let rendered = render_sessions_cli_text(&execution).expect("rendered mutation");

    assert!(rendered.contains("branch-summary for `root-session`"));
    assert!(rendered.contains("kind=branch_summary"));
    assert!(rendered.contains("summary=alpha summary"));
}
