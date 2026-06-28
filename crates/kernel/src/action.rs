//! Typed action and grant primitives for governed side effects.
//!
//! An action is finer-grained than the existing execution planes. The plane
//! remains the coarse Loong runtime lane used by audit and routing
//! (`Tool`, `Runtime`, `Memory`, or `Connector`), while the action kind and
//! operation identify the concrete side-effect intent inside that lane.

use std::collections::BTreeSet;

use async_trait::async_trait;

use crate::{
    audit::{ExecutionPlane, PlaneTier},
    contracts::Capability,
    errors::KernelError,
};

/// A typed, policy-facing unit of side-effect intent.
///
/// Tools should eventually express side effects by asking access facades to
/// build actions. The action can then be authorized, granted, audited, and
/// executed through a domain executor.
pub trait Action: Send + Sync + 'static {
    /// Stable action kind used by policy diagnostics and grant metadata.
    fn kind(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Human-readable operation within the coarse execution plane.
    fn operation(&self) -> String {
        self.kind().to_owned()
    }

    /// Optional resource label for audit records, such as a path, URL, or
    /// process command.
    fn audit_resource(&self) -> Option<String> {
        None
    }

    /// Coarse runtime lane for existing audit and dispatch taxonomy.
    ///
    /// This is deliberately not the concrete side-effect domain. For example,
    /// filesystem and process actions that originate from built-in tools both
    /// report `ExecutionPlane::Tool`; their specific policy surface comes from
    /// the action type, operation, resource, and required capabilities.
    fn execution_plane(&self) -> ExecutionPlane;

    /// Core-vs-extension tier within the coarse execution plane.
    fn plane_tier(&self) -> PlaneTier {
        PlaneTier::Core
    }

    /// Capabilities required before this action may be granted.
    fn required_capabilities(&self) -> BTreeSet<Capability>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActionGrantId(u64);

impl ActionGrantId {
    #[must_use]
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionGrantRecord {
    grant_id: ActionGrantId,
    action_kind: &'static str,
    plane: ExecutionPlane,
    tier: PlaneTier,
    operation: String,
    resource: Option<String>,
    required_capabilities: BTreeSet<Capability>,
}

impl ActionGrantRecord {
    #[must_use]
    pub fn for_action<A: Action>(grant_id: ActionGrantId, action: &A) -> Self {
        Self {
            grant_id,
            action_kind: action.kind(),
            plane: action.execution_plane(),
            tier: action.plane_tier(),
            operation: action.operation(),
            resource: action.audit_resource(),
            required_capabilities: action.required_capabilities(),
        }
    }

    #[must_use]
    pub const fn grant_id(&self) -> ActionGrantId {
        self.grant_id
    }

    #[must_use]
    pub const fn action_kind(&self) -> &'static str {
        self.action_kind
    }

    #[must_use]
    pub const fn plane(&self) -> ExecutionPlane {
        self.plane
    }

    #[must_use]
    pub const fn tier(&self) -> PlaneTier {
        self.tier
    }

    #[must_use]
    pub fn operation(&self) -> &str {
        &self.operation
    }

    #[must_use]
    pub fn resource(&self) -> Option<&str> {
        self.resource.as_deref()
    }

    #[must_use]
    pub const fn required_capabilities(&self) -> &BTreeSet<Capability> {
        &self.required_capabilities
    }
}

/// Authorization represented as a value.
///
/// The constructor is crate-private on purpose. External tool/access code can
/// inspect and execute a grant, but cannot forge one.
pub struct Granted<A: Action> {
    record: ActionGrantRecord,
    action: A,
}

impl<A: Action> Granted<A> {
    #[cfg(test)]
    fn new(record: ActionGrantRecord, action: A) -> Self {
        Self { record, action }
    }

    #[must_use]
    pub fn grant_id(&self) -> ActionGrantId {
        self.record.grant_id()
    }

    #[must_use]
    pub const fn record(&self) -> &ActionGrantRecord {
        &self.record
    }

    #[must_use]
    pub const fn action(&self) -> &A {
        &self.action
    }

    #[must_use]
    pub fn into_action(self) -> A {
        self.action
    }

    pub async fn execute_with<E>(self, executor: &E) -> Result<E::Output, KernelError>
    where
        E: ActionExecutor<A> + ?Sized,
    {
        executor.execute(self).await
    }
}

#[async_trait]
pub trait ActionExecutor<A: Action>: Send + Sync {
    type Output;

    async fn execute(&self, granted: Granted<A>) -> Result<Self::Output, KernelError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestReadAction;

    impl Action for TestReadAction {
        fn operation(&self) -> String {
            "test.read".to_owned()
        }

        fn audit_resource(&self) -> Option<String> {
            Some("/workspace/file.txt".to_owned())
        }

        fn execution_plane(&self) -> ExecutionPlane {
            ExecutionPlane::Tool
        }

        fn required_capabilities(&self) -> BTreeSet<Capability> {
            BTreeSet::from([Capability::FilesystemRead])
        }
    }

    struct EchoExecutor;

    #[async_trait]
    impl ActionExecutor<TestReadAction> for EchoExecutor {
        type Output = String;

        async fn execute(
            &self,
            granted: Granted<TestReadAction>,
        ) -> Result<Self::Output, KernelError> {
            Ok(format!(
                "{}:{}",
                granted.record().operation(),
                granted.record().grant_id().get()
            ))
        }
    }

    #[test]
    fn action_grant_record_captures_action_metadata() {
        let action = TestReadAction;
        let record = ActionGrantRecord::for_action(ActionGrantId::from_raw(7), &action);

        assert_eq!(record.grant_id().get(), 7);
        assert_eq!(record.operation(), "test.read");
        assert_eq!(record.resource(), Some("/workspace/file.txt"));
        assert_eq!(record.plane(), ExecutionPlane::Tool);
        assert_eq!(record.tier(), PlaneTier::Core);
        assert!(
            record
                .required_capabilities()
                .contains(&Capability::FilesystemRead)
        );
    }

    #[tokio::test]
    async fn granted_action_executes_through_executor() {
        let action = TestReadAction;
        let record = ActionGrantRecord::for_action(ActionGrantId::from_raw(11), &action);
        let granted = Granted::new(record, action);

        let result = granted.execute_with(&EchoExecutor).await;

        assert_eq!(result, Ok("test.read:11".to_owned()));
    }
}
