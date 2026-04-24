#![recursion_limit = "256"]
#![allow(clippy::print_stdout, clippy::print_stderr)] // CLI daemon binary
use loong_daemon::*;

#[cfg(debug_assertions)]
const DEBUG_TOKIO_WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOKIO_WORKER_STACK_BYTES: usize = 16 * 1024 * 1024;
const TOKIO_WORKER_STACK_ENV: &str = "LOONG_TOKIO_WORKER_STACK_BYTES";

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

#[cfg(debug_assertions)]
fn command_prefers_large_tokio_worker_stack(command: &Commands) -> bool {
    matches!(
        command,
        Commands::Channels {
            command: Some(ChannelsCommands::Serve(_)),
            ..
        } | Commands::Gateway {
            command: gateway::service::GatewayCommand::Run { .. },
        } | Commands::Feishu {
            command: feishu_cli::FeishuCommand::Serve(_),
        }
    )
}

fn default_tokio_worker_thread_stack_size(command: &Commands) -> Option<usize> {
    #[cfg(debug_assertions)]
    {
        command_prefers_large_tokio_worker_stack(command).then_some(DEBUG_TOKIO_WORKER_STACK_BYTES)
    }

    #[cfg(not(debug_assertions))]
    {
        let _ = command;
        None
    }
}

fn resolve_tokio_worker_thread_stack_size(
    raw: Option<&str>,
    command: &Commands,
) -> CliResult<Option<usize>> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => {
            let parsed_stack_size = value.parse::<usize>().map_err(|error| {
                format!(
                    "invalid {TOKIO_WORKER_STACK_ENV} value `{value}`: expected integer bytes in 1..={MAX_TOKIO_WORKER_STACK_BYTES} ({error})"
                )
            })?;

            let stack_size_is_in_range =
                (1..=MAX_TOKIO_WORKER_STACK_BYTES).contains(&parsed_stack_size);

            if !stack_size_is_in_range {
                return Err(format!(
                    "invalid {TOKIO_WORKER_STACK_ENV} value `{value}`: expected integer bytes in 1..={MAX_TOKIO_WORKER_STACK_BYTES}"
                ));
            }

            Ok(Some(parsed_stack_size))
        }
        None => Ok(default_tokio_worker_thread_stack_size(command)),
    }
}

fn tokio_worker_thread_stack_size(command: &Commands) -> CliResult<Option<usize>> {
    let raw = std::env::var(TOKIO_WORKER_STACK_ENV).ok();
    resolve_tokio_worker_thread_stack_size(raw.as_deref(), command)
}

fn build_daemon_runtime(command: &Commands) -> CliResult<tokio::runtime::Runtime> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();

    if let Some(stack_size) = tokio_worker_thread_stack_size(command)? {
        tracing::debug!(
            target: "loong.daemon",
            thread_stack_size = stack_size,
            override_env = TOKIO_WORKER_STACK_ENV,
            command = %redacted_command_name(command),
            "using configured Tokio worker thread stack size"
        );
        builder.thread_stack_size(stack_size);
    }

    builder
        .build()
        .map_err(|error| format!("failed to build Tokio runtime: {error}"))
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

