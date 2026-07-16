use loong_core::policy::context::ContextFactory;
use loong_core::policy::engine::PolicyEngine;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{
    audit::{
        AuditEventKind, AuditSink, ExecutionPlane, InMemoryAuditSink, PlaneTier, SharedAuditState,
    },
    clock::{Clock, SystemClock},
    connector::{ConnectorExtensionAdapter, ConnectorPlane, CoreConnectorAdapter},
    contracts::{
        Capability, CapabilityToken, ConnectorCommand, ConnectorOutcome, HarnessRequest, TaskIntent,
    },
    errors::{AuditError, KernelError},
    harness::{HarnessAdapter, HarnessBroker},
    memory::{
        CoreMemoryAdapter, MemoryCoreOutcome, MemoryCoreRequest, MemoryExtensionAdapter,
        MemoryExtensionOutcome, MemoryExtensionRequest, MemoryPlane,
    },
    pack::VerticalPackManifest,
    policy::{LegacyKernelAction, PolicyPipeline, PolicyPipelineBuilder, policy_engine_error},
    runtime::{
        CoreRuntimeAdapter, RuntimeCoreOutcome, RuntimeCoreRequest, RuntimeExtensionAdapter,
        RuntimeExtensionOutcome, RuntimeExtensionRequest, RuntimePlane,
    },
    tool::{
        CoreToolAdapter, LegacyToolPlane, ToolCoreOutcome, ToolCoreRequest, ToolExtensionAdapter,
        ToolExtensionOutcome, ToolExtensionRequest,
    },
};

mod action_execution_audit;

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

// TODO(kernel-contract): split a stable governance trait from this concrete
// runtime only when an external caller needs that contract. Do not add helper
// traits that merely forward the same Kernel methods.
pub struct Kernel<C: ContextFactory> {
    policy: PolicyPipeline<C>,
    revoked_tokens: Mutex<BTreeSet<String>>,
    revoked_below_generation: AtomicU64,

    audit_state: Arc<SharedAuditState>,

    legacy_tool_plane: LegacyToolPlane<C>,
    memory_plane: MemoryPlane,
    connector_plane: ConnectorPlane,
    runtime_plane: RuntimePlane,
    packs: BTreeMap<String, VerticalPackManifest>,
    namespaces: BTreeMap<String, loong_contracts::Namespace>,
    harness: HarnessBroker,
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

    #[must_use]
    pub fn with_runtime(clock: Arc<dyn Clock>, audit: Arc<dyn AuditSink>) -> Self {
        Self::with_policy_runtime(PolicyPipelineBuilder::new(), clock, audit)
    }

    /// Construct a migration runtime for old adapter planes.
    ///
    /// New typed actions must install their own typed policies; this fallback
    /// only grants `LegacyKernelAction` so default construction stays deny-by-default.
    #[must_use]
    pub fn with_legacy_allow_runtime(clock: Arc<dyn Clock>, audit: Arc<dyn AuditSink>) -> Self {
        Self::with_policy_runtime(
            PolicyPipelineBuilder::new_legacy_allow_fallback(),
            clock,
            audit,
        )
    }

    #[must_use]
    pub fn with_policy_runtime(
        policy: PolicyPipelineBuilder<C>,
        clock: Arc<dyn Clock>,
        audit: Arc<dyn AuditSink>,
    ) -> Self {
        // Kernel is the only installation boundary: sharing one state prevents
        // authorization identity, event sequencing, and sink selection from diverging.
        let audit_state = Arc::new(SharedAuditState::new(clock, audit));
        let policy = PolicyPipeline::install(policy, audit_state.clone());
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
            audit_state,
        }
    }

    #[must_use]
    pub fn now_epoch_s(&self) -> u64 {
        self.audit_state.now_epoch_s()
    }
}

