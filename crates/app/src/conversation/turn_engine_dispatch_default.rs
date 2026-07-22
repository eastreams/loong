use async_trait::async_trait;
use loong_contracts::{Capability, GovernedSessionMode};
use loong_core::policy::context::PolicyContext;

use super::*;

impl DefaultLegacyToolDispatcher {
    fn autonomy_policy_decision_base(
        tool_name: &str,
        policy_snapshot: &crate::tools::runtime_config::AutonomyPolicySnapshot,
        action_class: crate::tools::CapabilityActionClass,
    ) -> ToolDecisionTelemetry {
        let profile = policy_snapshot.profile.as_str();
        let action_class_name = action_class.as_str();
        let base = ToolDecisionTelemetry::allow(tool_name, "", AUTONOMY_POLICY_ALLOW_RULE_ID);
        let with_source = base.with_policy_source(AUTONOMY_POLICY_SOURCE);
        let with_profile = with_source.with_autonomy_profile(profile);
        let with_action_class = with_profile.with_capability_action_class(action_class_name);
        with_action_class.with_reason_code(AUTONOMY_POLICY_ALLOW_REASON_CODE)
    }

    fn autonomy_policy_allow_decision(
        tool_name: &str,
        policy_snapshot: &crate::tools::runtime_config::AutonomyPolicySnapshot,
        action_class: crate::tools::CapabilityActionClass,
    ) -> ToolDecisionTelemetry {
        let profile = policy_snapshot.profile.as_str();
        let reason =
            format!("autonomy policy allowed `{tool_name}` under `{profile}` product mode");
        let base = Self::autonomy_policy_decision_base(tool_name, policy_snapshot, action_class);
        ToolDecisionTelemetry { reason, ..base }
    }

    fn autonomy_policy_grant_satisfied_decision(
        tool_name: &str,
        policy_snapshot: &crate::tools::runtime_config::AutonomyPolicySnapshot,
        action_class: crate::tools::CapabilityActionClass,
        rule_id: &str,
        reason_code: &str,
        reason: String,
    ) -> ToolDecisionTelemetry {
        let base = Self::autonomy_policy_decision_base(tool_name, policy_snapshot, action_class);
        let decision = ToolDecisionTelemetry {
            reason,
            rule_id: rule_id.to_owned(),
            ..base
        };
        decision.with_reason_code(reason_code)
    }

    fn autonomy_policy_approval_required_decision(
        tool_name: &str,
        policy_snapshot: &crate::tools::runtime_config::AutonomyPolicySnapshot,
        action_class: crate::tools::CapabilityActionClass,
        rule_id: &str,
        reason_code: &str,
        reason: String,
    ) -> ToolDecisionTelemetry {
        let base = ToolDecisionTelemetry::approval_required(tool_name, reason, rule_id);
        let with_source = base.with_policy_source(AUTONOMY_POLICY_SOURCE);
        let with_profile = with_source.with_autonomy_profile(policy_snapshot.profile.as_str());
        let with_action_class = with_profile.with_capability_action_class(action_class.as_str());
        with_action_class.with_reason_code(reason_code)
    }

    fn autonomy_policy_denied_decision(
        tool_name: &str,
        policy_snapshot: &crate::tools::runtime_config::AutonomyPolicySnapshot,
        action_class: crate::tools::CapabilityActionClass,
        rule_id: &str,
        reason_code: &str,
        reason: String,
    ) -> ToolDecisionTelemetry {
        let base = ToolDecisionTelemetry::deny(tool_name, reason, rule_id);
        let with_source = base.with_policy_source(AUTONOMY_POLICY_SOURCE);
        let with_profile = with_source.with_autonomy_profile(policy_snapshot.profile.as_str());
        let with_action_class = with_profile.with_capability_action_class(action_class.as_str());
        with_action_class.with_reason_code(reason_code)
    }

    pub(super) fn effective_tool_config_for_session(
        &self,
        session_context: &Context<'_>,
    ) -> ToolConfig {
        let mut tool_config = self.tool_config.clone();
        if session_context.session().parent_session_id.is_some() {
            tool_config.sessions.visibility = SessionVisibility::SelfOnly;
        }
        tool_config
    }

    #[cfg(feature = "memory-sqlite")]
    async fn execute_sessions_send(
        &self,
        session_context: &Context<'_>,
        payload: serde_json::Value,
    ) -> Result<ToolCoreOutcome, String> {
        let app_config = self
            .app_config
            .as_ref()
            .ok_or_else(|| "sessions_send_not_configured".to_owned())?;
        let effective_tool_config = self.effective_tool_config_for_session(session_context);
        crate::tools::messaging::execute_sessions_send_with_config(
            payload,
            &session_context.session().session_id,
            &self.memory_config,
            &effective_tool_config,
            app_config.as_ref(),
        )
        .await
    }

    #[cfg(feature = "memory-sqlite")]
    fn lineage_root_session_id(
        repo: &SessionRepository,
        session_context: &Context<'_>,
    ) -> Result<String, String> {
        let session_graph = OperatorSessionGraph::new(repo);
        session_graph.effective_lineage_root_session_id(
            &session_context.session().session_id,
            session_context.session().parent_session_id.as_deref(),
        )
    }

