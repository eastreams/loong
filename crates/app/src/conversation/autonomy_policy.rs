use super::runtime_binding::ConversationRuntimeBinding;
use crate::tools::CapabilityActionClass;
use crate::tools::runtime_config::{AutonomyOperationMode, AutonomyPolicySnapshot};

pub const AUTONOMY_POLICY_SOURCE: &str = "autonomy_policy";
pub const CAPABILITY_ACQUISITION_DISALLOWED_CODE: &str =
    "autonomy_policy_capability_acquisition_disallowed";
pub const CAPABILITY_ACQUISITION_APPROVAL_CODE: &str =
    "autonomy_policy_capability_acquisition_requires_approval";
pub const CAPABILITY_ACQUISITION_BUDGET_EXCEEDED_CODE: &str =
    "autonomy_policy_capability_acquisition_budget_exceeded";
pub const PROVIDER_SWITCH_APPROVAL_CODE: &str = "autonomy_policy_provider_switch_requires_approval";
pub const PROVIDER_SWITCH_DISALLOWED_CODE: &str = "autonomy_policy_provider_switch_disallowed";
pub const PROVIDER_SWITCH_BUDGET_EXCEEDED_CODE: &str =
    "autonomy_policy_provider_switch_budget_exceeded";
pub const TOPOLOGY_MUTATION_APPROVAL_CODE: &str =
    "autonomy_policy_topology_mutation_requires_approval";
pub const TOPOLOGY_MUTATION_DISALLOWED_CODE: &str = "autonomy_policy_topology_mutation_disallowed";
pub const TOPOLOGY_MUTATION_BUDGET_EXCEEDED_CODE: &str =
    "autonomy_policy_topology_mutation_budget_exceeded";
pub const POLICY_MUTATION_APPROVAL_CODE: &str = "autonomy_policy_policy_mutation_requires_approval";
pub const POLICY_MUTATION_DISALLOWED_CODE: &str = "autonomy_policy_policy_mutation_disallowed";
pub const POLICY_MUTATION_BUDGET_EXCEEDED_CODE: &str =
    "autonomy_policy_policy_mutation_budget_exceeded";
pub const SESSION_MUTATION_APPROVAL_CODE: &str =
    "autonomy_policy_session_mutation_requires_approval";
pub const SESSION_MUTATION_DISALLOWED_CODE: &str = "autonomy_policy_session_mutation_disallowed";
pub const SESSION_MUTATION_BUDGET_EXCEEDED_CODE: &str =
    "autonomy_policy_session_mutation_budget_exceeded";
pub const BINDING_MISSING_CODE: &str = "autonomy_policy_binding_missing";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AutonomyTurnBudgetState {
    pub capability_acquisitions_used: usize,
    pub provider_switches_used: usize,
    pub topology_mutations_used: usize,
}

