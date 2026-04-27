use super::*;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Mutex as StdMutex;

#[test]
fn provider_turn_session_state_appends_user_input_and_keeps_estimate() {
    let session = ProviderTurnSessionState::from_assembled_context(
        AssembledConversationContext {
            messages: vec![serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            artifacts: vec![],
            estimated_tokens: Some(42),
            prompt_fragments: Vec::new(),
            system_prompt_addition: None,
        },
        "hello world",
        None,
    );

    assert_eq!(session.estimated_tokens, Some(42));
    assert_eq!(session.messages.len(), 2);
    assert_eq!(session.messages[1]["role"], "user");
    assert_eq!(session.messages[1]["content"], "hello world");
    assert_eq!(
        session.prompt_frame_summary().turn_ephemeral_segment_count,
        1
    );
    assert!(
        session
            .prompt_frame_summary()
            .turn_ephemeral_estimated_tokens
            > 0
    );
}

#[test]
fn provider_turn_session_state_after_turn_messages_appends_reply() {
    let session = ProviderTurnSessionState::from_assembled_context(
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "hello world",
        None,
    );

    let messages = session.after_turn_messages("done");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(messages[2]["content"], "done");
}

#[test]
fn provider_turn_reply_tail_phase_captures_reply_and_after_turn_context() {
    let session = ProviderTurnSessionState::from_assembled_context(
        AssembledConversationContext {
            messages: vec![serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            artifacts: vec![],
            estimated_tokens: Some(42),
            prompt_fragments: Vec::new(),
            system_prompt_addition: None,
        },
        "hello world",
        None,
    );

    let phase = ProviderTurnReplyTailPhase::from_session(&session, "done");

    assert_eq!(phase.reply(), "done");
    assert_eq!(phase.estimated_tokens(), Some(42));
    assert_eq!(phase.after_turn_messages().len(), 3);
    assert_eq!(phase.after_turn_messages()[2]["role"], "assistant");
    assert_eq!(phase.after_turn_messages()[2]["content"], "done");
}

#[test]
fn provider_turn_followup_preparation_preserves_stable_prefix_hash_and_updates_tail_hash() {
    let base_fragment = crate::conversation::PromptFragment::new(
        "base-system",
        crate::conversation::PromptLane::BaseSystem,
        "base-system",
        "base system prompt",
        crate::conversation::ContextArtifactKind::SystemPrompt,
    )
    .with_cacheable(true);
    let assembled = AssembledConversationContext {
        messages: vec![
            serde_json::json!({
                "role": "system",
                "content": "base system prompt"
            }),
            serde_json::json!({
                "role": "assistant",
                "content": "recent assistant turn"
            }),
        ],
        artifacts: vec![
            crate::conversation::ContextArtifactDescriptor {
                message_index: 0,
                artifact_kind: crate::conversation::ContextArtifactKind::SystemPrompt,
                maskable: false,
                streaming_policy: crate::conversation::ToolOutputStreamingPolicy::BufferFull,
            },
            crate::conversation::ContextArtifactDescriptor {
                message_index: 1,
                artifact_kind: crate::conversation::ContextArtifactKind::ConversationTurn,
                maskable: false,
                streaming_policy: crate::conversation::ToolOutputStreamingPolicy::BufferFull,
            },
        ],
        estimated_tokens: Some(24),
        prompt_fragments: vec![base_fragment],
        system_prompt_addition: None,
    };
    let preparation = ProviderTurnPreparation::from_assembled_context(
        &LoongConfig::default(),
        assembled,
        "use the recent result",
        None,
    );
    let mut followup_messages = preparation.session.messages.clone();

    followup_messages.push(serde_json::json!({
        "role": "assistant",
        "content": "[tool_result] compacted tail"
    }));
    followup_messages.push(serde_json::json!({
        "role": "user",
        "content": "continue"
    }));

    let followup_preparation = preparation.for_followup_messages(followup_messages);
    let initial_summary = preparation.session.prompt_frame_summary();
    let followup_summary = followup_preparation.session.prompt_frame_summary();

    assert_eq!(
        initial_summary.stable_prefix_hash_sha256,
        followup_summary.stable_prefix_hash_sha256
    );
    assert_eq!(
        initial_summary.cached_prefix_sha256,
        followup_summary.cached_prefix_sha256
    );
    assert_ne!(
        initial_summary.turn_ephemeral_hash_sha256,
        followup_summary.turn_ephemeral_hash_sha256
    );
    assert!(
        followup_summary.turn_ephemeral_segment_count
            > initial_summary.turn_ephemeral_segment_count
    );
}

#[test]
fn provider_turn_followup_preparation_retains_original_tail_across_multiple_followups() {
    let base_fragment = crate::conversation::PromptFragment::new(
        "base-system",
        crate::conversation::PromptLane::BaseSystem,
        "base-system",
        "base system prompt",
        crate::conversation::ContextArtifactKind::SystemPrompt,
    )
    .with_cacheable(true);
    let assembled = AssembledConversationContext {
        messages: vec![
            serde_json::json!({
                "role": "system",
                "content": "base system prompt"
            }),
            serde_json::json!({
                "role": "assistant",
                "content": "recent assistant turn"
            }),
        ],
        artifacts: vec![
            crate::conversation::ContextArtifactDescriptor {
                message_index: 0,
                artifact_kind: crate::conversation::ContextArtifactKind::SystemPrompt,
                maskable: false,
                streaming_policy: crate::conversation::ToolOutputStreamingPolicy::BufferFull,
            },
            crate::conversation::ContextArtifactDescriptor {
                message_index: 1,
                artifact_kind: crate::conversation::ContextArtifactKind::ConversationTurn,
                maskable: false,
                streaming_policy: crate::conversation::ToolOutputStreamingPolicy::BufferFull,
            },
        ],
        estimated_tokens: Some(24),
        prompt_fragments: vec![base_fragment],
        system_prompt_addition: None,
    };
    let preparation = ProviderTurnPreparation::from_assembled_context(
        &LoongConfig::default(),
        assembled,
        "use the recent result",
        None,
    );
    let mut first_followup_messages = preparation.session.messages.clone();

    first_followup_messages.push(serde_json::json!({
        "role": "assistant",
        "content": "[tool_result] compacted tail"
    }));
    first_followup_messages.push(serde_json::json!({
        "role": "user",
        "content": "continue"
    }));

    let first_followup_preparation = preparation.for_followup_messages(first_followup_messages);
    let mut second_followup_messages = first_followup_preparation.session.messages.clone();

    second_followup_messages.push(serde_json::json!({
        "role": "assistant",
        "content": "[tool_result] second tail"
    }));
    second_followup_messages.push(serde_json::json!({
        "role": "user",
        "content": "continue again"
    }));

    let second_followup_preparation =
        first_followup_preparation.for_followup_messages(second_followup_messages);
    let second_summary = second_followup_preparation.session.prompt_frame_summary();
    let tail_bucket = second_summary
        .bucket(crate::conversation::PromptFrameLayer::TurnEphemeralTail)
        .expect("turn-ephemeral bucket should exist");

    assert_eq!(tail_bucket.message_count, 5);
    assert_eq!(second_summary.turn_ephemeral_segment_count, 5);
    assert_eq!(
        second_summary.stable_prefix_hash_sha256,
        preparation
            .session
            .prompt_frame_summary()
            .stable_prefix_hash_sha256
    );
}

#[test]
fn provider_turn_lane_plan_fast_lane_uses_internal_limits() {
    let config = LoongConfig::default();

    let plan = ProviderTurnLanePlan::from_user_input(&config, "read note.md");

    assert_eq!(plan.decision.lane, ExecutionLane::Fast);
    assert_eq!(plan.max_tool_steps, 5);
    assert!(plan.decision.reasons.is_empty());
}

#[test]
fn provider_turn_preparation_derives_lane_plan_and_raw_mode() {
    let config = LoongConfig::default();

    let preparation = ProviderTurnPreparation::from_assembled_context(
        &config,
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "deploy to production and show raw tool output",
        None,
    );

    assert_eq!(preparation.session.messages.len(), 2);
    assert_eq!(preparation.session.messages[1]["role"], "user");
    assert_eq!(
        preparation.session.messages[1]["content"],
        "deploy to production and show raw tool output"
    );
    assert!(preparation.raw_tool_output_requested);
    assert_eq!(preparation.lane_plan.decision.lane, ExecutionLane::Safe);
    assert_eq!(preparation.lane_plan.max_tool_steps, 2);
}

#[test]
fn provider_turn_lane_plan_safe_plan_path_requires_safe_lane_and_tool_intents() {
    let config = LoongConfig::default();

    let safe_plan =
        ProviderTurnLanePlan::from_user_input(&config, "deploy to production and rotate the token");
    let tool_turn = ProviderTurn {
        assistant_text: "preface".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name: "shell.exec".to_owned(),
            args_json: json!({"command": "echo hi"}),
            source: "provider_tool_call".to_owned(),
            session_id: "session-safe".to_owned(),
            turn_id: "turn-safe".to_owned(),
            tool_call_id: "call-safe".to_owned(),
        }],
        raw_meta: Value::Null,
    };

    assert_eq!(safe_plan.decision.lane, ExecutionLane::Safe);
    assert!(safe_plan.should_use_safe_lane_plan_path(&config, &tool_turn));
    assert!(!safe_plan.should_use_safe_lane_plan_path(
        &config,
        &ProviderTurn {
            tool_intents: Vec::new(),
            ..tool_turn.clone()
        }
    ));

    let fast_plan = ProviderTurnLanePlan::from_user_input(&config, "say hello");
    assert_eq!(fast_plan.decision.lane, ExecutionLane::Fast);
    assert!(!fast_plan.should_use_safe_lane_plan_path(&config, &tool_turn));
}