    fn approval_key_for_descriptor(descriptor: &crate::tools::ToolDescriptor) -> String {
        OperatorApprovalRuntime::approval_key_for_tool_name(descriptor.name)
    }

    fn is_tool_call_preapproved(&self, approval_key: &str) -> bool {
        let approved_calls = &self.tool_config.approval.approved_calls;
        approved_calls.iter().any(|entry| entry == approval_key)
    }

    fn is_tool_call_predenied(&self, approval_key: &str) -> bool {
        let denied_calls = &self.tool_config.approval.denied_calls;
        denied_calls.iter().any(|entry| entry == approval_key)
    }

    #[cfg(feature = "memory-sqlite")]
    /// Build the legacy replay protocol in one place so every approval producer
    /// records the exact fallback owner, effective request, and trust state.
    fn approval_request_payload_json(
        session_context: &Context<'_>,
        intent: &ToolIntent,
        execution_request: &ToolCoreRequest,
        trusted_internal_context: bool,
        descriptor: &crate::tools::ToolDescriptor,
        dispatch_kind: LegacyToolDispatchKind,
        approval_request_id: &str,
        approval_key: &str,
        rule_id: &str,
    ) -> serde_json::Value {
        let payload = json!({
            "session_id": session_context.session().session_id,
            "parent_session_id": session_context.session().parent_session_id,
            "turn_id": intent.turn_id,
            "tool_call_id": intent.tool_call_id,
            "tool_name": execution_request.tool_name,
            "approval_key": approval_key,
            "approval_request_id": approval_request_id,
            "args_json": execution_request.payload,
            "trusted_internal_context": trusted_internal_context,
            "source": intent.source,
            "dispatch_kind": match dispatch_kind {
                LegacyToolDispatchKind::LegacyCore => "legacy_core",
                LegacyToolDispatchKind::LegacyApp => "legacy_app",
            },
        });
        let provenance_ref = if session_context.session().session_mode
            == GovernedSessionMode::AdvisoryOnly
            || !session_context
                .allowed_capabilities()
                .contains(Capability::InvokeTool)
        {
            "advisory_only"
        } else {
            "kernel"
        };
        let trust_event = approval_required_trust_event(
            &session_context.session().session_id,
            "conversation.approval",
            provenance_ref,
            rule_id,
            Some(approval_request_id),
            Some(descriptor.name),
        );

        embed_trust_event_payload(payload, trust_event)
    }

