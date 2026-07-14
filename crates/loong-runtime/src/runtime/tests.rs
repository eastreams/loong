use std::borrow::Cow;

use loong_contracts::{AuthorizationScope, AuthorizationSubject, Capabilities};
use loong_core::policy::context::{ContextFactory, PolicyContext};
use loong_kernel::Kernel;

use super::Runtime;
use crate::tool_plane::{ToolPath, ToolPlaneRegistry};

struct TestContextFactory;

impl ContextFactory for TestContextFactory {
    type Cx<'a> = TestContext;
}

struct TestContext;

impl PolicyContext for TestContext {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        static EMPTY: Capabilities = Capabilities::new();
        Cow::Borrowed(&EMPTY)
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        AuthorizationSubject {
            actor_id: "test:runtime:owner:actor".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "test:runtime:owner:session".to_owned(),
            },
        }
    }
}

#[test]
fn runtime_owns_kernel_and_selected_tool_plane() {
    let runtime = Runtime::new(
        Kernel::<TestContextFactory>::new(),
        ToolPlaneRegistry::new(),
    );

    assert!(runtime.kernel().now_epoch_s() > 0);
    assert_eq!(runtime.tools().registered_paths(), Vec::<ToolPath>::new());
}
