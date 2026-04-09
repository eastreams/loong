#![recursion_limit = "256"]
#![allow(clippy::print_stdout, clippy::print_stderr)] // CLI daemon binary
use loongclaw_daemon::*;

/// Discard any unread input from the terminal's tty input queue.
///
/// When a user pastes multi-line text at an interactive prompt, `read_line()`
/// consumes only the first line. The remaining lines stay in the kernel's tty
/// input queue (cooked mode). If the process exits without draining, the parent
/// shell reads those lines as commands — a potential code execution vector.
#[cfg(unix)]
#[allow(unsafe_code)]
fn flush_stdin() {
    // SAFETY: tcflush is a POSIX function that discards unread terminal input.
    // STDIN_FILENO is a well-defined constant. No memory or resource concerns.
    unsafe {
        libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
    }
}

#[cfg(not(unix))]
fn flush_stdin() {}

/// Guard that flushes the terminal input queue on drop.
///
/// Covers normal return and panic unwinding. For `process::exit()` paths,
/// `flush_stdin()` must be called explicitly before exit since
/// `process::exit()` does not run destructors.
struct StdinGuard;

impl Drop for StdinGuard {
    fn drop(&mut self) {
        flush_stdin();
    }
}

fn error_code(error: &str) -> String {
    let trimmed = error.trim();
    let mut segments = trimmed.split(':');
    let raw_candidate = segments.next().unwrap_or_default();
    let candidate = raw_candidate.trim();
    let is_empty = candidate.is_empty();
    let is_stable_code = !is_empty
        && candidate.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });

    if is_stable_code {
        return candidate.to_owned();
    }

    "unclassified".to_owned()
}

fn redacted_command_name(command: &Commands) -> &'static str {
    command.command_kind_for_logging()
}

fn check_legacy_home_migration() {
    if std::env::var_os("LOONG_HOME")
        .as_deref()
        .is_some_and(|v| !v.is_empty())
    {
        return;
    }
    let Some(user_home) = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
    else {
        return;
    };
    if let Some(legacy) = mvp::config::detect_legacy_home(&user_home) {
        let new_home = user_home.join(mvp::config::HOME_DIR_NAME);
        tracing::warn!(
            "Legacy home directory {} found, but {} does not exist. Rename {} to {} to migrate.",
            legacy.display(),
            new_home.display(),
            legacy.display(),
            new_home.display(),
        );
    }
}

