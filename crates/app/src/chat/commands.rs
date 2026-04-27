use super::*;

#[allow(clippy::print_stdout)] // CLI output
pub(super) async fn print_turn_checkpoint_startup_health(runtime: &CliTurnRuntime) {
    ops::print_turn_checkpoint_startup_health(runtime).await;
}

#[allow(clippy::print_stdout)] // CLI output
pub(super) async fn run_concurrent_cli_host_loop(
    runtime: &CliTurnRuntime,
    options: &CliChatOptions,
    shutdown: &ConcurrentCliShutdown,
) -> CliResult<()> {
    if shutdown.is_requested() {
        println!("bye.");
        return Ok(());
    }

    let mut stdin_reader = ConcurrentCliInputReader::new()?;

    loop {
        if shutdown.is_requested() {
            break;
        }

        print!("{CLI_CHAT_COMPOSER_PROMPT}");
        io::stdout()
            .flush()
            .map_err(|error| format!("flush stdout failed: {error}"))?;

        let next_line = tokio::select! {
            _ = shutdown.wait() => {
                println!();
                None
            },
            line = stdin_reader.next_line() => Some(line?),
        };

        let Some(line) = next_line else {
            break;
        };
        let Some(line) = line else {
            println!();
            break;
        };

        match process_cli_chat_input(runtime, line.trim(), options, None).await? {
            CliChatLoopControl::Continue => continue,
            CliChatLoopControl::Exit => break,
            CliChatLoopControl::AssistantText(assistant_text) => {
                let render_width = detect_cli_chat_render_width();
                let rendered_lines =
                    render_cli_chat_assistant_lines_with_width(&assistant_text, render_width);
                print_rendered_cli_chat_lines(&rendered_lines);
            }
        }
    }

    println!("bye.");
    Ok(())
}