impl<C> Kernel<C>
where
    C: ContextFactory,
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
        let pack = self.pack_manifest(pack_id)?;
        self.mint_capability_token(
            &pack.pack_id,
            agent_id,
            pack.granted_capabilities.clone(),
            ttl_s,
        )
    }

    pub fn issue_scoped_token(
        &self,
        pack_id: &str,
        agent_id: &str,
        allowed_capabilities: &BTreeSet<Capability>,
        ttl_s: u64,
    ) -> Result<CapabilityToken, KernelError> {
        let pack = self.pack_manifest(pack_id)?;
        self.assert_pack_grants(pack, allowed_capabilities)?;
        self.mint_capability_token(&pack.pack_id, agent_id, allowed_capabilities.clone(), ttl_s)
    }

    fn mint_capability_token(
        &self,
        pack_id: &str,
        agent_id: &str,
        allowed_capabilities: BTreeSet<Capability>,
        ttl_s: u64,
    ) -> Result<CapabilityToken, KernelError> {
        let issued_at_epoch_s = self.audit_state.now_epoch_s();
        // Full and scoped issuance perform different authority checks, then
        // converge here so token identity and its audit event commit together.
        Ok(self.audit_state.record_with_sequence(
            issued_at_epoch_s,
            Some(agent_id.to_owned()),
            |generation| {
                let token = CapabilityToken {
                    token_id: format!("tok-{generation:016x}"),
                    pack_id: pack_id.to_owned(),
                    agent_id: agent_id.to_owned(),
                    allowed_capabilities,
                    issued_at_epoch_s,
                    expires_at_epoch_s: issued_at_epoch_s.saturating_add(ttl_s),
                    generation,
                };
                (
                    AuditEventKind::TokenIssued {
                        token: token.clone(),
                    },
                    token,
                )
            },
        )?)
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
        self.audit_state.record(
            agent_id.map(std::string::ToString::to_string),
            AuditEventKind::TokenRevoked {
                token_id: token_id.to_owned(),
            },
        )?;
        Ok(())
    }

    /// Record operational evidence supplied by an orchestration owner.
    ///
    /// Persisted authorization and tool-execution evidence have narrower live
    /// writers that possess their corresponding typed proof. This operational
    /// recorder rejects both reserved families instead of accepting a caller-
    /// constructed persisted envelope.
    pub fn record_audit_event(
        &self,
        agent_id: Option<&str>,
        kind: AuditEventKind,
    ) -> Result<(), AuditError> {
        if matches!(kind, AuditEventKind::Authorization { .. }) {
            return Err(AuditError::AuthorizationEvidenceOwnedByPolicyEngine);
        }
        if matches!(kind, AuditEventKind::ActionExecution { .. }) {
            return Err(AuditError::ActionExecutionEvidenceRequiresGrant);
        }
        if matches!(kind, AuditEventKind::ToolInvocation { .. }) {
            return Err(AuditError::HistoricalToolInvocationEvidenceReadOnly);
        }
        self.audit_state
            .record(agent_id.map(std::string::ToString::to_string), kind)?;
        Ok(())
    }

    /// Resolve the registered manifest that defines a token's pack boundary.
    ///
    /// Context constructors use this lookup instead of accepting a second,
    /// caller-supplied manifest that could disagree with kernel authority.
    pub fn pack_manifest(&self, pack_id: &str) -> Result<&VerticalPackManifest, KernelError> {
        self.packs
            .get(pack_id)
            .ok_or_else(|| KernelError::PackNotFound(pack_id.to_owned()))
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
        self.audit_state.record_at(
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
        )?;
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
        self.audit_state.record_at(
            now_epoch_s,
            Some(token.agent_id.clone()),
            AuditEventKind::AuthorizationDenied {
                pack_id: pack.pack_id.clone(),
                token_id: token.token_id.clone(),
                reason: error.to_string(),
            },
        )?;
        Ok(())
    }

    fn record_authorization_denial(
        &self,
        pack: &VerticalPackManifest,
        token: &CapabilityToken,
        now_epoch_s: u64,
        error: &impl std::fmt::Display,
    ) -> Result<(), KernelError> {
        self.audit_state.record_at(
            now_epoch_s,
            Some(token.agent_id.clone()),
            AuditEventKind::AuthorizationDenied {
                pack_id: pack.pack_id.clone(),
                token_id: token.token_id.clone(),
                reason: error.to_string(),
            },
        )?;
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
}