#[derive(Default)]
struct MissingToolContinuationRuntime {
    queued_turns: StdMutex<Vec<ProviderTurn>>,
    request_turn_messages: StdMutex<Vec<Vec<Value>>>,
}

#[async_trait]
impl ConversationRuntime for MissingToolContinuationRuntime {
    async fn build_messages(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Ok(vec![json!({
            "role": "system",
            "content": "continuation test"
        })])
    }

    async fn request_completion(
        &self,
        _config: &LoongConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        panic!("request_completion should not run in missing-tool continuation tests")
    }

    async fn request_turn(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _turn_id: &str,
        messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        self.request_turn_messages
            .lock()
            .expect("request-turn messages lock should not be poisoned")
            .push(messages.to_vec());
        let mut queued_turns = self
            .queued_turns
            .lock()
            .expect("queued turns lock should not be poisoned");
        if queued_turns.is_empty() {
            panic!("request_turn called without a queued ProviderTurn");
        }
        Ok(queued_turns.remove(0))
    }

    async fn request_turn_streaming(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
        _on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        panic!("request_turn_streaming should not run in missing-tool continuation tests")
    }

    async fn persist_turn(
        &self,
        _session_id: &str,
        _role: &str,
        _content: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        Ok(())
    }
}

fn provider_continuation_test_preparation(
    config: &LoongConfig,
    user_input: &str,
) -> ProviderTurnPreparation {
    ProviderTurnPreparation::from_assembled_context(
        config,
        AssembledConversationContext::from_messages(vec![json!({
            "role": "system",
            "content": "sys"
        })]),
        user_input,
        None,
    )
}