pub(super) async fn process_cli_chat_input(
    runtime: &CliTurnRuntime,
    input: &str,
    options: &CliChatOptions,
    event_sink: Option<&dyn AcpTurnEventSink>,
) -> CliResult<CliChatLoopControl> {
    if input.is_empty() {
        return Ok(CliChatLoopControl::Continue);
    }
    if is_exit_command(&runtime.config, input) {
        return Ok(CliChatLoopControl::Exit);
    }
    match classify_chat_command_match_result(parse_exact_chat_command(
        input,
        &[CLI_CHAT_HELP_COMMAND],
        "usage: /help",
    ))? {
        ChatCommandMatchResult::Matched => {
            print_help();
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::UsageError(usage) => {
            let usage_lines = render_cli_chat_command_usage_lines_with_width(
                &usage,
                detect_cli_chat_render_width(),
            );
            print_rendered_cli_chat_lines(&usage_lines);
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::NotMatched => {}
    }
    match classify_chat_command_match_result(is_cli_chat_status_command(input))? {
        ChatCommandMatchResult::Matched => {
            print_cli_chat_status(runtime, options).await?;
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::UsageError(usage) => {
            let usage_lines = render_cli_chat_command_usage_lines_with_width(
                &usage,
                detect_cli_chat_render_width(),
            );
            print_rendered_cli_chat_lines(&usage_lines);
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::NotMatched => {}
    }
    match classify_chat_command_match_result(is_manual_compaction_command(input))? {
        ChatCommandMatchResult::Matched => {
            print_manual_compaction(runtime).await?;
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::UsageError(usage) => {
            let usage_lines = render_cli_chat_command_usage_lines_with_width(
                &usage,
                detect_cli_chat_render_width(),
            );
            print_rendered_cli_chat_lines(&usage_lines);
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::NotMatched => {}
    }
    match classify_chat_command_match_result(parse_exact_chat_command(
        input,
        &[CLI_CHAT_HISTORY_COMMAND],
        "usage: /history",
    ))? {
        ChatCommandMatchResult::Matched => {
            #[cfg(feature = "memory-sqlite")]
            print_history(
                &runtime.session_id,
                runtime.config.memory.sliding_window,
                runtime.conversation_binding(),
                &runtime.memory_config,
            )
            .await?;
            #[cfg(not(feature = "memory-sqlite"))]
            print_history(
                &runtime.session_id,
                runtime.config.memory.sliding_window,
                runtime.conversation_binding(),
            )
            .await?;
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::UsageError(usage) => {
            let usage_lines = render_cli_chat_command_usage_lines_with_width(
                &usage,
                detect_cli_chat_render_width(),
            );
            print_rendered_cli_chat_lines(&usage_lines);
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::NotMatched => {}
    }
    let fast_lane_limit_result =
        parse_fast_lane_summary_limit(input, runtime.config.memory.sliding_window);
    match fast_lane_limit_result {
        Ok(Some(limit)) => {
            #[cfg(feature = "memory-sqlite")]
            print_fast_lane_summary(
                &runtime.session_id,
                limit,
                runtime.conversation_binding(),
                &runtime.memory_config,
            )
            .await?;
            #[cfg(not(feature = "memory-sqlite"))]
            print_fast_lane_summary(&runtime.session_id, limit, runtime.conversation_binding())
                .await?;
            return Ok(CliChatLoopControl::Continue);
        }
        Ok(None) => {}
        Err(error) => {
            if let Some(usage_lines) = maybe_render_nonfatal_usage_error(error.as_str()) {
                print_rendered_cli_chat_lines(&usage_lines);
                return Ok(CliChatLoopControl::Continue);
            }

            return Err(error);
        }
    }

    let safe_lane_limit_result =
        parse_safe_lane_summary_limit(input, runtime.config.memory.sliding_window);
    match safe_lane_limit_result {
        Ok(Some(limit)) => {
            #[cfg(feature = "memory-sqlite")]
            print_safe_lane_summary(
                &runtime.session_id,
                limit,
                &runtime.config.conversation,
                runtime.conversation_binding(),
                &runtime.memory_config,
            )
            .await?;
            #[cfg(not(feature = "memory-sqlite"))]
            print_safe_lane_summary(
                &runtime.session_id,
                limit,
                &runtime.config.conversation,
                runtime.conversation_binding(),
            )
            .await?;
            return Ok(CliChatLoopControl::Continue);
        }
        Ok(None) => {}
        Err(error) => {
            if let Some(usage_lines) = maybe_render_nonfatal_usage_error(error.as_str()) {
                print_rendered_cli_chat_lines(&usage_lines);
                return Ok(CliChatLoopControl::Continue);
            }

            return Err(error);
        }
    }

    let turn_checkpoint_limit_result =
        parse_turn_checkpoint_summary_limit(input, runtime.config.memory.sliding_window);
    match turn_checkpoint_limit_result {
        Ok(Some(limit)) => {
            #[cfg(feature = "memory-sqlite")]
            print_turn_checkpoint_summary(
                &runtime.turn_coordinator,
                &runtime.config,
                &runtime.session_id,
                limit,
                runtime.conversation_binding(),
                &runtime.memory_config,
            )
            .await?;
            #[cfg(not(feature = "memory-sqlite"))]
            print_turn_checkpoint_summary(
                &runtime.turn_coordinator,
                &runtime.config,
                &runtime.session_id,
                limit,
                runtime.conversation_binding(),
            )
            .await?;
            return Ok(CliChatLoopControl::Continue);
        }
        Ok(None) => {}
        Err(error) => {
            if let Some(usage_lines) = maybe_render_nonfatal_usage_error(error.as_str()) {
                print_rendered_cli_chat_lines(&usage_lines);
                return Ok(CliChatLoopControl::Continue);
            }

            return Err(error);
        }
    }
    match classify_chat_command_match_result(is_turn_checkpoint_repair_command(input))? {
        ChatCommandMatchResult::Matched => {
            print_turn_checkpoint_repair(
                &runtime.turn_coordinator,
                &runtime.config,
                &runtime.session_id,
                runtime.conversation_binding(),
            )
            .await?;
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::UsageError(usage) => {
            let render_width = detect_cli_chat_render_width();
            let usage_lines = render_cli_chat_command_usage_lines_with_width(&usage, render_width);
            print_rendered_cli_chat_lines(&usage_lines);
            return Ok(CliChatLoopControl::Continue);
        }
        ChatCommandMatchResult::NotMatched => {}
    }

    let turn_request = crate::agent_runtime::AgentTurnRequest {
        message: input.to_owned(),
        turn_mode: crate::agent_runtime::AgentTurnMode::Interactive,
        channel_id: runtime.session_address.channel_id.clone(),
        account_id: runtime.session_address.account_id.clone(),
        conversation_id: runtime.session_address.conversation_id.clone(),
        participant_id: runtime.session_address.participant_id.clone(),
        thread_id: runtime.session_address.thread_id.clone(),
        metadata: BTreeMap::new(),
        acp: runtime.explicit_acp_request,
        acp_event_stream: event_sink.is_some(),
        acp_bootstrap_mcp_servers: runtime.effective_bootstrap_mcp_servers.clone(),
        acp_cwd: runtime
            .effective_working_directory
            .as_ref()
            .map(|path| path.display().to_string()),
        live_surface_enabled: true,
    };
    let turn_options = crate::agent_runtime::TurnExecutionOptions {
        event_sink,
        ..Default::default()
    };
    let turn_service = crate::agent_runtime::RuntimeTurnExecutionService::new(runtime);
    let turn_result = turn_service.execute(&turn_request, turn_options).await?;

    Ok(CliChatLoopControl::AssistantText(turn_result.output_text))
}
