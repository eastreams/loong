use core::{
    future::ready,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
};

use alloc::{
    borrow::{Cow, ToOwned},
    boxed::Box,
    string::String,
};

use async_trait::async_trait;
use loong_contracts::{
    capability::Capabilities,
    policy::{PolicyDecisionFinal, PolicyResultFinal},
};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    action::{ActionMeta, Granted},
    policy::{GrantOutcome, ParentGrantRequester, PolicyEngine, PolicyEngineImpl},
};

const LOCAL_GRANT_UUID: Uuid = Uuid::from_u128(1);
const PARENT_GRANT_UUID: Uuid = Uuid::from_u128(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestError {
    Record,
    Parent,
}

impl core::fmt::Display for TestError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Record => formatter.write_str("record failed"),
            Self::Parent => formatter.write_str("parent failed"),
        }
    }
}

impl core::error::Error for TestError {}

#[derive(Debug)]
struct TestAction(u8);

impl ActionMeta for TestAction {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed("test")
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(Value::from(self.0))
    }

    fn required_capabilities(&self) -> Capabilities {
        Capabilities::empty()
    }
}

struct TestEngine {
    result: PolicyResultFinal,
    record_result: Result<Uuid, TestError>,
    record_calls: AtomicUsize,
}

impl TestEngine {
    fn new(decision: PolicyDecisionFinal, reason: Option<&str>) -> Self {
        Self {
            result: PolicyResultFinal {
                decision,
                reason: reason.map(str::to_owned),
            },
            record_result: Ok(LOCAL_GRANT_UUID),
            record_calls: AtomicUsize::new(0),
        }
    }
}

impl<Cx: Sync> PolicyEngineImpl<Cx> for TestEngine {
    type Error = TestError;

    fn evaluate<A: ActionMeta>(
        &self,
        _ctx: &Cx,
        _action: &A,
    ) -> impl Future<Output = PolicyResultFinal> + Send {
        ready(self.result.clone())
    }

    fn record_action_granted<A: ActionMeta>(
        &self,
        _ctx: &Cx,
        _action: &A,
    ) -> impl Future<Output = Result<Uuid, Self::Error>> + Send {
        self.record_calls.fetch_add(1, Ordering::Relaxed);
        ready(self.record_result)
    }
}

enum ParentResult {
    Granted(Uuid),
    Denied(Option<String>),
    Error(TestError),
}

struct TestContext {
    parent_result: ParentResult,
    parent_calls: AtomicUsize,
}

impl TestContext {
    fn new(parent_result: ParentResult) -> Self {
        Self {
            parent_result,
            parent_calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ParentGrantRequester for TestContext {
    type Error = TestError;

    async fn grant<A: ActionMeta>(&self, action: A) -> Result<GrantOutcome<A>, Self::Error> {
        self.parent_calls.fetch_add(1, Ordering::Relaxed);
        match &self.parent_result {
            ParentResult::Granted(grant_id) => {
                Ok(GrantOutcome::Granted(Granted::new(*grant_id, action)))
            }
            ParentResult::Denied(reason) => Ok(GrantOutcome::Denied {
                reason: reason.clone(),
            }),
            ParentResult::Error(error) => Err(*error),
        }
    }
}

struct RecursiveContext<'a> {
    engine: &'a TestEngine,
    parent: Option<&'a RecursiveContext<'a>>,
}

#[async_trait]
impl ParentGrantRequester for RecursiveContext<'_> {
    type Error = TestError;

    async fn grant<A: ActionMeta>(&self, action: A) -> Result<GrantOutcome<A>, Self::Error> {
        match self.parent {
            Some(parent) => PolicyEngine::grant(self.engine, parent, action).await,
            None => Ok(GrantOutcome::Denied {
                reason: Some("no parent".to_owned()),
            }),
        }
    }
}

fn resolve<F: Future>(future: F) -> F::Output {
    let mut future = core::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());

    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("test future unexpectedly returned pending"),
    }
}