fn provider_continuation_test_continue_phase_with_lane(
    config: &LoongConfig,
    assistant_preface: String,
    had_tool_intents: bool,
    supports_provider_turn_followup: bool,
    malformed_parse_followup_turn: bool,
    turn_result: TurnResult,
) -> ProviderTurnContinuePhase {
    ProviderTurnContinuePhase::new(
        2,
        ProviderTurnLaneExecution {
            lane: ExecutionLane::Fast,
            assistant_preface,
            provider_usage: None,
            had_tool_intents,
            tool_request_summary: None,
            discovery_search_turn: false,
            search_tool_intents: 0,
            malformed_parse_followup_turn,
            supports_provider_turn_followup,
            raw_tool_output_requested: false,
            turn_result,
            safe_lane_terminal_route: None,
            tool_events: Vec::new(),
        },
        None,
        config.clone(),
        None,
    )
}

fn provider_continuation_test_intent(
    session_id: &str,
    turn_id: &str,
    tool_call_id: &str,
    tool_id: &str,
    arguments: Value,
) -> ToolIntent {
    ToolIntent {
        tool_name: tool_id.to_owned(),
        args_json: arguments,
        source: "provider_tool_call".to_owned(),
        session_id: session_id.to_owned(),
        turn_id: turn_id.to_owned(),
        tool_call_id: tool_call_id.to_owned(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_continuation_recovers_malformed_parse_followup_without_real_tool_call() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let mut config = LoongConfig::default();
    config.tools.file_root = Some(temp_dir.path().display().to_string());

    let user_input = "Write the repaired response to recovered.txt";
    let preparation = provider_continuation_test_preparation(&config, user_input);
    let continue_phase = provider_continuation_test_continue_phase_with_lane(
        &config,
        "let me retry.\n<function=shell.exec><parameter=command>ls /root</parameter>".to_owned(),
        false,
        true,
        true,
        TurnResult::FinalText(String::new()),
    );
    let runtime = MissingToolContinuationRuntime {
        queued_turns: StdMutex::new(vec![
            ProviderTurn {
                assistant_text: String::new(),
                tool_intents: vec![provider_continuation_test_intent(
                    "session-malformed",
                    "turn-write",
                    "call-write",
                    "write",
                    json!({
                        "path": "recovered.txt",
                        "content": "recovered body"
                    }),
                )],
                raw_meta: Value::Null,
            },
            ProviderTurn {
                assistant_text: "[followup_state:done]\nFinished writing recovered.txt.".to_owned(),
                tool_intents: Vec::new(),
                raw_meta: Value::Null,
            },
        ]),
        request_turn_messages: StdMutex::new(Vec::new()),
    };
    let turn_loop_policy = ProviderTurnLoopPolicy::from_config(&config);
    let mut turn_loop_state = ProviderTurnLoopState::default();

    let resolved = resolve_provider_turn_reply(
        &runtime,
        &config,
        "session-malformed",
        &preparation,
        &continue_phase,
        user_input,
        &turn_loop_policy,
        &mut turn_loop_state,
        3,
        ConversationRuntimeBinding::advisory_only(),
        None,
        None,
        None,
    )
    .await;

    assert_eq!(
        resolved.reply_text(),
        Some("Finished writing recovered.txt.")
    );
    let request_turn_messages = runtime
        .request_turn_messages
        .lock()
        .expect("request-turn messages lock should not be poisoned");
    assert_eq!(request_turn_messages.len(), 2);
}

#[test]
fn provider_turn_continue_phase_checkpoint_captures_continue_branch_kernel_shape() {
    let config = LoongConfig::default();
    let preparation = ProviderTurnPreparation::from_assembled_context(
        &config,
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "deploy to production",
        None,
    );
    let phase = ProviderTurnContinuePhase::new(
        2,
        ProviderTurnLaneExecution {
            lane: ExecutionLane::Safe,
            assistant_preface: "preface".to_owned(),
            provider_usage: None,
            had_tool_intents: true,
            tool_request_summary: None,
            discovery_search_turn: false,
            search_tool_intents: 0,
            malformed_parse_followup_turn: false,
            supports_provider_turn_followup: false,
            raw_tool_output_requested: false,
            turn_result: TurnResult::ToolError(TurnFailure::retryable(
                "safe_lane_plan_node_retryable_error",
                "transient",
            )),
            safe_lane_terminal_route: Some(SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                source: SafeLaneFailureRouteSource::SessionGovernor,
            }),
            tool_events: Vec::new(),
        },
        None,
        config,
        None,
    );

    let checkpoint = phase.checkpoint(&preparation, "deploy to production", "preface\ntransient");

    assert_eq!(
        checkpoint.request,
        TurnCheckpointRequest::Continue { tool_intents: 2 }
    );
    assert_eq!(
        checkpoint
            .lane
            .as_ref()
            .expect("lane snapshot should be present")
            .result_kind,
        TurnCheckpointResultKind::ToolError
    );
    assert_eq!(
        checkpoint
            .lane
            .as_ref()
            .and_then(|lane| lane.safe_lane_terminal_route)
            .expect("safe-lane route should be present")
            .source,
        SafeLaneFailureRouteSource::SessionGovernor
    );
    assert_eq!(
        checkpoint
            .reply
            .as_ref()
            .expect("reply checkpoint should be present")
            .decision,
        ReplyResolutionMode::CompletionPass
    );
    assert_eq!(
        checkpoint
            .reply
            .as_ref()
            .and_then(|reply| reply.followup_kind),
        Some(ToolDrivenFollowupKind::ToolFailure)
    );
    assert_eq!(
        checkpoint.finalization,
        TurnFinalizationCheckpoint::PersistReply {
            persistence_mode: ReplyPersistenceMode::Success,
            runs_after_turn: true,
            attempts_context_compaction: true,
        }
    );
    assert_eq!(
        checkpoint
            .identity
            .as_ref()
            .expect("identity should be present")
            .assistant_reply_chars,
        "preface\ntransient".chars().count()
    );
}

#[test]
fn scope_provider_turn_tool_intents_overrides_existing_provider_ids_with_runtime_scope() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![
            ToolIntent {
                tool_name: "tool.search".to_owned(),
                args_json: json!({"query": "read file"}),
                source: "provider_tool_call".to_owned(),
                session_id: String::new(),
                turn_id: String::new(),
                tool_call_id: "call-1".to_owned(),
            },
            ToolIntent {
                tool_name: "tool.invoke".to_owned(),
                args_json: json!({"tool_id": "file.read", "lease": "stub", "arguments": {"path": "README.md"}}),
                source: "provider_tool_call".to_owned(),
                session_id: "already-session".to_owned(),
                turn_id: "already-turn".to_owned(),
                tool_call_id: "call-2".to_owned(),
            },
        ],
        raw_meta: Value::Null,
    };

    let scoped = scope_provider_turn_tool_intents(turn, "session-a", "turn-a");

    // Provider-originated intents always get runtime scope overridden.
    assert_eq!(scoped.tool_intents[0].session_id, "session-a");
    assert_eq!(scoped.tool_intents[0].turn_id, "turn-a");
    assert_eq!(scoped.tool_intents[1].session_id, "session-a");
    assert_eq!(scoped.tool_intents[1].turn_id, "turn-a");
}