#[tokio::main]
async fn main() {
    let _stdin_guard = StdinGuard;
    init_tracing();
    mvp::config::set_active_cli_command_name(mvp::config::detect_invoked_cli_command_name());
    loongclaw_daemon::make_env_compatible();
    check_legacy_home_migration();
    let cli = parse_cli();
    let command_source = if cli.command.is_some() {
        "explicit"
    } else {
        "default"
    };
    let command = cli.command.unwrap_or_else(resolve_default_entry_command);
    let command_kind = command.command_kind_for_logging();
    let redacted_command = redacted_command_name(&command);
    tracing::debug!(
        target: "loongclaw.daemon",
        command_source,
        command = %redacted_command,
        "resolved CLI command"
    );
    let result = match command {
        Commands::Welcome => run_welcome_cli(),
        Commands::Demo => run_demo().await,
        Commands::RunTask { objective, payload } => run_task_cli(&objective, &payload).await,
        Commands::Turn { command } => match command {
            loongclaw_daemon::TurnCommands::Run {
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
        },
        Commands::InvokeConnector { operation, payload } => {
            invoke_connector_cli(&operation, &payload).await
        }
        Commands::AuditDemo => run_audit_demo().await,
        Commands::InitSpec { output, preset } => init_spec_cli(&output, preset),
        Commands::RunSpec {
            spec,
            print_audit,
            render_summary,
            bridge_support,
        } => run_spec_cli(&spec, print_audit, render_summary, &bridge_support).await,
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
            web_search_provider,
            web_search_api_key_env,
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
                web_search_provider,
                web_search_api_key_env,
                personality,
                memory_profile,
                system_prompt,
                skip_model_probe,
            })
            .await
        }
        Commands::Personalize { config } => personalize_cli::run_personalize_cli(config.as_deref()),
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
        Commands::Migrate {
            input,
            output,
            source,
            mode,
            json,
            source_id,
            safe_profile_merge,
            primary_source_id,
            apply_external_skills_plan,
            force,
        } => migrate_cli::run_migrate_cli(migrate_cli::MigrateCommandOptions {
            input,
            output,
            source,
            mode,
            json,
            source_id,
            safe_profile_merge,
            primary_source_id,
            apply_external_skills_plan,
            force,
        }),
        Commands::Doctor {
            config,
            fix,
            json,
            skip_model_probe,
            command,
        } => {
            doctor_cli::run_doctor_cli(doctor_cli::DoctorCommandOptions {
                config,
                fix,
                json,
                skip_model_probe,
                command,
            })
            .await
        }
        Commands::ControlPlaneServe {
            config,
            session,
            bind,
            port,
        } => {
            run_control_plane_serve_cli(
                config.as_deref(),
                session.as_deref(),
                bind.as_deref(),
                port,
            )
            .await
        }
        Commands::Audit {
            config,
            json,
            command,
        } => audit_cli::run_audit_cli(audit_cli::AuditCommandOptions {
            config,
            json,
            command,
        }),
        Commands::Skills {
            config,
            json,
            command,
        } => skills_cli::run_skills_cli(skills_cli::SkillsCommandOptions {
            config,
            json,
            command,
        }),
        Commands::Status { config, json } => {
            status_cli::run_status_cli(config.as_deref(), json).await
        }
        Commands::Tasks {
            config,
            json,
            session,
            command,
        } => {
            tasks_cli::run_tasks_cli(tasks_cli::TasksCommandOptions {
                config,
                json,
                session,
                command,
            })
            .await
        }
        Commands::DelegateChildRun {
            config_path,
            payload_file,
        } => run_detached_delegate_child_cli(&config_path, &payload_file).await,
        Commands::Sessions {
            config,
            json,
            session,
            command,
        } => {
            sessions_cli::run_sessions_cli(sessions_cli::SessionsCommandOptions {
                config,
                json,
                session,
                command,
            })
            .await
        }
        Commands::Plugins { json, command } => {
            plugins_cli::run_plugins_cli(plugins_cli::PluginsCommandOptions { json, command }).await
        }
        Commands::Channels {
            config,
            resolve,
            json,
        } => run_channels_cli(config.as_deref(), resolve.as_deref(), json),
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
        Commands::RuntimeCapability { command } => {
            runtime_capability_cli::run_runtime_capability_cli(command)
        }
        Commands::WorkUnit { command } => work_unit_cli::run_work_unit_cli(command),
        Commands::ListContextEngines { config, json } => {
            run_list_context_engines_cli(config.as_deref(), json)
        }
        Commands::ListMemorySystems { config, json } => {
            run_list_memory_systems_cli(config.as_deref(), json)
        }
        Commands::ListMcpServers { config, json } => {
            run_list_mcp_servers_cli(config.as_deref(), json)
        }
        Commands::ShowMcpServer { config, name, json } => {
            run_show_mcp_server_cli(config.as_deref(), name.as_str(), json)
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
            participant_id,
            thread_id,
            json,
        } => run_acp_dispatch_cli(
            config.as_deref(),
            session.as_deref(),
            channel.as_deref(),
            conversation_id.as_deref(),
            account_id.as_deref(),
            participant_id.as_deref(),
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
        Commands::SessionSearch {
            config,
            session,
            query,
            limit,
            output,
            include_archived,
            json,
        } => run_session_search_cli(
            config.as_deref(),
            session.as_deref(),
            &query,
            limit,
            output.as_deref(),
            include_archived,
            json,
        ),
        Commands::SessionSearchInspect { artifact, json } => {
            run_session_search_inspect_cli(&artifact, json)
        }
        Commands::TrajectoryExport {
            config,
            session,
            output,
            json,
        } => run_trajectory_export_cli(
            config.as_deref(),
            session.as_deref(),
            output.as_deref(),
            json,
        ),
        Commands::TrajectoryInspect { artifact, json } => {
            run_trajectory_inspect_cli(&artifact, json)
        }
        Commands::RuntimeTrajectory { command } => {
            runtime_trajectory_cli::execute_runtime_trajectory_command(command)
        }
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
                    target: Some(target.as_str()),
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
                Err(format!(
                    "legacy `feishu-send` no longer supports `message_reply` execution; use `{} feishu reply` for reply targets",
                    mvp::config::active_cli_command_name()
                ))
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
        Commands::MatrixSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                MATRIX_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::MatrixServe {
            config,
            once,
            account,
        } => {
            run_channel_serve_cli(
                MATRIX_SERVE_CLI_SPEC,
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
        Commands::WecomSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                WECOM_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::WecomServe { config, account } => {
            run_channel_serve_cli(
                WECOM_SERVE_CLI_SPEC,
                ChannelServeCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    once: false,
                    bind_override: None,
                    path_override: None,
                },
            )
            .await
        }
        Commands::WhatsappServe {
            config,
            account,
            bind,
            path,
        } => {
            run_channel_serve_cli(
                WHATSAPP_SERVE_CLI_SPEC,
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
        Commands::DiscordSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                DISCORD_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::DingtalkSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                DINGTALK_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: target.as_deref(),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::SlackSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                SLACK_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::LineSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                LINE_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::LineServe {
            config,
            account,
            bind,
            path,
        } => {
            run_channel_serve_cli(
                LINE_SERVE_CLI_SPEC,
                ChannelServeCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    once: false,
                    bind_override: Some(bind.as_str()),
                    path_override: path.as_deref(),
                },
            )
            .await
        }
        Commands::WhatsappSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                WHATSAPP_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::EmailSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                EMAIL_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::WebhookSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                WEBHOOK_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: target.as_deref(),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::WebhookServe {
            config,
            account,
            bind,
            path,
        } => {
            run_channel_serve_cli(
                WEBHOOK_SERVE_CLI_SPEC,
                ChannelServeCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    once: false,
                    bind_override: Some(bind.as_str()),
                    path_override: path.as_deref(),
                },
            )
            .await
        }
        Commands::GoogleChatSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                GOOGLE_CHAT_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: target.as_deref(),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::TeamsSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                TEAMS_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: target.as_deref(),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::TlonSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                TLON_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::SignalSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                SIGNAL_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::TwitchSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                TWITCH_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::MattermostSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                MATTERMOST_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::NextcloudTalkSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                NEXTCLOUD_TALK_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::SynologyChatSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                SYNOLOGY_CHAT_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: target.as_deref(),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::IrcSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                IRC_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::ImessageSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                IMESSAGE_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: Some(target.as_str()),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::NostrSend {
            config,
            account,
            target,
            target_kind,
            text,
        } => {
            run_channel_send_cli(
                NOSTR_SEND_CLI_SPEC,
                ChannelSendCliArgs {
                    config_path: config.as_deref(),
                    account: account.as_deref(),
                    target: target.as_deref(),
                    target_kind,
                    text: &text,
                    as_card: false,
                },
            )
            .await
        }
        Commands::MultiChannelServe {
            config,
            session,
            channel_account,
        } => run_multi_channel_serve_cli(config.as_deref(), &session, channel_account).await,
        Commands::Gateway { command } => gateway::service::run_gateway_cli(command).await,
        Commands::Feishu { command } => feishu_cli::run_feishu_command(command).await,
        Commands::Completions { shell } => {
            completions_cli::run_completions_cli(completions_cli::CompletionsCommandOptions {
                shell,
            })
        }
    };
    if let Err(error) = result {
        let error_code = error_code(error.as_str());
        tracing::error!(
            target: "loongclaw.daemon",
            command_kind = %command_kind,
            error_code = %error_code,
            "CLI command failed"
        );
        #[allow(clippy::print_stderr)]
        {
            eprintln!("error: {error}");
        }
        flush_stdin();
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::{error_code, redacted_command_name};
    use loongclaw_daemon::{Commands, MultiChannelServeChannelAccount, TurnCommands};

    #[test]
    fn command_kind_uses_stable_snake_case_labels() {
        let validate_config = Commands::ValidateConfig {
            config: None,
            json: false,
            output: None,
            locale: "en".to_owned(),
            fail_on_diagnostics: false,
        };
        let multi_channel_serve = Commands::MultiChannelServe {
            config: None,
            session: "session-1".to_owned(),
            channel_account: vec![MultiChannelServeChannelAccount {
                channel_id: "telegram".to_owned(),
                account_id: "ops".to_owned(),
            }],
        };

        assert_eq!(
            validate_config.command_kind_for_logging(),
            "validate_config"
        );
        assert_eq!(
            multi_channel_serve.command_kind_for_logging(),
            "multi_channel_serve"
        );
    }

    #[test]
    fn error_code_extracts_stable_prefixes_only() {
        let stable_error = "config_file_missing: could not read `/tmp/private.toml`";
        let unstable_error = "Failed to read `/tmp/private.toml`";

        assert_eq!(error_code(stable_error), "config_file_missing");
        assert_eq!(error_code(unstable_error), "unclassified");
    }

    #[test]
    fn redacted_command_name_omits_struct_field_values() {
        let command = Commands::Turn {
            command: TurnCommands::Run {
                config: None,
                session: None,
                message: "ship feature".to_owned(),
                acp: false,
                acp_event_stream: false,
                acp_bootstrap_mcp_server: Vec::new(),
                acp_cwd: None,
            },
        };

        let redacted = redacted_command_name(&command);

        assert_eq!(redacted, "turn_run");
    }

    #[test]
    fn redacted_command_name_handles_unit_variants() {
        let redacted = redacted_command_name(&Commands::Welcome);

        assert_eq!(redacted, "welcome");
    }
}