fn main() {
    let _stdin_guard = StdinGuard;
    init_tracing();
    mvp::config::set_active_cli_command_name(mvp::config::detect_invoked_cli_command_name());
    loong_daemon::make_env_compatible();
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
        target: "loong.daemon",
        command_source,
        command = %redacted_command,
        "resolved CLI command"
    );
    let result =
        build_daemon_runtime(&command).and_then(|runtime| runtime.block_on(run_command(command)));
    if let Err(error) = result {
        let error_code = error_code(error.as_str());
        tracing::error!(
            target: "loong.daemon",
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

async fn run_command(command: Commands) -> CliResult<()> {
    match command {
        Commands::Welcome => run_welcome_cli(),
        Commands::Demo => run_demo().await,
        Commands::Update => run_update_cli().await,
        Commands::RunTask { objective, payload } => run_task_cli(&objective, &payload).await,
        Commands::Turn { command } => match command {
            loong_daemon::TurnCommands::Run {
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
        Commands::Web { command } => run_web_command(command).await,
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
        Commands::Debug {
            config,
            json,
            session,
            command,
        } => {
            debug_cli::run_debug_cli(debug_cli::DebugCommandOptions {
                config,
                json,
                session,
                command,
            })
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
            command,
        } => run_grouped_channels_cli(config, resolve, json, command).await,
        Commands::Runtime { command } => run_runtime_cli(command).await,
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
        Commands::Gateway { command } => gateway::service::run_gateway_cli(command).await,
        Commands::Feishu { command } => feishu_cli::run_feishu_command(command).await,
        Commands::Completions { shell } => {
            completions_cli::run_completions_cli(completions_cli::CompletionsCommandOptions {
                shell,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEBUG_TOKIO_WORKER_STACK_BYTES, MAX_TOKIO_WORKER_STACK_BYTES, TOKIO_WORKER_STACK_ENV,
        error_code, redacted_command_name, resolve_tokio_worker_thread_stack_size,
    };
    use loong_daemon::{Commands, TurnCommands};

    #[test]
    fn command_kind_uses_stable_snake_case_labels() {
        let validate_config = Commands::ValidateConfig {
            config: None,
            json: false,
            output: None,
            locale: "en".to_owned(),
            fail_on_diagnostics: false,
        };
        assert_eq!(
            validate_config.command_kind_for_logging(),
            "validate_config"
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

    #[test]
    fn resolve_tokio_worker_thread_stack_size_uses_debug_default_for_grouped_channel_serve() {
        let stack_size = resolve_tokio_worker_thread_stack_size(
            None,
            &Commands::Channels {
                config: None,
                resolve: None,
                json: false,
                command: Some(loong_daemon::ChannelsCommands::Serve(
                    loong_daemon::channels_cli::ChannelsServeArgs {
                        config: None,
                        account: None,
                        channel: Some("telegram".to_owned()),
                        channel_name: None,
                        once: false,
                        stop: false,
                        stop_duplicates: false,
                        bind: None,
                        path: None,
                    },
                )),
            },
        )
        .expect("unset stack size should resolve");

        assert_eq!(stack_size, Some(DEBUG_TOKIO_WORKER_STACK_BYTES));
    }

    #[test]
    fn resolve_tokio_worker_thread_stack_size_keeps_default_for_non_serve_commands() {
        let stack_size = resolve_tokio_worker_thread_stack_size(None, &Commands::Welcome)
            .expect("non-serve command should resolve");

        assert_eq!(stack_size, None);
    }

    #[test]
    fn resolve_tokio_worker_thread_stack_size_accepts_explicit_override() {
        let stack_size =
            resolve_tokio_worker_thread_stack_size(Some("16777216"), &Commands::Welcome)
                .expect("explicit stack size should parse");

        assert_eq!(stack_size, Some(16 * 1024 * 1024));
    }

    #[test]
    fn resolve_tokio_worker_thread_stack_size_rejects_invalid_override() {
        let error = resolve_tokio_worker_thread_stack_size(Some("oops"), &Commands::Welcome)
            .expect_err("invalid stack size should fail");

        assert!(error.contains(TOKIO_WORKER_STACK_ENV));
    }

    #[test]
    fn resolve_tokio_worker_thread_stack_size_rejects_zero_override() {
        let error = resolve_tokio_worker_thread_stack_size(Some("0"), &Commands::Welcome)
            .expect_err("zero stack size should fail");

        assert!(error.contains(TOKIO_WORKER_STACK_ENV));
        assert!(error.contains("1..="));
    }

    #[test]
    fn resolve_tokio_worker_thread_stack_size_rejects_huge_override() {
        let too_large_stack_size = MAX_TOKIO_WORKER_STACK_BYTES + 1;
        let raw_stack_size = too_large_stack_size.to_string();
        let error = resolve_tokio_worker_thread_stack_size(
            Some(raw_stack_size.as_str()),
            &Commands::Welcome,
        )
        .expect_err("too-large stack size should fail");

        assert!(error.contains(TOKIO_WORKER_STACK_ENV));
        assert!(error.contains(MAX_TOKIO_WORKER_STACK_BYTES.to_string().as_str()));
    }
}