#[test]
fn scope_non_provider_turn_tool_intents_preserve_existing_ids() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![
            ToolIntent {
                tool_name: "tool.search".to_owned(),
                args_json: json!({"query": "read file"}),
                source: "local_followup".to_owned(),
                session_id: "existing-session".to_owned(),
                turn_id: "existing-turn".to_owned(),
                tool_call_id: "call-1".to_owned(),
            },
            ToolIntent {
                tool_name: "tool.invoke".to_owned(),
                args_json: json!({"tool_id": "file.read", "lease": "stub", "arguments": {"path": "README.md"}}),
                source: "local_followup".to_owned(),
                session_id: String::new(),
                turn_id: String::new(),
                tool_call_id: "call-2".to_owned(),
            },
        ],
        raw_meta: Value::Null,
    };

    let scoped = scope_provider_turn_tool_intents(turn, "session-a", "turn-a");

    assert_eq!(scoped.tool_intents[0].session_id, "existing-session");
    assert_eq!(scoped.tool_intents[0].turn_id, "existing-turn");
    assert_eq!(scoped.tool_intents[1].session_id, "session-a");
    assert_eq!(scoped.tool_intents[1].turn_id, "turn-a");
}

#[test]
fn reload_followup_provider_config_reads_provider_switch_wrapped_by_tool_invoke() {
    use std::fs;

    let root = std::env::temp_dir().join(format!(
        "loong-provider-switch-followup-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    let config_path = root.join("loong.toml");

    let mut expected = LoongConfig::default();
    let mut openai =
        crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
    openai.model = "gpt-5".to_owned();
    expected.set_active_provider_profile(
        "openai-gpt-5",
        crate::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: openai.clone(),
        },
    );
    expected.provider = openai;
    expected.active_provider = Some("openai-gpt-5".to_owned());
    fs::write(
        &config_path,
        crate::config::render(&expected).expect("render config"),
    )
    .expect("write config");

    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "tool.invoke".to_owned(),
            args_json: json!({
                "tool_id": "provider.switch",
                "lease": "ignored",
                "arguments": {
                    "selector": "openai",
                    "config_path": config_path.to_string_lossy()
                }
            }),
            source: "provider_tool_call".to_owned(),
            session_id: "session-a".to_owned(),
            turn_id: "turn-a".to_owned(),
            tool_call_id: "call-1".to_owned(),
        }],
        raw_meta: Value::Null,
    };

    let reloaded = ConversationTurnCoordinator::reload_followup_provider_config_after_tool_turn(
        &LoongConfig::default(),
        &turn,
    );

    assert_eq!(reloaded.active_provider_id(), Some("openai-gpt-5"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn provider_turn_continue_phase_checkpoint_keeps_direct_reply_without_followup() {
    let preparation = ProviderTurnPreparation::from_assembled_context(
        &LoongConfig::default(),
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "say hello",
        None,
    );
    let phase = ProviderTurnContinuePhase::new(
        0,
        ProviderTurnLaneExecution {
            lane: ExecutionLane::Fast,
            assistant_preface: "preface".to_owned(),
            provider_usage: None,
            had_tool_intents: false,
            tool_request_summary: None,
            discovery_search_turn: false,
            search_tool_intents: 0,
            malformed_parse_followup_turn: false,
            supports_provider_turn_followup: false,
            raw_tool_output_requested: false,
            turn_result: TurnResult::FinalText("hello there".to_owned()),
            safe_lane_terminal_route: None,
            tool_events: Vec::new(),
        },
        None,
        LoongConfig::default(),
        None,
    );

    let checkpoint = phase.checkpoint(&preparation, "say hello", "hello there");

    assert_eq!(
        checkpoint.request,
        TurnCheckpointRequest::Continue { tool_intents: 0 }
    );
    assert_eq!(
        checkpoint
            .lane
            .as_ref()
            .expect("lane snapshot should be present")
            .result_kind,
        TurnCheckpointResultKind::FinalText
    );
    assert_eq!(
        checkpoint
            .reply
            .as_ref()
            .expect("reply checkpoint should be present")
            .decision,
        ReplyResolutionMode::Direct
    );
    assert_eq!(
        checkpoint
            .reply
            .as_ref()
            .and_then(|reply| reply.followup_kind),
        None
    );
    assert_eq!(
        checkpoint
            .identity
            .as_ref()
            .expect("identity should be present")
            .assistant_reply_chars,
        "hello there".chars().count()
    );
}

#[test]
fn resolved_provider_turn_checkpoint_preserves_safe_lane_route_provenance() {
    let config = LoongConfig::default();

    let resolved = ResolvedProviderTurn::PersistReply(ResolvedProviderReply {
        reply: "preface\nsafe lane terminal".to_owned(),
        usage: None,
        checkpoint: TurnCheckpointSnapshot {
            identity: Some(TurnCheckpointIdentity::from_turn(
                "deploy to production",
                "preface\nsafe lane terminal",
            )),
            preparation: ProviderTurnPreparation::from_assembled_context(
                &config,
                AssembledConversationContext::from_messages(vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })]),
                "deploy to production",
                None,
            )
            .checkpoint(),
            request: TurnCheckpointRequest::Continue { tool_intents: 1 },
            lane: Some(TurnLaneExecutionSnapshot {
                lane: ExecutionLane::Safe,
                had_tool_intents: true,
                tool_request_summary: None,
                raw_tool_output_requested: false,
                result_kind: TurnCheckpointResultKind::ToolError,
                safe_lane_terminal_route: Some(SafeLaneFailureRoute {
                    decision: SafeLaneFailureRouteDecision::Terminal,
                    reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                    source: SafeLaneFailureRouteSource::SessionGovernor,
                }),
            }),
            reply: Some(TurnReplyCheckpoint {
                decision: ReplyResolutionMode::CompletionPass,
                followup_kind: Some(ToolDrivenFollowupKind::ToolFailure),
                continuation_state: None,
            }),
            finalization: TurnFinalizationCheckpoint::PersistReply {
                persistence_mode: ReplyPersistenceMode::Success,
                runs_after_turn: true,
                attempts_context_compaction: true,
            },
        },
    });
    let snapshot = resolved.checkpoint();

    assert_eq!(snapshot.preparation.lane, ExecutionLane::Safe);
    assert_eq!(snapshot.preparation.context_message_count, 2);
    assert_eq!(
        snapshot.preparation.context_fingerprint_sha256,
        checkpoint_context_fingerprint_sha256(&[
            serde_json::json!({
                "role": "system",
                "content": "sys"
            }),
            serde_json::json!({
                "role": "user",
                "content": "deploy to production"
            }),
        ])
    );
    assert_eq!(
        snapshot.request,
        TurnCheckpointRequest::Continue { tool_intents: 1 }
    );
    assert_eq!(
        snapshot.lane.as_ref().expect("lane snapshot").result_kind,
        TurnCheckpointResultKind::ToolError
    );
    assert_eq!(
        snapshot
            .lane
            .as_ref()
            .and_then(|lane| lane.safe_lane_terminal_route)
            .expect("safe-lane route")
            .source,
        SafeLaneFailureRouteSource::SessionGovernor
    );
    assert_eq!(
        snapshot.reply.as_ref().expect("reply checkpoint").decision,
        ReplyResolutionMode::CompletionPass
    );
    assert_eq!(
        snapshot
            .reply
            .as_ref()
            .and_then(|reply| reply.followup_kind),
        Some(ToolDrivenFollowupKind::ToolFailure)
    );
    assert_eq!(
        snapshot.finalization,
        TurnFinalizationCheckpoint::PersistReply {
            persistence_mode: ReplyPersistenceMode::Success,
            runs_after_turn: true,
            attempts_context_compaction: true,
        }
    );
    assert_eq!(
        snapshot
            .identity
            .as_ref()
            .expect("identity should be present")
            .user_input_chars,
        "deploy to production".chars().count()
    );
    assert_eq!(resolved.reply_text(), Some("preface\nsafe lane terminal"));
}

