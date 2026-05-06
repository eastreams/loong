use crate::context::bootstrap_test_kernel_context;
use std::fs;
use std::time::Duration;

use serde_json::json;

use super::*;
use crate::config::{AutonomyProfile, GovernedToolApprovalMode, ToolConfig};
use crate::session::repository::{
    ApprovalRequestStatus, NewApprovalGrantRecord, NewSessionEvent, NewSessionRecord, SessionKind,
    SessionRepository, SessionState,
};

fn isolated_memory_config(test_name: &str) -> SessionStoreConfig {
    let base = std::env::temp_dir().join(format!(
        "loong-turn-engine-approval-{test_name}-{}",
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

fn test_kernel_context(agent_id: &str) -> KernelContext {
    crate::context::bootstrap_test_kernel_context(agent_id, 60)
        .expect("bootstrap test kernel context")
}

fn kernel_context(agent_id: &str) -> KernelContext {
    test_kernel_context(agent_id)
}

#[test]
fn tool_decision_telemetry_builder_chain_preserves_policy_metadata() {
    let decision = ToolDecisionTelemetry::allow("shell.exec", "allowed", "rule-allow")
        .with_policy_source("autonomy")
        .with_autonomy_profile("full")
        .with_capability_action_class("shell")
        .with_reason_code("autonomy_policy_allow");

    assert_eq!(decision.tool_name, "shell.exec");
    assert_eq!(decision.decision_kind, ToolDecisionKind::Allow);
    assert!(decision.allow);
    assert!(!decision.deny);
    assert_eq!(decision.reason, "allowed");
    assert_eq!(decision.rule_id, "rule-allow");
    assert_eq!(
        decision.reason_code.as_deref(),
        Some("autonomy_policy_allow")
    );
    assert_eq!(decision.policy_source.as_deref(), Some("autonomy"));
    assert_eq!(decision.autonomy_profile.as_deref(), Some("full"));
    assert_eq!(decision.capability_action_class.as_deref(), Some("shell"));
}

#[test]
fn turn_failure_discovery_recovery_builder_marks_non_retryable_policy_denial() {
    let failure = TurnFailure::policy_denied_with_discovery_recovery(
        "tool_not_found",
        "search for a hidden tool instead",
    );

    assert_eq!(failure.kind, TurnFailureKind::PolicyDenied);
    assert_eq!(failure.code, "tool_not_found");
    assert_eq!(failure.reason, "search for a hidden tool instead");
    assert!(!failure.retryable);
    assert!(failure.supports_discovery_recovery);
}

#[test]
fn tool_execution_preflight_ready_clears_trusted_internal_context() {
    let preflight = ToolExecutionPreflight::ready(ToolCoreRequest {
        tool_name: "shell.exec".to_owned(),
        payload: json!({"command": "echo hello"}),
    });

    match preflight {
        ToolExecutionPreflight::Ready {
            request,
            trusted_internal_context,
        } => {
            assert_eq!(request.tool_name, "shell.exec");
            assert_eq!(request.payload, json!({"command": "echo hello"}));
            assert!(!trusted_internal_context);
        }
        ToolExecutionPreflight::NeedsApproval(requirement) => {
            panic!("unexpected approval requirement: {:?}", requirement)
        }
    }
}

#[test]
fn default_app_tool_dispatcher_scopes_child_sessions_to_self_only_visibility() {
    let dispatcher = DefaultAppToolDispatcher::new(
        store::current_session_store_config().clone(),
        ToolConfig::default(),
    );
    let root = SessionContext::root_with_tool_view("root-session", runtime_tool_view());
    let child = SessionContext::child("child-session", "root-session", runtime_tool_view());

    assert_eq!(
        dispatcher
            .effective_tool_config_for_session(&root)
            .sessions
            .visibility,
        SessionVisibility::Children
    );
    assert_eq!(
        dispatcher
            .effective_tool_config_for_session(&child)
            .sessions
            .visibility,
        SessionVisibility::SelfOnly
    );
}

#[test]
fn session_context_from_turn_uses_first_intent_session_id() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "shell.exec".to_owned(),
            args_json: json!({"command": "echo hello"}),
            source: "assistant".to_owned(),
            session_id: "session-from-first-intent".to_owned(),
            turn_id: "turn-1".to_owned(),
            tool_call_id: "call-1".to_owned(),
        }],
        raw_meta: Value::Null,
    };

    let session_context = session_context_from_turn(&turn, runtime_tool_view());
    assert_eq!(session_context.session_id, "session-from-first-intent");
}

#[test]
fn validate_turn_in_context_allows_internal_approval_control_resolve_tool() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "approval_request_resolve".to_owned(),
            args_json: json!({
                "approval_request_id": "apr-allow-1",
                "decision": "approve_once"
            }),
            source: "approval_control".to_owned(),
            session_id: "session-approval-control".to_owned(),
            turn_id: "turn-approval-control".to_owned(),
            tool_call_id: "call-approval-control".to_owned(),
        }],
        raw_meta: Value::Null,
    };
    let tool_view = crate::tools::ToolView::from_tool_names([
        "approval_request_resolve",
        "approval_request_status",
        "approval_requests_list",
    ]);
    let session_context =
        SessionContext::root_with_tool_view("session-approval-control", tool_view);

    let validation = TurnEngine::new(4)
        .validate_turn_in_context(&turn, &session_context)
        .expect("approval-control resolve should stay executable");

    assert_eq!(validation, TurnValidation::ToolExecutionRequired);
}

#[test]
fn prepare_tool_intent_uses_direct_shell_metadata_for_provider_shell_requests() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call(
        "shell.exec",
        json!({
            "command": "echo",
            "args": ["hello"],
        }),
    );
    let intent = ToolIntent {
        tool_name,
        args_json,
        source: "provider_tool_call".to_owned(),
        session_id: "session-shell-invoke-trace".to_owned(),
        turn_id: "turn-shell-invoke-trace".to_owned(),
        tool_call_id: "call-shell-invoke-trace".to_owned(),
    };
    let session_context =
        SessionContext::root_with_tool_view("session-shell-invoke-trace", runtime_tool_view());
    let engine = TurnEngine::new(4);
    let runtime = tokio::runtime::Runtime::new().expect("test runtime");
    let prepared_intent = runtime.block_on(async {
        let autonomy_budget_state = AutonomyTurnBudgetState::default();
        engine
            .prepare_tool_intent(
                &intent,
                0,
                &session_context,
                &DefaultAppToolDispatcher::runtime(),
                ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
                &autonomy_budget_state,
                None,
            )
            .await
            .expect("provider shell request should prepare successfully")
    });

    assert_eq!(prepared_intent.request.tool_name, "bash");
    assert_eq!(prepared_intent.intent.tool_name, "bash");
    assert_eq!(
        prepared_intent.intent.args_json,
        json!({
            "command": "echo",
            "args": ["hello"],
        })
    );
}

