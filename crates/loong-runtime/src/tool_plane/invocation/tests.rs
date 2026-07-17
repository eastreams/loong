use std::{
    borrow::Cow,
    collections::BTreeSet,
    future::pending,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use loong_contracts::{
    ActionExecutionEvent, AuditError, AuditEvent, AuditEventKind, AuthorizationAttempt,
    AuthorizationAttemptEvent, AuthorizationPolicyEvent, AuthorizationScope, AuthorizationSubject,
    AuthorizationTerminalOutcome, Capabilities, Capability, ToolInputError, ToolSchedulingClass,
    ToolSpec,
};
use loong_core::{
    error::PolicyGrantError,
    policy::context::{ContextFactory, PolicyContext},
    tool::ToolImpl,
};
use loong_kernel::{AllowPolicy, AuditSink, FixedClock, Kernel, policy::PolicyPipelineBuilder};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::Notify;

use super::ToolInvocationContext;
use crate::{
    runtime::Runtime,
    tool_plane::{
        RegisteredToolError, ToolPath, ToolPlaneRegistry,
        error::{CapabilityNarrowingError, ToolInvocationError},
    },
};

struct TestContextFactory;

impl ContextFactory for TestContextFactory {
    type Cx<'a> = TestContext;
}

#[derive(Clone)]
struct TestContext {
    capabilities: Capabilities,
    expand_child: bool,
}

impl PolicyContext for TestContext {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        Cow::Borrowed(&self.capabilities)
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        AuthorizationSubject {
            actor_id: "test:runtime:tool-invocation:actor".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "test:runtime:tool-invocation:session".to_owned(),
            },
        }
    }
}

impl ToolInvocationContext for TestContext {
    fn derive_tool_child(
        &self,
        capabilities: Capabilities,
    ) -> Result<Self, CapabilityNarrowingError> {
        if !capabilities.is_subset(&self.capabilities) {
            return Err(CapabilityNarrowingError {
                allowed: self.capabilities.clone(),
                derived: capabilities,
            });
        }
        if self.expand_child {
            return Ok(self.clone());
        }
        Ok(Self {
            capabilities,
            expand_child: false,
        })
    }
}

#[derive(Debug, Error)]
#[error("test tool execution failed")]
struct TestToolError;

struct TestTool {
    executions: Arc<AtomicUsize>,
    fail: bool,
    required_capabilities: BTreeSet<Capability>,
}

struct PendingTool {
    entered: Arc<Notify>,
    executions: Arc<AtomicUsize>,
}

#[async_trait]
impl ToolImpl<TestContextFactory> for TestTool {
    type Input = String;
    type Output = Value;
    type Error = TestToolError;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            description: "Test runtime invocation.".to_owned(),
            input_schema: json!({ "type": "object" }),
            required_capabilities: self.required_capabilities.clone(),
            scheduling: ToolSchedulingClass::SerialOnly,
            argument_hint: None,
            search_hint: None,
            tags: Vec::new(),
        }
    }

    fn parse_input(&self, payload: Value) -> Result<Self::Input, ToolInputError> {
        payload
            .get("message")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| ToolInputError::missing_field("message"))
    }

    async fn execute(
        &self,
        _ctx: &<TestContextFactory as ContextFactory>::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        self.executions.fetch_add(1, Ordering::Relaxed);
        if self.fail {
            return Err(TestToolError);
        }
        Ok(json!({ "message": input }))
    }
}

#[async_trait]
impl ToolImpl<TestContextFactory> for PendingTool {
    type Input = ();
    type Output = Value;
    type Error = TestToolError;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            description: "Never completes after dispatch starts.".to_owned(),
            input_schema: json!({ "type": "object" }),
            required_capabilities: BTreeSet::new(),
            scheduling: ToolSchedulingClass::SerialOnly,
            argument_hint: None,
            search_hint: None,
            tags: Vec::new(),
        }
    }

    fn parse_input(&self, _payload: Value) -> Result<Self::Input, ToolInputError> {
        Ok(())
    }

    async fn execute(
        &self,
        _ctx: &<TestContextFactory as ContextFactory>::Cx<'_>,
        (): Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        self.executions.fetch_add(1, Ordering::Relaxed);
        self.entered.notify_one();
        pending().await
    }
}