impl AutonomyTurnBudgetState {
    pub fn record_action(&mut self, action_class: CapabilityActionClass) {
        match action_class {
            CapabilityActionClass::CapabilityFetch
            | CapabilityActionClass::CapabilityInstall
            | CapabilityActionClass::CapabilityLoad => {
                self.capability_acquisitions_used =
                    self.capability_acquisitions_used.saturating_add(1);
            }
            CapabilityActionClass::RuntimeSwitch => {
                self.provider_switches_used = self.provider_switches_used.saturating_add(1);
            }
            CapabilityActionClass::TopologyExpand
            | CapabilityActionClass::PolicyMutation
            | CapabilityActionClass::SessionMutation => {
                self.topology_mutations_used = self.topology_mutations_used.saturating_add(1);
            }
            CapabilityActionClass::Discover | CapabilityActionClass::ExecuteExisting => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    ApprovalRequired {
        rule_id: &'static str,
        reason_code: &'static str,
    },
    Deny {
        rule_id: &'static str,
        reason_code: &'static str,
    },
}

#[derive(Clone, Copy)]
pub struct PolicyDecisionInput<'a> {
    pub snapshot: &'a AutonomyPolicySnapshot,
    pub action_class: CapabilityActionClass,
    pub binding: ConversationRuntimeBinding<'a>,
    pub budget: &'a AutonomyTurnBudgetState,
}

pub fn evaluate_policy(input: PolicyDecisionInput<'_>) -> PolicyDecision {
    let Some(base_mode) = action_mode(input.snapshot, input.action_class) else {
        return PolicyDecision::Allow;
    };
    if base_mode == AutonomyOperationMode::Deny {
        return PolicyDecision::Deny {
            rule_id: deny_rule_id(input.action_class),
            reason_code: deny_reason_code(input.action_class),
        };
    }

    if input.snapshot.requires_kernel_binding && !input.binding.is_kernel_bound() {
        return PolicyDecision::Deny {
            rule_id: "autonomy_policy_requires_kernel_binding",
            reason_code: BINDING_MISSING_CODE,
        };
    }

    if let Some(deny_reason_code) = budget_deny_reason(input) {
        return PolicyDecision::Deny {
            rule_id: "autonomy_policy_budget_guard",
            reason_code: deny_reason_code,
        };
    }

    match base_mode {
        AutonomyOperationMode::Allow => PolicyDecision::Allow,
        AutonomyOperationMode::ApprovalRequired => PolicyDecision::ApprovalRequired {
            rule_id: approval_rule_id(input.action_class),
            reason_code: approval_reason_code(input.action_class),
        },
        AutonomyOperationMode::Deny => PolicyDecision::Deny {
            rule_id: deny_rule_id(input.action_class),
            reason_code: deny_reason_code(input.action_class),
        },
    }
}

pub fn action_mode(
    snapshot: &AutonomyPolicySnapshot,
    action_class: CapabilityActionClass,
) -> Option<AutonomyOperationMode> {
    match action_class {
        CapabilityActionClass::CapabilityFetch
        | CapabilityActionClass::CapabilityInstall
        | CapabilityActionClass::CapabilityLoad => Some(snapshot.capability_acquisition_mode),
        CapabilityActionClass::RuntimeSwitch => Some(snapshot.provider_switch_mode),
        CapabilityActionClass::TopologyExpand
        | CapabilityActionClass::PolicyMutation
        | CapabilityActionClass::SessionMutation => Some(snapshot.topology_mutation_mode),
        CapabilityActionClass::Discover | CapabilityActionClass::ExecuteExisting => None,
    }
}

fn budget_deny_reason(input: PolicyDecisionInput<'_>) -> Option<&'static str> {
    match input.action_class {
        CapabilityActionClass::CapabilityFetch
        | CapabilityActionClass::CapabilityInstall
        | CapabilityActionClass::CapabilityLoad => {
            let used = input.budget.capability_acquisitions_used;
            let limit = input.snapshot.budget.max_capability_acquisitions_per_turn;
            would_exceed_budget(used, limit).then_some(CAPABILITY_ACQUISITION_BUDGET_EXCEEDED_CODE)
        }
        CapabilityActionClass::RuntimeSwitch => {
            let used = input.budget.provider_switches_used;
            let limit = input.snapshot.budget.max_provider_switches_per_turn;
            would_exceed_budget(used, limit).then_some(PROVIDER_SWITCH_BUDGET_EXCEEDED_CODE)
        }
        CapabilityActionClass::TopologyExpand
        | CapabilityActionClass::PolicyMutation
        | CapabilityActionClass::SessionMutation => {
            let used = input.budget.topology_mutations_used;
            let limit = input.snapshot.budget.max_topology_mutations_per_turn;
            let reason_code = topology_budget_reason_code(input.action_class);
            would_exceed_budget(used, limit).then_some(reason_code)
        }
        CapabilityActionClass::Discover | CapabilityActionClass::ExecuteExisting => None,
    }
}

fn would_exceed_budget(used: usize, limit: usize) -> bool {
    used.saturating_add(1) > limit
}

fn approval_rule_id(action_class: CapabilityActionClass) -> &'static str {
    match action_class {
        CapabilityActionClass::CapabilityFetch
        | CapabilityActionClass::CapabilityInstall
        | CapabilityActionClass::CapabilityLoad => {
            "autonomy_policy_capability_acquisition_requires_approval"
        }
        CapabilityActionClass::RuntimeSwitch => "autonomy_policy_provider_switch_requires_approval",
        CapabilityActionClass::TopologyExpand => {
            "autonomy_policy_topology_mutation_requires_approval"
        }
        CapabilityActionClass::PolicyMutation => {
            "autonomy_policy_policy_mutation_requires_approval"
        }
        CapabilityActionClass::SessionMutation => {
            "autonomy_policy_session_mutation_requires_approval"
        }
        CapabilityActionClass::Discover | CapabilityActionClass::ExecuteExisting => {
            "autonomy_policy_not_applicable"
        }
    }
}

fn approval_reason_code(action_class: CapabilityActionClass) -> &'static str {
    match action_class {
        CapabilityActionClass::CapabilityFetch
        | CapabilityActionClass::CapabilityInstall
        | CapabilityActionClass::CapabilityLoad => CAPABILITY_ACQUISITION_APPROVAL_CODE,
        CapabilityActionClass::RuntimeSwitch => PROVIDER_SWITCH_APPROVAL_CODE,
        CapabilityActionClass::TopologyExpand => TOPOLOGY_MUTATION_APPROVAL_CODE,
        CapabilityActionClass::PolicyMutation => POLICY_MUTATION_APPROVAL_CODE,
        CapabilityActionClass::SessionMutation => SESSION_MUTATION_APPROVAL_CODE,
        CapabilityActionClass::Discover | CapabilityActionClass::ExecuteExisting => {
            "autonomy_policy_not_applicable"
        }
    }
}

