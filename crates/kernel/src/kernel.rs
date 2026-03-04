use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use crate::{
    audit::{AuditEvent, AuditEventKind, AuditSink, ExecutionPlane, NoopAuditSink, PlaneTier},
    clock::{Clock, SystemClock},
    connector::{
        ConnectorAdapter, ConnectorExtensionAdapter, ConnectorPlane, ConnectorRegistry,
        CoreConnectorAdapter,
    },
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
    policy::PolicyEngine,
    policy_ext::{PolicyExtension, PolicyExtensionChain, PolicyExtensionContext},
    runtime::{
        CoreRuntimeAdapter, RuntimeCoreOutcome, RuntimeCoreRequest, RuntimeExtensionAdapter,
        RuntimeExtensionOutcome, RuntimeExtensionRequest, RuntimePlane,
    },
    tool::{
        CoreToolAdapter, ToolCoreOutcome, ToolCoreRequest, ToolExtensionAdapter,
        ToolExtensionOutcome, ToolExtensionRequest, ToolPlane,
    },
};

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

pub struct ChumosKernel<P: PolicyEngine> {
    policy: P,
    packs: BTreeMap<String, VerticalPackManifest>,
    harness: HarnessBroker,
    connectors: ConnectorRegistry,
    connector_plane: ConnectorPlane,
    runtime_plane: RuntimePlane,
    tool_plane: ToolPlane,
    memory_plane: MemoryPlane,
    policy_extensions: PolicyExtensionChain,
    clock: Arc<dyn Clock>,
    audit: Arc<dyn AuditSink>,
    event_seq: AtomicU64,
}

impl<P: PolicyEngine> ChumosKernel<P> {
    #[must_use]
    pub fn new(policy: P) -> Self {
        Self::with_runtime(policy, Arc::new(SystemClock), Arc::new(NoopAuditSink))
    }

    #[must_use]
    pub fn with_runtime(policy: P, clock: Arc<dyn Clock>, audit: Arc<dyn AuditSink>) -> Self {
        Self {
            policy,
            packs: BTreeMap::new(),
            harness: HarnessBroker::new(),
            connectors: ConnectorRegistry::new(),
            connector_plane: ConnectorPlane::new(),
            runtime_plane: RuntimePlane::new(),
            tool_plane: ToolPlane::new(),
            memory_plane: MemoryPlane::new(),
            policy_extensions: PolicyExtensionChain::new(),
            clock,
            audit,
            event_seq: AtomicU64::new(0),
        }
    }

    pub fn register_pack(&mut self, pack: VerticalPackManifest) -> Result<(), KernelError> {
        pack.validate()?;
        if self.packs.contains_key(&pack.pack_id) {
            return Err(KernelError::DuplicatePack(pack.pack_id));
        }
        self.packs.insert(pack.pack_id.clone(), pack);
        Ok(())
    }

