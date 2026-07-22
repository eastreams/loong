use std::fs;
use std::sync::Arc;
use std::time::Duration;

use loong_contracts::ToolPath;
use serde_json::json;

use super::prepare::{
    PreparedLegacyToolInvocation, PreparedToolInvocation, PreparedTypedToolInvocation,
};
use super::*;
use crate::config::{AutonomyProfile, GovernedToolApprovalMode, ToolConfig};
use crate::session::repository::{
    ApprovalRequestStatus, NewApprovalGrantRecord, NewSessionEvent, NewSessionRecord, SessionKind,
    SessionRepository, SessionState,
};
use crate::tools::{runtime_tool_view, runtime_tool_view_for_config};

// Contracts tests validation; turn-engine fixtures use valid one-segment
// identities so these tests stay focused on routing and execution semantics.
#[allow(clippy::expect_used)]
fn tool_path(segment: &str) -> ToolPath {
    ToolPath::new([segment]).expect("test tool path must be valid")
}

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

struct TypedOnlyPrepareTool;

#[async_trait::async_trait]
impl<C> loong_core::tool::ToolImpl<C> for TypedOnlyPrepareTool
where
    C: loong_core::policy::context::ContextFactory,
{
    type Input = serde_json::Value;
    type Output = serde_json::Value;
    type Error = std::convert::Infallible;

    fn spec(&self) -> loong_contracts::ToolSpec {
        loong_contracts::ToolSpec {
            description: "Typed-only preparation test tool.".to_owned(),
            input_schema: json!({ "type": "object" }),
            required_capabilities: std::collections::BTreeSet::new(),
            scheduling: loong_contracts::ToolSchedulingClass::ParallelSafe,
            argument_hint: None,
            search_hint: None,
            tags: Vec::new(),
        }
    }

    fn parse_input(
        &self,
        payload: serde_json::Value,
    ) -> Result<Self::Input, loong_contracts::ToolInputError> {
        Ok(payload)
    }

    async fn execute(
        &self,
        _ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        Ok(input)
    }
}

struct TypedIngressTestOwner {
    runtime: std::sync::Arc<loong_runtime::runtime::Runtime<crate::context::RuntimeContextFactory>>,
    session: crate::Session,
}

impl TypedIngressTestOwner {
    fn context(&self) -> crate::Context<'_> {
        crate::Context::new(&self.runtime, &self.session).expect("typed ingress owner")
    }
}

/// Build typed ingress without registering pack or minting bearer evidence.
/// A test that reaches legacy dispatch must supply that owner separately.
fn typed_ingress_test_runtime_session(
    tools: loong_runtime::tool_plane::ToolPlaneRegistry<crate::context::RuntimeContextFactory>,
    tool_view: crate::tools::ToolView,
) -> (
    TypedIngressTestOwner,
    std::sync::Arc<loong_kernel::InMemoryAuditSink>,
) {
    use std::sync::Arc;

    use loong_contracts::{Capabilities, Capability, GovernedSessionMode};
    use loong_kernel::{FixedClock, InMemoryAuditSink, Kernel};
    use loong_runtime::runtime::Runtime;

    let audit = Arc::new(InMemoryAuditSink::default());
    let policy =
        loong_kernel::policy::PolicyPipelineBuilder::<crate::context::RuntimeContextFactory>::new()
            .with_pre_policy(crate::tools::plane::ToolVisibilityPolicy)
            .with_policy(crate::tools::plane::ToolInvocationAllowPolicy);
    let kernel = Kernel::<crate::context::RuntimeContextFactory>::with_policy_runtime(
        policy,
        Arc::new(FixedClock::new(1_700_000_000)),
        audit.clone(),
    );
    let runtime = Arc::new(Runtime::new(kernel, tools));
    let session = crate::context::Session::root(
        runtime.as_ref(),
        "typed-ingress-agent",
        "typed-ingress-session",
        GovernedSessionMode::MutatingCapable,
        Capabilities::from([Capability::InvokeTool]),
        crate::tools::runtime_config::ToolRuntimeConfig::default(),
        crate::memory::runtime_config::MemoryRuntimeConfig::default(),
        tool_view,
        None,
        None,
    )
    .expect("build typed ingress test session");
    let owner = TypedIngressTestOwner { runtime, session };
    (owner, audit)
}