struct ScriptedAuditSink {
    fail_on: Option<usize>,
    calls: AtomicUsize,
    events: Mutex<Vec<AuditEvent>>,
}

impl ScriptedAuditSink {
    fn new(fail_on: Option<usize>) -> Self {
        Self {
            fail_on,
            calls: AtomicUsize::new(0),
            events: Mutex::new(Vec::new()),
        }
    }

    fn snapshot(&self) -> Vec<AuditEvent> {
        self.events
            .lock()
            .expect("test audit sink lock should remain available")
            .clone()
    }
}

impl AuditSink for ScriptedAuditSink {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
        if self.fail_on == Some(call) {
            return Err(AuditError::Sink(format!(
                "scripted audit failure on call {call}"
            )));
        }
        self.events
            .lock()
            .map_err(|_poisoned| AuditError::Sink("test audit sink mutex poisoned".to_owned()))?
            .push(event);
        Ok(())
    }
}

fn test_runtime(
    fail_on_audit_call: Option<usize>,
    tool_fails: bool,
) -> (
    Runtime<TestContextFactory>,
    Arc<ScriptedAuditSink>,
    Arc<AtomicUsize>,
) {
    let audit = Arc::new(ScriptedAuditSink::new(fail_on_audit_call));
    let policy = PolicyPipelineBuilder::new().with_fallback_policy(AllowPolicy);
    let kernel = Kernel::with_policy_runtime(policy, Arc::new(FixedClock::new(1)), audit.clone());
    let executions = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolPlaneRegistry::new();
    tools
        .register(
            ToolPath::from("test.echo"),
            TestTool {
                executions: executions.clone(),
                fail: tool_fails,
                required_capabilities: BTreeSet::new(),
            },
        )
        .expect("test tool should register");
    (Runtime::new(kernel, tools), audit, executions)
}

fn context() -> TestContext {
    TestContext {
        capabilities: Capabilities::from([Capability::InvokeTool]),
        expand_child: false,
    }
}

#[tokio::test]
async fn capability_override_escalation_is_audited_before_grant() {
    let (runtime, audit, executions) = test_runtime(None, false);
    let requested = Capabilities::from([Capability::FilesystemRead]);

    let error = runtime
        .tool(&context(), ToolPath::from("test.echo"))
        .expect("registered tool should resolve")
        .with_capabilities_override(requested.clone())
        .invoke(json!({ "message": "hello" }))
        .await
        .expect_err("override must not expand declared tool authority");

    assert!(matches!(
        error,
        ToolInvocationError::CapabilityOverride(ref rejection)
            if rejection.path == ToolPath::from("test.echo")
                && rejection.requested == requested
                && rejection.declared == Capabilities::new()
    ));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
    assert!(matches!(
        audit.snapshot().as_slice(),
        [AuditEvent {
            kind: AuditEventKind::ToolCapabilityOverrideRejected {
                path_display,
                requested: event_requested,
                declared,
                ..
            },
            ..
        }] if path_display == "test.echo"
            && event_requested == &requested
            && declared == &Capabilities::new()
    ));
}

