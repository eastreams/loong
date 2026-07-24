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
    action::{ActionMeta, Denied, Granted},
    policy::{ParentGrantRequester, PolicyEngine, PolicyEngineImpl},
};

const LOCAL_GRANT_UUID: Uuid = Uuid::from_u128(1);
const PARENT_GRANT_UUID: Uuid = Uuid::from_u128(2);

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
    record_result: Uuid,
    record_calls: AtomicUsize,
}

impl TestEngine {
    fn new(decision: PolicyDecisionFinal, reason: Option<&str>) -> Self {
        Self {
            result: PolicyResultFinal {
                decision,
                reason: reason.map(str::to_owned),
            },
            record_result: LOCAL_GRANT_UUID,
            record_calls: AtomicUsize::new(0),
        }
    }
}

impl<Cx: Sync> PolicyEngineImpl<Cx> for TestEngine {
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
    ) -> impl Future<Output = Uuid> + Send {
        self.record_calls.fetch_add(1, Ordering::Relaxed);
        ready(self.record_result)
    }
}

enum ParentResult {
    Granted(Uuid),
    Denied(Option<String>),
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
    async fn grant<A: ActionMeta>(&self, action: A) -> Result<Granted<A>, Denied> {
        self.parent_calls.fetch_add(1, Ordering::Relaxed);
        match &self.parent_result {
            ParentResult::Granted(grant_id) => Ok(Granted::new(*grant_id, action)),
            ParentResult::Denied(reason) => Err(Denied {
                reason: reason.clone(),
            }),
        }
    }
}

struct RecursiveContext<'a> {
    engine: &'a TestEngine,
    parent: Option<&'a RecursiveContext<'a>>,
}

#[async_trait]
impl ParentGrantRequester for RecursiveContext<'_> {
    async fn grant<A: ActionMeta>(&self, action: A) -> Result<Granted<A>, Denied> {
        match self.parent {
            Some(parent) => PolicyEngine::grant(self.engine, parent, action).await,
            None => Err(Denied {
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
    let Ok(granted) = result else {
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
        Err(Denied { reason }) if reason.as_deref() == Some("not allowed")
    ));
    assert_eq!(engine.record_calls.load(Ordering::Relaxed), 0);
    assert_eq!(context.parent_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn approval_preserves_the_parent_grant_and_action() {
    let engine = TestEngine::new(PolicyDecisionFinal::RequiresApproval, None);
    let context = TestContext::new(ParentResult::Granted(PARENT_GRANT_UUID));

    let result = resolve(PolicyEngine::grant(&engine, &context, TestAction(7)));
    let Ok(granted) = result else {
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
        Err(Denied { reason }) if reason.as_deref() == Some("parent denied")
    ));
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
        Err(Denied { reason }) if reason.as_deref() == Some("no parent")
    ));
    assert_eq!(engine.record_calls.load(Ordering::Relaxed), 0);
}