#[test]
fn allow_records_before_returning_a_grant() {
    let engine = TestEngine::new(PolicyDecisionFinal::Allow, None);
    let context = TestContext::new(ParentResult::Denied(None));

    let result = resolve(PolicyEngine::grant(&engine, &context, TestAction(7)));
    let Ok(GrantOutcome::Granted(granted)) = result else {
        panic!("allow should return a grant");
    };
    let expected_grant_id = Granted::new(LOCAL_GRANT_UUID, TestAction(0)).grant_id();
    let (grant_id, action) = granted.into_parts();

    assert_eq!(grant_id, expected_grant_id);
    assert_eq!(action.0, 7);
    assert_eq!(engine.record_calls.load(Ordering::Relaxed), 1);
    assert_eq!(context.parent_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn deny_keeps_reason_without_recording() {
    let engine = TestEngine::new(PolicyDecisionFinal::Deny, Some("not allowed"));
    let context = TestContext::new(ParentResult::Granted(PARENT_GRANT_UUID));

    let result = resolve(PolicyEngine::grant(&engine, &context, TestAction(7)));

    assert!(matches!(
        result,
        Ok(GrantOutcome::Denied { reason }) if reason.as_deref() == Some("not allowed")
    ));
    assert_eq!(engine.record_calls.load(Ordering::Relaxed), 0);
    assert_eq!(context.parent_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn approval_preserves_the_parent_grant_and_action() {
    let engine = TestEngine::new(PolicyDecisionFinal::RequiresApproval, None);
    let context = TestContext::new(ParentResult::Granted(PARENT_GRANT_UUID));

    let result = resolve(PolicyEngine::grant(&engine, &context, TestAction(7)));
    let Ok(GrantOutcome::Granted(granted)) = result else {
        panic!("parent allow should return its grant");
    };
    let expected_grant_id = Granted::new(PARENT_GRANT_UUID, TestAction(0)).grant_id();
    let (grant_id, action) = granted.into_parts();

    assert_eq!(grant_id, expected_grant_id);
    assert_eq!(action.0, 7);
    assert_eq!(engine.record_calls.load(Ordering::Relaxed), 0);
    assert_eq!(context.parent_calls.load(Ordering::Relaxed), 1);
}

#[test]
fn approval_keeps_the_parent_denial() {
    let engine = TestEngine::new(PolicyDecisionFinal::RequiresApproval, None);
    let context = TestContext::new(ParentResult::Denied(Some("parent denied".to_owned())));

    let result = resolve(PolicyEngine::grant(&engine, &context, TestAction(7)));

    assert!(matches!(
        result,
        Ok(GrantOutcome::Denied { reason }) if reason.as_deref() == Some("parent denied")
    ));
    assert_eq!(engine.record_calls.load(Ordering::Relaxed), 0);
    assert_eq!(context.parent_calls.load(Ordering::Relaxed), 1);
}

#[test]
fn recording_failure_does_not_return_a_grant() {
    let mut engine = TestEngine::new(PolicyDecisionFinal::Allow, None);
    engine.record_result = Err(TestError::Record);
    let context = TestContext::new(ParentResult::Denied(None));

    let result = resolve(PolicyEngine::grant(&engine, &context, TestAction(7)));

    assert!(matches!(result, Err(TestError::Record)));
    assert_eq!(engine.record_calls.load(Ordering::Relaxed), 1);
    assert_eq!(context.parent_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn parent_failure_stays_an_operational_error() {
    let engine = TestEngine::new(PolicyDecisionFinal::RequiresApproval, None);
    let context = TestContext::new(ParentResult::Error(TestError::Parent));

    let result = resolve(PolicyEngine::grant(&engine, &context, TestAction(7)));

    assert!(matches!(result, Err(TestError::Parent)));
    assert_eq!(engine.record_calls.load(Ordering::Relaxed), 0);
    assert_eq!(context.parent_calls.load(Ordering::Relaxed), 1);
}

#[test]
fn parent_request_can_reenter_the_policy_engine() {
    let engine = TestEngine::new(PolicyDecisionFinal::RequiresApproval, None);
    let root = RecursiveContext {
        engine: &engine,
        parent: None,
    };
    let child = RecursiveContext {
        engine: &engine,
        parent: Some(&root),
    };

    let result = resolve(PolicyEngine::grant(&engine, &child, TestAction(7)));

    assert!(matches!(
        result,
        Ok(GrantOutcome::Denied { reason }) if reason.as_deref() == Some("no parent")
    ));
    assert_eq!(engine.record_calls.load(Ordering::Relaxed), 0);
}