#[cfg(feature = "tool-file")]
async fn assert_migrated_tool_has_no_legacy_fallback(
    tool_name: &'static str,
    payload: serde_json::Value,
) {
    let (owner, _) = typed_ingress_test_runtime_session(
        loong_runtime::tool_plane::ToolPlaneRegistry::new(),
        crate::tools::ToolView::from_legacy_paths([tool_name]),
    );
    let session_context = owner.context();
    let intent = ToolIntent {
        tool_name: tool_name.into(),
        args_json: payload,
        source: "assistant".to_owned(),
        turn_id: format!("missing-{tool_name}-turn"),
        tool_call_id: format!("missing-{tool_name}-call"),
    };

    let failure = TurnEngine::new(1)
        .prepare_tool_intent(
            &intent,
            0,
            &session_context,
            &NoopLegacyToolDispatcher,
            &AutonomyTurnBudgetState::default(),
            None,
        )
        .await
        .expect_err("migrated typed tool must not fall back when registration is absent");

    assert!(matches!(
        failure.turn_result,
        TurnResult::ToolDenied(ref error)
            if error.code == "tool_not_found" && error.reason.contains(tool_name)
    ));
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
    let preflight = LegacyToolExecutionPreflight::ready(ToolCoreRequest {
        tool_name: "shell.exec".to_owned(),
        payload: json!({"command": "echo hello"}),
    });

    match preflight {
        LegacyToolExecutionPreflight::Ready {
            request,
            trusted_internal_context,
        } => {
            assert_eq!(request.tool_name, "shell.exec");
            assert_eq!(request.payload, json!({"command": "echo hello"}));
            assert!(!trusted_internal_context);
        }
        LegacyToolExecutionPreflight::NeedsApproval(requirement) => {
            panic!("unexpected approval requirement: {:?}", requirement)
        }
    }
}

#[test]
fn default_legacy_tool_dispatcher_scopes_child_sessions_to_self_only_visibility() {
    let root_owner =
        crate::test_support::runtime_session_for_test("root-session", runtime_tool_view());
    let dispatcher = DefaultLegacyToolDispatcher::new(
        Arc::clone(&root_owner.runtime),
        &root_owner.session,
        store::current_session_store_config().clone(),
        ToolConfig::default(),
    )
    .expect("legacy app tool dispatcher");
    let child_owner = crate::test_support::child_runtime_session_for_test(
        "child-session",
        "root-session",
        runtime_tool_view(),
    );
    let root = root_owner.context();
    let child = child_owner.context();

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
fn prepare_tool_intent_uses_direct_shell_metadata_for_provider_shell_requests() {
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call(
        "shell.exec",
        json!({
            "command": "echo",
            "args": ["hello"],
        }),
    );
    let intent = ToolIntent {
        tool_name: tool_name.into(),
        args_json,
        source: "provider_tool_call".to_owned(),
        turn_id: "turn-shell-invoke-trace".to_owned(),
        tool_call_id: "call-shell-invoke-trace".to_owned(),
    };
    let owner = crate::test_support::runtime_session_for_test(
        "session-shell-invoke-trace",
        runtime_tool_view(),
    );
    let session_context = owner.context();
    let engine = TurnEngine::new(4);
    let runtime = tokio::runtime::Runtime::new().expect("test runtime");
    let prepared_intent = runtime.block_on(async {
        let autonomy_budget_state = AutonomyTurnBudgetState::default();
        engine
            .prepare_tool_intent(
                &intent,
                0,
                &session_context,
                &owner.legacy_tools,
                &autonomy_budget_state,
                None,
            )
            .await
            .expect("provider shell request should prepare successfully")
    });

    let PreparedToolInvocation::Legacy {
        invocation: PreparedLegacyToolInvocation::Core { request, .. },
        ..
    } = &prepared_intent.invocation
    else {
        panic!("shell request should remain in the legacy core fallback");
    };
    assert_eq!(request.tool_name, "shell.exec");
    assert_eq!(prepared_intent.intent.tool_name.name(), "shell.exec");
    assert_eq!(
        prepared_intent.intent.args_json,
        json!({
            "command": "echo",
            "args": ["hello"],
        })
    );
}

#[tokio::test]
async fn typed_only_registration_executes_without_a_legacy_catalog_row() {
    use loong_contracts::ToolSchedulingClass;
    use loong_runtime::tool_plane::{ToolPlaneRegistry, ToolRegistration};

    let mut tools = ToolPlaneRegistry::new();
    tools
        .register(
            tool_path("typed.only"),
            ToolRegistration::direct("typed_only"),
            TypedOnlyPrepareTool,
        )
        .expect("register typed-only tool");
    let (owner, audit) = typed_ingress_test_runtime_session(
        tools,
        crate::tools::ToolView::from_legacy_paths(["typed.only"]),
    );
    let session_context = owner.context();
    let intent = ToolIntent {
        tool_name: ToolIntentTarget::registered(tool_path("typed.only"), "typed_only"),
        args_json: json!({}),
        source: "provider_tool_call".to_owned(),
        turn_id: "typed-only-turn".to_owned(),
        tool_call_id: "typed-only-call".to_owned(),
    };

    let prepared = TurnEngine::new(1)
        .prepare_tool_intent(
            &intent,
            0,
            &session_context,
            &NoopLegacyToolDispatcher,
            &AutonomyTurnBudgetState::default(),
            None,
        )
        .await
        .expect("typed-only registration should not require a legacy descriptor");

    let PreparedToolInvocation::Typed(PreparedTypedToolInvocation {
        invocation,
        payload,
    }) = &prepared.invocation
    else {
        panic!("typed registration should prepare a typed invocation");
    };
    assert_eq!(invocation.path().to_string(), "/typed.only");
    assert_eq!(prepared.intent.tool_name.name(), "typed_only");
    assert_eq!(
        prepared.intent.tool_name.registered_path(),
        Some(&tool_path("typed.only"))
    );
    assert_eq!(payload, &json!({}));
    assert_eq!(prepared.scheduling_class, ToolSchedulingClass::ParallelSafe);

    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![intent],
        raw_meta: serde_json::Value::Null,
    };
    let (result, trace) = TurnEngine::new(1)
        .execute_turn_in_context_with_trace(
            &turn,
            &session_context,
            &NoopLegacyToolDispatcher,
            None,
            None,
        )
        .await;

    assert!(matches!(
        result,
        TurnResult::FinalText(ref output)
            if output.contains("typed-only-call")
                && output.contains("\"tool\":\"typed_only\"")
                && !output.contains("\"tool\":\"typed.only\"")
    ));
    let trace = trace.expect("typed execution trace");
    assert!(
        trace.decision_records.is_empty(),
        "typed authorization must come from PolicyEngine evidence, not a synthetic preflight allow"
    );
    assert_eq!(trace.intent_outcomes.len(), 1);
    assert_eq!(trace.intent_outcomes[0].tool_name, "typed_only");
    let events = audit.snapshot();
    assert!(
        events.iter().any(|event| matches!(
            event.kind,
            loong_contracts::AuditEventKind::ActionExecution { .. }
        )),
        "typed execution audit: {events:?}"
    );
}