#[test]
fn resolved_provider_turn_checkpoint_keeps_inline_provider_error_terminal_shape() {
    let resolved = ResolvedProviderTurn::PersistReply(ResolvedProviderReply {
        reply: "provider unavailable".to_owned(),
        usage: None,
        checkpoint: TurnCheckpointSnapshot {
            identity: Some(TurnCheckpointIdentity::from_turn(
                "say hello",
                "provider unavailable",
            )),
            preparation: ProviderTurnPreparation::from_assembled_context(
                &LoongConfig::default(),
                AssembledConversationContext::from_messages(vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })]),
                "say hello",
                None,
            )
            .checkpoint(),
            request: TurnCheckpointRequest::FinalizeInlineProviderError,
            lane: None,
            reply: None,
            finalization: TurnFinalizationCheckpoint::PersistReply {
                persistence_mode: ReplyPersistenceMode::InlineProviderError,
                runs_after_turn: true,
                attempts_context_compaction: true,
            },
        },
    });
    let snapshot = resolved.checkpoint();

    assert_eq!(
        snapshot.request,
        TurnCheckpointRequest::FinalizeInlineProviderError
    );
    assert!(snapshot.lane.is_none());
    assert!(snapshot.reply.is_none());
    assert!(snapshot.identity.is_some());
    assert_eq!(
        snapshot.finalization,
        TurnFinalizationCheckpoint::PersistReply {
            persistence_mode: ReplyPersistenceMode::InlineProviderError,
            runs_after_turn: true,
            attempts_context_compaction: true,
        }
    );
    assert_eq!(resolved.reply_text(), Some("provider unavailable"));
}

