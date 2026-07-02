use std::{
    borrow::Cow,
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use loong_contracts::{
    Capability, CapabilityToken, ExecutionPlane, GrantId, PlaneTier, PolicyDecision, PolicyEntry,
    PolicyGrant, PolicyOutcome, VerticalPackManifest,
};
use loong_core::{
    action::Action,
    error::AuthorizationError,
    policy::{
        context::{PolicyContext, PolicyContextFactory},
        engine::PolicyEngine,
        policy::PolicyAny,
    },
};

use crate::{
    errors::PolicyError,
    policy_ext::{PolicyExtension, PolicyExtensionChain, PolicyExtensionContext},
};

const DEFAULT_DENY_REASON: &str = "No matching policy.";

pub struct KernelPolicyContext<'a> {
    pub pack: &'a VerticalPackManifest,
    pub token: &'a CapabilityToken,
    pub now_epoch_s: u64,
    pub request_parameters: Option<&'a serde_json::Value>,
}

impl PolicyContext for KernelPolicyContext<'_> {
    fn capabilities(&self) -> BTreeSet<Capability> {
        self.token.allowed_capabilities.clone()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct KernelPolicyContextFactory;

impl PolicyContextFactory for KernelPolicyContextFactory {
    type Context<'a> = KernelPolicyContext<'a>;
}

pub struct LegacyKernelAction {
    plane: ExecutionPlane,
    tier: PlaneTier,
    operation: String,
    required_capabilities: BTreeSet<Capability>,
}

impl LegacyKernelAction {
    pub fn new(
        plane: ExecutionPlane,
        tier: PlaneTier,
        operation: impl Into<String>,
        required_capabilities: BTreeSet<Capability>,
    ) -> Self {
        Self {
            plane,
            tier,
            operation: operation.into(),
            required_capabilities,
        }
    }
}

impl Action for LegacyKernelAction {
    fn kind(&self) -> &'static str {
        "action.legacy"
    }

    fn execution_plane(&self) -> ExecutionPlane {
        self.plane
    }

    fn plane_tier(&self) -> PlaneTier {
        self.tier
    }

    fn operation(&self) -> Cow<'static, str> {
        self.operation.clone().into()
    }

    fn required_capabilities(&self) -> BTreeSet<Capability> {
        self.required_capabilities.clone()
    }
}

// TODO: This is the temporary Policy Pipeline, other
// policies' support will be added later.
pub struct PolicyPipeline {
    policies: Vec<Arc<dyn PolicyAny<KernelPolicyContextFactory>>>,
    policy_extensions: PolicyExtensionChain,
    grant_seq: AtomicU64,
}

impl Default for PolicyPipeline {
    fn default() -> Self {
        Self::new().with_policy(AllowPolicy)
    }
}

impl PolicyPipeline {
    #[must_use]
    pub fn new() -> Self {
        Self {
            policies: Vec::new(),
            policy_extensions: PolicyExtensionChain::new(),
            grant_seq: AtomicU64::new(0),
        }
    }

    #[must_use]
    pub fn with_policy<P>(mut self, policy: P) -> Self
    where
        P: PolicyAny<KernelPolicyContextFactory> + 'static,
    {
        self.push_policy(policy);
        self
    }

    pub fn push_policy<P>(&mut self, policy: P)
    where
        P: PolicyAny<KernelPolicyContextFactory> + 'static,
    {
        self.policies.push(Arc::new(policy));
    }

    pub fn register_policy_extension<E: PolicyExtension + 'static>(&mut self, extension: E) {
        self.policy_extensions.register(extension);
    }

    pub async fn authorize_kernel_action<A: Action>(
        &self,
        ctx: &KernelPolicyContext<'_>,
        action: A,
    ) -> Result<(), PolicyError> {
        let required_capabilities = action.required_capabilities();
        self.grant(ctx, action).await.map_err(policy_engine_error)?;

        self.policy_extensions.authorize(&PolicyExtensionContext {
            pack: ctx.pack,
            token: ctx.token,
            now_epoch_s: ctx.now_epoch_s,
            required_capabilities: &required_capabilities,
            request_parameters: ctx.request_parameters,
        })
    }

    fn next_grant_id_sync(&self) -> GrantId {
        let seq = self.grant_seq.fetch_add(1, Ordering::Relaxed) + 1;
        GrantId(seq)
    }
}

fn policy_engine_error(error: AuthorizationError) -> PolicyError {
    PolicyError::ExtensionDenied {
        extension: "policy-engine".to_owned(),
        reason: error.to_string(),
    }
}