#[tokio::test]
async fn capability_override_rejection_and_audit_failure_keep_both_sources() {
    let (runtime, _audit, executions) = test_runtime(Some(1), false);

    let error = runtime
        .tool(&context(), ToolPath::from("test.echo"))
        .expect("registered tool should resolve")
        .with_capabilities_override(Capabilities::from([Capability::FilesystemRead]))
        .invoke(json!({ "message": "hello" }))
        .await
        .expect_err("audit failure must remain attached to override rejection");

    assert!(matches!(
        error,
        ToolInvocationError::CapabilityOverrideAndAudit {
            rejection,
            audit_source: AuditError::Sink(ref reason),
        } if rejection.path == ToolPath::from("test.echo") && reason.contains("call 1")
    ));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn derived_child_cannot_exceed_runtime_selected_authority() {
    let (runtime, audit, executions) = test_runtime(None, false);
    let parent = TestContext {
        capabilities: Capabilities::from([Capability::InvokeTool, Capability::FilesystemRead]),
        expand_child: true,
    };

    let error = runtime
        .tool(&parent, ToolPath::from("test.echo"))
        .expect("registered tool should resolve")
        .invoke(json!({ "message": "hello" }))
        .await
        .expect_err("derived child must not regain capabilities outside the tool declaration");

    assert!(matches!(
        error,
        ToolInvocationError::CapabilityNarrowing(CapabilityNarrowingError {
            ref allowed,
            ref derived,
        }) if allowed == &Capabilities::from([Capability::InvokeTool])
            && derived == &parent.capabilities
    ));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
    assert!(audit.snapshot().is_empty());
}

#[tokio::test]
async fn missing_tool_capability_is_denied_and_audited_by_policy_engine() {
    let audit = Arc::new(ScriptedAuditSink::new(None));
    let policy = PolicyPipelineBuilder::new().with_fallback_policy(AllowPolicy);
    let kernel = Kernel::with_policy_runtime(policy, Arc::new(FixedClock::new(1)), audit.clone());
    let executions = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolPlaneRegistry::new();
    tools
        .register(
            ToolPath::from("test.read"),
            TestTool {
                executions: executions.clone(),
                fail: false,
                required_capabilities: BTreeSet::from([Capability::FilesystemRead]),
            },
        )
        .expect("test tool should register");
    let runtime = Runtime::new(kernel, tools);

    let error = runtime
        .tool(&context(), ToolPath::from("test.read"))
        .expect("registered tool should resolve")
        .invoke(json!({ "message": "hello" }))
        .await
        .expect_err("missing filesystem capability must deny invocation");

    assert!(matches!(
        error,
        ToolInvocationError::Authorization(PolicyGrantError::MissingCapability {
            capability: Capability::FilesystemRead,
        })
    ));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
    let events = audit.snapshot();
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        AuditEventKind::Authorization {
            evidence: loong_contracts::AuthorizationEvidence {
                attempt: AuthorizationAttempt::Started {
                    event: AuthorizationAttemptEvent::CapabilityDenied {
                        capability: Capability::FilesystemRead,
                    },
                    ..
                },
                ..
            }
        }
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind, AuditEventKind::ActionExecution { .. }))
    );
}

#[tokio::test]
async fn invocation_correlates_authorization_start_and_completion() {
    let (runtime, audit, executions) = test_runtime(None, false);

    let output = runtime
        .tool(&context(), ToolPath::from("test.echo"))
        .expect("registered tool should resolve")
        .invoke(json!({ "message": "hello" }))
        .await
        .expect("tool should complete");

    assert_eq!(output, json!({ "message": "hello" }));
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    let events = audit.snapshot();
    let authorization_grant = events.iter().find_map(|event| {
        let AuditEventKind::Authorization { evidence } = &event.kind else {
            return None;
        };
        let AuthorizationAttempt::Started {
            event:
                AuthorizationAttemptEvent::Policy {
                    event:
                        AuthorizationPolicyEvent::Terminal(AuthorizationTerminalOutcome::Allow {
                            grant_id,
                        }),
                    ..
                },
            ..
        } = &evidence.attempt
        else {
            return None;
        };
        Some(*grant_id)
    });
    let execution_events = events
        .iter()
        .filter_map(|event| {
            let AuditEventKind::ActionExecution { grant_id, event } = &event.kind else {
                return None;
            };
            Some((*grant_id, event))
        })
        .collect::<Vec<_>>();

    let grant_id = authorization_grant.expect("authorization should mint a grant id");
    assert_eq!(
        execution_events,
        vec![
            (grant_id, &ActionExecutionEvent::Started),
            (grant_id, &ActionExecutionEvent::Completed),
        ]
    );
}

