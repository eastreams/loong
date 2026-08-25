use std::borrow::Cow;

use contracts::capability::{Capabilities, Capability};
use loac::Shutdown;
use serde_json::Value;

use super::*;

#[derive(Debug)]
struct TestAction;

impl ActionMeta for TestAction {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed("test")
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(Value::Null)
    }

    fn required_capabilities(&self) -> contracts::capability::Capabilities {
        Capability::FsRead.into()
    }
}

#[tokio::test]
async fn explicit_capability_policy_grants_unique_ids() {
    let owner = loac::spawn::<Kernel>(PolicyEngine::allow_capabilities());
    let facade = Facade::new(owner.actor_ref(), Capability::FsRead.into());

    let first = facade.grant(TestAction).await.unwrap();
    let second = facade.grant(TestAction).await.unwrap();
    assert_ne!(first.grant_id(), second.grant_id());
    assert_eq!(first.action().name(), "test");

    owner.shutdown(Shutdown::Stop).await;
}

#[tokio::test]
async fn default_policy_denies_without_a_matching_policy() {
    let owner = loac::spawn::<Kernel>(PolicyEngine::default());
    let facade = Facade::new(owner.actor_ref(), Capabilities::empty());

    let error = facade.grant(TestAction).await.unwrap_err();
    assert!(matches!(error, GrantSendError::Denied(_)));

    owner.shutdown(Shutdown::Stop).await;
}