#[tokio::test]
async fn typed_registration_precedes_same_name_legacy_catalog_validation() {
    use loong_runtime::tool_plane::{ToolPlaneRegistry, ToolRegistration};

    let mut tools = ToolPlaneRegistry::new();
    tools
        .register(
            tool_path("config.import"),
            ToolRegistration::direct("config.import"),
            TypedOnlyPrepareTool,
        )
        .expect("register typed owner for a legacy catalog name");
    let (owner, _) = typed_ingress_test_runtime_session(
        tools,
        crate::tools::ToolView::from_legacy_paths(["config.import"]),
    );
    let context = owner.context();
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: ToolIntentTarget::Unresolved {
                name: "config.import".to_owned(),
            },
            args_json: json!({ "typed_owner": true }),
            source: "provider_tool_call".to_owned(),
            turn_id: "typed-owner-before-legacy-turn".to_owned(),
            tool_call_id: "typed-owner-before-legacy-call".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };

    let result = TurnEngine::new(1)
        .execute_turn_in_context(&turn, &context, &NoopLegacyToolDispatcher, None)
        .await;

    assert!(matches!(
        result,
        TurnResult::FinalText(ref output)
            if output.contains("typed-owner-before-legacy-call")
                && output.contains("typed_owner")
    ));
}

#[tokio::test]
async fn registered_target_execution_never_reparses_its_provider_name() {
    use loong_runtime::tool_plane::{ToolPlaneRegistry, ToolRegistration};

    let mut tools = ToolPlaneRegistry::new();
    tools
        .register(
            tool_path("typed.only"),
            ToolRegistration::direct("config.import"),
            TypedOnlyPrepareTool,
        )
        .expect("register typed path with a colliding legacy provider name");
    let (owner, _) = typed_ingress_test_runtime_session(
        tools,
        crate::tools::ToolView::from_legacy_paths(["typed.only"]),
    );
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: ToolIntentTarget::registered(tool_path("typed.only"), "config.import"),
            args_json: json!({}),
            source: "provider_tool_call".to_owned(),
            turn_id: "registered-path-validation-turn".to_owned(),
            tool_call_id: "registered-path-validation-call".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };

    let result = TurnEngine::new(1)
        .execute_turn_in_context(&turn, &owner.context(), &NoopLegacyToolDispatcher, None)
        .await;

    assert!(
        matches!(result, TurnResult::FinalText(ref output) if output.contains("registered-path-validation-call"))
    );
}