#[tokio::test]
async fn invalid_input_is_audited_without_executing_the_tool() {
    let (runtime, audit, executions) = test_runtime(None, false);

    let error = runtime
        .tool(&context(), ToolPath::from("test.echo"))
        .expect("registered tool should resolve")
        .invoke(json!({}))
        .await
        .expect_err("registered input parser should reject missing message");

    assert!(matches!(
        error,
        ToolInvocationError::Dispatch {
            source: RegisteredToolError::Input(ToolInputError::MissingField { ref field }),
            ..
        } if field == "message"
    ));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
    assert!(audit.snapshot().iter().any(|event| matches!(
        &event.kind,
        AuditEventKind::ActionExecution {
            event: ActionExecutionEvent::InputRejected {
                error: ToolInputError::MissingField { field },
            },
            ..
        } if field == "message"
    )));
}

#[tokio::test]
async fn bound_registered_tool_runs_its_success_observer() {
    let audit = Arc::new(ScriptedAuditSink::new(None));
    let kernel = Kernel::with_policy_runtime(
        PolicyPipelineBuilder::new().with_fallback_policy(AllowPolicy),
        Arc::new(FixedClock::new(1)),
        audit,
    );
    let executions = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut tools = ToolPlaneRegistry::new();
    tools
        .register_with_success_observer(
            ToolPath::from("test.echo"),
            TestTool {
                executions: executions.clone(),
                fail: false,
                required_capabilities: BTreeSet::new(),
            },
            {
                let observed = observed.clone();
                move |_ctx, output: &Value| {
                    observed
                        .lock()
                        .expect("observer lock should remain available")
                        .push(output["message"].as_str().unwrap_or_default().to_owned());
                }
            },
        )
        .expect("observed test tool should register");
    let runtime = Runtime::new(kernel, tools);

    runtime
        .tool(&context(), ToolPath::from("test.echo"))
        .expect("registered tool should resolve")
        .invoke(json!({ "message": "hello" }))
        .await
        .expect("observed tool should complete");

    assert_eq!(executions.load(Ordering::Relaxed), 1);
    assert_eq!(
        observed
            .lock()
            .expect("observer lock should remain available")
            .as_slice(),
        ["hello"]
    );
}

