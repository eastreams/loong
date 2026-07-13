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
    fn allowed_capabilities(&self) -> &std::collections::BTreeSet<loong_contracts::Capability> {
        static EMPTY: std::collections::BTreeSet<loong_contracts::Capability> =
            std::collections::BTreeSet::new();
        &EMPTY
    }
}

#[test]
fn runtime_owns_kernel_and_selected_tool_plane() {
    let runtime = Runtime::new(
        Kernel::<TestContextFactory>::new_without_audit(),
        ToolPlaneRegistry::new(),
    );

    assert!(runtime.kernel().now_epoch_s() > 0);
    assert_eq!(runtime.tools().registered_paths(), Vec::<ToolPath>::new());
}