#[tokio::test]
async fn typed_execution_does_not_interpret_legacy_runtime_overlay() {
    use loong_runtime::tool_plane::{ToolPlaneRegistry, ToolRegistration};

    let mut tools = ToolPlaneRegistry::new();
    tools
        .register(
            tool_path("typed.only"),
            ToolRegistration::direct("typed.only"),
            TypedOnlyPrepareTool,
        )
        .expect("register typed-only tool");
    let (owner, _) = typed_ingress_test_runtime_session(
        tools,
        crate::tools::ToolView::from_legacy_paths(["typed.only"]),
    );
    let context = owner.context();

    let outcome = context
        .tool(tool_path("typed.only"))
        .expect("typed tool should be registered")
        .invoke(json!({
            "_loong": {},
            "visible_input": true,
        }))
        .await
        .expect("legacy overlay keys are ordinary typed payload data");

    assert_eq!(outcome, json!({ "_loong": {}, "visible_input": true }));
}

#[tokio::test]
async fn registered_tool_invoke_path_precedes_the_legacy_envelope() {
    use loong_runtime::tool_plane::{ToolPlaneRegistry, ToolRegistration};

    let mut tools = ToolPlaneRegistry::new();
    tools
        .register(
            tool_path("tool.invoke"),
            ToolRegistration::direct("tool.invoke"),
            TypedOnlyPrepareTool,
        )
        .expect("register concrete tool.invoke tool");
    let (owner, _) = typed_ingress_test_runtime_session(
        tools,
        crate::tools::ToolView::from_legacy_paths(["tool.invoke"]),
    );
    let context = owner.context();
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "tool.invoke".into(),
            args_json: json!({ "visible_input": true }),
            source: "assistant".to_owned(),
            turn_id: "registered-tool-invoke-turn".to_owned(),
            tool_call_id: "registered-tool-invoke-call".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };

    let result = TurnEngine::new(1)
        .execute_turn_in_context(&turn, &context, &NoopLegacyToolDispatcher, None)
        .await;

    assert!(matches!(
        result,
        TurnResult::FinalText(ref output)
            if output.contains("registered-tool-invoke-call")
                && output.contains("visible_input")
    ));
}