#[async_trait]
impl PolicyEngine for PolicyPipeline {
    type Factory = KernelPolicyContextFactory;

    async fn decide<A: Action>(
        &self,
        ctx: &<Self::Factory as PolicyContextFactory>::Context<'_>,
        action: &A,
    ) -> PolicyOutcome {
        let mut allow: Option<(PolicyEntry, Cow<'static, str>)> = None;

        for (index, policy) in self.policies.iter().enumerate() {
            let grant = policy.grant(ctx, action).await;
            let source = PolicyEntry {
                policy_name: policy.name().into(),
                policy_id: index as u64,
            };

            match grant.decision {
                PolicyDecision::Allow => {
                    allow = Some((source, grant.reason));
                }
                PolicyDecision::Deny => {
                    return PolicyOutcome::Deny {
                        grant_source: Some(source),
                        reason: grant.reason,
                    };
                }
                PolicyDecision::Abstain => {}
            }
        }

        match allow {
            Some((source, reason)) => PolicyOutcome::Allow { source, reason },
            None => PolicyOutcome::Deny {
                grant_source: None,
                reason: DEFAULT_DENY_REASON.into(),
            },
        }
    }

    async fn next_grant_id(&self) -> GrantId {
        self.next_grant_id_sync()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AllowPolicy;

#[async_trait]
impl PolicyAny<KernelPolicyContextFactory> for AllowPolicy {
    fn name(&self) -> &'static str {
        "allow"
    }

    async fn grant(&self, _ctx: &KernelPolicyContext<'_>, _action: &dyn Action) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: None,
            reason: "allowed by allow policy".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DenyNetworkExtension;

    impl PolicyExtension for DenyNetworkExtension {
        fn name(&self) -> &str {
            "deny-network"
        }

        fn authorize_extension(
            &self,
            context: &PolicyExtensionContext<'_>,
        ) -> Result<(), PolicyError> {
            if context
                .required_capabilities
                .contains(&Capability::NetworkEgress)
            {
                return Err(PolicyError::ExtensionDenied {
                    extension: self.name().to_owned(),
                    reason: "network egress denied by test extension".to_owned(),
                });
            }
            Ok(())
        }
    }

    fn pack() -> VerticalPackManifest {
        VerticalPackManifest {
            pack_id: "pack".to_owned(),
            domain: "test".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: loong_contracts::ExecutionRoute {
                harness_kind: loong_contracts::HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: Default::default(),
        }
    }

    fn token() -> CapabilityToken {
        CapabilityToken {
            token_id: "tok".to_owned(),
            pack_id: "pack".to_owned(),
            agent_id: "agent".to_owned(),
            allowed_capabilities: BTreeSet::from([Capability::InvokeTool]),
            issued_at_epoch_s: 1,
            expires_at_epoch_s: 10,
            generation: 1,
        }
    }

    #[tokio::test]
    async fn policy_pipeline_grants_actions_allowed_by_registered_policy() {
        let engine = PolicyPipeline::new().with_policy(AllowPolicy);
        let pack = pack();
        let token = token();
        let ctx = KernelPolicyContext {
            pack: &pack,
            token: &token,
            now_epoch_s: 1,
            request_parameters: None,
        };
        let action = LegacyKernelAction::new(
            ExecutionPlane::Tool,
            PlaneTier::Core,
            "tool",
            BTreeSet::from([Capability::InvokeTool]),
        );

        let grant = engine
            .grant(&ctx, action)
            .await
            .expect("allow policy should grant action");

        assert_eq!(grant.id.0, 1);
        let _info = grant.info;
        assert_eq!(grant.granted.into_action().operation(), "tool");
    }

    #[tokio::test]
    async fn policy_pipeline_runs_registered_policy_extensions() {
        let mut engine = PolicyPipeline::default();
        engine.register_policy_extension(DenyNetworkExtension);
        let pack = pack();
        let mut token = token();
        token.allowed_capabilities.insert(Capability::NetworkEgress);
        let ctx = KernelPolicyContext {
            pack: &pack,
            token: &token,
            now_epoch_s: 1,
            request_parameters: None,
        };
        let action = LegacyKernelAction::new(
            ExecutionPlane::Runtime,
            PlaneTier::Core,
            "fetch",
            BTreeSet::from([Capability::NetworkEgress]),
        );

        let error = engine
            .authorize_kernel_action(&ctx, action)
            .await
            .expect_err("registered policy extension should deny the action");

        assert_eq!(
            error,
            PolicyError::ExtensionDenied {
                extension: "deny-network".to_owned(),
                reason: "network egress denied by test extension".to_owned(),
            }
        );
    }
}
