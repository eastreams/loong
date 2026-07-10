use loong_core::policy::context::ContextFactory;
use loong_core::policy::engine::PolicyEngine;
use loong_core::policy::grant::ActionGrant;
use loong_core::tool::ToolInvocationAction;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{
    audit::{
        AuditEvent, AuditEventKind, AuditSink, ExecutionPlane, InMemoryAuditSink, NoopAuditSink,
        PlaneTier,
    },
    clock::{Clock, SystemClock},
    connector::{ConnectorExtensionAdapter, ConnectorPlane, CoreConnectorAdapter},
    contracts::{
        Capability, CapabilityToken, ConnectorCommand, ConnectorOutcome, HarnessRequest, TaskIntent,
    },
    errors::KernelError,
    harness::{HarnessAdapter, HarnessBroker},
    memory::{
        CoreMemoryAdapter, MemoryCoreOutcome, MemoryCoreRequest, MemoryExtensionAdapter,
        MemoryExtensionOutcome, MemoryExtensionRequest, MemoryPlane,
    },
    pack::VerticalPackManifest,
    policy::{KernelInvocationContext, LegacyKernelAction, PolicyPipeline, policy_engine_error},
    policy_ext::PolicyExtension,
    runtime::{
        CoreRuntimeAdapter, RuntimeCoreOutcome, RuntimeCoreRequest, RuntimeExtensionAdapter,
        RuntimeExtensionOutcome, RuntimeExtensionRequest, RuntimePlane,
    },
    tool::{
        CoreToolAdapter, LegacyToolPlane, ToolCoreOutcome, ToolCoreRequest, ToolExtensionAdapter,
        ToolExtensionOutcome, ToolExtensionRequest,
    },
};
use loong_contracts::{ToolInvocationOutcome, ToolPath};

#[derive(Debug, Clone, PartialEq)]
pub struct KernelDispatch {
    pub adapter_route: crate::contracts::ExecutionRoute,
    pub outcome: crate::contracts::HarnessOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorDispatch {
    pub connector_name: String,
    pub outcome: ConnectorOutcome,
}

struct PlaneInvocationRecord<'a> {
    timestamp_epoch_s: u64,
    agent_id: &'a str,
    pack_id: &'a str,
    plane: ExecutionPlane,
    tier: PlaneTier,
    primary_adapter: String,
    delegated_core_adapter: Option<String>,
    operation: String,
    required_capabilities: &'a BTreeSet<Capability>,
}

// TODO: methods should be implemented in trait from core
pub struct Kernel<C: ContextFactory> {
    policy: PolicyPipeline<C>,
    revoked_tokens: Mutex<BTreeSet<String>>,
    revoked_below_generation: AtomicU64,

    audit: Arc<dyn AuditSink>,

    legacy_tool_plane: LegacyToolPlane<C>,
    memory_plane: MemoryPlane,
    connector_plane: ConnectorPlane,
    runtime_plane: RuntimePlane,
    packs: BTreeMap<String, VerticalPackManifest>,
    namespaces: BTreeMap<String, loong_contracts::Namespace>,
    harness: HarnessBroker,
    clock: Arc<dyn Clock>,
    event_seq: AtomicU64,
}

