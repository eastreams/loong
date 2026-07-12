use async_trait::async_trait;

use super::*;

impl DefaultAppToolDispatcher {
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
        session_context: &AppContext,
    ) -> ToolConfig {
        let mut tool_config = self.tool_config.clone();
        if session_context.parent_session_id.is_some() {
            tool_config.sessions.visibility = SessionVisibility::SelfOnly;
        }
        tool_config
    }

    #[cfg(feature = "memory-sqlite")]
    fn effective_tool_view_for_session(
        &self,
        session_context: &AppContext,
    ) -> Result<ToolView, String> {
        let repo = SessionRepository::new(&self.memory_config)?;
        if let Some(session) = repo.load_session(&session_context.session_id)? {
            if session.parent_session_id.is_some() {
                let subagent_contract = match session_context.resolved_subagent_contract() {
                    Some(subagent_contract) => Some(subagent_contract),
                    None => resolve_delegate_child_contract(
                        &repo,
                        &session_context.session_id,
                        self.tool_config.delegate.max_depth,
                    )?,
                };
                return Ok(delegate_child_tool_view_for_contract(
                    &self.tool_config,
                    subagent_contract.as_ref(),
                ));
            }
            return Ok(runtime_tool_view_for_config(&self.tool_config));
        }
        if repo
            .load_session_summary_with_legacy_fallback(&session_context.session_id)?
            .is_some_and(|session| session.kind == SessionKind::DelegateChild)
        {
            let subagent_contract = resolve_delegate_child_contract(
                &repo,
                &session_context.session_id,
                self.tool_config.delegate.max_depth,
            )?;
            return Ok(delegate_child_tool_view_for_contract(
                &self.tool_config,
                subagent_contract.as_ref(),
            ));
        }
        Ok(runtime_tool_view_for_config(&self.tool_config))
    }

    #[cfg(not(feature = "memory-sqlite"))]
    fn effective_tool_view_for_session(
        &self,
        session_context: &AppContext,
    ) -> Result<ToolView, String> {
        let _ = session_context;
        Ok(runtime_tool_view_for_config(&self.tool_config))
    }

    #[cfg(feature = "memory-sqlite")]
    async fn execute_sessions_send(
        &self,
        session_context: &AppContext,
        payload: serde_json::Value,
    ) -> Result<ToolCoreOutcome, String> {
        let app_config = self
            .app_config
            .as_ref()
            .ok_or_else(|| "sessions_send_not_configured".to_owned())?;
        let effective_tool_config = self.effective_tool_config_for_session(session_context);
        crate::tools::messaging::execute_sessions_send_with_config(
            payload,
            &session_context.session_id,
            &self.memory_config,
            &effective_tool_config,
            app_config.as_ref(),
        )
        .await
    }

    #[cfg(feature = "memory-sqlite")]
    fn lineage_root_session_id(
        repo: &SessionRepository,
        session_context: &AppContext,
    ) -> Result<String, String> {
        let session_graph = OperatorSessionGraph::new(repo);
        session_graph.effective_lineage_root_session_id(
            &session_context.session_id,
            session_context.parent_session_id.as_deref(),
        )
    }

    fn autonomy_policy_snapshot(&self) -> crate::tools::runtime_config::AutonomyPolicySnapshot {
        crate::tools::runtime_config::AutonomyPolicySnapshot::from_profile(
            self.tool_config.autonomy_profile,
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
    fn approval_request_payload_json(
        session_context: &AppContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        approval_request_id: &str,
        approval_key: &str,
        rule_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> serde_json::Value {
        let payload = json!({
            "session_id": session_context.session_id,
            "parent_session_id": session_context.parent_session_id,
            "turn_id": intent.turn_id,
            "tool_call_id": intent.tool_call_id,
            "tool_name": descriptor.name,
            "approval_key": approval_key,
            "approval_request_id": approval_request_id,
            "args_json": intent.args_json,
            "source": intent.source,
            "execution_kind": match descriptor.execution_kind {
                ToolExecutionKind::Core => "core",
                ToolExecutionKind::App => "app",
            },
        });
        let provenance_ref = approval_request_provenance_ref(binding);
        let trust_event = approval_required_trust_event(
            &session_context.session_id,
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
        session_context: &AppContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        approval_key: &str,
        reason: &str,
        rule_id: &str,
        governance_snapshot_json: serde_json::Value,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ApprovalRequirement, String> {
        let repo = SessionRepository::new(&self.memory_config)?;
        let kind = if session_context.parent_session_id.is_some() {
            SessionKind::DelegateChild
        } else {
            SessionKind::Root
        };
        let _ = repo.ensure_session(NewSessionRecord {
            session_id: session_context.session_id.clone(),
            kind,
            parent_session_id: session_context.parent_session_id.clone(),
            label: None,
            state: SessionState::Ready,
        })?;

        let approval_request_id =
            governed_approval_request_id(session_context, descriptor.name, intent);
        let request_payload_json = Self::approval_request_payload_json(
            session_context,
            intent,
            descriptor,
            &approval_request_id,
            approval_key,
            rule_id,
            binding,
        );
        let stored = repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id,
            session_id: session_context.session_id.clone(),
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
        session_context: &AppContext,
        approval_key: &str,
    ) -> Result<bool, String> {
        let repo = SessionRepository::new(&self.memory_config)?;
        let approval_runtime = OperatorApprovalRuntime::new(&repo);
        let grant = approval_runtime.load_runtime_grant_for_context(
            &session_context.session_id,
            session_context.parent_session_id.as_deref(),
            approval_key,
        )?;
        Ok(grant.is_some())
    }

    #[cfg(feature = "memory-sqlite")]
    fn maybe_require_governed_tool_approval_with_binding(
        &self,
        session_context: &AppContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<Option<ApprovalRequirement>, String> {
        let governance = governance_profile_for_descriptor(descriptor);
        if descriptor.execution_kind != ToolExecutionKind::App
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
            &session_context.session_id,
            session_context.parent_session_id.as_deref(),
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
        let approval_request = GovernedToolApprovalRequest {
            session_id: &session_context.session_id,
            parent_session_id: session_context.parent_session_id.as_deref(),
            turn_id: &intent.turn_id,
            tool_call_id: &intent.tool_call_id,
            tool_name: descriptor.name,
            args_json: intent.args_json.clone(),
            source: &intent.source,
            governance_scope: governance.scope.as_str(),
            risk_class: governance.risk_class.as_str(),
            approval_mode: governance.approval_mode.as_str(),
            reason: &reason,
            rule_id,
            provenance_ref: approval_request_provenance_ref(binding),
        };
        let stored = approval_runtime.ensure_governed_tool_approval_request(approval_request)?;
        let requirement = ApprovalRequirement::governed_tool(
            descriptor.name,
            approval_key,
            reason,
            rule_id,
            Some(stored.approval_request_id),
        );
        Ok(Some(requirement))
    }

    #[cfg(not(feature = "memory-sqlite"))]
    fn maybe_require_governed_tool_approval_with_binding(
        &self,
        session_context: &AppContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<Option<ApprovalRequirement>, String> {
        let _ = (session_context, intent, descriptor, binding);
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
        session_context: &AppContext,
    ) -> Result<String, String> {
        let session_kind = if session_context.parent_session_id.is_some() {
            SessionKind::DelegateChild
        } else {
            SessionKind::Root
        };
        let session_record = NewSessionRecord {
            session_id: session_context.session_id.clone(),
            kind: session_kind,
            parent_session_id: session_context.parent_session_id.clone(),
            label: None,
            state: SessionState::Ready,
        };
        let _ = repo.ensure_session(session_record)?;
        Self::lineage_root_session_id(repo, session_context)
    }

    #[cfg(feature = "memory-sqlite")]
    fn governed_app_tool_preflight(
        &self,
        session_context: &AppContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<GovernedToolPreflight, String> {
        let governance = governance_profile_for_descriptor(descriptor);
        if descriptor.execution_kind != ToolExecutionKind::App
            || governance.approval_mode != ToolApprovalMode::PolicyDriven
        {
            return Ok(GovernedToolPreflight::Allowed);
        }

        let requires_approval = self.governed_tool_requires_operator_approval(descriptor);
        if !requires_approval {
            return Ok(GovernedToolPreflight::Allowed);
        }

        let approval_key = format!("tool:{}", descriptor.name);
        let approved_calls = &self.tool_config.approval.approved_calls;
        let approved_by_policy = approved_calls.iter().any(|entry| entry == &approval_key);
        if approved_by_policy {
            return Ok(GovernedToolPreflight::Allowed);
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
            return Ok(GovernedToolPreflight::Allowed);
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
            descriptor,
            &approval_request_id,
            &approval_key,
            rule_id,
            binding,
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
            session_id: session_context.session_id.clone(),
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
        Ok(GovernedToolPreflight::NeedsApproval(requirement))
    }

    #[cfg(feature = "memory-sqlite")]
    fn governed_shell_tool_preflight(
        &self,
        session_context: &AppContext,
        intent: &ToolIntent,
        request: &ToolCoreRequest,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<GovernedToolPreflight, String> {
        if descriptor.name != crate::tools::SHELL_EXEC_TOOL_NAME {
            return Ok(GovernedToolPreflight::Allowed);
        }

        let payload = request.payload.as_object();
        let Some(payload) = payload else {
            return Ok(GovernedToolPreflight::Allowed);
        };
        let command = payload.get("command").and_then(Value::as_str);
        let Some(command) = command else {
            return Ok(GovernedToolPreflight::Allowed);
        };
        let trimmed_command = command.trim();
        if trimmed_command.is_empty() {
            return Ok(GovernedToolPreflight::Allowed);
        }
        let normalized_command = crate::tools::shell_policy_ext::validate_shell_command_name(
            trimmed_command,
        )
        .map_err(|reason| {
            if crate::tools::shell_policy_ext::is_repairable_tool_input_reason(reason.as_str()) {
                let stripped = crate::tools::shell_policy_ext::strip_repairable_tool_input_prefix(
                    reason.as_str(),
                );
                return RepairableToolPreflight::encode(stripped);
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
            return Ok(GovernedToolPreflight::Allowed);
        }

        let requires_approval = self.governed_tool_requires_operator_approval(descriptor);
        if !requires_approval {
            return Ok(GovernedToolPreflight::Allowed);
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
            return Ok(GovernedToolPreflight::AllowedWithTrustedInternalContext(
                internal_context,
            ));
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
            return Ok(GovernedToolPreflight::AllowedWithTrustedInternalContext(
                internal_context,
            ));
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
            descriptor,
            &approval_request_id,
            &approval_key,
            rule_id,
            binding,
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
            session_id: session_context.session_id.clone(),
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
        Ok(GovernedToolPreflight::NeedsApproval(requirement))
    }

    #[cfg(feature = "memory-sqlite")]
    fn governed_tool_preflight(
        &self,
        session_context: &AppContext,
        intent: &ToolIntent,
        request: &ToolCoreRequest,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<GovernedToolPreflight, String> {
        let governance = governance_profile_for_descriptor(descriptor);
        if governance.approval_mode != ToolApprovalMode::PolicyDriven {
            return Ok(GovernedToolPreflight::Allowed);
        }

        if descriptor.name == crate::tools::SHELL_EXEC_TOOL_NAME {
            return self.governed_shell_tool_preflight(
                session_context,
                intent,
                request,
                descriptor,
                binding,
            );
        }

        self.governed_app_tool_preflight(session_context, intent, descriptor, binding)
    }
}

impl Default for DefaultAppToolDispatcher {
    fn default() -> Self {
        Self::runtime()
    }
}

#[async_trait]
impl AppToolDispatcher for DefaultAppToolDispatcher {
    fn memory_config(&self) -> Option<&SessionStoreConfig> {
        Some(&self.memory_config)
    }

    async fn preflight_tool_intent_with_binding(
        &self,
        session_context: &AppContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
        budget_state: &AutonomyTurnBudgetState,
    ) -> Result<ToolPreflightOutcome, String> {
        let policy_snapshot = self.autonomy_policy_snapshot();
        let action_class = descriptor.capability_action_class();
        let policy_input = PolicyDecisionInput {
            snapshot: &policy_snapshot,
            action_class,
            binding,
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
                let reason =
                    render_reason(&policy_snapshot, action_class, descriptor.name, reason_code);
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
                    return Ok(ToolPreflightOutcome::Denied { failure, decision });
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
                        return Ok(ToolPreflightOutcome::Denied { failure, decision });
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
                            descriptor,
                            approval_key.as_str(),
                            reason.as_str(),
                            rule_id,
                            governance_snapshot_json,
                            binding,
                        )?;
                        let decision = Self::autonomy_policy_approval_required_decision(
                            descriptor.name,
                            &policy_snapshot,
                            action_class,
                            rule_id,
                            reason_code,
                            reason,
                        );
                        return Ok(ToolPreflightOutcome::NeedsApproval {
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
                let reason =
                    render_reason(&policy_snapshot, action_class, descriptor.name, reason_code);
                let failure = TurnFailure::policy_denied(reason_code, reason.clone());
                let decision = Self::autonomy_policy_denied_decision(
                    descriptor.name,
                    &policy_snapshot,
                    action_class,
                    rule_id,
                    reason_code,
                    reason,
                );
                return Ok(ToolPreflightOutcome::Denied { failure, decision });
            }
        }

        match self
            .maybe_require_approval_with_binding(session_context, intent, descriptor, binding)
            .await
        {
            Ok(Some(requirement)) => {
                let decision = approval_required_tool_decision(descriptor.name, &requirement);
                Ok(ToolPreflightOutcome::NeedsApproval {
                    requirement,
                    decision,
                })
            }
            Ok(None) => {
                let decision = autonomy_allow_decision
                    .unwrap_or_else(|| generic_allow_tool_decision(descriptor.name));
                Ok(ToolPreflightOutcome::Allow(decision))
            }
            Err(reason) => Err(reason),
        }
    }

    async fn maybe_require_approval_with_binding(
        &self,
        session_context: &AppContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<Option<ApprovalRequirement>, String> {
        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (session_context, intent, descriptor, binding);
            Ok(None)
        }

        #[cfg(feature = "memory-sqlite")]
        {
            let _ = binding;
            let governance = governance_profile_for_descriptor(descriptor);
            let approval_key = Self::approval_key_for_descriptor(descriptor);
            let governed_approval_eligible = descriptor.execution_kind == ToolExecutionKind::App
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
            let kind = if session_context.parent_session_id.is_some() {
                SessionKind::DelegateChild
            } else {
                SessionKind::Root
            };
            let _ = repo.ensure_session(NewSessionRecord {
                session_id: session_context.session_id.clone(),
                kind,
                parent_session_id: session_context.parent_session_id.clone(),
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
                return self.maybe_require_governed_tool_approval_with_binding(
                    session_context,
                    intent,
                    descriptor,
                    binding,
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
                descriptor,
                approval_key.as_str(),
                reason.as_str(),
                rule_id,
                governance_snapshot_json,
                binding,
            )?;

            Ok(Some(requirement))
        }
    }

    async fn preflight_tool_execution_with_binding(
        &self,
        session_context: &AppContext,
        intent: &ToolIntent,
        request: ToolCoreRequest,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolExecutionPreflight, String> {
        let repairable_issue = detect_repairable_tool_request_issue(descriptor, &request);

        if let Some(repairable_issue) = repairable_issue {
            let repairable_reason = repairable_issue.reason(descriptor.name);
            let encoded_reason = RepairableToolPreflight::encode(repairable_reason.as_str());
            return Err(encoded_reason);
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (session_context, intent, descriptor, binding);
            Ok(ToolExecutionPreflight::ready(request))
        }

        #[cfg(feature = "memory-sqlite")]
        {
            if descriptor.name != crate::tools::SHELL_EXEC_TOOL_NAME {
                return Ok(ToolExecutionPreflight::ready(request));
            }

            let preflight = self.governed_tool_preflight(
                session_context,
                intent,
                &request,
                descriptor,
                binding,
            )?;
            match preflight {
                GovernedToolPreflight::Allowed => Ok(ToolExecutionPreflight::ready(request)),
                GovernedToolPreflight::NeedsApproval(requirement) => {
                    Ok(ToolExecutionPreflight::NeedsApproval(requirement))
                }
                GovernedToolPreflight::AllowedWithTrustedInternalContext(internal_context) => {
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
                    Ok(ToolExecutionPreflight::Ready {
                        request,
                        trusted_internal_context: true,
                    })
                }
            }
        }
    }

    async fn execute_app_tool(
        &self,
        session_context: &AppContext,
        request: ToolCoreRequest,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolCoreOutcome, String> {
        let canonical_tool_name = crate::tools::canonical_tool_name(request.tool_name.as_str());
        let effective_tool_view = self.effective_tool_view_for_session(session_context)?;
        let descriptor = tool_catalog().descriptor(canonical_tool_name);
        let has_app_context = binding.context().is_some();

        if let Some(descriptor) = descriptor
            && descriptor.execution_kind == ToolExecutionKind::App
            && (!session_context.tool_view.contains(descriptor.name)
                || !effective_tool_view.contains(descriptor.name))
        {
            return Err(format!("tool_not_visible: {}", descriptor.name));
        }

        let requires_kernel_binding = descriptor
            .map(crate::tools::ToolDescriptor::requires_kernel_binding)
            .unwrap_or(false);
        let effective_tool_config = self.effective_tool_config_for_session(session_context);

        #[cfg(feature = "memory-sqlite")]
        if canonical_tool_name == "session_continue" {
            let app_config = self
                .app_config
                .as_ref()
                .ok_or_else(|| "session_continue_not_configured".to_owned())?;
            let runtime = load_default_conversation_runtime(app_config.as_ref())?;
            return crate::tools::continue_session_with_runtime(
                request.payload,
                session_context,
                &session_context.session_id,
                &self.memory_config,
                &effective_tool_config,
                app_config.as_ref(),
                &runtime,
                binding,
            )
            .await;
        }

        if requires_kernel_binding && !has_app_context {
            return Err("app_tool_denied: no_app_context".to_owned());
        }

        if canonical_tool_name == "session_wait" {
            return crate::tools::wait_for_session_with_config(
                request.payload,
                &session_context.session_id,
                &self.memory_config,
                &effective_tool_config,
            )
            .await;
        }
        if canonical_tool_name == "task_wait" {
            return crate::tools::wait_for_task_with_config(
                request.payload,
                &session_context.session_id,
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
        crate::tools::execute_app_tool_with_visibility_checked_config(
            request,
            &session_context.session_id,
            &self.memory_config,
            &effective_tool_config,
        )
    }
}