fn delegate_async_turn(session_id: &str, turn_id: &str, tool_call_id: &str) -> ProviderTurn {
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
        "delegate_async",
        json!({
            "task": "inspect the child task"
        }),
        Some(session_id),
        Some(turn_id),
    );
    ProviderTurn {
        assistant_text: "queueing child delegate".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name,
            args_json,
            source: "assistant".to_owned(),
            session_id: session_id.to_owned(),
            turn_id: turn_id.to_owned(),
            tool_call_id: tool_call_id.to_owned(),
        }],
        raw_meta: json!({}),
    }
}

fn discovered_delegate_async_turn(
    session_id: &str,
    turn_id: &str,
    tool_call_id: &str,
) -> ProviderTurn {
    delegate_async_turn(session_id, turn_id, tool_call_id)
}

fn skills_policy_get_turn(session_id: &str, turn_id: &str, tool_call_id: &str) -> ProviderTurn {
    let payload = json!({
        "action": "get"
    });
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
        "skills.policy",
        payload,
        Some(session_id),
        Some(turn_id),
    );
    ProviderTurn {
        assistant_text: "reading skills policy".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name,
            args_json,
            source: "assistant".to_owned(),
            session_id: session_id.to_owned(),
            turn_id: turn_id.to_owned(),
            tool_call_id: tool_call_id.to_owned(),
        }],
        raw_meta: json!({}),
    }
}

fn discovered_shell_exec_turn(session_id: &str, turn_id: &str, tool_call_id: &str) -> ProviderTurn {
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
        "shell.exec",
        json!({
            "command": "cargo",
            "args": ["--version"]
        }),
        Some(session_id),
        Some(turn_id),
    );
    ProviderTurn {
        assistant_text: "checking cargo version".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name,
            args_json,
            source: "assistant".to_owned(),
            session_id: session_id.to_owned(),
            turn_id: turn_id.to_owned(),
            tool_call_id: tool_call_id.to_owned(),
        }],
        raw_meta: json!({}),
    }
}

fn provider_tool_turn(
    tool_name: &str,
    args_json: serde_json::Value,
    session_id: &str,
    turn_id: &str,
    tool_call_id: &str,
) -> ProviderTurn {
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
        tool_name,
        args_json,
        Some(session_id),
        Some(turn_id),
    );
    ProviderTurn {
        assistant_text: format!("calling {tool_name}"),
        tool_intents: vec![ToolIntent {
            tool_name,
            args_json,
            source: "assistant".to_owned(),
            session_id: session_id.to_owned(),
            turn_id: turn_id.to_owned(),
            tool_call_id: tool_call_id.to_owned(),
        }],
        raw_meta: json!({}),
    }
}

fn provider_app_tool_intent(
    tool_name: &str,
    args_json: serde_json::Value,
    session_id: &str,
    turn_id: &str,
    tool_call_id: &str,
) -> ToolIntent {
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
        tool_name,
        args_json,
        Some(session_id),
        Some(turn_id),
    );
    ToolIntent {
        tool_name,
        args_json,
        source: "assistant".to_owned(),
        session_id: session_id.to_owned(),
        turn_id: turn_id.to_owned(),
        tool_call_id: tool_call_id.to_owned(),
    }
}

fn fast_lane_observed_execution_turn(
    session_id: &str,
    turn_id: &str,
    call_prefix: &str,
) -> ProviderTurn {
    ProviderTurn {
        assistant_text: "observing mixed fast-lane execution".to_owned(),
        tool_intents: vec![
            provider_app_tool_intent(
                "sessions_list",
                json!({}),
                session_id,
                turn_id,
                &format!("{call_prefix}-1"),
            ),
            provider_app_tool_intent(
                "sessions_list",
                json!({}),
                session_id,
                turn_id,
                &format!("{call_prefix}-2"),
            ),
            provider_app_tool_intent(
                "session_status",
                json!({"session_id": session_id}),
                session_id,
                turn_id,
                &format!("{call_prefix}-3"),
            ),
            provider_app_tool_intent(
                "sessions_list",
                json!({}),
                session_id,
                turn_id,
                &format!("{call_prefix}-4"),
            ),
            provider_app_tool_intent(
                "sessions_list",
                json!({}),
                session_id,
                turn_id,
                &format!("{call_prefix}-5"),
            ),
        ],
        raw_meta: json!({}),
    }
}

struct DelayedObservedExecutionDispatcher;

#[async_trait::async_trait]
impl AppToolDispatcher for DelayedObservedExecutionDispatcher {
    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolCoreOutcome, String> {
        let payload_delay_ms = request.payload.get("delay_ms").and_then(Value::as_u64);
        let delay_ms = match payload_delay_ms {
            Some(delay_ms) => delay_ms,
            None => match request.tool_name.as_str() {
                "sessions_list" => 25,
                "session_status" => 10,
                other => return Err(format!("app_tool_not_found: {other}")),
            },
        };
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "tool": request.tool_name,
                "session_id": session_context.session_id,
            }),
        })
    }
}

struct AfterExecutionSequenceRecordingDispatcher {
    after_calls: std::sync::Arc<std::sync::Mutex<Vec<(String, usize)>>>,
}

#[async_trait::async_trait]
impl AppToolDispatcher for AfterExecutionSequenceRecordingDispatcher {
    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolCoreOutcome, String> {
        let delay_ms = match request.tool_name.as_str() {
            "sessions_list" => 25,
            "session_status" => 10,
            other => return Err(format!("app_tool_not_found: {other}")),
        };
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "tool": request.tool_name,
                "session_id": session_context.session_id,
            }),
        })
    }

    async fn after_tool_execution(
        &self,
        _session_context: &SessionContext,
        intent: &ToolIntent,
        intent_sequence: usize,
        _request: &ToolCoreRequest,
        _outcome: &ToolCoreOutcome,
        _binding: ConversationRuntimeBinding<'_>,
    ) {
        let mut after_calls = self.after_calls.lock().expect("after call lock");
        let call_record = (intent.tool_call_id.clone(), intent_sequence);
        after_calls.push(call_record);
    }
}