    #[cfg(feature = "memory-sqlite")]
    fn persist_approval_request(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        execution_request: &ToolCoreRequest,
        trusted_internal_context: bool,
        descriptor: &crate::tools::ToolDescriptor,
        dispatch_kind: LegacyToolDispatchKind,
        approval_key: &str,
        reason: &str,
        rule_id: &str,
        governance_snapshot_json: serde_json::Value,
    ) -> Result<ApprovalRequirement, String> {
        let repo = SessionRepository::new(&self.memory_config)?;
        let kind = if session_context.session().parent_session_id.is_some() {
            SessionKind::DelegateChild
        } else {
            SessionKind::Root
        };
        let _ = repo.ensure_session(NewSessionRecord {
            session_id: session_context.session().session_id.clone(),
            kind,
            parent_session_id: session_context.session().parent_session_id.clone(),
            label: None,
            state: SessionState::Ready,
        })?;

        let approval_request_id =
            governed_approval_request_id(session_context, descriptor.name, intent);
        let request_payload_json = Self::approval_request_payload_json(
            session_context,
            intent,
            execution_request,
            trusted_internal_context,
            descriptor,
            dispatch_kind,
            &approval_request_id,
            approval_key,
            rule_id,
        );
        let stored = repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id,
            session_id: session_context.session().session_id.clone(),
            turn_id: intent.turn_id.clone(),
            tool_call_id: intent.tool_call_id.clone(),
            tool_name: descriptor.name.to_owned(),
            approval_key: approval_key.to_owned(),
            request_payload_json,
            governance_snapshot_json,
        })?;

        Ok(ApprovalRequirement::governed_tool(
            descriptor.name,
            approval_key,
            reason,
            rule_id,
            Some(stored.approval_request_id),
        ))
    }

    #[cfg(feature = "memory-sqlite")]
    fn has_approval_grant(
        &self,
        session_context: &Context<'_>,
        approval_key: &str,
    ) -> Result<bool, String> {
        let repo = SessionRepository::new(&self.memory_config)?;
        let approval_runtime = OperatorApprovalRuntime::new(&repo);
        let grant = approval_runtime.load_runtime_grant_for_context(
            &session_context.session().session_id,
            session_context.session().parent_session_id.as_deref(),
            approval_key,
        )?;
        Ok(grant.is_some())
    }

    #[cfg(feature = "memory-sqlite")]
    fn maybe_require_governed_tool_approval(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        execution_request: &ToolCoreRequest,
        trusted_internal_context: bool,
        descriptor: &crate::tools::ToolDescriptor,
        dispatch_kind: LegacyToolDispatchKind,
    ) -> Result<Option<ApprovalRequirement>, String> {
        let governance = governance_profile_for_descriptor(descriptor);
        if descriptor.owner != ToolOwner::LegacyApp
            || governance.approval_mode != ToolApprovalMode::PolicyDriven
        {
            return Ok(None);
        }

        let requires_approval = match self.tool_config.approval.mode {
            GovernedToolApprovalMode::Disabled => false,
            GovernedToolApprovalMode::MediumBalanced => {
                governance.risk_class == crate::tools::ToolRiskClass::High
            }
            GovernedToolApprovalMode::Strict => true,
        };
        if !requires_approval {
            return Ok(None);
        }

        let approval_key = Self::approval_key_for_descriptor(descriptor);
        let is_preapproved = self.is_tool_call_preapproved(&approval_key);
        if is_preapproved {
            return Ok(None);
        }
        let is_predenied = self.is_tool_call_predenied(&approval_key);
        if is_predenied {
            return Err(format!(
                "app_tool_denied: governed tool `{approval_key}` is denied by approval policy"
            ));
        }
        let repo = SessionRepository::new(&self.memory_config)?;
        let approval_runtime = OperatorApprovalRuntime::new(&repo);
        let runtime_grant = approval_runtime.load_runtime_grant_for_context(
            &session_context.session().session_id,
            session_context.session().parent_session_id.as_deref(),
            &approval_key,
        )?;
        if runtime_grant.is_some() {
            return Ok(None);
        }

        let reason = format!(
            "operator approval required before running `{}`",
            descriptor.name
        );
        let rule_id = "governed_tool_requires_approval";
        let governance_snapshot_json = json!({
            "governance_scope": governance.scope.as_str(),
            "risk_class": governance.risk_class.as_str(),
            "approval_mode": governance.approval_mode.as_str(),
            "rule_id": rule_id,
            "reason": reason,
        });
        let requirement = self.persist_approval_request(
            session_context,
            intent,
            execution_request,
            trusted_internal_context,
            descriptor,
            dispatch_kind,
            approval_key.as_str(),
            reason.as_str(),
            rule_id,
            governance_snapshot_json,
        )?;
        Ok(Some(requirement))
    }

    #[cfg(not(feature = "memory-sqlite"))]
    fn maybe_require_governed_tool_approval(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        execution_request: &ToolCoreRequest,
        trusted_internal_context: bool,
        descriptor: &crate::tools::ToolDescriptor,
        dispatch_kind: LegacyToolDispatchKind,
    ) -> Result<Option<ApprovalRequirement>, String> {
        let _ = (
            session_context,
            intent,
            descriptor,
            execution_request,
            trusted_internal_context,
            dispatch_kind,
        );
        Ok(None)
    }

    fn governed_tool_requires_operator_approval(
        &self,
        descriptor: &crate::tools::ToolDescriptor,
    ) -> bool {
        let governance = governance_profile_for_descriptor(descriptor);
        match self.tool_config.approval.mode {
            GovernedToolApprovalMode::Disabled => false,
            GovernedToolApprovalMode::MediumBalanced => {
                governance.risk_class == crate::tools::ToolRiskClass::High
            }
            GovernedToolApprovalMode::Strict => {
                governance.approval_mode == ToolApprovalMode::PolicyDriven
            }
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn ensure_governed_tool_session_scope(
        &self,
        repo: &SessionRepository,
        session_context: &Context<'_>,
    ) -> Result<String, String> {
        let session_kind = if session_context.session().parent_session_id.is_some() {
            SessionKind::DelegateChild
        } else {
            SessionKind::Root
        };
        let session_record = NewSessionRecord {
            session_id: session_context.session().session_id.clone(),
            kind: session_kind,
            parent_session_id: session_context.session().parent_session_id.clone(),
            label: None,
            state: SessionState::Ready,
        };
        let _ = repo.ensure_session(session_record)?;
        Self::lineage_root_session_id(repo, session_context)
    }

    #[cfg(feature = "memory-sqlite")]
    fn governed_app_tool_preflight(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        execution_request: &ToolCoreRequest,
        trusted_internal_context: bool,
        descriptor: &crate::tools::ToolDescriptor,
    ) -> Result<LegacyGovernedToolPreflight, String> {
        let governance = governance_profile_for_descriptor(descriptor);
        if descriptor.owner != ToolOwner::LegacyApp
            || governance.approval_mode != ToolApprovalMode::PolicyDriven
        {
            return Ok(LegacyGovernedToolPreflight::Allowed);
        }

        let requires_approval = self.governed_tool_requires_operator_approval(descriptor);
        if !requires_approval {
            return Ok(LegacyGovernedToolPreflight::Allowed);
        }

        let approval_key = format!("tool:{}", descriptor.name);
        let approved_calls = &self.tool_config.approval.approved_calls;
        let approved_by_policy = approved_calls.iter().any(|entry| entry == &approval_key);
        if approved_by_policy {
            return Ok(LegacyGovernedToolPreflight::Allowed);
        }

        let denied_calls = &self.tool_config.approval.denied_calls;
        let denied_by_policy = denied_calls.iter().any(|entry| entry == &approval_key);
        if denied_by_policy {
            let reason = format!(
                "app_tool_denied: governed tool `{approval_key}` is denied by approval policy"
            );
            return Err(reason);
        }

        let repo = SessionRepository::new(&self.memory_config)?;
        let scope_session_id = self.ensure_governed_tool_session_scope(&repo, session_context)?;
        let grant_record = repo.load_approval_grant(&scope_session_id, &approval_key)?;
        if grant_record.is_some() {
            return Ok(LegacyGovernedToolPreflight::Allowed);
        }

        let approval_request_id =
            governed_approval_request_id(session_context, descriptor.name, intent);
        let reason = format!(
            "operator approval required before running `{}`",
            descriptor.name
        );
        let rule_id = "governed_tool_requires_approval";
        let request_payload_json = Self::approval_request_payload_json(
            session_context,
            intent,
            execution_request,
            trusted_internal_context,
            descriptor,
            LegacyToolDispatchKind::LegacyApp,
            &approval_request_id,
            &approval_key,
            rule_id,
        );
        let governance_snapshot_json = json!({
            "governance_scope": governance.scope.as_str(),
            "risk_class": governance.risk_class.as_str(),
            "approval_mode": governance.approval_mode.as_str(),
            "rule_id": rule_id,
            "reason": reason,
        });
        let stored = repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id,
            session_id: session_context.session().session_id.clone(),
            turn_id: intent.turn_id.clone(),
            tool_call_id: intent.tool_call_id.clone(),
            tool_name: descriptor.name.to_owned(),
            approval_key: approval_key.clone(),
            request_payload_json,
            governance_snapshot_json,
        })?;
        let requirement = ApprovalRequirement::governed_tool(
            descriptor.name,
            approval_key,
            reason,
            rule_id,
            Some(stored.approval_request_id),
        );
        Ok(LegacyGovernedToolPreflight::NeedsApproval(requirement))
    }

    #[cfg(feature = "memory-sqlite")]
    fn governed_shell_tool_preflight(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        request: &ToolCoreRequest,
        descriptor: &crate::tools::ToolDescriptor,
    ) -> Result<LegacyGovernedToolPreflight, String> {
        if descriptor.name != crate::tools::SHELL_EXEC_TOOL_NAME {
            return Ok(LegacyGovernedToolPreflight::Allowed);
        }

        let payload = request.payload.as_object();
        let Some(payload) = payload else {
            return Ok(LegacyGovernedToolPreflight::Allowed);
        };
        let command = payload.get("command").and_then(Value::as_str);
        let Some(command) = command else {
            return Ok(LegacyGovernedToolPreflight::Allowed);
        };
        let trimmed_command = command.trim();
        if trimmed_command.is_empty() {
            return Ok(LegacyGovernedToolPreflight::Allowed);
        }
        let normalized_command = crate::tools::shell_policy_ext::validate_shell_command_name(
            trimmed_command,
        )
        .map_err(|reason| {
            if crate::tools::shell_policy_ext::is_repairable_tool_input_reason(reason.as_str()) {
                let stripped = crate::tools::shell_policy_ext::strip_repairable_tool_input_prefix(
                    reason.as_str(),
                );
                return LegacyRepairablePreflight::encode(stripped);
            }
            format!("tool_preflight_denied: {reason}")
        })?;

        let shell_deny = &self.tool_config.shell_deny;
        let hard_denied = shell_deny
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(&normalized_command));
        if hard_denied {
            let reason = format!(
                "tool_preflight_denied: shell command `{normalized_command}` is blocked by shell policy"
            );
            return Err(reason);
        }

        let shell_allow = &self.tool_config.shell_allow;
        let explicitly_allowed = shell_allow
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(&normalized_command));
        let default_allows = self.tool_config.shell_default_mode == "allow";
        if explicitly_allowed || default_allows {
            return Ok(LegacyGovernedToolPreflight::Allowed);
        }

        let requires_approval = self.governed_tool_requires_operator_approval(descriptor);
        if !requires_approval {
            return Ok(LegacyGovernedToolPreflight::Allowed);
        }

        let approval_key =
            crate::tools::shell_policy_ext::shell_exec_approval_key_for_normalized_command(
                normalized_command.as_str(),
            );
        let approved_calls = &self.tool_config.approval.approved_calls;
        let approved_by_policy = approved_calls.iter().any(|entry| entry == &approval_key);
        if approved_by_policy {
            let internal_context =
                crate::tools::shell_policy_ext::shell_exec_internal_approval_context(
                    approval_key.as_str(),
                );
            return Ok(
                LegacyGovernedToolPreflight::AllowedWithTrustedInternalContext(internal_context),
            );
        }

        let denied_calls = &self.tool_config.approval.denied_calls;
        let denied_by_policy = denied_calls.iter().any(|entry| entry == &approval_key);
        if denied_by_policy {
            let reason = format!(
                "tool_preflight_denied: governed tool `{approval_key}` is denied by approval policy"
            );
            return Err(reason);
        }

        let repo = SessionRepository::new(&self.memory_config)?;
        let scope_session_id = self.ensure_governed_tool_session_scope(&repo, session_context)?;
        let grant_record = repo.load_approval_grant(&scope_session_id, &approval_key)?;
        if grant_record.is_some() {
            let internal_context =
                crate::tools::shell_policy_ext::shell_exec_internal_approval_context(
                    approval_key.as_str(),
                );
            return Ok(
                LegacyGovernedToolPreflight::AllowedWithTrustedInternalContext(internal_context),
            );
        }

        let approval_request_id =
            governed_approval_request_id(session_context, descriptor.name, intent);
        let visible_tool_name = crate::tools::model_visible_tool_name(descriptor.name);
        let reason = format!(
            "operator approval required before running shell command `{normalized_command}` via `{visible_tool_name}`"
        );
        let rule_id = crate::tools::shell_policy_ext::SHELL_EXEC_APPROVAL_RULE_ID;
        let request_payload_json = Self::approval_request_payload_json(
            session_context,
            intent,
            request,
            crate::tools::payload_uses_reserved_internal_tool_context(&request.payload),
            descriptor,
            LegacyToolDispatchKind::LegacyCore,
            &approval_request_id,
            &approval_key,
            rule_id,
        );
        let governance = governance_profile_for_descriptor(descriptor);
        let governance_snapshot_json = json!({
            "governance_scope": governance.scope.as_str(),
            "risk_class": governance.risk_class.as_str(),
            "approval_mode": governance.approval_mode.as_str(),
            "rule_id": rule_id,
            "reason": reason,
        });
        let stored = repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id,
            session_id: session_context.session().session_id.clone(),
            turn_id: intent.turn_id.clone(),
            tool_call_id: intent.tool_call_id.clone(),
            tool_name: descriptor.name.to_owned(),
            approval_key: approval_key.clone(),
            request_payload_json,
            governance_snapshot_json,
        })?;
        let requirement = ApprovalRequirement::governed_tool(
            descriptor.name,
            approval_key,
            reason,
            rule_id,
            Some(stored.approval_request_id),
        );
        Ok(LegacyGovernedToolPreflight::NeedsApproval(requirement))
    }

    #[cfg(feature = "memory-sqlite")]
    fn governed_tool_preflight(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        request: &ToolCoreRequest,
        descriptor: &crate::tools::ToolDescriptor,
    ) -> Result<LegacyGovernedToolPreflight, String> {
        let governance = governance_profile_for_descriptor(descriptor);
        if governance.approval_mode != ToolApprovalMode::PolicyDriven {
            return Ok(LegacyGovernedToolPreflight::Allowed);
        }

        if descriptor.name == crate::tools::SHELL_EXEC_TOOL_NAME {
            return self.governed_shell_tool_preflight(
                session_context,
                intent,
                request,
                descriptor,
            );
        }

        self.governed_app_tool_preflight(
            session_context,
            intent,
            request,
            crate::tools::payload_uses_reserved_internal_tool_context(&request.payload),
            descriptor,
        )
    }
}