impl<C> Kernel<C>
where
    C: ContextFactory,
{
    /// Safe convenience constructor for callers that do not need to customize
    /// runtime components. This defaults to in-memory audit rather than silent
    /// audit dropping.
    ///
    /// This constructs a bare kernel runtime. It does not register builtin
    /// adapters or the default pack; maintainers looking for the standard
    /// product/spec bootstrap path should start at `loong_spec::KernelBuilder`
    /// in `crates/spec/src/kernel_bootstrap.rs`.
    #[must_use]
    pub fn new() -> Self {
        Self::new_with_in_memory_audit().0
    }

    /// Construct a kernel with the default system clock and an inspectable
    /// in-memory audit sink.
    #[must_use]
    pub fn new_with_in_memory_audit() -> (Self, Arc<InMemoryAuditSink>) {
        let audit = Arc::new(InMemoryAuditSink::default());
        let kernel = Self::with_runtime(Arc::new(SystemClock), audit.clone());
        (kernel, audit)
    }

    /// Construct a kernel that intentionally discards audit events.
    ///
    /// This is reserved for narrow fixture paths where callers explicitly do
    /// not need audit assertions or evidence retention.
    #[must_use]
    pub fn new_without_audit() -> Self {
        Self::with_runtime(Arc::new(SystemClock), Arc::new(NoopAuditSink))
    }

    #[must_use]
    pub fn with_runtime(clock: Arc<dyn Clock>, audit: Arc<dyn AuditSink>) -> Self {
        Self::with_policy_runtime(PolicyPipeline::default(), clock, audit)
    }

    #[must_use]
    pub fn with_policy_runtime(
        policy: PolicyPipeline<C>,
        clock: Arc<dyn Clock>,
        audit: Arc<dyn AuditSink>,
    ) -> Self {
        Self {
            policy,
            packs: BTreeMap::new(),
            namespaces: BTreeMap::new(),
            harness: HarnessBroker::new(),
            connector_plane: ConnectorPlane::new(),
            runtime_plane: RuntimePlane::new(),
            legacy_tool_plane: LegacyToolPlane::new(),
            memory_plane: MemoryPlane::new(),
            revoked_tokens: Mutex::new(BTreeSet::new()),
            revoked_below_generation: AtomicU64::new(0),
            clock,
            audit,
            event_seq: AtomicU64::new(0),
        }
    }

    #[must_use]
    pub fn now_epoch_s(&self) -> u64 {
        self.clock.now_epoch_s()
    }
}