    pub fn register_policy_extension<E: PolicyExtension + 'static>(&mut self, extension: E) {
        self.policy_extensions.register(extension);
    }

    pub fn register_harness_adapter<A: HarnessAdapter + 'static>(&mut self, adapter: A) {
        self.harness.register(adapter);
    }

    pub fn register_connector<A: ConnectorAdapter + 'static>(&mut self, connector: A) {
        self.connectors.register(connector);
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
        self.connector_plane
            .set_default_core_adapter(name)
            .map_err(KernelError::from)
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
        self.runtime_plane
            .set_default_core_adapter(name)
            .map_err(KernelError::from)
    }

    pub fn register_core_tool_adapter<A: CoreToolAdapter + 'static>(&mut self, adapter: A) {
        self.tool_plane.register_core_adapter(adapter);
    }

    pub fn register_tool_extension_adapter<A: ToolExtensionAdapter + 'static>(
        &mut self,
        adapter: A,
    ) {
        self.tool_plane.register_extension_adapter(adapter);
    }

    pub fn set_default_core_tool_adapter(&mut self, name: &str) -> Result<(), KernelError> {
        self.tool_plane
            .set_default_core_adapter(name)
            .map_err(KernelError::from)
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
        self.memory_plane
            .set_default_core_adapter(name)
            .map_err(KernelError::from)
    }

    pub fn issue_token(
        &self,
        pack_id: &str,
        agent_id: &str,
        ttl_s: u64,
    ) -> Result<CapabilityToken, KernelError> {
        let pack = self
            .packs
            .get(pack_id)
            .ok_or_else(|| KernelError::PackNotFound(pack_id.to_owned()))?;

        let now = self.clock.now_epoch_s();
        let token = self.policy.issue_token(pack, agent_id, now, ttl_s)?;

        self.audit.record(self.new_event(
            now,
            Some(agent_id.to_owned()),
            AuditEventKind::TokenIssued {
                token: token.clone(),
            },
        ))?;

        Ok(token)
    }

    pub fn revoke_token(
        &self,
        token_id: &str,
        actor_agent_id: Option<&str>,
    ) -> Result<(), KernelError> {
        self.policy.revoke_token(token_id)?;
        self.audit.record(self.new_event(
            self.clock.now_epoch_s(),
            actor_agent_id.map(std::string::ToString::to_string),
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

    pub async fn execute_task(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        task: TaskIntent,
    ) -> Result<KernelDispatch, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self.authorize_pack_operation(pack, token, &task.required_capabilities)?;

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

    pub async fn invoke_connector(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        command: ConnectorCommand,
    ) -> Result<ConnectorDispatch, KernelError> {
        let pack = self.get_pack(pack_id)?;
        self.assert_connector_allowed(pack, &command.connector_name)?;
        let now = self.authorize_pack_operation(pack, token, &command.required_capabilities)?;

        let connector_name = command.connector_name.clone();
        let operation = command.operation.clone();
        let required_capabilities = command.required_capabilities.clone();
        let outcome = self.connectors.invoke(command).await?;

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

        self.record_plane_invocation(
            now,
            &token.agent_id,
            &pack.pack_id,
            ExecutionPlane::Connector,
            PlaneTier::Legacy,
            connector_name.clone(),
            None,
            operation,
            &required_capabilities,
        )?;

        Ok(ConnectorDispatch {
            connector_name,
            outcome,
        })
    }

    pub async fn execute_connector_core(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        core_name: Option<&str>,
        command: ConnectorCommand,
    ) -> Result<ConnectorDispatch, KernelError> {
        let pack = self.get_pack(pack_id)?;
        self.assert_connector_allowed(pack, &command.connector_name)?;
        let now = self.authorize_pack_operation(pack, token, &command.required_capabilities)?;
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

        self.record_plane_invocation(
            now,
            &token.agent_id,
            &pack.pack_id,
            ExecutionPlane::Connector,
            PlaneTier::Core,
            resolved_core_adapter,
            None,
            operation,
            &required_capabilities,
        )?;

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
    ) -> Result<ConnectorDispatch, KernelError> {
        let pack = self.get_pack(pack_id)?;
        self.assert_connector_allowed(pack, &command.connector_name)?;
        let now = self.authorize_pack_operation(pack, token, &command.required_capabilities)?;
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

        self.record_plane_invocation(
            now,
            &token.agent_id,
            &pack.pack_id,
            ExecutionPlane::Connector,
            PlaneTier::Extension,
            extension_name.to_owned(),
            Some(resolved_core_adapter),
            operation,
            &required_capabilities,
        )?;

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
    ) -> Result<RuntimeCoreOutcome, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self.authorize_pack_operation(pack, token, required_capabilities)?;
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
            .execute_core(core_name, request)
            .await
            .map_err(KernelError::from)?;

        self.record_plane_invocation(
            now,
            &token.agent_id,
            &pack.pack_id,
            ExecutionPlane::Runtime,
            PlaneTier::Core,
            resolved_core_adapter,
            None,
            action,
            required_capabilities,
        )?;

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
    ) -> Result<RuntimeExtensionOutcome, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self.authorize_pack_operation(pack, token, required_capabilities)?;
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
            .await
            .map_err(KernelError::from)?;

        self.record_plane_invocation(
            now,
            &token.agent_id,
            &pack.pack_id,
            ExecutionPlane::Runtime,
            PlaneTier::Extension,
            extension_name.to_owned(),
            Some(resolved_core_adapter),
            action,
            required_capabilities,
        )?;

        Ok(outcome)
    }

    pub async fn execute_tool_core(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        required_capabilities: &BTreeSet<Capability>,
        core_name: Option<&str>,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self.authorize_pack_operation(pack, token, required_capabilities)?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.tool_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());
        let tool_name = request.tool_name.clone();
        let outcome = self
            .tool_plane
            .execute_core(core_name, request)
            .await
            .map_err(KernelError::from)?;

        self.record_plane_invocation(
            now,
            &token.agent_id,
            &pack.pack_id,
            ExecutionPlane::Tool,
            PlaneTier::Core,
            resolved_core_adapter,
            None,
            tool_name,
            required_capabilities,
        )?;

        Ok(outcome)
    }

    pub async fn execute_tool_extension(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        required_capabilities: &BTreeSet<Capability>,
        extension_name: &str,
        core_name: Option<&str>,
        request: ToolExtensionRequest,
    ) -> Result<ToolExtensionOutcome, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self.authorize_pack_operation(pack, token, required_capabilities)?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.tool_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());
        let action = request.extension_action.clone();
        let outcome = self
            .tool_plane
            .execute_extension(extension_name, core_name, request)
            .await
            .map_err(KernelError::from)?;

        self.record_plane_invocation(
            now,
            &token.agent_id,
            &pack.pack_id,
            ExecutionPlane::Tool,
            PlaneTier::Extension,
            extension_name.to_owned(),
            Some(resolved_core_adapter),
            action,
            required_capabilities,
        )?;

        Ok(outcome)
    }

    pub async fn execute_memory_core(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        required_capabilities: &BTreeSet<Capability>,
        core_name: Option<&str>,
        request: MemoryCoreRequest,
    ) -> Result<MemoryCoreOutcome, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self.authorize_pack_operation(pack, token, required_capabilities)?;
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
            .execute_core(core_name, request)
            .await
            .map_err(KernelError::from)?;

        self.record_plane_invocation(
            now,
            &token.agent_id,
            &pack.pack_id,
            ExecutionPlane::Memory,
            PlaneTier::Core,
            resolved_core_adapter,
            None,
            operation,
            required_capabilities,
        )?;

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
    ) -> Result<MemoryExtensionOutcome, KernelError> {
        let pack = self.get_pack(pack_id)?;
        let now = self.authorize_pack_operation(pack, token, required_capabilities)?;
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
            .await
            .map_err(KernelError::from)?;

        self.record_plane_invocation(
            now,
            &token.agent_id,
            &pack.pack_id,
            ExecutionPlane::Memory,
            PlaneTier::Extension,
            extension_name.to_owned(),
            Some(resolved_core_adapter),
            operation,
            required_capabilities,
        )?;

        Ok(outcome)
    }

    fn get_pack(&self, pack_id: &str) -> Result<&VerticalPackManifest, KernelError> {
        self.packs
            .get(pack_id)
            .ok_or_else(|| KernelError::PackNotFound(pack_id.to_owned()))
    }

    fn authorize_pack_operation(
        &self,
        pack: &VerticalPackManifest,
        token: &CapabilityToken,
        required_capabilities: &BTreeSet<Capability>,
    ) -> Result<u64, KernelError> {
        self.assert_pack_grants(pack, required_capabilities)?;
        let now = self.clock.now_epoch_s();
        self.authorize_or_audit_denial(pack, token, now, required_capabilities)?;
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
        timestamp_epoch_s: u64,
        agent_id: &str,
        pack_id: &str,
        plane: ExecutionPlane,
        tier: PlaneTier,
        primary_adapter: String,
        delegated_core_adapter: Option<String>,
        operation: String,
        required_capabilities: &BTreeSet<Capability>,
    ) -> Result<(), KernelError> {
        self.audit.record(self.new_event(
            timestamp_epoch_s,
            Some(agent_id.to_owned()),
            AuditEventKind::PlaneInvoked {
                pack_id: pack_id.to_owned(),
                plane,
                tier,
                primary_adapter,
                delegated_core_adapter,
                operation,
                required_capabilities: required_capabilities.iter().copied().collect(),
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

    fn authorize_or_audit_denial(
        &self,
        pack: &VerticalPackManifest,
        token: &CapabilityToken,
        now_epoch_s: u64,
        required_capabilities: &BTreeSet<Capability>,
    ) -> Result<(), KernelError> {
        if let Err(policy_error) =
            self.policy
                .authorize(token, &pack.pack_id, now_epoch_s, required_capabilities)
        {
            self.audit.record(self.new_event(
                now_epoch_s,
                Some(token.agent_id.clone()),
                AuditEventKind::AuthorizationDenied {
                    pack_id: pack.pack_id.clone(),
                    token_id: token.token_id.clone(),
                    reason: policy_error.to_string(),
                },
            ))?;
            return Err(KernelError::Policy(policy_error));
        }

        if let Err(policy_error) = self.policy_extensions.authorize(&PolicyExtensionContext {
            pack,
            token,
            now_epoch_s,
            required_capabilities,
        }) {
            self.audit.record(self.new_event(
                now_epoch_s,
                Some(token.agent_id.clone()),
                AuditEventKind::AuthorizationDenied {
                    pack_id: pack.pack_id.clone(),
                    token_id: token.token_id.clone(),
                    reason: policy_error.to_string(),
                },
            ))?;
            return Err(KernelError::Policy(policy_error));
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