impl<C> Kernel<C>
where
    C: ContextFactory,
{
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
        ctx: &C::Cx<'_>,
    ) -> Result<(), KernelError> {
        let pack = self.pack_manifest(pack_id)?;
        let (now, _) = self
            .authorize_pack_operation(
                ctx,
                pack,
                token,
                operation,
                required_capabilities,
                json!({}),
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
        ctx: &C::Cx<'_>,
    ) -> Result<KernelDispatch, KernelError> {
        let pack = self.pack_manifest(pack_id)?;
        let TaskIntent {
            task_id,
            objective,
            required_capabilities,
            payload,
        } = task;
        let (now, payload) = self
            .authorize_pack_operation(
                ctx,
                pack,
                token,
                "execute_task",
                &required_capabilities,
                payload,
            )
            .await?;

        let request = HarnessRequest {
            token_id: token.token_id.clone(),
            pack_id: pack.pack_id.clone(),
            agent_id: token.agent_id.clone(),
            task_id: task_id.clone(),
            objective,
            payload,
        };

        let route = pack.default_route.clone();
        let outcome = self.harness.execute(&route, request).await?;

        self.audit_state.record_at(
            now,
            Some(token.agent_id.clone()),
            AuditEventKind::TaskDispatched {
                pack_id: pack.pack_id.clone(),
                task_id,
                route: route.clone(),
                required_capabilities: required_capabilities.iter().copied().collect(),
            },
        )?;

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
        ctx: &C::Cx<'_>,
    ) -> Result<ConnectorDispatch, KernelError> {
        let pack = self.pack_manifest(pack_id)?;
        let ConnectorCommand {
            connector_name,
            operation,
            required_capabilities,
            payload,
        } = command;
        self.assert_connector_allowed(pack, &connector_name)?;
        let (now, payload) = self
            .authorize_pack_operation(
                ctx,
                pack,
                token,
                &operation,
                &required_capabilities,
                payload,
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

        let command = ConnectorCommand {
            connector_name: connector_name.clone(),
            operation: operation.clone(),
            required_capabilities: required_capabilities.clone(),
            payload,
        };
        let outcome = self.connector_plane.invoke_core(core_name, command).await?;

        self.audit_state.record_at(
            now,
            Some(token.agent_id.clone()),
            AuditEventKind::ConnectorInvoked {
                pack_id: pack.pack_id.clone(),
                connector_name: connector_name.clone(),
                operation: operation.clone(),
                required_capabilities: required_capabilities.iter().copied().collect(),
            },
        )?;

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
        ctx: &C::Cx<'_>,
    ) -> Result<ConnectorDispatch, KernelError> {
        let pack = self.pack_manifest(pack_id)?;
        let ConnectorCommand {
            connector_name,
            operation,
            required_capabilities,
            payload,
        } = command;
        self.assert_connector_allowed(pack, &connector_name)?;
        let (now, payload) = self
            .authorize_pack_operation(
                ctx,
                pack,
                token,
                &operation,
                &required_capabilities,
                payload,
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

        let command = ConnectorCommand {
            connector_name: connector_name.clone(),
            operation: operation.clone(),
            required_capabilities: required_capabilities.clone(),
            payload,
        };
        let outcome = self
            .connector_plane
            .invoke_extension(extension_name, core_name, command)
            .await?;

        self.audit_state.record_at(
            now,
            Some(token.agent_id.clone()),
            AuditEventKind::ConnectorInvoked {
                pack_id: pack.pack_id.clone(),
                connector_name: connector_name.clone(),
                operation: operation.clone(),
                required_capabilities: required_capabilities.iter().copied().collect(),
            },
        )?;

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
        ctx: &C::Cx<'_>,
    ) -> Result<RuntimeCoreOutcome, KernelError> {
        let pack = self.pack_manifest(pack_id)?;
        let RuntimeCoreRequest { action, payload } = request;
        let (now, payload) = self
            .authorize_pack_operation(ctx, pack, token, &action, required_capabilities, payload)
            .await?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.runtime_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());
        let request = RuntimeCoreRequest {
            action: action.clone(),
            payload,
        };
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
        ctx: &C::Cx<'_>,
    ) -> Result<RuntimeExtensionOutcome, KernelError> {
        let pack = self.pack_manifest(pack_id)?;
        let RuntimeExtensionRequest { action, payload } = request;
        let (now, payload) = self
            .authorize_pack_operation(ctx, pack, token, &action, required_capabilities, payload)
            .await?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.runtime_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());
        let request = RuntimeExtensionRequest {
            action: action.clone(),
            payload,
        };
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
    /// The context exists only for legacy pack/token authorization. Once this
    /// ingress selects fallback, the adapter cannot re-enter typed dispatch;
    /// migrated tools must have been handled before this method is called.
    pub async fn execute_tool_core(
        &self,
        pack_id: &str,
        token: &CapabilityToken,
        required_capabilities: &BTreeSet<Capability>,
        core_name: Option<&str>,
        request: ToolCoreRequest,
        policy_context: &C::Cx<'_>,
    ) -> Result<ToolCoreOutcome, KernelError> {
        let pack = self.pack_manifest(pack_id)?;
        let ToolCoreRequest { tool_name, payload } = request;
        let (now, payload) = self
            .authorize_pack_operation(
                policy_context,
                pack,
                token,
                &tool_name,
                required_capabilities,
                payload,
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
        let request = ToolCoreRequest {
            tool_name: tool_name.clone(),
            payload,
        };
        let outcome = self
            .legacy_tool_plane
            .execute_core(core_name, request)
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
        ctx: &C::Cx<'_>,
    ) -> Result<ToolExtensionOutcome, KernelError> {
        let pack = self.pack_manifest(pack_id)?;
        let ToolExtensionRequest {
            extension_action,
            payload,
        } = request;
        let (now, payload) = self
            .authorize_pack_operation(
                ctx,
                pack,
                token,
                &extension_action,
                required_capabilities,
                payload,
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
        let request = ToolExtensionRequest {
            extension_action: extension_action.clone(),
            payload,
        };
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
            operation: extension_action,
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
        ctx: &C::Cx<'_>,
    ) -> Result<MemoryCoreOutcome, KernelError> {
        let pack = self.pack_manifest(pack_id)?;
        let MemoryCoreRequest { operation, payload } = request;
        let (now, payload) = self
            .authorize_pack_operation(ctx, pack, token, &operation, required_capabilities, payload)
            .await?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.memory_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());
        let request = MemoryCoreRequest {
            operation: operation.clone(),
            payload,
        };
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
        ctx: &C::Cx<'_>,
    ) -> Result<MemoryExtensionOutcome, KernelError> {
        let pack = self.pack_manifest(pack_id)?;
        let MemoryExtensionRequest { operation, payload } = request;
        let (now, payload) = self
            .authorize_pack_operation(ctx, pack, token, &operation, required_capabilities, payload)
            .await?;
        let resolved_core_adapter = core_name
            .map(std::string::ToString::to_string)
            .or_else(|| {
                self.memory_plane
                    .default_core_adapter_name()
                    .map(std::string::ToString::to_string)
            })
            .unwrap_or_else(|| "default".to_owned());
        let request = MemoryExtensionRequest {
            operation: operation.clone(),
            payload,
        };
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

    async fn authorize_pack_operation(
        &self,
        ctx: &C::Cx<'_>,
        pack: &VerticalPackManifest,
        token: &CapabilityToken,
        operation: &str,
        required_capabilities: &BTreeSet<Capability>,
        payload: Value,
    ) -> Result<(u64, Value), KernelError> {
        let now = self.now_epoch_s();
        if let Err(error) = self.assert_pack_grants(pack, required_capabilities) {
            self.record_authorization_denial(pack, token, now, &error)?;
            return Err(error);
        }
        self.authorize_or_audit_denial(
            ctx,
            pack,
            token,
            now,
            operation,
            required_capabilities,
            payload,
        )
        .await
    }

    // TODO(deprecate-legacy-kernel-auth): add `#[deprecated]` once old kernel
    // envelopes stop returning `PolicyError`; new side effects must consume
    // `Granted<ConcreteAction>` at their execution boundary.
    // Keep this context bound local to legacy pack/token envelopes; typed
    // Kernel/Access APIs must use their action-specific context bounds.
    async fn authorize_or_audit_denial(
        &self,
        ctx: &C::Cx<'_>,
        pack: &VerticalPackManifest,
        token: &CapabilityToken,
        now_epoch_s: u64,
        operation: &str,
        required_capabilities: &BTreeSet<Capability>,
        payload: Value,
    ) -> Result<(u64, Value), KernelError> {
        if let Err(policy_error) =
            self.authorize_token(pack, token, now_epoch_s, required_capabilities)
        {
            self.record_authorization_denial(pack, token, now_epoch_s, &policy_error)?;
            return Err(KernelError::Policy(policy_error));
        }

        let action = LegacyKernelAction::new(operation, required_capabilities.clone(), payload);
        let granted = match self.policy.grant(ctx, action).await {
            Ok(grant) => grant.into_granted(),
            Err(error) => return Err(KernelError::Policy(policy_engine_error(error))),
        };

        // Permission-capable policy may have awaited an external authority.
        // Legacy envelopes must not execute under a token invalidated while
        // that request was pending.
        let post_policy_now_epoch_s = self.now_epoch_s();
        if let Err(policy_error) =
            self.authorize_token(pack, token, post_policy_now_epoch_s, required_capabilities)
        {
            self.record_authorization_denial(pack, token, post_policy_now_epoch_s, &policy_error)?;
            return Err(KernelError::Policy(policy_error));
        }

        let payload = granted.into_action().into_payload();
        Ok((post_policy_now_epoch_s, payload))
    }
}

#[cfg(test)]
mod tests;

impl<C> loong_core::kernel::Kernel<C> for Kernel<C>
where
    C: ContextFactory,
{
    fn policy_engine(&self) -> &impl PolicyEngine<C> {
        &self.policy
    }
}