#[tokio::test]
async fn dropping_started_invocation_records_unknown_outcome() {
    let audit = Arc::new(ScriptedAuditSink::new(None));
    let kernel = Kernel::with_policy_runtime(
        PolicyPipelineBuilder::new().with_fallback_policy(AllowPolicy),
        Arc::new(FixedClock::new(1)),
        audit.clone(),
    );
    let entered = Arc::new(Notify::new());
    let executions = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolPlaneRegistry::new();
    tools
        .register(
            ToolPath::from("test.pending"),
            PendingTool {
                entered: entered.clone(),
                executions: executions.clone(),
            },
        )
        .expect("pending test tool should register");
    let runtime = Runtime::new(kernel, tools);
    let context = context();
    let mut invocation = Box::pin(
        runtime
            .tool(&context, ToolPath::from("test.pending"))
            .expect("registered tool should resolve")
            .invoke(json!({})),
    );

    tokio::select! {
        result = &mut invocation => panic!("pending tool returned unexpectedly: {result:?}"),
        () = entered.notified() => {}
    }
    drop(invocation);

    assert_eq!(executions.load(Ordering::Relaxed), 1);
    let execution = audit
        .snapshot()
        .into_iter()
        .filter_map(|audit| {
            if let AuditEventKind::ActionExecution { event, .. } = audit.kind {
                Some(event)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        execution,
        vec![
            ActionExecutionEvent::Started,
            ActionExecutionEvent::OutcomeUnknown,
        ]
    );
}

#[tokio::test]
async fn dropping_started_invocation_cannot_propagate_terminal_audit_failure() {
    // Authorization allow is call 1, Started is call 2, and the guard's
    // best-effort OutcomeUnknown write is call 3.
    let audit = Arc::new(ScriptedAuditSink::new(Some(3)));
    let kernel = Kernel::with_policy_runtime(
        PolicyPipelineBuilder::new().with_fallback_policy(AllowPolicy),
        Arc::new(FixedClock::new(1)),
        audit.clone(),
    );
    let entered = Arc::new(Notify::new());
    let executions = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolPlaneRegistry::new();
    tools
        .register(
            ToolPath::from("test.pending"),
            PendingTool {
                entered: entered.clone(),
                executions: executions.clone(),
            },
        )
        .expect("pending test tool should register");
    let runtime = Runtime::new(kernel, tools);
    let context = context();
    let mut invocation = Box::pin(
        runtime
            .tool(&context, ToolPath::from("test.pending"))
            .expect("registered tool should resolve")
            .invoke(json!({})),
    );

    tokio::select! {
        result = &mut invocation => panic!("pending tool returned unexpectedly: {result:?}"),
        () = entered.notified() => {}
    }
    drop(invocation);

    assert_eq!(executions.load(Ordering::Relaxed), 1);
    assert_eq!(audit.calls.load(Ordering::Relaxed), 3);
    let execution = audit
        .snapshot()
        .into_iter()
        .filter_map(|audit| {
            if let AuditEventKind::ActionExecution { event, .. } = audit.kind {
                Some(event)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(execution, vec![ActionExecutionEvent::Started]);
}

#[tokio::test]
async fn start_audit_failure_prevents_dispatch() {
    let (runtime, _audit, executions) = test_runtime(Some(2), false);

    let error = runtime
        .tool(&context(), ToolPath::from("test.echo"))
        .expect("registered tool should resolve")
        .invoke(json!({ "message": "hello" }))
        .await
        .expect_err("start audit failure should stop dispatch");

    assert!(matches!(
        error,
        ToolInvocationError::StartAudit {
            grant_id: _,
            source: AuditError::Sink(ref reason),
        } if reason.contains("call 2")
    ));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn policy_denial_stays_typed_and_emits_no_execution_event() {
    let audit = Arc::new(ScriptedAuditSink::new(None));
    let kernel = Kernel::with_policy_runtime(
        PolicyPipelineBuilder::new(),
        Arc::new(FixedClock::new(1)),
        audit.clone(),
    );
    let executions = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolPlaneRegistry::new();
    tools
        .register(
            ToolPath::from("test.echo"),
            TestTool {
                executions: executions.clone(),
                fail: false,
                required_capabilities: BTreeSet::new(),
            },
        )
        .expect("test tool should register");
    let runtime = Runtime::new(kernel, tools);

    let error = runtime
        .tool(&context(), ToolPath::from("test.echo"))
        .expect("registered tool should resolve")
        .invoke(json!({ "message": "hello" }))
        .await
        .expect_err("default-deny policy should reject invocation");

    assert!(matches!(
        error,
        ToolInvocationError::Authorization(PolicyGrantError::Denied { .. })
    ));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
    assert!(
        !audit
            .snapshot()
            .iter()
            .any(|event| matches!(event.kind, AuditEventKind::ActionExecution { .. }))
    );
}

#[tokio::test]
async fn completed_audit_error_preserves_output_without_leaking_it_through_debug() {
    let (runtime, _audit, executions) = test_runtime(Some(3), false);
    let sentinel = "sensitive-file-content";

    let error = runtime
        .tool(&context(), ToolPath::from("test.echo"))
        .expect("registered tool should resolve")
        .invoke(json!({ "message": sentinel }))
        .await
        .expect_err("terminal audit failure should remain visible");
    let debug = format!("{error:?}");

    assert!(matches!(
        error,
        ToolInvocationError::CompletedAudit {
            grant_id: _,
            ref output,
            source: AuditError::Sink(ref reason),
        } if output == &json!({ "message": sentinel }) && reason.contains("call 3")
    ));
    assert!(debug.contains("[redacted]"));
    assert!(!debug.contains(sentinel));
    assert_eq!(executions.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn dispatch_and_terminal_audit_failures_keep_both_sources() {
    let (runtime, _audit, executions) = test_runtime(Some(3), true);

    let error = runtime
        .tool(&context(), ToolPath::from("test.echo"))
        .expect("registered tool should resolve")
        .invoke(json!({ "message": "hello" }))
        .await
        .expect_err("both failures should remain visible");

    assert!(matches!(
        error,
        ToolInvocationError::DispatchAndAudit {
            grant_id: _,
            dispatch_source: RegisteredToolError::Execution { ref source },
            audit_source: AuditError::Sink(ref reason),
        } if source.is::<TestToolError>() && reason.contains("call 3")
    ));
    assert_eq!(executions.load(Ordering::Relaxed), 1);
}