#[test]
fn resolved_provider_turn_checkpoint_marks_return_error_finalization() {
    let resolved = ResolvedProviderTurn::ReturnError(ResolvedProviderError {
        error: "provider unavailable".to_owned(),
        checkpoint: TurnCheckpointSnapshot {
            identity: None,
            preparation: ProviderTurnPreparation::from_assembled_context(
                &LoongConfig::default(),
                AssembledConversationContext::from_messages(vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })]),
                "say hello",
                None,
            )
            .checkpoint(),
            request: TurnCheckpointRequest::ReturnError,
            lane: None,
            reply: None,
            finalization: TurnFinalizationCheckpoint::ReturnError,
        },
    });
    let snapshot = resolved.checkpoint();

    assert_eq!(snapshot.request, TurnCheckpointRequest::ReturnError);
    assert!(snapshot.identity.is_none());
    assert!(snapshot.lane.is_none());
    assert!(snapshot.reply.is_none());
    assert_eq!(
        snapshot.finalization,
        TurnFinalizationCheckpoint::ReturnError
    );
    assert_eq!(resolved.reply_text(), None);
}

#[test]
fn resolved_provider_turn_terminal_phase_builds_reply_tail_and_checkpoint() {
    let session = ProviderTurnSessionState::from_assembled_context(
        AssembledConversationContext {
            messages: vec![serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            artifacts: vec![],
            estimated_tokens: Some(42),
            prompt_fragments: Vec::new(),
            system_prompt_addition: None,
        },
        "say hello",
        None,
    );
    let resolved = ResolvedProviderTurn::PersistReply(ResolvedProviderReply {
        reply: "done".to_owned(),
        usage: None,
        checkpoint: TurnCheckpointSnapshot {
            identity: Some(TurnCheckpointIdentity::from_turn("say hello", "done")),
            preparation: ProviderTurnPreparation::from_assembled_context(
                &LoongConfig::default(),
                AssembledConversationContext::from_messages(vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })]),
                "say hello",
                None,
            )
            .checkpoint(),
            request: TurnCheckpointRequest::Continue { tool_intents: 0 },
            lane: Some(TurnLaneExecutionSnapshot {
                lane: ExecutionLane::Fast,
                had_tool_intents: false,
                tool_request_summary: None,
                raw_tool_output_requested: false,
                result_kind: TurnCheckpointResultKind::FinalText,
                safe_lane_terminal_route: None,
            }),
            reply: Some(TurnReplyCheckpoint {
                decision: ReplyResolutionMode::Direct,
                followup_kind: None,
                continuation_state: None,
            }),
            finalization: TurnFinalizationCheckpoint::persist_reply(ReplyPersistenceMode::Success),
        },
    });

    let phase = resolved.terminal_phase(&session);

    match phase {
        ProviderTurnTerminalPhase::PersistReply(phase) => {
            assert_eq!(
                phase.checkpoint.request,
                TurnCheckpointRequest::Continue { tool_intents: 0 }
            );
            assert_eq!(phase.tail_phase.reply(), "done");
            assert_eq!(phase.tail_phase.estimated_tokens(), Some(42));
            assert_eq!(phase.tail_phase.after_turn_messages().len(), 3);
            assert_eq!(phase.tail_phase.after_turn_messages()[2]["content"], "done");
        }
        ProviderTurnTerminalPhase::ReturnError(_) => {
            panic!("persist reply should build persist terminal phase")
        }
    }
}