fn deny_rule_id(action_class: CapabilityActionClass) -> &'static str {
    match action_class {
        CapabilityActionClass::CapabilityFetch
        | CapabilityActionClass::CapabilityInstall
        | CapabilityActionClass::CapabilityLoad => {
            "autonomy_policy_capability_acquisition_disallowed"
        }
        CapabilityActionClass::RuntimeSwitch => "autonomy_policy_provider_switch_disallowed",
        CapabilityActionClass::TopologyExpand => "autonomy_policy_topology_mutation_disallowed",
        CapabilityActionClass::PolicyMutation => "autonomy_policy_policy_mutation_disallowed",
        CapabilityActionClass::SessionMutation => "autonomy_policy_session_mutation_disallowed",
        CapabilityActionClass::Discover | CapabilityActionClass::ExecuteExisting => {
            "autonomy_policy_not_applicable"
        }
    }
}

fn deny_reason_code(action_class: CapabilityActionClass) -> &'static str {
    match action_class {
        CapabilityActionClass::CapabilityFetch
        | CapabilityActionClass::CapabilityInstall
        | CapabilityActionClass::CapabilityLoad => CAPABILITY_ACQUISITION_DISALLOWED_CODE,
        CapabilityActionClass::RuntimeSwitch => PROVIDER_SWITCH_DISALLOWED_CODE,
        CapabilityActionClass::TopologyExpand => TOPOLOGY_MUTATION_DISALLOWED_CODE,
        CapabilityActionClass::PolicyMutation => POLICY_MUTATION_DISALLOWED_CODE,
        CapabilityActionClass::SessionMutation => SESSION_MUTATION_DISALLOWED_CODE,
        CapabilityActionClass::Discover | CapabilityActionClass::ExecuteExisting => {
            "autonomy_policy_not_applicable"
        }
    }
}

fn topology_budget_reason_code(action_class: CapabilityActionClass) -> &'static str {
    match action_class {
        CapabilityActionClass::TopologyExpand => TOPOLOGY_MUTATION_BUDGET_EXCEEDED_CODE,
        CapabilityActionClass::PolicyMutation => POLICY_MUTATION_BUDGET_EXCEEDED_CODE,
        CapabilityActionClass::SessionMutation => SESSION_MUTATION_BUDGET_EXCEEDED_CODE,
        CapabilityActionClass::Discover
        | CapabilityActionClass::ExecuteExisting
        | CapabilityActionClass::CapabilityFetch
        | CapabilityActionClass::CapabilityInstall
        | CapabilityActionClass::CapabilityLoad
        | CapabilityActionClass::RuntimeSwitch => "autonomy_policy_not_applicable",
    }
}