fn partially_failing_observed_execution_turn(session_id: &str, turn_id: &str) -> ProviderTurn {
    ProviderTurn {
        assistant_text: "observing a partial tool failure".to_owned(),
        tool_intents: vec![
            provider_app_tool_intent(
                "sessions_list",
                json!({}),
                session_id,
                turn_id,
                "call-partial-1",
            ),
            provider_app_tool_intent(
                "session_status",
                json!({"session_id": session_id}),
                session_id,
                turn_id,
                "call-partial-2",
            ),
        ],
        raw_meta: json!({}),
    }
}

struct PartiallyFailingObservedExecutionDispatcher;

#[async_trait::async_trait]
impl AppToolDispatcher for PartiallyFailingObservedExecutionDispatcher {
    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolCoreOutcome, String> {
        match request.tool_name.as_str() {
            "sessions_list" => Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "tool": request.tool_name,
                    "session_id": session_context.session_id,
                }),
            }),
            "session_status" => Err("simulated observed tool failure".to_owned()),
            other => Err(format!("app_tool_not_found: {other}")),
        }
    }
}

#[tokio::test]
async fn autonomy_policy_approval_request_is_persisted_for_delegate_async() {
    let memory_config = isolated_memory_config("persist");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");

    let tool_config = ToolConfig {
        autonomy_profile: AutonomyProfile::GuidedAcquisition,
        ..ToolConfig::default()
    };
    let tool_view = runtime_tool_view_for_config(&tool_config);
    let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
    let kernel_ctx = test_kernel_context("turn-engine-governed-approval-delegate-async");

    let result = TurnEngine::new(4)
        .execute_turn_in_context(
            &delegate_async_turn("root-session", "turn-1", "call-1"),
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;

    let approval_request_id = match result {
        TurnResult::NeedsApproval(requirement) => {
            assert_eq!(requirement.tool_name.as_deref(), Some("delegate_async"));
            assert_eq!(
                requirement.approval_key.as_deref(),
                Some("tool:delegate_async")
            );
            assert_eq!(
                requirement.rule_id.as_str(),
                "autonomy_policy_topology_mutation_requires_approval"
            );
            requirement
                .approval_request_id
                .expect("approval request id should be present")
        }
        other @ TurnResult::FinalText(_)
        | other @ TurnResult::StreamingText(_)
        | other @ TurnResult::StreamingDone(_)
        | other @ TurnResult::ToolDenied(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_) => {
            panic!("expected NeedsApproval, got {other:?}")
        }
    };

    let stored = repo
        .load_approval_request(&approval_request_id)
        .expect("load approval request")
        .expect("approval request row");
    assert_eq!(stored.status, ApprovalRequestStatus::Pending);
    assert_eq!(stored.tool_name, "delegate_async");
    assert_eq!(stored.tool_call_id, "call-1");
    assert_eq!(stored.turn_id, "turn-1");
    assert_eq!(stored.approval_key, "tool:delegate_async");
    assert_eq!(
        stored.governance_snapshot_json["policy_source"],
        "autonomy_policy"
    );
    assert_eq!(
        stored.governance_snapshot_json["capability_action_class"],
        "topology_expand"
    );
}

#[tokio::test]
async fn autonomy_policy_approval_request_is_persisted_for_discovered_delegate_async() {
    let memory_config = isolated_memory_config("persist-discovered");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");

    let tool_config = ToolConfig {
        autonomy_profile: AutonomyProfile::GuidedAcquisition,
        ..ToolConfig::default()
    };
    let tool_view = runtime_tool_view_for_config(&tool_config);
    let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
    let kernel_ctx = test_kernel_context("turn-engine-governed-approval-discovered-delegate-async");

    let result = TurnEngine::new(4)
        .execute_turn_in_context(
            &discovered_delegate_async_turn("root-session", "turn-discovered", "call-discovered"),
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;

    let approval_request_id = match result {
        TurnResult::NeedsApproval(requirement) => {
            assert_eq!(requirement.tool_name.as_deref(), Some("delegate_async"));
            assert_eq!(
                requirement.approval_key.as_deref(),
                Some("tool:delegate_async")
            );
            requirement
                .approval_request_id
                .expect("approval request id should be present")
        }
        other @ TurnResult::FinalText(_)
        | other @ TurnResult::StreamingText(_)
        | other @ TurnResult::StreamingDone(_)
        | other @ TurnResult::ToolDenied(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_) => {
            panic!("expected NeedsApproval, got {other:?}")
        }
    };

    let stored = repo
        .load_approval_request(&approval_request_id)
        .expect("load approval request")
        .expect("approval request row");
    assert_eq!(stored.status, ApprovalRequestStatus::Pending);
    assert_eq!(stored.tool_name, "delegate_async");
    assert_eq!(stored.turn_id, "turn-discovered");
    assert_eq!(stored.tool_call_id, "call-discovered");
    assert_eq!(stored.approval_key, "tool:delegate_async");
    assert_eq!(stored.request_payload_json["tool_name"], "delegate_async");
    assert_eq!(
        stored.request_payload_json["args_json"],
        json!({
            "task": "inspect the child task"
        })
    );
}

#[tokio::test]
async fn auto_mode_requires_approval_for_high_risk_core_tool() {
    let memory_config = isolated_memory_config("claw-migrate-core-approval");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");

    let mut tool_config = ToolConfig::default();
    tool_config.consent.default_mode = ToolConsentMode::Auto;
    let tool_view = runtime_tool_view_for_config(&tool_config);
    let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
    let kernel_ctx = kernel_context("turn-engine-config-import-auto");

    let result = TurnEngine::new(4)
        .execute_turn_in_context(
            &provider_tool_turn(
                "config.import",
                json!({}),
                "root-session",
                "turn-config-import-auto",
                "call-config-import-auto",
            ),
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;

    let TurnResult::NeedsApproval(requirement) = result else {
        panic!("expected NeedsApproval, got {result:?}");
    };
    assert_eq!(requirement.tool_name.as_deref(), Some("config.import"));
    assert_eq!(
        requirement.approval_key.as_deref(),
        Some("tool:config.import")
    );
    assert_eq!(
        requirement.rule_id.as_str(),
        "session_tool_consent_auto_blocked"
    );
    let approval_request_id = requirement
        .approval_request_id
        .expect("approval request id should be present");

    let stored = repo
        .load_approval_request(&approval_request_id)
        .expect("load approval request")
        .expect("approval request row");
    assert_eq!(stored.status, ApprovalRequestStatus::Pending);
    assert_eq!(stored.tool_name, "config.import");
    assert_eq!(stored.request_payload_json["execution_kind"], "core");
}

#[tokio::test]
async fn full_session_consent_skips_approval_for_high_risk_core_tool() {
    let memory_config = isolated_memory_config("claw-migrate-core-full");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");
    repo.upsert_session_tool_consent(crate::session::repository::NewSessionToolConsentRecord {
        scope_session_id: "root-session".to_owned(),
        mode: ToolConsentMode::Full,
        updated_by_session_id: Some("root-session".to_owned()),
    })
    .expect("persist full session consent");

    let tool_config = ToolConfig::default();
    let tool_view = runtime_tool_view_for_config(&tool_config);
    let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
    let kernel_ctx = kernel_context("turn-engine-config-import-full");

    let result = TurnEngine::new(4)
        .execute_turn_in_context(
            &provider_tool_turn(
                "config.import",
                json!({}),
                "root-session",
                "turn-config-import-full",
                "call-config-import-full",
            ),
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;

    let TurnResult::ToolError(failure) = result else {
        panic!("expected direct tool execution, got {result:?}");
    };
    assert!(
        failure
            .reason
            .contains("config.import requires payload.input_path"),
        "expected execution to reach the core tool, got: {failure:?}"
    );
}

#[tokio::test]
async fn autonomy_policy_approval_request_reuses_deterministic_id_for_same_blocked_call() {
    let memory_config = isolated_memory_config("reuse");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");

    let tool_config = ToolConfig {
        autonomy_profile: AutonomyProfile::GuidedAcquisition,
        ..ToolConfig::default()
    };
    let tool_view = runtime_tool_view_for_config(&tool_config);
    let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
    let turn = delegate_async_turn("root-session", "turn-reuse", "call-reuse");
    let kernel_ctx = test_kernel_context("turn-engine-governed-approval-reuse");

    let first = TurnEngine::new(4)
        .execute_turn_in_context(
            &turn,
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;
    let second = TurnEngine::new(4)
        .execute_turn_in_context(
            &turn,
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;

    let first_request_id = match first {
        TurnResult::NeedsApproval(requirement) => requirement
            .approval_request_id
            .expect("first approval request id"),
        other @ TurnResult::FinalText(_)
        | other @ TurnResult::StreamingText(_)
        | other @ TurnResult::StreamingDone(_)
        | other @ TurnResult::ToolDenied(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_) => {
            panic!("expected first NeedsApproval, got {other:?}")
        }
    };
    let second_request_id = match second {
        TurnResult::NeedsApproval(requirement) => requirement
            .approval_request_id
            .expect("second approval request id"),
        other @ TurnResult::FinalText(_)
        | other @ TurnResult::StreamingText(_)
        | other @ TurnResult::StreamingDone(_)
        | other @ TurnResult::ToolDenied(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_) => {
            panic!("expected second NeedsApproval, got {other:?}")
        }
    };

    assert_eq!(first_request_id, second_request_id);

    let requests = repo
        .list_approval_requests_for_session("root-session", None)
        .expect("list approval requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].approval_request_id, first_request_id);
}

#[tokio::test]
async fn autonomy_policy_preapproved_call_executes_without_persisting_request() {
    let memory_config = isolated_memory_config("preapproved");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");

    let approval_key = "tool:skills.policy".to_owned();
    let mut tool_config = ToolConfig {
        autonomy_profile: AutonomyProfile::GuidedAcquisition,
        ..ToolConfig::default()
    };
    let approved_calls = &mut tool_config.approval.approved_calls;
    approved_calls.push(approval_key);

    let tool_view = runtime_tool_view_for_config(&tool_config);
    let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
    let kernel_ctx = kernel_context("turn-engine-autonomy-preapproved");
    let turn = skills_policy_get_turn("root-session", "turn-preapproved", "call-preapproved");

    let result = TurnEngine::new(4)
        .execute_turn_in_context(
            &turn,
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;

    let failure = match result {
        TurnResult::ToolDenied(failure) => failure,
        other @ TurnResult::FinalText(_)
        | other @ TurnResult::NeedsApproval(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_)
        | other @ TurnResult::StreamingText(_)
        | other @ TurnResult::StreamingDone(_) => {
            panic!("expected ToolDenied, got {other:?}")
        }
    };
    assert_eq!(failure.code, "tool_not_found");
    assert!(failure.reason.contains("skills.policy"));

    let requests = repo
        .list_approval_requests_for_session("root-session", None)
        .expect("list approval requests");
    assert!(requests.is_empty());
}

#[tokio::test]
async fn autonomy_policy_predenied_call_returns_policy_denial_without_persisting_request() {
    let memory_config = isolated_memory_config("predenied");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");

    let denial_key = "tool:skills.policy".to_owned();
    let mut tool_config = ToolConfig {
        autonomy_profile: AutonomyProfile::GuidedAcquisition,
        ..ToolConfig::default()
    };
    let denied_calls = &mut tool_config.approval.denied_calls;
    denied_calls.push(denial_key);

    let tool_view = runtime_tool_view_for_config(&tool_config);
    let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
    let kernel_ctx = kernel_context("turn-engine-autonomy-predenied");
    let turn = skills_policy_get_turn("root-session", "turn-predenied", "call-predenied");

    let result = TurnEngine::new(4)
        .execute_turn_in_context(
            &turn,
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;

    let failure = match result {
        TurnResult::ToolDenied(failure) => failure,
        other @ TurnResult::FinalText(_)
        | other @ TurnResult::NeedsApproval(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_)
        | other @ TurnResult::StreamingText(_)
        | other @ TurnResult::StreamingDone(_) => {
            panic!("expected ToolDenied, got {other:?}")
        }
    };

    assert_eq!(failure.code, "tool_not_found");
    assert!(failure.reason.contains("skills.policy"));

    let requests = repo
        .list_approval_requests_for_session("root-session", None)
        .expect("list approval requests");
    assert!(requests.is_empty());
}

#[tokio::test]
async fn governed_tool_approval_request_is_persisted_for_discovered_shell_exec() {
    let memory_config = isolated_memory_config("persist-shell");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");

    let mut tool_config = ToolConfig::default();
    tool_config.approval.mode = GovernedToolApprovalMode::Strict;
    tool_config.consent.default_mode = ToolConsentMode::Prompt;
    let tool_view = runtime_tool_view_for_config(&tool_config);
    let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
    let kernel_ctx = bootstrap_test_kernel_context("turn-engine-governed-shell-approval", 60)
        .expect("kernel context");

    let result = TurnEngine::new(4)
        .execute_turn_in_context(
            &discovered_shell_exec_turn(
                "root-session",
                "turn-shell-discovered",
                "call-shell-discovered",
            ),
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;

    let approval_request_id = match result {
        TurnResult::NeedsApproval(requirement) => {
            assert_eq!(requirement.tool_name.as_deref(), Some("bash"));
            assert_eq!(requirement.approval_key.as_deref(), Some("tool:bash"));
            requirement
                .approval_request_id
                .expect("approval request id should be present")
        }
        other @ TurnResult::FinalText(_)
        | other @ TurnResult::StreamingText(_)
        | other @ TurnResult::StreamingDone(_)
        | other @ TurnResult::ToolDenied(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_) => {
            panic!("expected NeedsApproval, got {other:?}")
        }
    };

    let stored = repo
        .load_approval_request(&approval_request_id)
        .expect("load approval request")
        .expect("approval request row");
    assert_eq!(stored.status, ApprovalRequestStatus::Pending);
    assert_eq!(stored.tool_name, "bash");
    assert_eq!(stored.turn_id, "turn-shell-discovered");
    assert_eq!(stored.tool_call_id, "call-shell-discovered");
    assert_eq!(stored.approval_key, "tool:bash");
    assert_eq!(stored.request_payload_json["tool_name"], "bash");
    assert_eq!(stored.request_payload_json["execution_kind"], "core");
    assert_eq!(
        stored.request_payload_json["args_json"],
        json!({
            "command": "cargo",
            "args": ["--version"]
        })
    );
}

#[tokio::test]
async fn governed_tool_approval_request_reuses_deterministic_id_for_same_blocked_call() {
    let memory_config = isolated_memory_config("reuse-shell");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");

    let mut tool_config = ToolConfig::default();
    tool_config.approval.mode = GovernedToolApprovalMode::Strict;
    tool_config.consent.default_mode = ToolConsentMode::Prompt;

    let tool_view = runtime_tool_view_for_config(&tool_config);
    let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
    let kernel_ctx = bootstrap_test_kernel_context("turn-engine-governed-shell-reuse", 60)
        .expect("kernel context");
    let turn = discovered_shell_exec_turn("root-session", "turn-reuse", "call-reuse");

    let first = TurnEngine::new(4)
        .execute_turn_in_context(
            &turn,
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;
    let second = TurnEngine::new(4)
        .execute_turn_in_context(
            &turn,
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;

    let first_request_id = match first {
        TurnResult::NeedsApproval(requirement) => requirement
            .approval_request_id
            .expect("first approval request id"),
        other @ TurnResult::FinalText(_)
        | other @ TurnResult::StreamingText(_)
        | other @ TurnResult::StreamingDone(_)
        | other @ TurnResult::ToolDenied(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_) => {
            panic!("expected first NeedsApproval, got {other:?}")
        }
    };
    let second_request_id = match second {
        TurnResult::NeedsApproval(requirement) => requirement
            .approval_request_id
            .expect("second approval request id"),
        other @ TurnResult::FinalText(_)
        | other @ TurnResult::StreamingText(_)
        | other @ TurnResult::StreamingDone(_)
        | other @ TurnResult::ToolDenied(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_) => {
            panic!("expected second NeedsApproval, got {other:?}")
        }
    };

    assert_eq!(first_request_id, second_request_id);

    let requests = repo
        .list_approval_requests_for_session("root-session", None)
        .expect("list approval requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].approval_request_id, first_request_id);
}

#[tokio::test]
async fn autonomy_policy_allowlist_does_not_bypass_prompt_session_consent() {
    let memory_config = isolated_memory_config("autonomy-allowlist-prompt");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");

    let mut tool_config = ToolConfig {
        autonomy_profile: AutonomyProfile::GuidedAcquisition,
        ..ToolConfig::default()
    };
    tool_config.consent.default_mode = ToolConsentMode::Prompt;
    let approved_calls = &mut tool_config.approval.approved_calls;
    approved_calls.push("tool:skills.policy".to_owned());

    let tool_view = runtime_tool_view_for_config(&tool_config);
    let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
    let kernel_ctx = kernel_context("turn-engine-autonomy-allowlist-prompt");
    let turn = skills_policy_get_turn(
        "root-session",
        "turn-autonomy-allowlist-prompt",
        "call-autonomy-allowlist-prompt",
    );

    let result = TurnEngine::new(4)
        .execute_turn_in_context(
            &turn,
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;

    let TurnResult::ToolDenied(failure) = result else {
        panic!("expected ToolDenied, got {result:?}");
    };
    assert_eq!(failure.code, "tool_not_found");
    assert!(failure.reason.contains("skills.policy"));

    let requests = repo
        .list_approval_requests_for_session("root-session", None)
        .expect("list approval requests");
    assert!(requests.is_empty());
}

#[tokio::test]
async fn autonomy_policy_grant_does_not_bypass_prompt_session_consent() {
    let memory_config = isolated_memory_config("autonomy-grant-prompt");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");
    repo.upsert_approval_grant(NewApprovalGrantRecord {
        scope_session_id: "root-session".to_owned(),
        approval_key: "tool:skills.policy".to_owned(),
        created_by_session_id: Some("root-session".to_owned()),
    })
    .expect("persist approval grant");

    let mut tool_config = ToolConfig {
        autonomy_profile: AutonomyProfile::GuidedAcquisition,
        ..ToolConfig::default()
    };
    tool_config.consent.default_mode = ToolConsentMode::Prompt;

    let tool_view = runtime_tool_view_for_config(&tool_config);
    let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
    let kernel_ctx = kernel_context("turn-engine-autonomy-grant-prompt");
    let turn = skills_policy_get_turn(
        "root-session",
        "turn-autonomy-grant-prompt",
        "call-autonomy-grant-prompt",
    );

    let result = TurnEngine::new(4)
        .execute_turn_in_context(
            &turn,
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;

    let TurnResult::ToolDenied(failure) = result else {
        panic!("expected ToolDenied, got {result:?}");
    };
    assert_eq!(failure.code, "tool_not_found");
    assert!(failure.reason.contains("skills.policy"));

    let requests = repo
        .list_approval_requests_for_session("root-session", None)
        .expect("list approval requests");
    assert!(requests.is_empty());
}

#[tokio::test]
async fn governed_tool_predenied_reason_omits_internal_prefix() {
    let failure = TurnFailure {
        kind: TurnFailureKind::PolicyDenied,
        code: "app_tool_denied".to_owned(),
        reason: "app_tool_denied: tool:browse.click".to_owned(),
        retryable: false,
        supports_discovery_recovery: false,
    };
    let rendered = super::render_app_tool_denied_reason(&failure.reason);
    assert_eq!(failure.code, "app_tool_denied");
    assert_eq!(rendered, "tool:browse.click");
}
#[tokio::test]
async fn observed_fast_lane_execution_trace_records_batch_and_segment_metrics() {
    let turn = fast_lane_observed_execution_turn(
        "session-observed-fast-lane",
        "turn-observed-fast-lane",
        "call-observed-fast-lane",
    );
    let session_context =
        SessionContext::root_with_tool_view("session-observed-fast-lane", runtime_tool_view());
    let dispatcher = DelayedObservedExecutionDispatcher;
    let engine = TurnEngine::with_parallel_tool_execution(8, 512, true, 2);

    let (result, trace) = engine
        .execute_turn_in_context_with_trace(
            &turn,
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::direct(),
            None,
            None,
        )
        .await;

    assert!(
        matches!(result, TurnResult::FinalText(_)),
        "expected FinalText, got {result:?}"
    );

    let trace = trace.expect("trace should exist");
    assert_eq!(trace.total_intents, 5);
    assert!(trace.parallel_execution_enabled);
    assert_eq!(trace.parallel_execution_max_in_flight, 2);
    assert_eq!(trace.observed_peak_in_flight, 2);
    assert!(
        trace.observed_wall_time_ms >= 40,
        "expected batch wall time to reflect execution, got {}",
        trace.observed_wall_time_ms
    );
    assert_eq!(trace.segments.len(), 3);
    assert_eq!(
        trace.segments[0].execution_mode,
        ToolBatchExecutionMode::Parallel
    );
    assert_eq!(trace.segments[0].observed_peak_in_flight, Some(2));
    assert!(
        trace.segments[0]
            .observed_wall_time_ms
            .expect("parallel segment wall time")
            >= 20
    );
    assert_eq!(
        trace.segments[1].execution_mode,
        ToolBatchExecutionMode::Sequential
    );
    assert_eq!(trace.segments[1].observed_peak_in_flight, Some(1));
    assert_eq!(
        trace.segments[2].execution_mode,
        ToolBatchExecutionMode::Parallel
    );
    assert_eq!(trace.segments[2].observed_peak_in_flight, Some(2));
}

#[tokio::test]
async fn parallel_execution_reports_global_intent_sequence_to_after_tool_execution() {
    let turn = fast_lane_observed_execution_turn(
        "session-observed-sequence",
        "turn-observed-sequence",
        "call-observed-sequence",
    );
    let session_context =
        SessionContext::root_with_tool_view("session-observed-sequence", runtime_tool_view());
    let after_calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let dispatcher = AfterExecutionSequenceRecordingDispatcher {
        after_calls: std::sync::Arc::clone(&after_calls),
    };
    let engine = TurnEngine::with_parallel_tool_execution(8, 512, true, 2);

    let (result, _trace) = engine
        .execute_turn_in_context_with_trace(
            &turn,
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::direct(),
            None,
            None,
        )
        .await;

    assert!(
        matches!(result, TurnResult::FinalText(_)),
        "expected FinalText, got {result:?}"
    );

    let after_calls = after_calls.lock().expect("after call lock");
    let after_call_map = after_calls
        .iter()
        .cloned()
        .collect::<std::collections::BTreeMap<String, usize>>();

    assert_eq!(after_call_map.len(), 5);
    assert_eq!(after_call_map.get("call-observed-sequence-1"), Some(&0));
    assert_eq!(after_call_map.get("call-observed-sequence-2"), Some(&1));
    assert_eq!(after_call_map.get("call-observed-sequence-3"), Some(&2));
    assert_eq!(after_call_map.get("call-observed-sequence-4"), Some(&3));
    assert_eq!(after_call_map.get("call-observed-sequence-5"), Some(&4));
}

#[tokio::test]
async fn observed_fast_lane_execution_treats_single_in_flight_batches_as_sequential() {
    let turn = fast_lane_observed_execution_turn(
        "session-observed-fast-lane-single",
        "turn-observed-fast-lane-single",
        "call-observed-fast-lane-single",
    );
    let session_context = SessionContext::root_with_tool_view(
        "session-observed-fast-lane-single",
        runtime_tool_view(),
    );
    let dispatcher = DelayedObservedExecutionDispatcher;
    let engine = TurnEngine::with_parallel_tool_execution(8, 512, true, 1);

    let (_result, trace) = engine
        .execute_turn_in_context_with_trace(
            &turn,
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::direct(),
            None,
            None,
        )
        .await;

    let trace = trace.expect("trace should exist");
    assert_eq!(trace.parallel_execution_max_in_flight, 1);
    assert_eq!(
        trace
            .segments
            .iter()
            .filter(|segment| segment.execution_mode == ToolBatchExecutionMode::Parallel)
            .count(),
        0
    );
    assert!(
        trace
            .segments
            .iter()
            .all(|segment| segment.execution_mode == ToolBatchExecutionMode::Sequential)
    );
}

#[tokio::test]
async fn parallel_execution_records_trace_items_in_intent_order() {
    let turn = ProviderTurn {
        assistant_text: "observing ordered trace records".to_owned(),
        tool_intents: vec![
            provider_app_tool_intent(
                "sessions_list",
                json!({"delay_ms": 25}),
                "session-observed-trace-order",
                "turn-observed-trace-order",
                "call-observed-trace-order-1",
            ),
            provider_app_tool_intent(
                "sessions_list",
                json!({"delay_ms": 5}),
                "session-observed-trace-order",
                "turn-observed-trace-order",
                "call-observed-trace-order-2",
            ),
        ],
        raw_meta: json!({}),
    };
    let session_context =
        SessionContext::root_with_tool_view("session-observed-trace-order", runtime_tool_view());
    let dispatcher = DelayedObservedExecutionDispatcher;
    let engine = TurnEngine::with_parallel_tool_execution(8, 512, true, 2);

    let (result, trace) = engine
        .execute_turn_in_context_with_trace(
            &turn,
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::direct(),
            None,
            None,
        )
        .await;

    assert!(
        matches!(result, TurnResult::FinalText(_)),
        "expected FinalText, got {result:?}"
    );

    let trace = trace.expect("trace should exist");
    let intent_outcome_ids = trace
        .intent_outcomes
        .iter()
        .map(|intent_outcome| intent_outcome.tool_call_id.as_str())
        .collect::<Vec<_>>();
    let outcome_record_ids = trace
        .outcome_records
        .iter()
        .map(|outcome_record| outcome_record.tool_call_id.as_str())
        .collect::<Vec<_>>();
    let expected_ids = vec!["call-observed-trace-order-1", "call-observed-trace-order-2"];

    assert_eq!(intent_outcome_ids, expected_ids);
    assert_eq!(outcome_record_ids, expected_ids);
}

#[tokio::test]
async fn observed_fast_lane_execution_trace_records_partial_tool_failure_outcomes() {
    let turn = partially_failing_observed_execution_turn(
        "session-observed-partial-failure",
        "turn-observed-partial-failure",
    );
    let session_context = SessionContext::root_with_tool_view(
        "session-observed-partial-failure",
        runtime_tool_view(),
    );
    let dispatcher = PartiallyFailingObservedExecutionDispatcher;
    let engine = TurnEngine::with_parallel_tool_execution(4, 512, false, 1);

    let (result, trace) = engine
        .execute_turn_in_context_with_trace(
            &turn,
            &session_context,
            &dispatcher,
            ConversationRuntimeBinding::direct(),
            None,
            None,
        )
        .await;

    assert!(
        matches!(result, TurnResult::ToolError(_)),
        "expected ToolError, got {result:?}"
    );

    let trace = trace.expect("trace should exist");
    assert_eq!(trace.intent_outcomes.len(), 2);
    assert_eq!(
        trace.intent_outcomes[0].status,
        ToolBatchExecutionIntentStatus::Completed
    );
    assert_eq!(trace.intent_outcomes[0].tool_call_id, "call-partial-1");
    assert_eq!(
        trace.intent_outcomes[1].status,
        ToolBatchExecutionIntentStatus::Failed
    );
    assert_eq!(trace.intent_outcomes[1].tool_call_id, "call-partial-2");
    assert!(
        trace.intent_outcomes[1]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("simulated observed tool failure")),
        "expected failure detail in trace, got {:?}",
        trace.intent_outcomes[1].detail
    );
}

#[test]
fn success_outcome_trace_record_bounds_large_payloads() {
    let intent = provider_app_tool_intent(
        "file.read",
        json!({"path": "note.md"}),
        "session-bounded-payload",
        "turn-bounded-payload",
        "call-bounded-payload",
    );
    let large_payload = json!({
        "contents": "x".repeat(TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS + 128),
    });
    let outcome = ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: large_payload,
    };

    let record = build_success_tool_outcome_trace_record(&intent, &outcome);

    assert_eq!(record.outcome.tool_name, "read");
    assert_eq!(record.outcome.status, "ok");
    assert_eq!(record.turn_id, "turn-bounded-payload");
    assert_eq!(record.tool_call_id, "call-bounded-payload");
    assert_eq!(record.outcome.payload["payload_truncated"], json!(true));
    let payload_summary = record.outcome.payload["payload_summary"]
        .as_str()
        .expect("expected truncated payload summary");
    let payload_chars = record.outcome.payload["payload_chars"]
        .as_u64()
        .expect("expected original payload char count");
    assert!(
        payload_summary.len() < payload_chars as usize,
        "expected bounded payload summary, got {:?}",
        record.outcome.payload
    );
    assert!(
        payload_chars > TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS as u64,
        "expected original payload char count, got {:?}",
        record.outcome.payload
    );
}

#[test]
fn continuation_payload_summary_is_compacted_before_low_limit_truncation() {
    let intent = provider_app_tool_intent(
        "session_wait",
        json!({"session_id": "child-session"}),
        "session-continuation-payload",
        "turn-continuation-payload",
        "call-continuation-payload",
    );
    let outcome = ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "session": {
                "session_id": "child-session",
                "state": "running",
                "label": "Child",
                "kind": "delegate_child",
                "details": "x".repeat(512),
            },
            "wait_status": "waiting",
            "events": [
                {
                    "event_kind": "delegate_result",
                    "payload": "x".repeat(512),
                }
            ],
            "continuation": {
                "state": "waiting",
                "is_terminal": false,
                "recommended_tool": "session_wait",
                "recommended_payload": {
                    "session_id": "child-session",
                    "timeout_ms": 1000,
                },
                "note": "Keep waiting before presenting final completion.",
            }
        }),
    };

    let envelope = result::build_tool_result_envelope(&intent, &outcome, 256);
    let payload_summary =
        serde_json::from_str::<Value>(envelope.payload_summary.as_str()).expect("payload json");

    assert!(!envelope.payload_truncated, "envelope: {envelope:?}");
    assert_eq!(payload_summary["continuation"]["state"], "waiting");
    assert_eq!(
        payload_summary["continuation"]["recommended_tool"],
        "session_wait"
    );
    assert!(
        payload_summary.as_object().is_some_and(|object| {
            object.contains_key("continuation") && !object.contains_key("events")
        }),
        "compacted summary should keep only compact continuation-safe fields: {payload_summary:?}"
    );
}

#[test]
fn augment_tool_payload_injects_browser_scope_for_browse_request() {
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::ToolView::from_tool_names(["browse"]),
    );
    let augmented = augment_tool_payload_for_kernel(
        "browser.open",
        json!({
            "url": "https://example.com"
        }),
        &session_context,
        &SessionStoreConfig::default(),
    );

    assert_eq!(
        augmented.payload[crate::tools::BROWSER_SESSION_SCOPE_FIELD],
        "root-session"
    );
}

#[test]
fn augment_tool_payload_uses_active_skill_root_for_absolute_direct_read_targets() {
    let workspace_root = crate::test_support::unique_temp_dir("turn-engine-active-skill-workspace");
    let skill_root = workspace_root.join(".loong/skills/demo-skill");
    std::fs::create_dir_all(skill_root.join("references")).expect("create skill root");
    let reference_path = skill_root.join("references/guide.md");
    std::fs::write(&reference_path, "# Guide\n").expect("write guide");
    let canonical_skill_root = std::fs::canonicalize(&skill_root).expect("canonical skill root");

    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::ToolView::from_tool_names(["read"]),
    )
    .with_workspace_root(workspace_root)
    .with_active_skill_roots(vec![skill_root]);
    let payload = json!({
        "path": reference_path.display().to_string(),
    });

    let augmented = augment_tool_payload_for_kernel(
        "read",
        payload,
        &session_context,
        &SessionStoreConfig::default(),
    );

    assert_eq!(
        augmented.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]
            [crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY],
        json!(canonical_skill_root.display().to_string())
    );
}

#[test]
fn augment_tool_payload_uses_visible_skill_root_for_absolute_skill_direct_reads() {
    let workspace_root =
        crate::test_support::unique_temp_dir("turn-engine-visible-skill-workspace");
    let skill_root = workspace_root.join(".loong/skills/demo-skill");
    std::fs::create_dir_all(&skill_root).expect("create skill root");
    let skill_path = skill_root.join("SKILL.md");
    std::fs::write(&skill_path, "# Demo Skill\n").expect("write skill file");
    let canonical_skill_root = std::fs::canonicalize(&skill_root).expect("canonical skill root");

    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::ToolView::from_tool_names(["read"]),
    )
    .with_workspace_root(workspace_root)
    .with_visible_skill_roots(vec![skill_root]);
    let payload = json!({
        "path": skill_path.display().to_string(),
    });

    let augmented = augment_tool_payload_for_kernel(
        "read",
        payload,
        &session_context,
        &SessionStoreConfig::default(),
    );

    assert_eq!(
        augmented.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]
            [crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY],
        json!(canonical_skill_root.display().to_string())
    );
}

#[test]
fn augment_tool_payload_uses_visible_skill_root_for_absolute_skill_resource_direct_reads() {
    let workspace_root =
        crate::test_support::unique_temp_dir("turn-engine-visible-skill-resource-workspace");
    let skill_root = workspace_root.join(".loong/skills/demo-skill");
    std::fs::create_dir_all(skill_root.join("references")).expect("create skill root");
    let reference_path = skill_root.join("references/guide.md");
    std::fs::write(&reference_path, "# Guide\n").expect("write guide");
    let canonical_skill_root = std::fs::canonicalize(&skill_root).expect("canonical skill root");

    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::ToolView::from_tool_names(["read"]),
    )
    .with_workspace_root(workspace_root)
    .with_visible_skill_roots(vec![skill_root]);
    let payload = json!({
        "path": reference_path.display().to_string(),
    });

    let augmented = augment_tool_payload_for_kernel(
        "read",
        payload,
        &session_context,
        &SessionStoreConfig::default(),
    );

    assert_eq!(
        augmented.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]
            [crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY],
        json!(canonical_skill_root.display().to_string())
    );
}

#[test]
fn augment_tool_payload_uses_unique_active_skill_root_for_relative_direct_read_targets() {
    let workspace_root =
        crate::test_support::unique_temp_dir("turn-engine-active-skill-relative-workspace");
    let first_skill_root = workspace_root.join(".loong/skills/demo-skill");
    let second_skill_root = workspace_root.join(".loong/skills/other-skill");
    std::fs::create_dir_all(first_skill_root.join("references")).expect("create first skill");
    std::fs::create_dir_all(second_skill_root.join("references")).expect("create second skill");
    std::fs::write(first_skill_root.join("references/guide.md"), "# First\n")
        .expect("write first guide");
    std::fs::write(second_skill_root.join("references/other.md"), "# Second\n")
        .expect("write second guide");
    let canonical_first_skill_root =
        std::fs::canonicalize(&first_skill_root).expect("canonical first skill root");

    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::ToolView::from_tool_names(["read"]),
    )
    .with_workspace_root(workspace_root)
    .with_active_skill_roots(vec![first_skill_root, second_skill_root]);
    let payload = json!({
        "path": "references/guide.md",
    });

    let augmented = augment_tool_payload_for_kernel(
        "read",
        payload,
        &session_context,
        &SessionStoreConfig::default(),
    );

    assert_eq!(
        augmented.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]
            [crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY],
        json!(canonical_first_skill_root.display().to_string())
    );
}

#[test]
fn augment_tool_payload_does_not_guess_when_relative_direct_read_matches_multiple_skill_roots() {
    let workspace_root =
        crate::test_support::unique_temp_dir("turn-engine-active-skill-relative-ambiguous");
    let first_skill_root = workspace_root.join(".loong/skills/demo-skill");
    let second_skill_root = workspace_root.join(".loong/skills/other-skill");
    std::fs::create_dir_all(first_skill_root.join("references")).expect("create first skill");
    std::fs::create_dir_all(second_skill_root.join("references")).expect("create second skill");
    std::fs::write(first_skill_root.join("references/shared.md"), "# First\n")
        .expect("write first guide");
    std::fs::write(second_skill_root.join("references/shared.md"), "# Second\n")
        .expect("write second guide");
    let expected_workspace_root = workspace_root.display().to_string();

    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::ToolView::from_tool_names(["read"]),
    )
    .with_workspace_root(workspace_root)
    .with_active_skill_roots(vec![first_skill_root, second_skill_root]);
    let payload = json!({
        "path": "references/shared.md",
    });

    let augmented = augment_tool_payload_for_kernel(
        "read",
        payload,
        &session_context,
        &SessionStoreConfig::default(),
    );

    assert_eq!(
        augmented.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]
            [crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY],
        json!(expected_workspace_root)
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn augment_tool_payload_injects_canonical_task_id_for_task_tools() {
    let memory_config = isolated_memory_config("task-tool-scope");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: SessionState::Running,
    })
    .expect("create session");
    repo.append_event(NewSessionEvent {
        session_id: "root-session".to_owned(),
        event_kind: TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Waiting,
                intent_summary: None,
                verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                active_handles: Vec::new(),
                resume_recipe: None,
                updated_at: 123,
            },
        ),
    })
    .expect("append task progress");

    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::ToolView::from_tool_names(["task_wait"]),
    );

    let augmented =
        augment_tool_payload_for_kernel("task_wait", json!({}), &session_context, &memory_config);

    assert_eq!(augmented.payload["task_id"], "task-root");
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn augment_tool_payload_injects_canonical_task_id_for_task_events() {
    let memory_config = isolated_memory_config("task-events-tool-scope");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: SessionState::Running,
    })
    .expect("create session");
    repo.append_event(NewSessionEvent {
        session_id: "root-session".to_owned(),
        event_kind: TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Waiting,
                intent_summary: None,
                verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                active_handles: Vec::new(),
                resume_recipe: None,
                updated_at: 123,
            },
        ),
    })
    .expect("append task progress");

    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::ToolView::from_tool_names(["task_events"]),
    );

    let augmented =
        augment_tool_payload_for_kernel("task_events", json!({}), &session_context, &memory_config);

    assert_eq!(augmented.payload["task_id"], "task-root");
}