#[async_trait]
impl LegacyToolDispatcher for DefaultLegacyToolDispatcher {
    fn memory_config(&self) -> Option<&SessionStoreConfig> {
        Some(&self.memory_config)
    }

    async fn preflight_tool_intent(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        execution_request: &ToolCoreRequest,
        trusted_internal_context: bool,
        descriptor: &crate::tools::ToolDescriptor,
        dispatch_kind: LegacyToolDispatchKind,
        budget_state: &AutonomyTurnBudgetState,
    ) -> Result<LegacyToolPreflightOutcome, String> {
        let policy_snapshot = session_context
            .tool_runtime_config()
            .autonomy_policy_snapshot();
        let action_class = descriptor.capability_action_class();
        let capabilities = session_context.allowed_capabilities();
        let policy_input = PolicyDecisionInput {
            snapshot: &policy_snapshot,
            action_class,
            capabilities: capabilities.as_ref(),
            budget: budget_state,
        };
        let autonomy_policy_applies =
            super::super::autonomy_policy::action_mode(&policy_snapshot, action_class).is_some();
        let policy_decision = evaluate_policy(policy_input);
        let mut autonomy_allow_decision = None;
        match policy_decision {
            PolicyDecision::Allow => {
                if autonomy_policy_applies {
                    let decision = Self::autonomy_policy_allow_decision(
                        descriptor.name,
                        &policy_snapshot,
                        action_class,
                    );
                    autonomy_allow_decision = Some(decision);
                }
            }
            PolicyDecision::ApprovalRequired {
                rule_id,
                reason_code,
            } => {
                let reason = render_reason(&policy_snapshot, descriptor.name, reason_code);
                let approval_key = Self::approval_key_for_descriptor(descriptor);

                #[cfg(not(feature = "memory-sqlite"))]
                {
                    let _ = (session_context, intent, approval_key);
                    let failure = TurnFailure::policy_denied(
                        "autonomy_policy_approval_support_missing",
                        reason.clone(),
                    );
                    let decision = Self::autonomy_policy_denied_decision(
                        descriptor.name,
                        &policy_snapshot,
                        action_class,
                        rule_id,
                        reason_code,
                        reason,
                    );
                    return Ok(LegacyToolPreflightOutcome::Denied { failure, decision });
                }

                #[cfg(feature = "memory-sqlite")]
                {
                    let is_preapproved = self.is_tool_call_preapproved(&approval_key);
                    let is_predenied = self.is_tool_call_predenied(&approval_key);
                    if is_predenied {
                        let reason =
                            format!("governed tool `{approval_key}` is denied by approval policy");
                        let failure = TurnFailure::policy_denied("app_tool_denied", reason);
                        let decision = denied_tool_decision(descriptor.name, &failure);
                        return Ok(LegacyToolPreflightOutcome::Denied { failure, decision });
                    }

                    let has_approval_grant =
                        self.has_approval_grant(session_context, approval_key.as_str())?;
                    let autonomy_approval_is_satisfied = is_preapproved || has_approval_grant;
                    if !autonomy_approval_is_satisfied {
                        let governance_snapshot_json = json!({
                            "policy_source": AUTONOMY_POLICY_SOURCE,
                            "decision_kind": ToolDecisionKind::ApprovalRequired,
                            "autonomy_profile": policy_snapshot.profile.as_str(),
                            "capability_action_class": action_class.as_str(),
                            "rule_id": rule_id,
                            "reason_code": reason_code,
                            "reason": reason,
                        });
                        let requirement = self.persist_approval_request(
                            session_context,
                            intent,
                            execution_request,
                            trusted_internal_context,
                            descriptor,
                            dispatch_kind,
                            approval_key.as_str(),
                            reason.as_str(),
                            rule_id,
                            governance_snapshot_json,
                        )?;
                        let decision = Self::autonomy_policy_approval_required_decision(
                            descriptor.name,
                            &policy_snapshot,
                            action_class,
                            rule_id,
                            reason_code,
                            reason,
                        );
                        return Ok(LegacyToolPreflightOutcome::NeedsApproval {
                            requirement,
                            decision,
                        });
                    }

                    let satisfied_reason = if is_preapproved {
                        format!(
                            "configured approval policy already allows `{}` under `{}` product mode",
                            descriptor.name,
                            policy_snapshot.profile.as_str()
                        )
                    } else {
                        format!(
                            "stored approval grant satisfied `{}` under `{}` product mode",
                            descriptor.name,
                            policy_snapshot.profile.as_str()
                        )
                    };
                    let decision = Self::autonomy_policy_grant_satisfied_decision(
                        descriptor.name,
                        &policy_snapshot,
                        action_class,
                        rule_id,
                        reason_code,
                        satisfied_reason,
                    );
                    autonomy_allow_decision = Some(decision);
                }
            }
            PolicyDecision::Deny {
                rule_id,
                reason_code,
            } => {
                let reason = render_reason(&policy_snapshot, descriptor.name, reason_code);
                let failure = TurnFailure::policy_denied(reason_code, reason.clone());
                let decision = Self::autonomy_policy_denied_decision(
                    descriptor.name,
                    &policy_snapshot,
                    action_class,
                    rule_id,
                    reason_code,
                    reason,
                );
                return Ok(LegacyToolPreflightOutcome::Denied { failure, decision });
            }
        }

        match self
            .maybe_require_approval(
                session_context,
                intent,
                execution_request,
                trusted_internal_context,
                descriptor,
                dispatch_kind,
            )
            .await
        {
            Ok(Some(requirement)) => {
                let decision = approval_required_tool_decision(descriptor.name, &requirement);
                Ok(LegacyToolPreflightOutcome::NeedsApproval {
                    requirement,
                    decision,
                })
            }
            Ok(None) => {
                let decision = autonomy_allow_decision
                    .unwrap_or_else(|| generic_allow_tool_decision(descriptor.name));
                Ok(LegacyToolPreflightOutcome::Allow(decision))
            }
            Err(reason) => Err(reason),
        }
    }