pub fn render_reason(
    snapshot: &AutonomyPolicySnapshot,
    action_class: CapabilityActionClass,
    tool_name: &str,
    reason_code: &str,
) -> String {
    match reason_code {
        CAPABILITY_ACQUISITION_DISALLOWED_CODE => format!(
            "autonomy policy denied `{tool_name}`: capability acquisition is disabled in `{}`",
            snapshot.profile.as_str()
        ),
        CAPABILITY_ACQUISITION_APPROVAL_CODE => format!(
            "operator approval required before running `{tool_name}` under `{}` product mode",
            snapshot.profile.as_str()
        ),
        CAPABILITY_ACQUISITION_BUDGET_EXCEEDED_CODE => format!(
            "autonomy policy denied `{tool_name}`: capability acquisition budget exceeded for `{}`",
            snapshot.profile.as_str()
        ),
        PROVIDER_SWITCH_APPROVAL_CODE => format!(
            "operator approval required before running `{tool_name}` under `{}` product mode",
            snapshot.profile.as_str()
        ),
        PROVIDER_SWITCH_DISALLOWED_CODE => format!(
            "autonomy policy denied `{tool_name}`: provider switch is disabled in `{}`",
            snapshot.profile.as_str()
        ),
        PROVIDER_SWITCH_BUDGET_EXCEEDED_CODE => format!(
            "autonomy policy denied `{tool_name}`: provider switch budget exceeded for `{}`",
            snapshot.profile.as_str()
        ),
        TOPOLOGY_MUTATION_APPROVAL_CODE => format!(
            "operator approval required before running `{tool_name}` under `{}` product mode",
            snapshot.profile.as_str()
        ),
        TOPOLOGY_MUTATION_DISALLOWED_CODE => format!(
            "autonomy policy denied `{tool_name}`: topology expansion is disabled in `{}`",
            snapshot.profile.as_str()
        ),
        TOPOLOGY_MUTATION_BUDGET_EXCEEDED_CODE => format!(
            "autonomy policy denied `{tool_name}`: topology mutation budget exceeded for `{}`",
            snapshot.profile.as_str()
        ),
        POLICY_MUTATION_APPROVAL_CODE => format!(
            "operator approval required before running `{tool_name}` under `{}` product mode",
            snapshot.profile.as_str()
        ),
        POLICY_MUTATION_DISALLOWED_CODE => format!(
            "autonomy policy denied `{tool_name}`: policy mutation is disabled in `{}`",
            snapshot.profile.as_str()
        ),
        POLICY_MUTATION_BUDGET_EXCEEDED_CODE => format!(
            "autonomy policy denied `{tool_name}`: topology mutation budget exceeded for `{}`",
            snapshot.profile.as_str()
        ),
        SESSION_MUTATION_APPROVAL_CODE => format!(
            "operator approval required before running `{tool_name}` under `{}` product mode",
            snapshot.profile.as_str()
        ),
        SESSION_MUTATION_DISALLOWED_CODE => format!(
            "autonomy policy denied `{tool_name}`: session mutation is disabled in `{}`",
            snapshot.profile.as_str()
        ),
        SESSION_MUTATION_BUDGET_EXCEEDED_CODE => format!(
            "autonomy policy denied `{tool_name}`: topology mutation budget exceeded for `{}`",
            snapshot.profile.as_str()
        ),
        BINDING_MISSING_CODE => format!(
            "autonomy policy denied `{tool_name}`: `{}` requires kernel-bound execution for {:?}",
            snapshot.profile.as_str(),
            action_class
        ),
        _ => format!(
            "autonomy policy denied `{tool_name}` with reason `{reason_code}` under `{}`",
            snapshot.profile.as_str()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KernelContext;
    use crate::config::AutonomyProfile;
    use crate::tools::runtime_config::AutonomyPolicySnapshot;
    use loongclaw_contracts::{Capability, ExecutionRoute, HarnessKind};
    use loongclaw_kernel::{
        FixedClock, InMemoryAuditSink, LoongClawKernel, StaticPolicyEngine, VerticalPackManifest,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    #[test]
    fn autonomy_policy_decision_requires_kernel_binding_before_guided_capability_install() {
        let snapshot = AutonomyPolicySnapshot::from_profile(AutonomyProfile::GuidedAcquisition);
        let budget = AutonomyTurnBudgetState::default();
        let input = PolicyDecisionInput {
            snapshot: &snapshot,
            action_class: CapabilityActionClass::CapabilityInstall,
            binding: ConversationRuntimeBinding::direct(),
            budget: &budget,
        };

        let decision = evaluate_policy(input);

        assert_eq!(
            decision,
            PolicyDecision::Deny {
                rule_id: "autonomy_policy_requires_kernel_binding",
                reason_code: BINDING_MISSING_CODE,
            }
        );
    }

    #[test]
    fn autonomy_policy_decision_routes_topology_expand_through_mutation_mode() {
        let snapshot = AutonomyPolicySnapshot {
            profile: AutonomyProfile::GuidedAcquisition,
            capability_acquisition_mode: AutonomyOperationMode::Deny,
            provider_switch_mode: AutonomyOperationMode::Deny,
            topology_mutation_mode: AutonomyOperationMode::ApprovalRequired,
            requires_kernel_binding: false,
            budget: crate::tools::runtime_config::AutonomyBudgetPolicy {
                max_capability_acquisitions_per_turn: 0,
                max_provider_switches_per_turn: 0,
                max_topology_mutations_per_turn: 1,
            },
        };
        let budget = AutonomyTurnBudgetState::default();
        let input = PolicyDecisionInput {
            snapshot: &snapshot,
            action_class: CapabilityActionClass::TopologyExpand,
            binding: ConversationRuntimeBinding::direct(),
            budget: &budget,
        };

        let decision = evaluate_policy(input);

        assert_eq!(
            decision,
            PolicyDecision::ApprovalRequired {
                rule_id: "autonomy_policy_topology_mutation_requires_approval",
                reason_code: TOPOLOGY_MUTATION_APPROVAL_CODE,
            }
        );
    }

    #[test]
    fn autonomy_policy_decision_enforces_capability_budget() {
        let snapshot = AutonomyPolicySnapshot::from_profile(AutonomyProfile::BoundedAutonomous);
        let budget = AutonomyTurnBudgetState {
            capability_acquisitions_used: 2,
            provider_switches_used: 0,
            topology_mutations_used: 0,
        };
        let input = PolicyDecisionInput {
            snapshot: &snapshot,
            action_class: CapabilityActionClass::CapabilityInstall,
            binding: ConversationRuntimeBinding::kernel(kernel_context_placeholder()),
            budget: &budget,
        };

        let decision = evaluate_policy(input);

        assert_eq!(
            decision,
            PolicyDecision::Deny {
                rule_id: "autonomy_policy_budget_guard",
                reason_code: CAPABILITY_ACQUISITION_BUDGET_EXCEEDED_CODE,
            }
        );
    }

    #[test]
    fn autonomy_policy_decision_enforces_mutation_budget_for_session_mutation() {
        let snapshot = AutonomyPolicySnapshot {
            profile: AutonomyProfile::BoundedAutonomous,
            capability_acquisition_mode: AutonomyOperationMode::Deny,
            provider_switch_mode: AutonomyOperationMode::Deny,
            topology_mutation_mode: AutonomyOperationMode::Allow,
            requires_kernel_binding: false,
            budget: crate::tools::runtime_config::AutonomyBudgetPolicy {
                max_capability_acquisitions_per_turn: 0,
                max_provider_switches_per_turn: 0,
                max_topology_mutations_per_turn: 1,
            },
        };
        let budget = AutonomyTurnBudgetState {
            capability_acquisitions_used: 0,
            provider_switches_used: 0,
            topology_mutations_used: 1,
        };
        let input = PolicyDecisionInput {
            snapshot: &snapshot,
            action_class: CapabilityActionClass::SessionMutation,
            binding: ConversationRuntimeBinding::direct(),
            budget: &budget,
        };

        let decision = evaluate_policy(input);

        assert_eq!(
            decision,
            PolicyDecision::Deny {
                rule_id: "autonomy_policy_budget_guard",
                reason_code: SESSION_MUTATION_BUDGET_EXCEEDED_CODE,
            }
        );
    }

    fn kernel_context_placeholder() -> &'static KernelContext {
        static HOLDER: std::sync::OnceLock<KernelContext> = std::sync::OnceLock::new();
        HOLDER.get_or_init(|| {
            let audit = Arc::new(InMemoryAuditSink::default());
            let clock = Arc::new(FixedClock::new(1_700_000_000));
            let mut kernel =
                LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);
            let pack = VerticalPackManifest {
                pack_id: "autonomy-policy-test-pack".to_owned(),
                domain: "testing".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: None,
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
                metadata: BTreeMap::new(),
            };
            kernel.register_pack(pack).expect("register pack");
            let token = kernel
                .issue_token("autonomy-policy-test-pack", "autonomy-policy-agent", 60)
                .expect("issue token");
            KernelContext {
                kernel: Arc::new(kernel),
                token,
            }
        })
    }
}
