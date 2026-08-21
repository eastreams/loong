use std::borrow::Cow;

use contracts::capability::Capabilities;
use serde_json::Value;
use uuid::Uuid;

use super::{ActionMeta, Granted};

struct TestAction;

impl ActionMeta for TestAction {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed("test")
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Borrowed(&Value::Null)
    }

    fn required_capabilities(&self) -> Capabilities {
        Capabilities::empty()
    }
}

#[test]
fn granted_binds_the_recorded_id_to_the_action() {
    for recorded_id in [Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(42)] {
        let granted = Granted::new(recorded_id, TestAction);

        assert_eq!(granted.grant_id.0, recorded_id);
        assert!(matches!(granted.action, TestAction));
    }
}