#[test]
fn resolved_provider_turn_terminal_phase_preserves_return_error_checkpoint() {
    let session = ProviderTurnSessionState::from_assembled_context(
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "say hello",
        None,
    );
    let resolved = ResolvedProviderTurn::ReturnError(ResolvedProviderError {
        error: "provider unavailable".to_owned(),
        checkpoint: TurnCheckpointSnapshot {
            identity: None,
            preparation: ProviderTurnPreparation::from_assembled_context(
                &LoongConfig::default(),
                AssembledConversationContext::from_messages(vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })]),
                "say hello",
                None,
            )
            .checkpoint(),
            request: TurnCheckpointRequest::ReturnError,
            lane: None,
            reply: None,
            finalization: TurnFinalizationCheckpoint::ReturnError,
        },
    });

    let phase = resolved.terminal_phase(&session);

    match phase {
        ProviderTurnTerminalPhase::ReturnError(phase) => {
            assert_eq!(phase.checkpoint.request, TurnCheckpointRequest::ReturnError);
            assert_eq!(phase.error, "provider unavailable");
        }
        ProviderTurnTerminalPhase::PersistReply(_) => {
            panic!("return error should build return-error terminal phase")
        }
    }
}

#[test]
fn provider_turn_request_terminal_phase_builds_inline_provider_error_reply() {
    let preparation = ProviderTurnPreparation::from_assembled_context(
        &LoongConfig::default(),
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "say hello",
        None,
    );

    let resolved = ProviderTurnRequestTerminalPhase::persist_inline_provider_error(
        "provider unavailable".to_owned(),
    )
    .resolve(&preparation, "say hello");

    match resolved {
        ResolvedProviderTurn::PersistReply(reply) => {
            assert_eq!(reply.reply, "provider unavailable");
            assert_eq!(
                reply.checkpoint.request,
                TurnCheckpointRequest::FinalizeInlineProviderError
            );
            assert!(reply.checkpoint.lane.is_none());
            assert!(reply.checkpoint.reply.is_none());
            assert_eq!(
                reply.checkpoint.finalization,
                TurnFinalizationCheckpoint::persist_reply(
                    ReplyPersistenceMode::InlineProviderError,
                )
            );
            assert!(reply.checkpoint.identity.is_some());
        }
        ResolvedProviderTurn::ReturnError(_) => {
            panic!("inline provider error should resolve to persisted reply")
        }
    }
}

#[test]
fn provider_turn_request_terminal_phase_builds_return_error_without_reply_identity() {
    let preparation = ProviderTurnPreparation::from_assembled_context(
        &LoongConfig::default(),
        AssembledConversationContext::from_messages(vec![serde_json::json!({
            "role": "system",
            "content": "sys"
        })]),
        "say hello",
        None,
    );

    let resolved =
        ProviderTurnRequestTerminalPhase::return_error("provider unavailable".to_owned())
            .resolve(&preparation, "say hello");

    match resolved {
        ResolvedProviderTurn::ReturnError(error) => {
            assert_eq!(error.error, "provider unavailable");
            assert_eq!(error.checkpoint.request, TurnCheckpointRequest::ReturnError);
            assert!(error.checkpoint.identity.is_none());
            assert!(error.checkpoint.lane.is_none());
            assert!(error.checkpoint.reply.is_none());
            assert_eq!(
                error.checkpoint.finalization,
                TurnFinalizationCheckpoint::ReturnError
            );
        }
        ResolvedProviderTurn::PersistReply(_) => {
            panic!("propagated provider error should resolve to return-error outcome")
        }
    }
}