#[tokio::test]
async fn registered_tool_invoke_path_still_requires_visibility() {
    use loong_runtime::tool_plane::{ToolPlaneRegistry, ToolRegistration};

    let mut tools = ToolPlaneRegistry::new();
    tools
        .register(
            tool_path("tool.invoke"),
            ToolRegistration::direct("tool.invoke"),
            TypedOnlyPrepareTool,
        )
        .expect("register concrete tool.invoke tool");
    let (owner, _) = typed_ingress_test_runtime_session(tools, crate::tools::ToolView::default());
    let context = owner.context();
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "tool.invoke".into(),
            args_json: json!({}),
            source: "assistant".to_owned(),
            turn_id: "hidden-tool-invoke-turn".to_owned(),
            tool_call_id: "hidden-tool-invoke-call".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };

    let result = TurnEngine::new(1)
        .execute_turn_in_context(&turn, &context, &NoopLegacyToolDispatcher, None)
        .await;

    let TurnResult::FinalText(output) = result else {
        panic!("expected batch-local typed visibility denial, got {result:?}");
    };
    assert!(output.contains("tool_authorization_denied"), "{output}");
    assert!(output.contains("not visible"), "{output}");
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn read_requires_its_typed_runtime_registration() {
    assert_migrated_tool_has_no_legacy_fallback("read", json!({ "path": "README.md" })).await;
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn write_requires_its_typed_runtime_registration() {
    assert_migrated_tool_has_no_legacy_fallback(
        "write",
        json!({ "path": "notes.txt", "content": "text" }),
    )
    .await;
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn edit_requires_its_typed_runtime_registration() {
    assert_migrated_tool_has_no_legacy_fallback(
        "edit",
        json!({
            "path": "notes.txt",
            "edits": [{ "old_text": "old", "new_text": "new" }],
        }),
    )
    .await;
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn glob_search_requires_its_typed_runtime_registration() {
    assert_migrated_tool_has_no_legacy_fallback("glob.search", json!({ "pattern": "**/*.rs" }))
        .await;
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn content_search_requires_its_typed_runtime_registration() {
    assert_migrated_tool_has_no_legacy_fallback("content.search", json!({ "query": "needle" }))
        .await;
}

#[tokio::test]
async fn turn_validates_lease_before_resolving_an_unknown_target() {
    let owner =
        crate::test_support::runtime_session_for_test("invalid-lease-session", runtime_tool_view());
    let session_context = owner.context();
    let intent = ToolIntent {
        tool_name: "tool.invoke".into(),
        args_json: json!({
            "tool_id": "typed.missing",
            "lease": "invalid-lease",
            "arguments": {},
        }),
        source: "assistant".to_owned(),
        turn_id: "invalid-lease-turn".to_owned(),
        tool_call_id: "invalid-lease-call".to_owned(),
    };

    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![intent],
        raw_meta: serde_json::Value::Null,
    };
    let engine = TurnEngine::new(1);
    let result = engine
        .execute_turn_in_context(&turn, &session_context, &NoopLegacyToolDispatcher, None)
        .await;

    let TurnResult::ToolDenied(failure) = &result else {
        panic!("expected tool denial, got {result:?}")
    };
    assert_eq!(failure.code, "invalid_tool_lease");
    assert!(failure.supports_discovery_recovery);
    assert!(
        !failure.reason.contains("typed.missing"),
        "invalid lease denial must not disclose the unresolved target: {failure:?}"
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
            tool_name: tool_name.into(),
            args_json,
            source: "assistant".to_owned(),
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
            tool_name: tool_name.into(),
            args_json,
            source: "assistant".to_owned(),
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
            tool_name: tool_name.into(),
            args_json,
            source: "assistant".to_owned(),
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
            tool_name: tool_name.into(),
            args_json,
            source: "assistant".to_owned(),
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
        tool_name: tool_name.into(),
        args_json,
        source: "assistant".to_owned(),
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
impl LegacyToolDispatcher for DelayedObservedExecutionDispatcher {
    async fn execute_app_tool(
        &self,
        session_context: &Context<'_>,
        request: ToolCoreRequest,
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
                "session_id": session_context.session().session_id,
            }),
        })
    }
}

struct AfterExecutionSequenceRecordingDispatcher {
    after_calls: std::sync::Arc<std::sync::Mutex<Vec<(String, usize)>>>,
}

#[async_trait::async_trait]
impl LegacyToolDispatcher for AfterExecutionSequenceRecordingDispatcher {
    async fn execute_app_tool(
        &self,
        session_context: &Context<'_>,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, String> {
        if request
            .payload
            .get("deny")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err("app_tool_denied: denied by test policy".to_owned());
        }

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
                "session_id": session_context.session().session_id,
            }),
        })
    }

    async fn after_tool_execution(
        &self,
        _session_context: &Context<'_>,
        intent: &ToolIntent,
        intent_sequence: usize,
        _request: &ToolCoreRequest,
        _outcome: &ToolCoreOutcome,
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
impl LegacyToolDispatcher for PartiallyFailingObservedExecutionDispatcher {
    async fn execute_app_tool(
        &self,
        session_context: &Context<'_>,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, String> {
        match request.tool_name.as_str() {
            "sessions_list" => Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "tool": request.tool_name,
                    "session_id": session_context.session().session_id,
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
    let owner_config = crate::config::LoongConfig {
        tools: tool_config.clone(),
        ..crate::config::LoongConfig::default()
    };
    let owner = crate::test_support::TestRuntimeSession::from_config(
        &owner_config,
        "root-session",
        "test-agent",
        loong_contracts::GovernedSessionMode::MutatingCapable,
    )
    .expect("guided runtime session");
    let session_context = owner.context();
    let dispatcher = DefaultLegacyToolDispatcher::new(
        Arc::clone(&owner.runtime),
        &owner.session,
        memory_config.clone(),
        tool_config,
    )
    .expect("legacy app tool dispatcher");

    let result = TurnEngine::new(4)
        .execute_turn_in_context(
            &delegate_async_turn("root-session", "turn-1", "call-1"),
            &session_context,
            &dispatcher,
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
    let owner_config = crate::config::LoongConfig {
        tools: tool_config.clone(),
        ..crate::config::LoongConfig::default()
    };
    let owner = crate::test_support::TestRuntimeSession::from_config(
        &owner_config,
        "root-session",
        "test-agent",
        loong_contracts::GovernedSessionMode::MutatingCapable,
    )
    .expect("guided runtime session");
    let session_context = owner.context();
    let dispatcher = DefaultLegacyToolDispatcher::new(
        Arc::clone(&owner.runtime),
        &owner.session,
        memory_config.clone(),
        tool_config,
    )
    .expect("legacy app tool dispatcher");

    let result = TurnEngine::new(4)
        .execute_turn_in_context(
            &discovered_delegate_async_turn("root-session", "turn-discovered", "call-discovered"),
            &session_context,
            &dispatcher,
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
    let owner = crate::test_support::runtime_session_for_test("root-session", tool_view);
    let session_context = owner.context();
    let dispatcher = DefaultLegacyToolDispatcher::new(
        Arc::clone(&owner.runtime),
        &owner.session,
        memory_config.clone(),
        tool_config,
    )
    .expect("legacy app tool dispatcher");

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
    assert_eq!(stored.request_payload_json["dispatch_kind"], "legacy_core");
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
    let owner = crate::test_support::runtime_session_for_test("root-session", tool_view);
    let session_context = owner.context();
    let dispatcher = DefaultLegacyToolDispatcher::new(
        Arc::clone(&owner.runtime),
        &owner.session,
        memory_config.clone(),
        tool_config,
    )
    .expect("legacy app tool dispatcher");

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
    let owner_config = crate::config::LoongConfig {
        tools: tool_config.clone(),
        ..crate::config::LoongConfig::default()
    };
    let owner = crate::test_support::TestRuntimeSession::from_config(
        &owner_config,
        "root-session",
        "test-agent",
        loong_contracts::GovernedSessionMode::MutatingCapable,
    )
    .expect("guided runtime session");
    let session_context = owner.context();
    let dispatcher = DefaultLegacyToolDispatcher::new(
        Arc::clone(&owner.runtime),
        &owner.session,
        memory_config.clone(),
        tool_config,
    )
    .expect("legacy app tool dispatcher");
    let turn = delegate_async_turn("root-session", "turn-reuse", "call-reuse");

    let first = TurnEngine::new(4)
        .execute_turn_in_context(&turn, &session_context, &dispatcher, None)
        .await;
    let second = TurnEngine::new(4)
        .execute_turn_in_context(&turn, &session_context, &dispatcher, None)
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
    let owner = crate::test_support::runtime_session_for_test("root-session", tool_view);
    let session_context = owner.context();
    let dispatcher = DefaultLegacyToolDispatcher::new(
        Arc::clone(&owner.runtime),
        &owner.session,
        memory_config.clone(),
        tool_config,
    )
    .expect("legacy app tool dispatcher");
    let turn = skills_policy_get_turn("root-session", "turn-preapproved", "call-preapproved");

    let result = TurnEngine::new(4)
        .execute_turn_in_context(&turn, &session_context, &dispatcher, None)
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
    let owner = crate::test_support::runtime_session_for_test("root-session", tool_view);
    let session_context = owner.context();
    let dispatcher = DefaultLegacyToolDispatcher::new(
        Arc::clone(&owner.runtime),
        &owner.session,
        memory_config.clone(),
        tool_config,
    )
    .expect("legacy app tool dispatcher");
    let turn = skills_policy_get_turn("root-session", "turn-predenied", "call-predenied");

    let result = TurnEngine::new(4)
        .execute_turn_in_context(&turn, &session_context, &dispatcher, None)
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
    let owner = crate::test_support::runtime_session_for_test("root-session", tool_view);
    let session_context = owner.context();
    let dispatcher = DefaultLegacyToolDispatcher::new(
        Arc::clone(&owner.runtime),
        &owner.session,
        memory_config.clone(),
        tool_config,
    )
    .expect("legacy app tool dispatcher");

    let result = TurnEngine::new(4)
        .execute_turn_in_context(
            &discovered_shell_exec_turn(
                "root-session",
                "turn-shell-discovered",
                "call-shell-discovered",
            ),
            &session_context,
            &dispatcher,
            None,
        )
        .await;

    let approval_request_id = match result {
        TurnResult::NeedsApproval(requirement) => {
            assert_eq!(requirement.tool_name.as_deref(), Some("shell.exec"));
            assert_eq!(requirement.approval_key.as_deref(), Some("tool:shell.exec"));
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
    assert_eq!(stored.tool_name, "shell.exec");
    assert_eq!(stored.turn_id, "turn-shell-discovered");
    assert_eq!(stored.tool_call_id, "call-shell-discovered");
    assert_eq!(stored.approval_key, "tool:shell.exec");
    assert_eq!(stored.request_payload_json["tool_name"], "shell.exec");
    assert_eq!(stored.request_payload_json["dispatch_kind"], "legacy_core");
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
    let owner = crate::test_support::runtime_session_for_test("root-session", tool_view);
    let session_context = owner.context();
    let dispatcher = DefaultLegacyToolDispatcher::new(
        Arc::clone(&owner.runtime),
        &owner.session,
        memory_config.clone(),
        tool_config,
    )
    .expect("legacy app tool dispatcher");
    let turn = discovered_shell_exec_turn("root-session", "turn-reuse", "call-reuse");

    let first = TurnEngine::new(4)
        .execute_turn_in_context(&turn, &session_context, &dispatcher, None)
        .await;
    let second = TurnEngine::new(4)
        .execute_turn_in_context(&turn, &session_context, &dispatcher, None)
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
    let owner = crate::test_support::runtime_session_for_test("root-session", tool_view);
    let session_context = owner.context();
    let dispatcher = DefaultLegacyToolDispatcher::new(
        Arc::clone(&owner.runtime),
        &owner.session,
        memory_config.clone(),
        tool_config,
    )
    .expect("legacy app tool dispatcher");
    let turn = skills_policy_get_turn(
        "root-session",
        "turn-autonomy-allowlist-prompt",
        "call-autonomy-allowlist-prompt",
    );

    let result = TurnEngine::new(4)
        .execute_turn_in_context(&turn, &session_context, &dispatcher, None)
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
    let owner = crate::test_support::runtime_session_for_test("root-session", tool_view);
    let session_context = owner.context();
    let dispatcher = DefaultLegacyToolDispatcher::new(
        Arc::clone(&owner.runtime),
        &owner.session,
        memory_config.clone(),
        tool_config,
    )
    .expect("legacy app tool dispatcher");
    let turn = skills_policy_get_turn(
        "root-session",
        "turn-autonomy-grant-prompt",
        "call-autonomy-grant-prompt",
    );

    let result = TurnEngine::new(4)
        .execute_turn_in_context(&turn, &session_context, &dispatcher, None)
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
        tool_input: None,
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
    let owner = crate::test_support::runtime_session_for_test(
        "session-observed-fast-lane",
        runtime_tool_view(),
    );
    let session_context = owner.context();
    let dispatcher = DelayedObservedExecutionDispatcher;
    let engine = TurnEngine::with_parallel_tool_execution(8, 512, true, 2);

    let (result, trace) = engine
        .execute_turn_in_context_with_trace(&turn, &session_context, &dispatcher, None, None)
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
    let owner = crate::test_support::runtime_session_for_test(
        "session-observed-sequence",
        runtime_tool_view(),
    );
    let session_context = owner.context();
    let after_calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let dispatcher = AfterExecutionSequenceRecordingDispatcher {
        after_calls: std::sync::Arc::clone(&after_calls),
    };
    let engine = TurnEngine::with_parallel_tool_execution(8, 512, true, 2);

    let (result, _trace) = engine
        .execute_turn_in_context_with_trace(&turn, &session_context, &dispatcher, None, None)
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
    let owner = crate::test_support::runtime_session_for_test(
        "session-observed-fast-lane-single",
        runtime_tool_view(),
    );
    let session_context = owner.context();
    let dispatcher = DelayedObservedExecutionDispatcher;
    let engine = TurnEngine::with_parallel_tool_execution(8, 512, true, 1);

    let (_result, trace) = engine
        .execute_turn_in_context_with_trace(&turn, &session_context, &dispatcher, None, None)
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
    let owner = crate::test_support::runtime_session_for_test(
        "session-observed-trace-order",
        runtime_tool_view(),
    );
    let session_context = owner.context();
    let dispatcher = DelayedObservedExecutionDispatcher;
    let engine = TurnEngine::with_parallel_tool_execution(8, 512, true, 2);

    let (result, trace) = engine
        .execute_turn_in_context_with_trace(&turn, &session_context, &dispatcher, None, None)
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
async fn parallel_execution_keeps_successful_tool_results_when_one_tool_is_denied() {
    let turn = ProviderTurn {
        assistant_text: "observing partial policy denial".to_owned(),
        tool_intents: vec![
            provider_app_tool_intent(
                "sessions_list",
                json!({}),
                "session-observed-partial-denial",
                "turn-observed-partial-denial",
                "call-partial-denial-success",
            ),
            provider_app_tool_intent(
                "sessions_list",
                json!({"deny": true}),
                "session-observed-partial-denial",
                "turn-observed-partial-denial",
                "call-partial-denial-denied",
            ),
        ],
        raw_meta: json!({}),
    };
    let owner = crate::test_support::runtime_session_for_test(
        "session-observed-partial-denial",
        runtime_tool_view(),
    );
    let session_context = owner.context();
    let dispatcher = AfterExecutionSequenceRecordingDispatcher {
        after_calls: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
    };
    let engine = TurnEngine::with_parallel_tool_execution(8, 512, true, 2);

    let (result, trace) = engine
        .execute_turn_in_context_with_trace(&turn, &session_context, &dispatcher, None, None)
        .await;

    let TurnResult::FinalText(text) = result else {
        panic!("expected per-tool denial result lines, got {result:?}");
    };
    assert!(text.contains("call-partial-denial-success"), "{text}");
    assert!(text.contains("call-partial-denial-denied"), "{text}");
    assert!(text.contains("denied by test policy"), "{text}");

    let trace = trace.expect("trace should exist");
    assert_eq!(trace.intent_outcomes.len(), 2);
    assert_eq!(
        trace.intent_outcomes[0].status,
        ToolBatchExecutionIntentStatus::Completed
    );
    assert_eq!(
        trace.intent_outcomes[1].status,
        ToolBatchExecutionIntentStatus::Denied
    );
}

#[tokio::test]
async fn sequential_execution_continues_after_single_tool_denial() {
    let turn = ProviderTurn {
        assistant_text: "observing sequential policy denial".to_owned(),
        tool_intents: vec![
            provider_app_tool_intent(
                "sessions_list",
                json!({}),
                "session-observed-sequential-denial",
                "turn-observed-sequential-denial",
                "call-sequential-denial-success-before",
            ),
            provider_app_tool_intent(
                "sessions_list",
                json!({"deny": true}),
                "session-observed-sequential-denial",
                "turn-observed-sequential-denial",
                "call-sequential-denial-denied",
            ),
            provider_app_tool_intent(
                "sessions_list",
                json!({}),
                "session-observed-sequential-denial",
                "turn-observed-sequential-denial",
                "call-sequential-denial-success-after",
            ),
        ],
        raw_meta: json!({}),
    };
    let owner = crate::test_support::runtime_session_for_test(
        "session-observed-sequential-denial",
        runtime_tool_view(),
    );
    let session_context = owner.context();
    let after_calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let dispatcher = AfterExecutionSequenceRecordingDispatcher {
        after_calls: std::sync::Arc::clone(&after_calls),
    };
    let engine = TurnEngine::with_parallel_tool_execution(8, 512, false, 1);

    let (result, trace) = engine
        .execute_turn_in_context_with_trace(&turn, &session_context, &dispatcher, None, None)
        .await;

    let TurnResult::FinalText(text) = result else {
        panic!("expected per-tool denial result lines, got {result:?}");
    };
    assert!(
        text.contains("call-sequential-denial-success-before"),
        "{text}"
    );
    assert!(text.contains("call-sequential-denial-denied"), "{text}");
    assert!(
        text.contains("call-sequential-denial-success-after"),
        "{text}"
    );

    let after_calls = after_calls.lock().expect("after call lock").clone();
    assert_eq!(
        after_calls,
        vec![
            ("call-sequential-denial-success-before".to_owned(), 0),
            ("call-sequential-denial-success-after".to_owned(), 2),
        ]
    );

    let trace = trace.expect("trace should exist");
    let statuses = trace
        .intent_outcomes
        .iter()
        .map(|intent_outcome| intent_outcome.status)
        .collect::<Vec<_>>();
    assert_eq!(
        statuses,
        vec![
            ToolBatchExecutionIntentStatus::Completed,
            ToolBatchExecutionIntentStatus::Denied,
            ToolBatchExecutionIntentStatus::Completed,
        ]
    );
}

#[tokio::test]
async fn observed_fast_lane_execution_trace_records_partial_tool_failure_outcomes() {
    let turn = partially_failing_observed_execution_turn(
        "session-observed-partial-failure",
        "turn-observed-partial-failure",
    );
    let owner = crate::test_support::runtime_session_for_test(
        "session-observed-partial-failure",
        runtime_tool_view(),
    );
    let session_context = owner.context();
    let dispatcher = PartiallyFailingObservedExecutionDispatcher;
    let engine = TurnEngine::with_parallel_tool_execution(4, 512, false, 1);

    let (result, trace) = engine
        .execute_turn_in_context_with_trace(&turn, &session_context, &dispatcher, None, None)
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
        "read",
        json!({"path": "note.md"}),
        "session-bounded-payload",
        "turn-bounded-payload",
        "call-bounded-payload",
    );
    let large_payload = json!({
        "contents": "x".repeat(TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS + 128),
    });
    let record = build_success_tool_outcome_trace_record(&intent, "ok", &large_payload);

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

    let envelope =
        result::build_tool_result_envelope(&intent, outcome.status.as_str(), &outcome.payload, 256);
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
    let owner = crate::test_support::runtime_session_for_test(
        "root-session",
        crate::tools::ToolView::from_legacy_paths(["browse"]),
    );
    let session_context = owner.context();
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

    let owner = crate::test_support::runtime_session_for_test(
        "root-session",
        crate::tools::ToolView::from_legacy_paths(["task_wait"]),
    );
    let session_context = owner.context();

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

    let owner = crate::test_support::runtime_session_for_test(
        "root-session",
        crate::tools::ToolView::from_legacy_paths(["task_events"]),
    );
    let session_context = owner.context();

    let augmented =
        augment_tool_payload_for_kernel("task_events", json!({}), &session_context, &memory_config);

    assert_eq!(augmented.payload["task_id"], "task-root");
}
