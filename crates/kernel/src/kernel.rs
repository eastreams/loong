use serde_json::Value;
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
    errors::{KernelError, PolicyError},
    harness::{HarnessAdapter, HarnessBroker},
    memory::{
        CoreMemoryAdapter, MemoryCoreOutcome, MemoryCoreRequest, MemoryExtensionAdapter,
        MemoryExtensionOutcome, MemoryExtensionRequest, MemoryPlane,
    },
    pack::VerticalPackManifest,
    policy::{PolicyContext, PolicyDecision, PolicyEngine, PolicyRequest},
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

pub struct LoongClawKernel<P: PolicyEngine> {
    policy: P,
    packs: BTreeMap<String, VerticalPackManifest>,
    namespaces: BTreeMap<String, loongclaw_contracts::Namespace>,
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

impl<P: PolicyEngine> LoongClawKernel<P> {
    #[must_use]
    pub fn new(policy: P) -> Self {
        Self::with_runtime(policy, Arc::new(SystemClock), Arc::new(NoopAuditSink))
    }

    #[must_use]
    pub fn with_runtime(policy: P, clock: Arc<dyn Clock>, audit: Arc<dyn AuditSink>) -> Self {
        Self {
            policy,
            packs: BTreeMap::new(),
            namespaces: BTreeMap::new(),
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
        let namespace = loongclaw_contracts::Namespace {
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

    pub fn get_namespace(&self, pack_id: &str) -> Option<&loongclaw_contracts::Namespace> {
        self.namespaces.get(pack_id)
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

    pub fn revoke_generation(&self, below: u64) {
        self.policy.revoke_generation(below);
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

        self.record_plane_invocation(PlaneInvocationRecord {
            timestamp_epoch_s: now,
            agent_id: &token.agent_id,
            pack_id: &pack.pack_id,
            plane: ExecutionPlane::Connector,
            tier: PlaneTier::Legacy,
            primary_adapter: connector_name.clone(),
            delegated_core_adapter: None,
            operation,
            required_capabilities: &required_capabilities,
        })?;

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
        self.enforce_tool_policy(
            pack,
            token,
            now,
            required_capabilities,
            &tool_name,
            &request.payload,
        )?;
        let outcome = self
            .tool_plane
            .execute_core(core_name, request)
            .await
            .map_err(KernelError::from)?;

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
        self.enforce_tool_policy(
            pack,
            token,
            now,
            required_capabilities,
            &action,
            &request.payload,
        )?;
        let outcome = self
            .tool_plane
            .execute_extension(extension_name, core_name, request)
            .await
            .map_err(KernelError::from)?;

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

    fn enforce_tool_policy(
        &self,
        pack: &VerticalPackManifest,
        token: &CapabilityToken,
        now_epoch_s: u64,
        required_capabilities: &BTreeSet<Capability>,
        tool_name: &str,
        parameters: &Value,
    ) -> Result<(), KernelError> {
        let policy_request = PolicyRequest {
            tool_name: tool_name.to_owned(),
            parameters: parameters.clone(),
            pack_id: pack.pack_id.clone(),
            agent_id: token.agent_id.clone(),
            capabilities_used: required_capabilities.iter().copied().collect(),
            context: PolicyContext::default(),
        };

        match self.policy.check_tool_call(&policy_request) {
            PolicyDecision::Allow => Ok(()),
            PolicyDecision::Deny(reason) => {
                let error = PolicyError::ToolCallDenied {
                    tool_name: tool_name.to_owned(),
                    reason,
                };
                self.audit.record(self.new_event(
                    now_epoch_s,
                    Some(token.agent_id.clone()),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: pack.pack_id.clone(),
                        token_id: token.token_id.clone(),
                        reason: error.to_string(),
                    },
                ))?;
                Err(KernelError::Policy(error))
            }
            PolicyDecision::RequireApproval(prompt) => {
                let error = PolicyError::ToolCallApprovalRequired {
                    tool_name: tool_name.to_owned(),
                    prompt,
                };
                self.audit.record(self.new_event(
                    now_epoch_s,
                    Some(token.agent_id.clone()),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: pack.pack_id.clone(),
                        token_id: token.token_id.clone(),
                        reason: error.to_string(),
                    },
                ))?;
                Err(KernelError::Policy(error))
            }
        }
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
