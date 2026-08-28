//! Tool registry: type-erased tool handles, the concrete [`ToolRegistry`]
//! host the agent invokes, and the immutable [`ToolSnapshot`] readers use.

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use contracts::tool::ToolSpec;
use kernel::Facade;
use serde_json::Value;

use crate::tool::{RegistrationError, ToolContext, ToolError, ToolImpl};

/// Immutable index of registered tools shared by snapshots.
type ToolIndex = BTreeMap<String, Arc<RegisteredTool>>;

/// Registered metadata for a tool.
#[derive(Debug, Clone)]
pub struct ToolRegistration {
    spec: ToolSpec,
}

impl ToolRegistration {
    fn new(spec: ToolSpec) -> Result<Self, RegistrationError> {
        if spec.name.is_empty() {
            return Err(RegistrationError::InvalidSpec);
        }
        Ok(Self { spec })
    }

    #[must_use]
    pub fn spec(&self) -> &ToolSpec {
        &self.spec
    }
}

#[async_trait]
trait ToolAdapter: Send + Sync {
    async fn invoke(&self, ctx: &ToolContext<'_>, payload: Value) -> Result<Value, ToolError>;
}

/// Type-erased tool handle stored in the registry.
pub struct RegisteredTool {
    registration: ToolRegistration,
    adapter: Box<dyn ToolAdapter>,
}

impl RegisteredTool {
    pub fn from_tool<T: ToolImpl>(tool: T) -> Result<Self, RegistrationError> {
        let registration = ToolRegistration::new(tool.spec())?;
        let adapter = Box::new(CoreToolAdapter::new(tool));
        Ok(Self {
            registration,
            adapter,
        })
    }

    #[must_use]
    pub fn registration(&self) -> &ToolRegistration {
        &self.registration
    }

    #[must_use]
    pub fn spec(&self) -> &ToolSpec {
        self.registration.spec()
    }

    pub async fn invoke(&self, ctx: &ToolContext<'_>, payload: Value) -> Result<Value, ToolError> {
        self.adapter.invoke(ctx, payload).await
    }
}

struct CoreToolAdapter<T> {
    inner: T,
}

impl<T: ToolImpl> CoreToolAdapter<T> {
    fn new(inner: T) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<T: ToolImpl> ToolAdapter for CoreToolAdapter<T> {
    async fn invoke(&self, ctx: &ToolContext<'_>, payload: Value) -> Result<Value, ToolError> {
        let input = self
            .inner
            .parse_input(payload)
            .map_err(ToolError::InvalidInput)?;
        let output = self
            .inner
            .execute(ctx, input)
            .await
            .map_err(|error| ToolError::Execution(Box::new(error)))?;
        serde_json::to_value(output).map_err(ToolError::Output)
    }
}

/// The concrete tool host for the loong agent.
///
/// The actor owns one registry and is the only writer. Register and unregister
/// therefore take `&mut self` and use copy-on-write so readers that already
/// took a snapshot keep their immutable view.
pub struct ToolRegistry {
    facade: Facade,
    tools: Arc<ToolIndex>,
}

impl ToolRegistry {
    #[must_use]
    pub fn new(facade: Facade) -> Self {
        Self {
            facade,
            tools: Arc::new(BTreeMap::new()),
        }
    }

    #[must_use]
    pub fn facade(&self) -> &Facade {
        &self.facade
    }

    /// Returns an immutable snapshot for stream tasks.
    ///
    /// The snapshot clones the current index pointer, not the map. Later
    /// registrations or removals publish a new index and never mutate the old
    /// one, so the snapshot stays valid for the whole prompt loop.
    #[must_use]
    pub fn snapshot(&self) -> ToolSnapshot {
        ToolSnapshot {
            facade: self.facade.clone(),
            tools: Arc::clone(&self.tools),
        }
    }

    pub fn register<T: ToolImpl>(
        &mut self,
        name: String,
        tool: T,
    ) -> Result<(), RegistrationError> {
        if self.tools.contains_key(&name) {
            return Err(RegistrationError::Duplicate(name));
        }
        let registered = Arc::new(RegisteredTool::from_tool(tool)?);

        let mut new_tools = (*self.tools).clone();
        new_tools.insert(name, registered);
        self.tools = Arc::new(new_tools);
        Ok(())
    }

    pub fn unregister(&mut self, name: &str) -> Option<Arc<RegisteredTool>> {
        let removed = self.tools.get(name).cloned()?;

        let mut new_tools = (*self.tools).clone();
        new_tools.remove(name);
        self.tools = Arc::new(new_tools);
        Some(removed)
    }

    #[must_use]
    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        self.snapshot().tool_specs()
    }

    pub async fn invoke(&self, name: &str, payload: Value) -> Result<Value, ToolError> {
        self.snapshot().invoke(name, payload).await
    }
}

/// Immutable, owned tool index snapshot passed into stream tasks.
///
/// Snapshots never change after they are created. They hold the facade needed
/// to build per-call tool contexts, so they can invoke tools without borrowing
/// the actor or the mutable registry.
pub struct ToolSnapshot {
    facade: Facade,
    tools: Arc<ToolIndex>,
}

impl ToolSnapshot {
    #[must_use]
    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        self.tools
            .values()
            .map(|registered| registered.spec().clone())
            .collect()
    }

    pub async fn invoke(&self, name: &str, payload: Value) -> Result<Value, ToolError> {
        let registered = self
            .tools
            .get(name)
            .cloned()
            .ok_or_else(|| ToolError::UnknownTool(name.to_owned()))?;
        let ctx = ToolContext::new(&self.facade);
        registered.invoke(&ctx, payload).await
    }
}
