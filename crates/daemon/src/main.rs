#![allow(clippy::print_stdout, clippy::print_stderr)] // CLI daemon binary
use clap::Parser;
use loongclaw_daemon::*;

/// Guard that flushes the terminal input queue on drop.
///
/// When a user pastes multi-line text at an interactive prompt, `read_line()`
/// consumes only the first line. The remaining lines stay in the kernel's tty
/// input queue (cooked mode). If the process exits without draining, the parent
/// shell reads those lines as commands — a potential code execution vector.
///
/// This guard calls `tcflush(STDIN_FILENO, TCIFLUSH)` on drop to discard any
/// unread input, covering all exit paths including early returns and panics.
struct StdinGuard;

impl Drop for StdinGuard {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        #[cfg(unix)]
        // SAFETY: tcflush is a POSIX function that discards unread terminal input.
        // STDIN_FILENO is a well-defined constant. No memory or resource concerns.
        unsafe {
            libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
        }
    }
}

#[tokio::main]
async fn main() {
    let _stdin_guard = StdinGuard;
    let cli = Cli::parse();
    let result = match cli.command.unwrap_or(Commands::Demo) {
        Commands::Demo => run_demo().await,
        Commands::RunTask { objective, payload } => run_task_cli(&objective, &payload).await,
        Commands::InvokeConnector { operation, payload } => {
            invoke_connector_cli(&operation, &payload).await
        }
        Commands::AuditDemo => run_audit_demo().await,
        Commands::InitSpec { output } => init_spec_cli(&output),
        Commands::RunSpec { spec, print_audit } => run_spec_cli(&spec, print_audit).await,
        Commands::BenchmarkProgrammaticPressure {
            matrix,
            baseline,
            output,
            enforce_gate,
            preflight_fail_on_warnings,
        } => {
            run_programmatic_pressure_benchmark_cli(
                &matrix,
                baseline.as_deref(),
                &output,
                enforce_gate,
                preflight_fail_on_warnings,
                Some(native_spec_tool_executor),
            )
            .await
        }
        Commands::BenchmarkProgrammaticPressureLint {
            matrix,
            baseline,
            output,
            enforce_gate,
            fail_on_warnings,
        } => run_programmatic_pressure_baseline_lint_cli(
            &matrix,
            baseline.as_deref(),
            &output,
            enforce_gate,
            fail_on_warnings,
        ),
        Commands::BenchmarkWasmCache {
            wasm,
            output,
            cold_iterations,
            hot_iterations,
            warmup_iterations,
            enforce_gate,
            min_speedup_ratio,
        } => run_wasm_cache_benchmark_cli(
            &wasm,
            &output,
            cold_iterations,
            hot_iterations,
            warmup_iterations,
            enforce_gate,
            min_speedup_ratio,
        ),
        Commands::BenchmarkMemoryContext {
            output,
            temp_root,
            history_turns,
            sliding_window,
            summary_max_chars,
            words_per_turn,
            rebuild_iterations,
            hot_iterations,
            warmup_iterations,
            suite_repetitions,
            enforce_gate,
            min_steady_state_speedup_ratio,
        } => run_memory_context_benchmark_cli(
            &output,
            temp_root.as_deref(),
            history_turns,
            sliding_window,
            summary_max_chars,
            words_per_turn,
            rebuild_iterations,
            hot_iterations,
            warmup_iterations,
            suite_repetitions,
            enforce_gate,
            min_steady_state_speedup_ratio,
        ),
        Commands::ValidateConfig {
            config,
            json,
            output,
            locale,
            fail_on_diagnostics,
        } => run_validate_config_cli(
            config.as_deref(),
            json,
            output,
            &locale,
            fail_on_diagnostics,
        ),
        Commands::Onboard {
            output,
            force,
            non_interactive,
            accept_risk,
            provider,
            model,
            api_key_env,
            personality,
            memory_profile,
            system_prompt,
            skip_model_probe,
        } => {
            onboard_cli::run_onboard_cli(onboard_cli::OnboardCommandOptions {
                output,
                force,
                non_interactive,
                accept_risk,
                provider,
                model,
                api_key_env,
                personality,
                memory_profile,
                system_prompt,
                skip_model_probe,
            })
            .await
        }
        Commands::Import {
            output,
            force,
            preview,
            apply,
            json,
            from,
            source_path,
            provider,
            include,
            exclude,
        } => {
            import_cli::run_import_cli(import_cli::ImportCommandOptions {
                output,
                force,
                preview,
                apply,
                json,
                from,
                source_path,
                provider,
                include,
                exclude,
            })
            .await
        }
        Commands::Doctor {
            config,
            fix,
            json,
            skip_model_probe,
        } => {
            doctor_cli::run_doctor_cli(doctor_cli::DoctorCommandOptions {
                config,
                fix,
                json,
                skip_model_probe,
            })
            .await
        }
        Commands::Skills {
            config,
            json,
            command,
        } => skills_cli::run_skills_cli(skills_cli::SkillsCommandOptions {
            config,
            json,
            command,
        }),
        Commands::Channels { config, json } => run_channels_cli(config.as_deref(), json),
        Commands::ListModels { config, json } => run_list_models_cli(config.as_deref(), json).await,
        Commands::RuntimeSnapshot {
            config,
            json,
            output,
            label,
            experiment_id,
            parent_snapshot_id,
        } => run_runtime_snapshot_cli(
            config.as_deref(),
            json,
            output.as_deref(),
            label.as_deref(),
            experiment_id.as_deref(),
            parent_snapshot_id.as_deref(),
        ),
        Commands::RuntimeRestore {
            config,
            snapshot,
            json,
            apply,
        } => runtime_restore_cli::run_runtime_restore_cli(
            runtime_restore_cli::RuntimeRestoreCommandOptions {
                config,
                snapshot,
                json,
                apply,
            },
        ),
        Commands::RuntimeExperiment { command } => {
            runtime_experiment_cli::run_runtime_experiment_cli(command)
        }
        Commands::ListContextEngines { config, json } => {
            run_list_context_engines_cli(config.as_deref(), json)
        }
        Commands::ListMemorySystems { config, json } => {
            run_list_memory_systems_cli(config.as_deref(), json)
        }
        Commands::ListAcpBackends { config, json } => {
            run_list_acp_backends_cli(config.as_deref(), json)
        }
        Commands::ListAcpSessions { config, json } => {
            run_list_acp_sessions_cli(config.as_deref(), json)
        }
        Commands::AcpStatus {
            config,
            session,
            conversation_id,
            route_session_id,
            json,
        } => {
            run_acp_status_cli(
                config.as_deref(),
                session.as_deref(),
                conversation_id.as_deref(),
                route_session_id.as_deref(),
                json,
            )
            .await
        }
        Commands::AcpObservability { config, json } => {
            run_acp_observability_cli(config.as_deref(), json).await
        }
        Commands::AcpEventSummary {
            config,
            session,
            limit,
            json,
        } => run_acp_event_summary_cli(config.as_deref(), session.as_deref(), limit, json),
        Commands::AcpDispatch {
            config,
            session,
            channel,
            conversation_id,
            account_id,
            thread_id,
            json,
        } => run_acp_dispatch_cli(
            config.as_deref(),
            session.as_deref(),
            channel.as_deref(),
            conversation_id.as_deref(),
            account_id.as_deref(),
            thread_id.as_deref(),
            json,
        ),
        Commands::AcpDoctor {
            config,
            backend,
            json,
        } => run_acp_doctor_cli(config.as_deref(), backend.as_deref(), json).await,
        Commands::Ask {
            config,
            session,
            message,
            acp,
            acp_event_stream,
            acp_bootstrap_mcp_server,
            acp_cwd,
        } => {
            run_ask_cli(
                config.as_deref(),
                session.as_deref(),
                &message,
                acp,
                acp_event_stream,
                &acp_bootstrap_mcp_server,
                acp_cwd.as_deref(),
            )
            .await
        }
        Commands::Chat {
            config,
            session,
            acp,
            acp_event_stream,
            acp_bootstrap_mcp_server,
            acp_cwd,
        } => {
            run_chat_cli(
                config.as_deref(),
                session.as_deref(),
                acp,
                acp_event_stream,
                &acp_bootstrap_mcp_server,
                acp_cwd.as_deref(),
            )
            .await
        }
        Commands::SafeLaneSummary {
            config,
            session,
            limit,
            json,
        } => run_safe_lane_summary_cli(config.as_deref(), session.as_deref(), limit, json),
        Commands::TelegramSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                TELEGRAM_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: &target,
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::TelegramServe {
            config,
            once,
            account,
        } => {
            run_channel_serve_cli(
                TELEGRAM_SERVE_CLI_SPEC,
                ChannelServeCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    once,
                    bind_override: None,
                    path_override: None,
                },
            )
            .await
        }
        Commands::FeishuSend {
            config,
            account,
            receive_id_type,
            target,
            target_kind,
            text,
            post_json,
            image_key,
            file_key,
            image_path,
            file_path,
            file_type,
            card,
            uuid,
        } => {
            if target_kind == mvp::channel::ChannelOutboundTargetKind::MessageReply {
                Err(
                    "legacy `feishu-send` no longer supports `message_reply` execution; use `loongclaw feishu reply` for reply targets".to_owned(),
                )
            } else {
                mvp::channel::run_feishu_send(
                    config.as_deref(),
                    account.as_deref(),
                    &mvp::channel::FeishuChannelSendRequest {
                        receive_id: target,
                        receive_id_type,
                        text,
                        post_json,
                        image_key,
                        file_key,
                        image_path,
                        file_path,
                        file_type,
                        card,
                        uuid,
                    },
                )
                .await
            }
        }
        Commands::FeishuServe {
            config,
            account,
            bind,
            path,
        } => {
            run_channel_serve_cli(
                FEISHU_SERVE_CLI_SPEC,
                ChannelServeCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    once: false,
                    bind_override: bind.as_deref(),
                    path_override: path.as_deref(),
                },
            )
            .await
        }
        Commands::Feishu { command } => feishu_cli::run_feishu_command(command).await,
    };
    if let Err(error) = result {
        #[allow(clippy::print_stderr)]
        {
            eprintln!("error: {error}");
        }
        std::process::exit(2);
    }
}