    async fn maybe_require_approval(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        execution_request: &ToolCoreRequest,
        trusted_internal_context: bool,
        descriptor: &crate::tools::ToolDescriptor,
        dispatch_kind: LegacyToolDispatchKind,
    ) -> Result<Option<ApprovalRequirement>, String> {
        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (
                session_context,
                intent,
                execution_request,
                trusted_internal_context,
                descriptor,
                dispatch_kind,
            );
            Ok(None)
        }

        #[cfg(feature = "memory-sqlite")]
        {
            let governance = governance_profile_for_descriptor(descriptor);
            let approval_key = Self::approval_key_for_descriptor(descriptor);
            let governed_approval_eligible = descriptor.owner == ToolOwner::LegacyApp
                && governance.approval_mode == ToolApprovalMode::PolicyDriven;
            let approval_key_is_denied = governed_approval_eligible
                && self
                    .tool_config
                    .approval
                    .denied_calls
                    .iter()
                    .any(|entry| entry == &approval_key);

            if approval_key_is_denied {
                return Err(format!(
                    "app_tool_denied: governed tool `{approval_key}` is denied by approval policy"
                ));
            }

            let repo = SessionRepository::new(&self.memory_config)?;
            let kind = if session_context.session().parent_session_id.is_some() {
                SessionKind::DelegateChild
            } else {
                SessionKind::Root
            };
            let _ = repo.ensure_session(NewSessionRecord {
                session_id: session_context.session().session_id.clone(),
                kind,
                parent_session_id: session_context.session().parent_session_id.clone(),
                label: None,
                state: SessionState::Ready,
            })?;

            let scope_session_id = Self::lineage_root_session_id(&repo, session_context)?;
            let session_consent_mode = repo
                .load_session_tool_consent(&scope_session_id)?
                .map(|record| record.mode)
                .unwrap_or(self.tool_config.consent.default_mode);

            let session_consent_requirement = if tool_is_session_consent_exempt(descriptor.name) {
                None
            } else {
                match session_consent_mode {
                    ToolConsentMode::Prompt => Some((
                        "session_tool_consent_prompt_mode",
                        format!(
                            "session confirmation required before running `{}`",
                            descriptor.name
                        ),
                    )),
                    ToolConsentMode::Auto if !tool_is_auto_eligible(descriptor, governance) => {
                        Some((
                            "session_tool_consent_auto_blocked",
                            format!(
                                "`{}` is not eligible for auto mode and needs operator confirmation",
                                descriptor.name
                            ),
                        ))
                    }
                    ToolConsentMode::Auto | ToolConsentMode::Full => None,
                }
            };
            let Some((rule_id, reason)) = session_consent_requirement else {
                return self.maybe_require_governed_tool_approval(
                    session_context,
                    intent,
                    execution_request,
                    trusted_internal_context,
                    descriptor,
                    dispatch_kind,
                );
            };

            let governance_snapshot_json = json!({
                "governance_scope": governance.scope.as_str(),
                "risk_class": governance.risk_class.as_str(),
                "approval_mode": governance.approval_mode.as_str(),
                "session_consent_mode": session_consent_mode.as_str(),
                "rule_id": rule_id,
                "reason": reason,
            });
            let requirement = self.persist_approval_request(
                session_context,
                intent,
                execution_request,
                trusted_internal_context,
                descriptor,
                dispatch_kind,
                approval_key.as_str(),
                reason.as_str(),
                rule_id,
                governance_snapshot_json,
            )?;

            Ok(Some(requirement))
        }
    }

    async fn preflight_tool_execution(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        request: ToolCoreRequest,
        descriptor: &crate::tools::ToolDescriptor,
    ) -> Result<LegacyToolExecutionPreflight, String> {
        let repairable_issue = detect_repairable_tool_request_issue(descriptor, &request);

        if let Some(repairable_issue) = repairable_issue {
            let repairable_reason = repairable_issue.reason(descriptor.name);
            let encoded_reason = LegacyRepairablePreflight::encode(repairable_reason.as_str());
            return Err(encoded_reason);
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (session_context, intent, descriptor);
            Ok(LegacyToolExecutionPreflight::ready(request))
        }

        #[cfg(feature = "memory-sqlite")]
        {
            if descriptor.name != crate::tools::SHELL_EXEC_TOOL_NAME {
                return Ok(LegacyToolExecutionPreflight::ready(request));
            }

            let preflight =
                self.governed_tool_preflight(session_context, intent, &request, descriptor)?;
            match preflight {
                LegacyGovernedToolPreflight::Allowed => {
                    Ok(LegacyToolExecutionPreflight::ready(request))
                }
                LegacyGovernedToolPreflight::NeedsApproval(requirement) => {
                    Ok(LegacyToolExecutionPreflight::NeedsApproval(requirement))
                }
                LegacyGovernedToolPreflight::AllowedWithTrustedInternalContext(
                    internal_context,
                ) => {
                    let mut request = request;
                    let payload = request.payload.as_object_mut().ok_or_else(|| {
                        format!(
                            "tool_preflight_invalid_payload: `{}` payload must be an object",
                            descriptor.name
                        )
                    })?;
                    crate::tools::merge_trusted_internal_tool_context_into_arguments(
                        payload,
                        &internal_context,
                    )?;
                    Ok(LegacyToolExecutionPreflight::Ready {
                        request,
                        trusted_internal_context: true,
                    })
                }
            }
        }
    }

    async fn execute_core_tool(
        &self,
        session_context: &Context<'_>,
        request: ToolCoreRequest,
        trusted_internal_context: bool,
    ) -> Result<ToolCoreOutcome, crate::tools::LegacyToolRequestError> {
        if self.execution_runtime.id() != session_context.runtime().id() {
            return Err(crate::tools::LegacyToolRequestError::RuntimeMismatch);
        }
        let request = ToolCoreRequest {
            tool_name: crate::tools::canonical_tool_name(request.tool_name.as_str()).to_owned(),
            payload: request.payload,
        };
        let execute = async {
            if request.tool_name == "tool.invoke" {
                return Err(crate::tools::LegacyToolRequestError::Input(
                    "legacy kernel replay requires a normalized concrete request".to_owned(),
                ));
            }
            // Session construction has already normalized roots and runtime narrowing.
            // Trusted legacy fields may reach the adapter payload, but never reshape
            // the typed policy context used to authorize this fallback.
            crate::tools::ensure_untrusted_payload_does_not_use_reserved_internal_tool_context(
                request.tool_name.as_str(),
                &request.payload,
                "payload",
            )
            .map_err(crate::tools::LegacyToolRequestError::ReservedContext)?;
            let capabilities = crate::tools::legacy_required_capabilities_for_request(&request);
            let observability = self
                .app_config
                .as_ref()
                .map(|config| config.observability.clone())
                .unwrap_or_else(crate::config::ObservabilityConfig::runtime_default);
            let kernel = session_context.runtime().legacy_kernel();
            if let Some(adapter_name) = kernel.default_legacy_core_tool_adapter_name() {
                return kernel
                    .execute_tool_core(
                        self.legacy_token.pack_id.as_str(),
                        &self.legacy_token,
                        &capabilities,
                        Some(adapter_name),
                        request,
                        session_context,
                    )
                    .await
                    .map_err(crate::tools::LegacyToolRequestError::Legacy);
            }

            // The shipped app has no registered legacy adapter: its final
            // typed-miss boundary owns the remaining context-aware legacy tools.
            // Explicit adapters above remain distinct owners and are never
            // shadowed by this fallback.
            kernel
                .execute_legacy_tool_core_with(
                    self.legacy_token.pack_id.as_str(),
                    &self.legacy_token,
                    &capabilities,
                    "mvp-tools",
                    request,
                    session_context,
                    |request| async {
                        crate::tools::tool_dispatch::execute_tool_core_with_config_and_observability(
                            Some(session_context.runtime()),
                            request,
                            session_context.tool_runtime_config(),
                            &observability,
                        )
                        .map_err(loong_kernel::ToolPlaneError::Execution)
                    },
                )
                .await
                .map_err(crate::tools::LegacyToolRequestError::Legacy)
        };
        if trusted_internal_context {
            return crate::tools::with_trusted_internal_tool_payload_async(execute).await;
        }
        execute.await
    }

    async fn execute_app_tool(
        &self,
        session_context: &Context<'_>,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, String> {
        if self.execution_runtime.id() != session_context.runtime().id() {
            return Err("legacy dispatcher belongs to another Runtime".to_owned());
        }
        let canonical_tool_name =
            crate::tools::canonical_tool_name(request.tool_name.as_str()).to_owned();
        let descriptor = tool_catalog().descriptor(canonical_tool_name.as_str());
        if let Some(descriptor) = descriptor
            && descriptor.owner == ToolOwner::LegacyApp
            && !session_context
                .session()
                .tool_view
                .contains(descriptor.name)
        {
            return Err(format!("tool_not_visible: {}", descriptor.name));
        }

        let requires_kernel_binding = descriptor
            .map(crate::tools::ToolDescriptor::requires_kernel_binding)
            .unwrap_or(false);
        let context_allows_legacy_execution = session_context.session().session_mode
            == GovernedSessionMode::MutatingCapable
            && session_context
                .allowed_capabilities()
                .contains(Capability::InvokeTool);
        if requires_kernel_binding && !context_allows_legacy_execution {
            return Err(
                "app_tool_denied: legacy_tool_authority_denied: session lacks mutating legacy tool authority"
                    .to_owned(),
            );
        }

        let effective_tool_config = self.effective_tool_config_for_session(session_context);

        #[cfg(feature = "memory-sqlite")]
        if matches!(
            canonical_tool_name.as_str(),
            "session_tool_policy_status" | "session_tool_policy_set" | "session_tool_policy_clear"
        ) {
            let app_config = self
                .app_config
                .as_deref()
                .ok_or_else(|| "session policy tools require app runtime config".to_owned())?;
            return crate::tools::session::execute_session_policy_tool(
                ToolCoreRequest {
                    tool_name: canonical_tool_name,
                    payload: request.payload,
                },
                session_context,
                app_config,
            );
        }

        if canonical_tool_name == "session_wait" {
            return crate::tools::wait_for_session_with_config(
                request.payload,
                session_context,
                &self.memory_config,
                &effective_tool_config,
            )
            .await;
        }
        if canonical_tool_name == "task_wait" {
            return crate::tools::wait_for_task_with_config(
                request.payload,
                session_context,
                &self.memory_config,
                &effective_tool_config,
            )
            .await;
        }
        #[cfg(feature = "memory-sqlite")]
        if canonical_tool_name == "sessions_send" {
            return self
                .execute_sessions_send(session_context, request.payload)
                .await;
        }
        crate::tools::execute_legacy_app_tool_in_view(
            request,
            &session_context.session().session_id,
            &self.memory_config,
            &effective_tool_config,
            &session_context.session().tool_view,
        )
    }
}

#[cfg(test)]
#[path = "turn_engine_dispatch_default/tests.rs"]
mod tests;