impl<C> Kernel<C>
where
    C: ContextFactory + Send + Sync,
    for<'a> C::Cx<'a>: KernelInvocationContext,
{
    pub fn register_pack(&mut self, pack: VerticalPackManifest) -> Result<(), KernelError> {
        pack.validate()?;
        if self.packs.contains_key(&pack.pack_id) {
            return Err(KernelError::DuplicatePack(pack.pack_id));
        }
        let namespace = loong_contracts::Namespace {
            pack_id: pack.pack_id.clone(),
            domain: pack.domain.clone(),
            membrane: pack.pack_id.clone(),
            default_route: pack.default_route.clone(),
            granted_capabilities: pack.granted_capabilities.clone(),
        };
        self.namespaces.insert(pack.pack_id.clone(), namespace);
        self.packs.insert(pack.pack_id.clone(), pack);
        Ok(())
    }

    pub fn get_namespace(&self, pack_id: &str) -> Option<&loong_contracts::Namespace> {
        self.namespaces.get(pack_id)
    }

    pub fn register_policy_extension<E: PolicyExtension + 'static>(&mut self, extension: E) {
        self.policy.register_policy_extension(extension);
    }

    pub fn register_harness_adapter<A: HarnessAdapter + 'static>(&mut self, adapter: A) {
        self.harness.register(adapter);
    }

    pub fn register_core_connector_adapter<A: CoreConnectorAdapter + 'static>(
        &mut self,
        adapter: A,
    ) {
        self.connector_plane.register_core_adapter(adapter);
    }

    pub fn register_connector_extension_adapter<A: ConnectorExtensionAdapter + 'static>(
        &mut self,
        adapter: A,
    ) {
        self.connector_plane.register_extension_adapter(adapter);
    }

    pub fn set_default_core_connector_adapter(&mut self, name: &str) -> Result<(), KernelError> {
        self.connector_plane.set_default_core_adapter(name)?;
        Ok(())
    }

    pub fn register_core_runtime_adapter<A: CoreRuntimeAdapter + 'static>(&mut self, adapter: A) {
        self.runtime_plane.register_core_adapter(adapter);
    }

    pub fn register_runtime_extension_adapter<A: RuntimeExtensionAdapter + 'static>(
        &mut self,
        adapter: A,
    ) {
        self.runtime_plane.register_extension_adapter(adapter);
    }

    pub fn set_default_core_runtime_adapter(&mut self, name: &str) -> Result<(), KernelError> {
        self.runtime_plane.set_default_core_adapter(name)?;
        Ok(())
    }

    /// Register an old core tool adapter.
    ///
    /// This is the compatibility path for tools that have not moved to the
    /// app-owned typed tool plane. New governed tools should not add another
    /// adapter here; they should be registered by app orchestration and call
    /// kernel only for authorization and audit.
    pub fn register_core_tool_adapter<A: CoreToolAdapter<C> + 'static>(&mut self, adapter: A) {
        self.legacy_tool_plane.register_core_adapter(adapter);
    }

    /// Register an old extension tool adapter.
    ///
    /// This remains only while unmigrated core/extension tools exist.
    pub fn register_tool_extension_adapter<A: ToolExtensionAdapter<C> + 'static>(
        &mut self,
        adapter: A,
    ) {
        self.legacy_tool_plane.register_extension_adapter(adapter);
    }

    pub fn set_default_core_tool_adapter(&mut self, name: &str) -> Result<(), KernelError> {
        self.legacy_tool_plane.set_default_core_adapter(name)?;
        Ok(())
    }

    /// Grant one app-owned typed tool invocation without executing it.
    ///
    /// Tool dispatch is an action in the policy pipeline. The grant only
    /// authorizes entering the `ToolImpl`; tool-internal side effects must
    /// request their own access grants.
    pub async fn grant_tool_invocation(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        action: ToolInvocationAction,
        policy_context: &C::Cx<'_>,
    ) -> Result<ActionGrant<ToolInvocationAction>, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let path = action.path().clone();
        let required_capabilities = action
            .required_capabilities()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        self.assert_pack_grants(pack, &required_capabilities)?;
        let now = policy_context.now_epoch_s();
        if let Err(policy_error) = self.authorize_token(pack, token, now, &required_capabilities) {
            self.record_tool_invocation_event(
                now,
                Some(token.agent_id.clone()),
                pack.pack_id.clone(),
                path,
                &required_capabilities,
                ToolInvocationOutcome::Denied {
                    reason: policy_error.to_string(),
                    report: None,
                },
            )?;
            return Err(KernelError::Policy(policy_error));
        }

        match self.policy.grant(policy_context, action).await {
            Ok(grant) => Ok(grant),
            Err(grant_error) => {
                let outcome = match &grant_error {
                    loong_core::PolicyGrantError::MissingCapability { capability } => {
                        ToolInvocationOutcome::Denied {
                            reason: format!("missing capability: {capability:?}"),
                            report: None,
                        }
                    }
                    loong_core::PolicyGrantError::Denied { report, reason } => {
                        ToolInvocationOutcome::Denied {
                            reason: reason.to_string(),
                            report: Some(report.clone()),
                        }
                    }
                };
                self.record_tool_invocation_event(
                    now,
                    Some(token.agent_id.clone()),
                    pack.pack_id.clone(),
                    path,
                    &required_capabilities,
                    outcome,
                )?;
                Err(KernelError::Policy(policy_engine_error(grant_error)))
            }
        }
    }

    /// Record the outcome of an app-owned tool invocation.
    ///
    /// Tool implementations never receive audit capability. App orchestration
    /// records the dispatch outcome here after consuming a tool invocation
    /// grant; legacy adapters keep `PlaneInvoked` until they are migrated.
    pub fn record_tool_invocation(
        &self,
        policy_context: &C::Cx<'_>,
        path: ToolPath,
        required_capabilities: &BTreeSet<Capability>,
        outcome: ToolInvocationOutcome,
    ) -> Result<(), KernelError> {
        self.record_tool_invocation_event(
            policy_context.now_epoch_s(),
            Some(policy_context.token().agent_id.clone()),
            policy_context.pack().pack_id.clone(),
            path,
            required_capabilities,
            outcome,
        )
    }

    pub fn register_core_memory_adapter<A: CoreMemoryAdapter + 'static>(&mut self, adapter: A) {
        self.memory_plane.register_core_adapter(adapter);
    }

    pub fn register_memory_extension_adapter<A: MemoryExtensionAdapter + 'static>(
        &mut self,
        adapter: A,
    ) {
        self.memory_plane.register_extension_adapter(adapter);
    }

    pub fn set_default_core_memory_adapter(&mut self, name: &str) -> Result<(), KernelError> {
        self.memory_plane.set_default_core_adapter(name)?;
        Ok(())
    }

    pub fn issue_token(
        &self,
        pack_id: &str,
        agent_id: &str,
        ttl_s: u64,
    ) -> Result<CapabilityToken, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let issued_at_epoch_s = self.clock.now_epoch_s();
        let generation = self.event_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let token = CapabilityToken {
            token_id: format!("tok-{generation:016x}"),
            pack_id: pack.pack_id.clone(),
            agent_id: agent_id.to_owned(),
            allowed_capabilities: pack.granted_capabilities.clone(),
            issued_at_epoch_s,
            expires_at_epoch_s: issued_at_epoch_s.saturating_add(ttl_s),
            generation,
        };

        self.audit.record(AuditEvent {
            event_id: format!("evt-{generation:016x}"),
            timestamp_epoch_s: issued_at_epoch_s,
            agent_id: Some(agent_id.to_owned()),
            kind: AuditEventKind::TokenIssued {
                token: token.clone(),
            },
        })?;

        Ok(token)
    }

    pub fn issue_scoped_token(
        &self,
        pack_id: &str,
        agent_id: &str,
        allowed_capabilities: &BTreeSet<Capability>,
        ttl_s: u64,
    ) -> Result<CapabilityToken, KernelError> {
        let pack = self.get_pack(pack_id)?;
        self.assert_pack_grants(pack, allowed_capabilities)?;
        let issued_at_epoch_s = self.clock.now_epoch_s();
        let generation = self.event_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let token = CapabilityToken {
            token_id: format!("tok-{generation:016x}"),
            pack_id: pack.pack_id.clone(),
            agent_id: agent_id.to_owned(),
            allowed_capabilities: allowed_capabilities.clone(),
            issued_at_epoch_s,
            expires_at_epoch_s: issued_at_epoch_s.saturating_add(ttl_s),
            generation,
        };

        self.audit.record(AuditEvent {
            event_id: format!("evt-{generation:016x}"),
            timestamp_epoch_s: issued_at_epoch_s,
            agent_id: Some(agent_id.to_owned()),
            kind: AuditEventKind::TokenIssued {
                token: token.clone(),
            },
        })?;

        Ok(token)
    }

    pub fn revoke_token(&self, token_id: &str, agent_id: Option<&str>) -> Result<(), KernelError> {
        self.revoked_tokens
            .lock()
            .map_err(|_err| {
                KernelError::Policy(crate::errors::PolicyError::RevokedToken {
                    token_id: token_id.to_owned(),
                })
            })?
            .insert(token_id.to_owned());
        let now = self.clock.now_epoch_s();
        self.audit.record(self.new_event(
            now,
            agent_id.map(std::string::ToString::to_string),
            AuditEventKind::TokenRevoked {
                token_id: token_id.to_owned(),
            },
        ))?;
        Ok(())
    }

    pub fn record_audit_event(
        &self,
        agent_id: Option<&str>,
        kind: AuditEventKind,
    ) -> Result<(), KernelError> {
        let now = self.clock.now_epoch_s();
        self.audit.record(self.new_event(
            now,
            agent_id.map(std::string::ToString::to_string),
            kind,
        ))?;
        Ok(())
    }

    pub async fn authorize_operation(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        plane: ExecutionPlane,
        tier: PlaneTier,
        primary_adapter: &str,
        delegated_core_adapter: Option<&str>,
        operation: &str,
        required_capabilities: &BTreeSet<Capability>,
        policy_context: &C::Cx<'_>,
    ) -> Result<(), KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self
            .authorize_pack_operation(
                policy_context,
                pack,
                token,
                operation,
                required_capabilities,
            )
            .await?;

        let primary_adapter = primary_adapter.to_owned();
        let delegated_core_adapter = delegated_core_adapter.map(std::string::ToString::to_string);
        let operation = operation.to_owned();
        let record = PlaneInvocationRecord {
            timestamp_epoch_s: now,
            agent_id: token.agent_id.as_str(),
            pack_id: pack.pack_id.as_str(),
            plane,
            tier,
            primary_adapter,
            delegated_core_adapter,
            operation,
            required_capabilities,
        };
        self.record_plane_invocation(record)?;
        Ok(())
    }

    pub async fn execute_task(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        task: TaskIntent,
        policy_context: &C::Cx<'_>,
    ) -> Result<KernelDispatch, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self
            .authorize_pack_operation(
                policy_context,
                pack,
                token,
                "execute_task",
                &task.required_capabilities,
            )
            .await?;

        let request = HarnessRequest {
            token_id: token.token_id.clone(),
            pack_id: pack.pack_id.clone(),
            agent_id: token.agent_id.clone(),
            task_id: task.task_id.clone(),
            objective: task.objective,
            payload: task.payload,
        };

        let route = pack.default_route.clone();
        let outcome = self.harness.execute(&route, request).await?;

        self.audit.record(self.new_event(
            now,
            Some(token.agent_id.clone()),
            AuditEventKind::TaskDispatched {
                pack_id: pack.pack_id.clone(),
                task_id: task.task_id,
                route: route.clone(),
                required_capabilities: task.required_capabilities.iter().copied().collect(),
            },
        ))?;

        Ok(KernelDispatch {
            adapter_route: route,
            outcome,
        })
    }

    pub async fn execute_connector_core(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        core_name: Option<&str>,
        command: ConnectorCommand,
        policy_context: &C::Cx<'_>,
    ) -> Result<ConnectorDispatch, KernelError> {
        let pack = self.get_pack(pack_id)?;
        self.assert_connector_allowed(pack, &command.connector_name)?;
        let now = self
            .authorize_pack_operation(
                policy_context,
                pack,
                token,
                &command.operation,
                &command.required_capabilities,
            )
            .await?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.connector_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());

        let connector_name = command.connector_name.clone();
        let operation = command.operation.clone();
        let required_capabilities = command.required_capabilities.clone();
        let outcome = self.connector_plane.invoke_core(core_name, command).await?;

        self.audit.record(self.new_event(
            now,
            Some(token.agent_id.clone()),
            AuditEventKind::ConnectorInvoked {
                pack_id: pack.pack_id.clone(),
                connector_name: connector_name.clone(),
                operation: operation.clone(),
                required_capabilities: required_capabilities.iter().copied().collect(),
            },
        ))?;

        self.record_plane_invocation(PlaneInvocationRecord {
            timestamp_epoch_s: now,
            agent_id: &token.agent_id,
            pack_id: &pack.pack_id,
            plane: ExecutionPlane::Connector,
            tier: PlaneTier::Core,
            primary_adapter: resolved_core_adapter,
            delegated_core_adapter: None,
            operation,
            required_capabilities: &required_capabilities,
        })?;

        Ok(ConnectorDispatch {
            connector_name,
            outcome,
        })
    }

    pub async fn execute_connector_extension(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        extension_name: &str,
        core_name: Option<&str>,
        command: ConnectorCommand,
        policy_context: &C::Cx<'_>,
    ) -> Result<ConnectorDispatch, KernelError> {
        let pack = self.get_pack(pack_id)?;
        self.assert_connector_allowed(pack, &command.connector_name)?;
        let now = self
            .authorize_pack_operation(
                policy_context,
                pack,
                token,
                &command.operation,
                &command.required_capabilities,
            )
            .await?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.connector_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());

        let connector_name = command.connector_name.clone();
        let operation = command.operation.clone();
        let required_capabilities = command.required_capabilities.clone();
        let outcome = self
            .connector_plane
            .invoke_extension(extension_name, core_name, command)
            .await?;

        self.audit.record(self.new_event(
            now,
            Some(token.agent_id.clone()),
            AuditEventKind::ConnectorInvoked {
                pack_id: pack.pack_id.clone(),
                connector_name: connector_name.clone(),
                operation: operation.clone(),
                required_capabilities: required_capabilities.iter().copied().collect(),
            },
        ))?;

        self.record_plane_invocation(PlaneInvocationRecord {
            timestamp_epoch_s: now,
            agent_id: &token.agent_id,
            pack_id: &pack.pack_id,
            plane: ExecutionPlane::Connector,
            tier: PlaneTier::Extension,
            primary_adapter: extension_name.to_owned(),
            delegated_core_adapter: Some(resolved_core_adapter),
            operation,
            required_capabilities: &required_capabilities,
        })?;

        Ok(ConnectorDispatch {
            connector_name,
            outcome,
        })
    }

    pub async fn execute_runtime_core(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        required_capabilities: &BTreeSet<Capability>,
        core_name: Option<&str>,
        request: RuntimeCoreRequest,
        policy_context: &C::Cx<'_>,
    ) -> Result<RuntimeCoreOutcome, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self
            .authorize_pack_operation(
                policy_context,
                pack,
                token,
                &request.action,
                required_capabilities,
            )
            .await?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.runtime_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());
        let action = request.action.clone();
        let outcome = self.runtime_plane.execute_core(core_name, request).await?;

        self.record_plane_invocation(PlaneInvocationRecord {
            timestamp_epoch_s: now,
            agent_id: &token.agent_id,
            pack_id: &pack.pack_id,
            plane: ExecutionPlane::Runtime,
            tier: PlaneTier::Core,
            primary_adapter: resolved_core_adapter,
            delegated_core_adapter: None,
            operation: action,
            required_capabilities,
        })?;

        Ok(outcome)
    }

    pub async fn execute_runtime_extension(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        required_capabilities: &BTreeSet<Capability>,
        extension_name: &str,
        core_name: Option<&str>,
        request: RuntimeExtensionRequest,
        policy_context: &C::Cx<'_>,
    ) -> Result<RuntimeExtensionOutcome, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self
            .authorize_pack_operation(
                policy_context,
                pack,
                token,
                &request.action,
                required_capabilities,
            )
            .await?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.runtime_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());
        let action = request.action.clone();
        let outcome = self
            .runtime_plane
            .execute_extension(extension_name, core_name, request)
            .await?;

        self.record_plane_invocation(PlaneInvocationRecord {
            timestamp_epoch_s: now,
            agent_id: &token.agent_id,
            pack_id: &pack.pack_id,
            plane: ExecutionPlane::Runtime,
            tier: PlaneTier::Extension,
            primary_adapter: extension_name.to_owned(),
            delegated_core_adapter: Some(resolved_core_adapter),
            operation: action,
            required_capabilities,
        })?;

        Ok(outcome)
    }

    /// Execute one core tool call through the legacy adapter plane.
    ///
    /// This is the temporary compatibility entry point for unmigrated tools.
    /// The kernel authorizes the caller for the requested tool, then hands the
    /// same unified context to the adapter. Access-backed tools must call
    /// `ctx.access().fs().read_file(...)`; the adapter should not perform the
    /// protected side effect itself.
    pub async fn execute_tool_core<'a>(
        &'a self,
        pack_id: &str,
        token: &CapabilityToken,
        required_capabilities: &BTreeSet<Capability>,
        core_name: Option<&str>,
        request: ToolCoreRequest,
        policy_context: C::Cx<'a>,
    ) -> Result<ToolCoreOutcome, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self
            .authorize_pack_operation(
                &policy_context,
                pack,
                token,
                &request.tool_name,
                required_capabilities,
            )
            .await?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.legacy_tool_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());
        let tool_name = request.tool_name.clone();
        let outcome = self
            .legacy_tool_plane
            .execute_core_with_context(core_name, request, &policy_context)
            .await?;

        self.record_plane_invocation(PlaneInvocationRecord {
            timestamp_epoch_s: now,
            agent_id: &token.agent_id,
            pack_id: &pack.pack_id,
            plane: ExecutionPlane::Tool,
            tier: PlaneTier::Core,
            primary_adapter: resolved_core_adapter,
            delegated_core_adapter: None,
            operation: tool_name,
            required_capabilities,
        })?;

        Ok(outcome)
    }

    /// Execute one extension tool call through the legacy adapter plane.
    pub async fn execute_tool_extension(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        required_capabilities: &BTreeSet<Capability>,
        extension_name: &str,
        core_name: Option<&str>,
        request: ToolExtensionRequest,
        policy_context: &C::Cx<'_>,
    ) -> Result<ToolExtensionOutcome, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self
            .authorize_pack_operation(
                policy_context,
                pack,
                token,
                &request.extension_action,
                required_capabilities,
            )
            .await?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.legacy_tool_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());
        let action = request.extension_action.clone();
        let outcome = self
            .legacy_tool_plane
            .execute_extension(extension_name, core_name, request)
            .await?;

        self.record_plane_invocation(PlaneInvocationRecord {
            timestamp_epoch_s: now,
            agent_id: &token.agent_id,
            pack_id: &pack.pack_id,
            plane: ExecutionPlane::Tool,
            tier: PlaneTier::Extension,
            primary_adapter: extension_name.to_owned(),
            delegated_core_adapter: Some(resolved_core_adapter),
            operation: action,
            required_capabilities,
        })?;

        Ok(outcome)
    }

    pub async fn execute_memory_core(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        required_capabilities: &BTreeSet<Capability>,
        core_name: Option<&str>,
        request: MemoryCoreRequest,
        policy_context: &C::Cx<'_>,
    ) -> Result<MemoryCoreOutcome, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self
            .authorize_pack_operation(
                policy_context,
                pack,
                token,
                &request.operation,
                required_capabilities,
            )
            .await?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.memory_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());
        let operation = request.operation.clone();
        let outcome = self.memory_plane.execute_core(core_name, request).await?;

        self.record_plane_invocation(PlaneInvocationRecord {
            timestamp_epoch_s: now,
            agent_id: &token.agent_id,
            pack_id: &pack.pack_id,
            plane: ExecutionPlane::Memory,
            tier: PlaneTier::Core,
            primary_adapter: resolved_core_adapter,
            delegated_core_adapter: None,
            operation,
            required_capabilities,
        })?;

        Ok(outcome)
    }

    pub async fn execute_memory_extension(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        required_capabilities: &BTreeSet<Capability>,
        extension_name: &str,
        core_name: Option<&str>,
        request: MemoryExtensionRequest,
        policy_context: &C::Cx<'_>,
    ) -> Result<MemoryExtensionOutcome, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self
            .authorize_pack_operation(
                policy_context,
                pack,
                token,
                &request.operation,
                required_capabilities,
            )
            .await?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.memory_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());
        let operation = request.operation.clone();
        let outcome = self
            .memory_plane
            .execute_extension(extension_name, core_name, request)
            .await?;

        self.record_plane_invocation(PlaneInvocationRecord {
            timestamp_epoch_s: now,
            agent_id: &token.agent_id,
            pack_id: &pack.pack_id,
            plane: ExecutionPlane::Memory,
            tier: PlaneTier::Extension,
            primary_adapter: extension_name.to_owned(),
            delegated_core_adapter: Some(resolved_core_adapter),
            operation,
            required_capabilities,
        })?;

        Ok(outcome)
    }

    fn get_pack(&self, pack_id: &str) -> Result<&VerticalPackManifest, KernelError> {
        self.packs
            .get(pack_id)
            .ok_or_else(|| KernelError::PackNotFound(pack_id.to_owned()))
    }

    async fn authorize_pack_operation(
        &self,
        policy_context: &C::Cx<'_>,
        pack: &VerticalPackManifest,
        token: &CapabilityToken,
        operation: &str,
        required_capabilities: &BTreeSet<Capability>,
    ) -> Result<u64, KernelError> {
        self.assert_pack_grants(pack, required_capabilities)?;
        let now = policy_context.now_epoch_s();
        self.authorize_or_audit_denial(
            policy_context,
            pack,
            token,
            now,
            operation,
            required_capabilities,
        )
        .await?;
        Ok(now)
    }

    fn assert_connector_allowed(
        &self,
        pack: &VerticalPackManifest,
        connector_name: &str,
    ) -> Result<(), KernelError> {
        if !pack.allows_connector(connector_name) {
            return Err(KernelError::ConnectorNotAllowed {
                connector: connector_name.to_owned(),
                pack_id: pack.pack_id.clone(),
            });
        }
        Ok(())
    }

    fn record_plane_invocation(
        &self,
        record: PlaneInvocationRecord<'_>,
    ) -> Result<(), KernelError> {
        self.audit.record(self.new_event(
            record.timestamp_epoch_s,
            Some(record.agent_id.to_owned()),
            AuditEventKind::PlaneInvoked {
                pack_id: record.pack_id.to_owned(),
                plane: record.plane,
                tier: record.tier,
                primary_adapter: record.primary_adapter,
                delegated_core_adapter: record.delegated_core_adapter,
                operation: record.operation,
                required_capabilities: record.required_capabilities.iter().copied().collect(),
            },
        ))?;
        Ok(())
    }

    fn record_tool_invocation_event(
        &self,
        timestamp_epoch_s: u64,
        agent_id: Option<String>,
        pack_id: String,
        path: ToolPath,
        required_capabilities: &BTreeSet<Capability>,
        outcome: ToolInvocationOutcome,
    ) -> Result<(), KernelError> {
        self.audit.record(self.new_event(
            timestamp_epoch_s,
            agent_id,
            AuditEventKind::ToolInvocation {
                pack_id,
                path_display: path.to_string(),
                required_capabilities: required_capabilities.iter().copied().collect(),
                outcome,
            },
        ))?;
        Ok(())
    }

    fn assert_pack_grants(
        &self,
        pack: &VerticalPackManifest,
        required_capabilities: &BTreeSet<Capability>,
    ) -> Result<(), KernelError> {
        for capability in required_capabilities {
            if !pack.grants(*capability) {
                return Err(KernelError::PackCapabilityBoundary {
                    pack_id: pack.pack_id.clone(),
                    capability: *capability,
                });
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn record_tool_call_denial(
        &self,
        pack: &VerticalPackManifest,
        token: &CapabilityToken,
        now_epoch_s: u64,
        error: &crate::errors::PolicyError,
    ) -> Result<(), KernelError> {
        self.audit.record(self.new_event(
            now_epoch_s,
            Some(token.agent_id.clone()),
            AuditEventKind::AuthorizationDenied {
                pack_id: pack.pack_id.clone(),
                token_id: token.token_id.clone(),
                reason: error.to_string(),
            },
        ))?;
        Ok(())
    }

    fn record_authorization_denial(
        &self,
        pack: &VerticalPackManifest,
        token: &CapabilityToken,
        now_epoch_s: u64,
        error: &crate::errors::PolicyError,
    ) -> Result<(), KernelError> {
        self.audit.record(self.new_event(
            now_epoch_s,
            Some(token.agent_id.clone()),
            AuditEventKind::AuthorizationDenied {
                pack_id: pack.pack_id.clone(),
                token_id: token.token_id.clone(),
                reason: error.to_string(),
            },
        ))?;
        Ok(())
    }

    // TODO: deprecate this
    async fn authorize_or_audit_denial(
        &self,
        policy_context: &C::Cx<'_>,
        pack: &VerticalPackManifest,
        token: &CapabilityToken,
        now_epoch_s: u64,
        operation: &str,
        required_capabilities: &BTreeSet<Capability>,
    ) -> Result<(), KernelError> {
        if let Err(policy_error) =
            self.authorize_token(pack, token, now_epoch_s, required_capabilities)
        {
            self.record_authorization_denial(pack, token, now_epoch_s, &policy_error)?;
            return Err(KernelError::Policy(policy_error));
        }

        let action = LegacyKernelAction::new(operation, required_capabilities.clone());
        if let Err(policy_error) = self
            .policy
            .authorize_kernel_action(policy_context, action)
            .await
        {
            self.record_authorization_denial(pack, token, now_epoch_s, &policy_error)?;
            return Err(KernelError::Policy(policy_error));
        }

        Ok(())
    }

    fn authorize_token(
        &self,
        pack: &VerticalPackManifest,
        token: &CapabilityToken,
        now_epoch_s: u64,
        required_capabilities: &BTreeSet<Capability>,
    ) -> Result<(), crate::errors::PolicyError> {
        if self
            .revoked_tokens
            .lock()
            .map_err(|_err| crate::errors::PolicyError::RevokedToken {
                token_id: token.token_id.clone(),
            })?
            .contains(&token.token_id)
        {
            return Err(crate::errors::PolicyError::RevokedToken {
                token_id: token.token_id.clone(),
            });
        }

        let threshold = self.revoked_below_generation.load(Ordering::Relaxed);
        if token.generation > 0 && token.generation <= threshold {
            return Err(crate::errors::PolicyError::RevokedToken {
                token_id: token.token_id.clone(),
            });
        }

        if token.pack_id != pack.pack_id {
            return Err(crate::errors::PolicyError::PackMismatch {
                token_pack_id: token.pack_id.clone(),
                runtime_pack_id: pack.pack_id.clone(),
            });
        }

        if now_epoch_s > token.expires_at_epoch_s {
            return Err(crate::errors::PolicyError::ExpiredToken {
                token_id: token.token_id.clone(),
                expires_at_epoch_s: token.expires_at_epoch_s,
            });
        }

        for capability in required_capabilities {
            if !token.allowed_capabilities.contains(capability) {
                return Err(crate::errors::PolicyError::MissingCapability {
                    token_id: token.token_id.clone(),
                    capability: *capability,
                });
            }
        }

        Ok(())
    }

    fn new_event(
        &self,
        timestamp_epoch_s: u64,
        agent_id: Option<String>,
        kind: AuditEventKind,
    ) -> AuditEvent {
        let seq = self.event_seq.fetch_add(1, Ordering::Relaxed) + 1;
        AuditEvent {
            event_id: format!("evt-{seq:016x}"),
            timestamp_epoch_s,
            agent_id,
            kind,
        }
    }
}

#[cfg(test)]
mod tests;

impl<C> loong_core::kernel::Kernel<C> for Kernel<C>
where
    C: ContextFactory,
{
    type PolicyEngine = PolicyPipeline<C>;

    fn policy_engine(&self) -> &Self::PolicyEngine {
        &self.policy
    }
}
