use super::*;

#[allow(dead_code)]
impl ConversationTurnCoordinator {
    #[cfg(feature = "memory-sqlite")]
    pub(super) async fn maybe_handle_pending_approval_control_turn<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        runtime: &R,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        binding: ConversationRuntimeBinding<'_>,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> CliResult<Option<String>> {
        let Some(control_decision) = parse_pending_approval_input_decision(user_input) else {
            return Ok(None);
        };

        if let Some(kernel_ctx) = binding.kernel_context() {
            runtime.bootstrap(config, session_id, kernel_ctx).await?;
        }

        let memory_config = store::session_store_config_from_memory_config(&config.memory);
        let repo = SessionRepository::new(&memory_config)?;
        let Some(pending_request) = repo
            .list_approval_requests_for_session(session_id, Some(ApprovalRequestStatus::Pending))?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };

        observe_turn_phase(observer, ConversationTurnPhaseEvent::preparing());
        let session_context = runtime.session_context(config, session_id, binding)?;
        let assembled_context = runtime
            .build_context(config, session_id, true, binding)
            .await?;
        let turn_id = next_conversation_turn_id();
        let preparation = ProviderTurnPreparation::from_assembled_context_with_turn_id(
            config,
            assembled_context,
            user_input,
            turn_id.as_str(),
            None,
        );
        observe_turn_phase(
            observer,
            ConversationTurnPhaseEvent::context_ready(
                preparation.session.messages.len(),
                preparation.session.estimated_tokens,
            ),
        );

        let mut approval_args = serde_json::Map::new();
        approval_args.insert(
            "approval_request_id".to_owned(),
            Value::String(pending_request.approval_request_id.clone()),
        );
        approval_args.insert(
            "decision".to_owned(),
            Value::String(control_decision.approval_decision().as_str().to_owned()),
        );
        if let Some(session_consent_mode) = control_decision.session_mode() {
            approval_args.insert(
                "session_consent_mode".to_owned(),
                Value::String(session_consent_mode.as_str().to_owned()),
            );
        }

        let approval_turn = ProviderTurn {
            assistant_text: String::new(),
            tool_intents: vec![ToolIntent {
                tool_name: "approval_request_resolve".to_owned(),
                args_json: Value::Object(approval_args),
                source: "approval_control".to_owned(),
                session_id: session_context.session_id.clone(),
                turn_id: preparation.turn_id.clone(),
                tool_call_id: format!(
                    "call-approval-control-{}",
                    normalize_pending_approval_control_input(user_input)
                ),
            }],
            raw_meta: Value::Null,
        };

        let resolved_turn = resolve_provider_turn(
            config,
            runtime,
            session_id,
            user_input,
            &preparation,
            Ok(approval_turn),
            error_mode,
            binding,
            None,
            observer,
            None,
        )
        .await;
        let reply = apply_resolved_provider_turn(
            config,
            runtime,
            session_id,
            user_input,
            &preparation,
            &resolved_turn,
            binding,
            observer,
        )
        .await?;
        Ok(Some(reply.reply))
    }

    pub(super) async fn maybe_handle_explicit_skill_activation_control_turn<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        runtime: &R,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        binding: ConversationRuntimeBinding<'_>,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> CliResult<Option<ConversationTurnOutcome>> {
        let Some(explicit_activation) = parse_explicit_skill_activation_input(user_input) else {
            return Ok(None);
        };

        let followup_request = explicit_activation.followup_request.as_str();
        let turn_id = next_conversation_turn_id();

        if let Some(kernel_ctx) = binding.kernel_context() {
            runtime.bootstrap(config, session_id, kernel_ctx).await?;
        }

        observe_turn_phase(observer, ConversationTurnPhaseEvent::preparing());
        let assembled_context = runtime
            .build_context(config, session_id, true, binding)
            .await?;
        let preparation = ProviderTurnPreparation::from_assembled_context_with_turn_id(
            config,
            assembled_context,
            followup_request,
            turn_id.as_str(),
            None,
        );
        observe_turn_phase(
            observer,
            ConversationTurnPhaseEvent::context_ready(
                preparation.session.messages.len(),
                preparation.session.estimated_tokens,
            ),
        );
        let tool_runtime_config =
            crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
        let activation_outcome = crate::tools::execute_tool_core_with_config(
            loong_contracts::ToolCoreRequest {
                tool_name: "external_skills.invoke".to_owned(),
                payload: json!({
                    "skill_id": explicit_activation.skill_id,
                }),
            },
            &tool_runtime_config,
        );
        let activation_outcome = match activation_outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                return match error_mode {
                    ProviderErrorMode::Propagate => Err(error),
                    ProviderErrorMode::InlineMessage => {
                        let synthetic = format_provider_error_reply(&error);
                        persist_reply_turns_raw_with_mode(
                            runtime,
                            session_id,
                            followup_request,
                            &synthetic,
                            ReplyPersistenceMode::InlineProviderError,
                            binding,
                        )
                        .await?;
                        Ok(Some(ConversationTurnOutcome {
                            reply: synthetic,
                            usage: None,
                        }))
                    }
                };
            }
        };
        let payload_summary =
            serde_json::to_string(&activation_outcome.payload).unwrap_or_else(|_| "{}".to_owned());
        let payload_chars = payload_summary.chars().count();
        let tool_result_text = format!(
            "[ok] {}",
            json!({
                "status": activation_outcome.status,
                "tool": "external_skills.invoke",
                "tool_call_id": explicit_skill_activation_tool_call_id(
                    explicit_activation.skill_id.as_str(),
                ),
                "payload_semantics": "external_skill_context",
                "payload_summary": payload_summary,
                "payload_chars": payload_chars,
                "payload_truncated": false,
            })
        );
        let followup_payload = ToolDrivenFollowupPayload::ToolResult {
            text: tool_result_text,
        };
        #[cfg(feature = "memory-sqlite")]
        persist_active_external_skills_from_followup_payload_if_needed(
            config,
            session_id,
            &followup_payload,
        );
        let follow_up_messages = build_turn_reply_followup_messages_with_warning(
            &preparation.session.messages,
            "",
            followup_payload,
            None,
            followup_request,
            None,
        );
        let reply = request_completion_with_raw_fallback(
            runtime,
            config,
            &follow_up_messages,
            binding,
            followup_request,
            None,
        )
        .await;
        persist_reply_turns_raw_with_mode(
            runtime,
            session_id,
            followup_request,
            &reply,
            ReplyPersistenceMode::Success,
            binding,
        )
        .await?;
        Ok(Some(ConversationTurnOutcome { reply, usage: None }))
    }

    pub(super) fn reload_followup_provider_config_after_tool_turn(
        config: &LoongConfig,
        turn: &ProviderTurn,
    ) -> LoongConfig {
        let config_path_from_tool = turn.tool_intents.iter().rev().find_map(|intent| {
            let canonical_tool_name = crate::tools::canonical_tool_name(intent.tool_name.as_str());
            let payload = if canonical_tool_name == "provider.switch" {
                intent.args_json.as_object()
            } else if canonical_tool_name == "tool.invoke" {
                crate::tools::invoked_discoverable_tool_request(&intent.args_json)
                    .filter(|(tool_name, _arguments)| *tool_name == "provider.switch")
                    .and_then(|(_tool_name, arguments)| arguments.as_object())
            } else {
                None
            };

            payload
                .and_then(|payload| payload.get("config_path"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from)
        });

        let config_path = config_path_from_tool.or_else(|| {
            crate::tools::runtime_config::get_tool_runtime_config()
                .config_path
                .clone()
        });
        let Some(config_path) = config_path else {
            return config.clone();
        };

        config
            .reload_provider_runtime_state_from_path(config_path.as_path())
            .unwrap_or_else(|_| config.clone())
    }

    pub(super) async fn handle_turn_via_acp<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        self.handle_turn_via_acp_with_manager(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            binding,
            None,
        )
        .await
    }

    pub(super) async fn handle_turn_via_acp_with_manager<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        acp_manager: Option<Arc<crate::acp::AcpSessionManager>>,
    ) -> CliResult<String> {
        let session_id = address.session_id.as_str();
        let executed = execute_acp_conversation_turn_for_address(
            config,
            address,
            user_input,
            acp_options,
            acp_manager,
        )
        .await?;

        consume_finalized_acp_conversation_turn(
            executed,
            |success| async move {
                let reply = success.result.output_text.clone();

                persist_reply_turns_raw_with_mode(
                    runtime,
                    session_id,
                    user_input,
                    &reply,
                    ReplyPersistenceMode::Success,
                    binding,
                )
                .await?;

                if config.acp.emit_runtime_events {
                    let runtime_events = &success.runtime_events;
                    let persistence_context = &success.persistence_context;
                    let result = &success.result;

                    let _ = persist_acp_runtime_events(
                        runtime,
                        session_id,
                        persistence_context,
                        runtime_events,
                        Some(result),
                        None,
                        binding,
                    )
                    .await;
                }

                Ok(reply)
            },
            |failure| async move {
                let error = failure.error;

                if config.acp.emit_runtime_events {
                    let error_text = error.as_str();
                    let runtime_events = &failure.runtime_events;
                    let persistence_context = &failure.persistence_context;

                    let _ = persist_acp_runtime_events(
                        runtime,
                        session_id,
                        persistence_context,
                        runtime_events,
                        None,
                        Some(error_text),
                        binding,
                    )
                    .await;
                }

                match error_mode {
                    ProviderErrorMode::Propagate => Err(error),
                    ProviderErrorMode::InlineMessage => {
                        let synthetic = format_provider_error_reply(&error);

                        persist_reply_turns_raw_with_mode(
                            runtime,
                            session_id,
                            user_input,
                            &synthetic,
                            ReplyPersistenceMode::InlineProviderError,
                            binding,
                        )
                        .await?;

                        Ok(synthetic)
                    }
                }
            },
        )
        .await
    }
}
