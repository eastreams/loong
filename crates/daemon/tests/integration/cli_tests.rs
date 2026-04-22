use super::*;

fn try_parse_cli_slice(args: &[&str]) -> Result<Cli, clap::Error> {
    let owned_args = args
        .iter()
        .map(|arg| OsString::from(*arg))
        .collect::<Vec<OsString>>();
    with_cli_stack("integration-cli-parse-slice", move || {
        Cli::try_parse_from(owned_args)
    })
}

fn cli_subcommand_names(path: &[&str], hidden: bool) -> Vec<String> {
    let owned_path = path
        .iter()
        .map(|segment| (*segment).to_owned())
        .collect::<Vec<_>>();
    with_cli_stack("integration-cli-subcommands", move || {
        let mut command = Cli::command();
        let mut current = &mut command;
        for segment in owned_path {
            current = current
                .find_subcommand_mut(segment.as_str())
                .unwrap_or_else(|| panic!("missing CLI subcommand `{segment}`"));
        }
        current
            .get_subcommands()
            .filter(|subcommand| subcommand.is_hide_set() == hidden)
            .map(|subcommand| subcommand.get_name().to_owned())
            .collect()
    })
}

fn cli_subcommand_is_hidden(path: &[&str], name: &str) -> bool {
    let owned_path = path
        .iter()
        .map(|segment| (*segment).to_owned())
        .collect::<Vec<_>>();
    let owned_name = name.to_owned();
    with_cli_stack("integration-cli-hidden-flag", move || {
        let mut command = Cli::command();
        let mut current = &mut command;
        for segment in owned_path {
            current = current
                .find_subcommand_mut(segment.as_str())
                .unwrap_or_else(|| panic!("missing CLI subcommand `{segment}`"));
        }
        current
            .find_subcommand_mut(owned_name.as_str())
            .unwrap_or_else(|| panic!("missing CLI subcommand `{owned_name}`"))
            .is_hide_set()
    })
}

fn render_bash_completions() -> String {
    let mut rendered = Vec::new();
    loong_daemon::completions_cli::generate_completions(clap_complete::Shell::Bash, &mut rendered)
        .expect("generate bash completions");
    String::from_utf8(rendered).expect("bash completions should be utf8")
}

fn parse_first_candidate(candidates: &[&[&str]]) -> Cli {
    let mut errors = Vec::new();
    for candidate in candidates {
        match try_parse_cli_slice(candidate) {
            Ok(cli) => return cli,
            Err(error) => errors.push(format!("{} => {}", candidate.join(" "), error)),
        }
    }

    panic!(
        "expected one canonical candidate to parse, but all failed:\n{}",
        errors.join("\n")
    );
}

fn channel_catalog_command_family(
    raw: &str,
) -> mvp::channel::ChannelCatalogCommandFamilyDescriptor {
    mvp::channel::resolve_channel_catalog_command_family_descriptor(raw)
        .expect("channel catalog command family")
}

fn channel_send_command(raw: &str) -> &'static str {
    channel_catalog_command_family(raw).send.command
}

fn channel_default_send_target_kind(raw: &str) -> mvp::channel::ChannelOutboundTargetKind {
    channel_catalog_command_family(raw).default_send_target_kind
}

#[test]
fn root_help_uses_onboarding_language() {
    let help = render_cli_help([]);

    assert!(help.contains("onboarding"));
    assert!(
        !help
            .lines()
            .any(|line| line.trim_start().starts_with("setup ")),
        "root help should not advertise a standalone `setup` subcommand: {help}"
    );
}

#[test]
fn root_help_prefers_grouped_namespaces_and_hides_flat_legacy_aliases() {
    let help = render_cli_help([]);
    let visible_root_subcommands = cli_subcommand_names(&[], false);

    for command in [
        "onboard",
        "ask",
        "chat",
        "doctor",
        "status",
        "update",
        "channels",
        "sessions",
        "skills",
        "gateway",
        "plugins",
        "feishu",
        "completions",
    ] {
        assert!(
            visible_root_subcommands
                .iter()
                .any(|value| value == command),
            "expected `{command}` to remain visible at the root: {visible_root_subcommands:?}"
        );
        assert!(
            help.lines()
                .any(|line| line.trim_start().starts_with(&format!("{command} "))),
            "root help should advertise `{command}` as a visible root namespace: {help}"
        );
    }

    assert!(
        visible_root_subcommands
            .iter()
            .any(|value| value == "runtime" || value == "ops"),
        "root help should expose one grouped runtime/operator namespace: {visible_root_subcommands:?}"
    );

    for legacy in [
        "telegram-send",
        "telegram-serve",
        "matrix-send",
        "matrix-serve",
        "runtime-restore",
        "runtime-trajectory",
        "session-search",
        "list-mcp-servers",
        "acp-status",
        "control-plane-serve",
    ] {
        assert!(
            !help
                .lines()
                .any(|line| line.trim_start().starts_with(&format!("{legacy} "))),
            "root help should hide flat compatibility alias `{legacy}` after the refactor: {help}"
        );
    }
}

#[test]
fn root_command_tree_removes_flat_legacy_aliases_entirely() {
    for legacy in [
        "telegram-send",
        "telegram-serve",
        "runtime-restore",
        "runtime-trajectory",
        "session-search",
        "list-mcp-servers",
        "acp-status",
        "control-plane-serve",
    ] {
        assert!(
            !cli_subcommand_names(&[], true)
                .iter()
                .any(|value| value == legacy)
                && !cli_subcommand_names(&[], false)
                    .iter()
                    .any(|value| value == legacy),
            "legacy alias `{legacy}` should be removed from the root command tree"
        );
    }
}

#[test]
fn channels_help_mentions_grouped_send_and_serve_surfaces() {
    let help = render_cli_help(["channels"]);
    let visible_channels_subcommands = cli_subcommand_names(&["channels"], false);

    for subcommand in ["send", "serve"] {
        assert!(
            visible_channels_subcommands
                .iter()
                .any(|value| value == subcommand),
            "channels should expose `{subcommand}` after grouping flat channel verbs: {visible_channels_subcommands:?}"
        );
        assert!(
            help.lines()
                .any(|line| line.trim_start().starts_with(&format!("{subcommand} "))),
            "channels help should advertise `{subcommand}` as a grouped subcommand: {help}"
        );
    }

    for legacy in [
        "telegram-send",
        "telegram-serve",
        "matrix-send",
        "matrix-serve",
    ] {
        assert!(
            !help.contains(legacy),
            "channels help should not surface flat legacy alias `{legacy}`: {help}"
        );
    }
}

#[test]
fn grouped_channels_send_accepts_a_canonical_shape() {
    let _cli = parse_first_candidate(&[
        &[
            "loong",
            "channels",
            "send",
            "telegram",
            "--target",
            "chat-42",
            "--text",
            "hello from grouped send",
        ],
        &[
            "loong",
            "channels",
            "send",
            "--channel",
            "telegram",
            "--target",
            "chat-42",
            "--text",
            "hello from grouped send",
        ],
    ]);
}

#[test]
fn grouped_channels_serve_accepts_a_canonical_shape() {
    let _cli = parse_first_candidate(&[
        &["loong", "channels", "serve", "telegram", "--stop"],
        &[
            "loong",
            "channels",
            "serve",
            "--channel",
            "telegram",
            "--stop",
        ],
    ]);
}

#[test]
fn removed_flat_legacy_aliases_now_fail_to_parse() {
    for candidate in [
        vec![
            "loong",
            "telegram-send",
            "--target",
            "chat-42",
            "--text",
            "compatibility send",
        ],
        vec!["loong", "telegram-serve", "--stop"],
        vec![
            "loong",
            "runtime-restore",
            "--snapshot",
            "/tmp/runtime.json",
        ],
        vec!["loong", "session-search", "--query", "hello world"],
        vec!["loong", "list-mcp-servers", "--json"],
    ] {
        let error = try_parse_cli_slice(candidate.as_slice())
            .expect_err("removed flat alias should now fail to parse");
        assert!(
            error.to_string().contains("unrecognized subcommand"),
            "unexpected parser error for removed alias {:?}: {}",
            candidate,
            error
        );
    }
}

#[test]
fn bash_completions_surface_grouped_namespaces_without_flat_legacy_aliases() {
    let completions = render_bash_completions();

    for visible in [
        "channels",
        "send",
        "serve",
        "sessions",
        "plugins",
        "completions",
    ] {
        assert!(
            completions.contains(visible),
            "bash completions should include grouped CLI surface `{visible}`: {completions}"
        );
    }

    assert!(
        completions.contains("runtime") || completions.contains("ops"),
        "bash completions should include the grouped runtime/operator namespace: {completions}"
    );

    for legacy in [
        "telegram-send",
        "telegram-serve",
        "matrix-send",
        "matrix-serve",
        "runtime-restore",
        "session-search",
        "list-mcp-servers",
        "acp-status",
        "control-plane-serve",
    ] {
        assert!(
            !completions.contains(legacy),
            "bash completions should hide legacy flat alias `{legacy}` once the canonical namespaces land"
        );
    }
}

#[test]
fn welcome_subcommand_help_advertises_first_run_shortcuts() {
    let help = render_cli_help(["welcome"]);

    assert!(
        help.contains("quick commands"),
        "welcome help should frame the configured path as a quick-command entrypoint: {help}"
    );
    assert!(
        help.contains("loong ask --config <path>") || help.contains("loong ask --config <path>"),
        "welcome help should mention ask with an explicit config placeholder: {help}"
    );
    assert!(
        help.contains("loong chat --config <path>") || help.contains("loong chat --config <path>"),
        "welcome help should mention chat with an explicit config placeholder: {help}"
    );
    assert!(
        help.contains("loong doctor --config <path>")
            || help.contains("loong doctor --config <path>"),
        "welcome help should mention doctor with an explicit config placeholder: {help}"
    );
    assert!(
        help.contains("LOONG_CONFIG_PATH") || help.contains("LOONG_CONFIG_PATH"),
        "welcome help should explain how config-path environment overrides interact with the quick commands: {help}"
    );
}

#[test]
fn update_subcommand_help_mentions_latest_stable_release_only() {
    let help = render_cli_help(["update"]);

    assert!(
        help.contains("latest stable GitHub release"),
        "update help should describe the stable release channel: {help}"
    );
    assert!(
        help.contains("prereleases are excluded"),
        "update help should explicitly say prereleases are not used: {help}"
    );
}

#[test]
fn update_subcommand_parses_without_flags() {
    let cli = try_parse_cli(["loong", "update"]).expect("`loong update` should parse");

    match cli.command {
        Some(Commands::Update) => {}
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn setup_subcommand_is_removed() {
    let error = try_parse_cli(["loong", "setup"])
        .expect_err("`setup` should no longer parse as a valid subcommand");
    assert!(
        error
            .to_string()
            .contains("unrecognized subcommand 'setup'")
    );
}

#[test]
fn migrate_cli_parses_discover_mode_with_defaults() {
    let cli = try_parse_cli([
        "loong",
        "migrate",
        "--mode",
        "discover",
        "--input",
        "/tmp/legacy-root",
    ])
    .expect("`migrate --mode discover` should parse");

    match cli.command {
        Some(Commands::Migrate {
            input,
            output,
            mode,
            json,
            force,
            ..
        }) => {
            assert_eq!(mode, loong_daemon::migrate_cli::MigrateMode::Discover);
            assert_eq!(input.as_deref(), Some("/tmp/legacy-root"));
            assert_eq!(output, None);
            assert!(!json);
            assert!(!force);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn migrate_cli_requires_mode_flag() {
    let error = try_parse_cli(["loong", "migrate", "--input", "/tmp/legacy-root"])
        .expect_err("`migrate` without --mode should fail");
    let rendered = error.to_string();

    assert!(
        rendered.contains("--mode <MODE>"),
        "parse failure should mention the required mode flag: {rendered}"
    );
}

#[test]
fn migrate_cli_parses_apply_selected_flags() {
    let cli = try_parse_cli([
        "loong",
        "migrate",
        "--mode",
        "apply_selected",
        "--input",
        "/tmp/discovery-root",
        "--output",
        "/tmp/loong.toml",
        "--source-id",
        "openclaw",
        "--primary-source-id",
        "openclaw",
        "--safe-profile-merge",
        "--apply-external-skills-plan",
        "--json",
        "--force",
    ])
    .expect("`migrate --mode apply_selected` should parse");

    match cli.command {
        Some(Commands::Migrate {
            input,
            output,
            mode,
            json,
            source_id,
            safe_profile_merge,
            primary_source_id,
            apply_external_skills_plan,
            force,
            ..
        }) => {
            assert_eq!(mode, loong_daemon::migrate_cli::MigrateMode::ApplySelected);
            assert_eq!(input.as_deref(), Some("/tmp/discovery-root"));
            assert_eq!(output.as_deref(), Some("/tmp/loong.toml"));
            assert_eq!(source_id.as_deref(), Some("openclaw"));
            assert_eq!(primary_source_id.as_deref(), Some("openclaw"));
            assert!(safe_profile_merge);
            assert!(apply_external_skills_plan);
            assert!(json);
            assert!(force);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn safe_lane_summary_cli_rejects_zero_limit() {
    let error = run_safe_lane_summary_cli(None, Some("session-a"), 0, false)
        .expect_err("zero limit must be rejected");
    assert!(error.contains(">= 1"));
}

#[test]
fn runtime_trajectory_export_help_mentions_export_and_lineage() {
    let help = render_cli_help(["runtime", "trajectory", "runtime", "export"]);

    assert!(
        help.contains("trajectory"),
        "runtime-trajectory export help should mention trajectory export: {help}"
    );
    assert!(
        help.contains("--session <SESSION>"),
        "runtime-trajectory export help should require a session id: {help}"
    );
    assert!(
        help.contains("--turn-limit <TURN_LIMIT>")
            && help.contains("--event-page-limit <EVENT_PAGE_LIMIT>"),
        "runtime-trajectory export help should surface the bounded export controls: {help}"
    );
}

#[test]
fn runtime_trajectory_cli_parses_export_flags() {
    let cli = try_parse_cli([
        "loong",
        "runtime",
        "trajectory",
        "runtime",
        "export",
        "--config",
        "/tmp/loong.toml",
        "--session",
        "root-session",
        "--output",
        "/tmp/runtime-trajectory.json",
        "--json",
    ])
    .expect("`runtime trajectory runtime export` should parse");

    match cli.command {
        Some(Commands::Runtime {
            command:
                loong_daemon::runtime_cli::RuntimeCommands::Trajectory {
                    command:
                        loong_daemon::runtime_cli::RuntimeTrajectoryCommands::Runtime {
                            command:
                                loong_daemon::runtime_trajectory_cli::RuntimeTrajectoryCommands::Export(
                                    options,
                                ),
                        },
                },
        }) => {
            assert_eq!(options.config.as_deref(), Some("/tmp/loong.toml"));
            assert_eq!(options.session.as_deref(), Some("root-session"));
            assert_eq!(options.turn_limit, None);
            assert_eq!(
                options.event_page_limit,
                loong_daemon::runtime_trajectory_cli::ARTIFACT_MODE_EVENT_PAGE_LIMIT_DEFAULT
            );
            assert_eq!(
                options.output.as_deref(),
                Some("/tmp/runtime-trajectory.json")
            );
            assert!(options.json);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_trajectory_cli_parses_show_flags() {
    let cli = try_parse_cli([
        "loong",
        "runtime",
        "trajectory",
        "runtime",
        "show",
        "--artifact",
        "/tmp/runtime-trajectory.json",
        "--json",
    ])
    .expect("`runtime trajectory runtime show` should parse");

    match cli.command {
        Some(Commands::Runtime {
            command:
                loong_daemon::runtime_cli::RuntimeCommands::Trajectory {
                    command:
                        loong_daemon::runtime_cli::RuntimeTrajectoryCommands::Runtime {
                            command:
                                loong_daemon::runtime_trajectory_cli::RuntimeTrajectoryCommands::Show(
                                    options,
                                ),
                        },
                },
        }) => {
            assert_eq!(options.artifact, "/tmp/runtime-trajectory.json");
            assert!(options.json);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn onboard_cli_accepts_generic_api_key_flag() {
    let cli = try_parse_cli([
        "loong",
        "onboard",
        "--non-interactive",
        "--accept-risk",
        "--api-key",
        "OPENAI_API_KEY",
    ])
    .expect("`--api-key` should parse");

    match cli.command {
        Some(Commands::Onboard { api_key_env, .. }) => {
            assert_eq!(api_key_env.as_deref(), Some("OPENAI_API_KEY"));
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn onboard_cli_keeps_legacy_api_key_env_alias() {
    let cli = try_parse_cli([
        "loong",
        "onboard",
        "--non-interactive",
        "--accept-risk",
        "--api-key-env",
        "OPENAI_API_KEY",
    ])
    .expect("legacy `--api-key-env` alias should still parse");

    match cli.command {
        Some(Commands::Onboard { api_key_env, .. }) => {
            assert_eq!(api_key_env.as_deref(), Some("OPENAI_API_KEY"));
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn onboard_cli_accepts_web_search_provider_flag() {
    let cli = try_parse_cli([
        "loong",
        "onboard",
        "--non-interactive",
        "--accept-risk",
        "--web-search-provider",
        "tavily",
    ])
    .expect("`--web-search-provider` should parse");

    match cli.command {
        Some(Commands::Onboard {
            web_search_provider,
            ..
        }) => {
            assert_eq!(web_search_provider.as_deref(), Some("tavily"));
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn onboard_cli_accepts_web_search_api_key_flag() {
    let cli = try_parse_cli([
        "loong",
        "onboard",
        "--non-interactive",
        "--accept-risk",
        "--web-search-api-key",
        "TAVILY_API_KEY",
    ])
    .expect("`--web-search-api-key` should parse");

    match cli.command {
        Some(Commands::Onboard {
            web_search_api_key_env,
            ..
        }) => {
            assert_eq!(web_search_api_key_env.as_deref(), Some("TAVILY_API_KEY"));
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn onboard_cli_accepts_personality_flag() {
    let cli = try_parse_cli([
        "loong",
        "onboard",
        "--non-interactive",
        "--accept-risk",
        "--personality",
        "friendly_collab",
    ])
    .expect("`--personality` should parse");

    match cli.command {
        Some(Commands::Onboard { personality, .. }) => {
            assert_eq!(personality.as_deref(), Some("friendly_collab"));
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn onboard_cli_accepts_memory_profile_flag() {
    let cli = try_parse_cli([
        "loong",
        "onboard",
        "--non-interactive",
        "--accept-risk",
        "--memory-profile",
        "profile_plus_window",
    ])
    .expect("`--memory-profile` should parse");

    match cli.command {
        Some(Commands::Onboard { memory_profile, .. }) => {
            assert_eq!(memory_profile.as_deref(), Some("profile_plus_window"));
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn benchmark_memory_context_cli_parses_custom_knobs() {
    let cli = try_parse_cli([
        "loong",
        "benchmark-memory-context",
        "--output",
        "target/benchmarks/test-memory-context-report.json",
        "--temp-root",
        "target/benchmarks/tmp-local",
        "--history-turns",
        "96",
        "--sliding-window",
        "12",
        "--summary-max-chars",
        "640",
        "--words-per-turn",
        "18",
        "--rebuild-iterations",
        "3",
        "--hot-iterations",
        "7",
        "--warmup-iterations",
        "2",
        "--suite-repetitions",
        "3",
        "--enforce-gate",
        "--min-steady-state-speedup-ratio",
        "1.35",
    ])
    .expect("benchmark-memory-context CLI should parse");

    match cli.command {
        Some(Commands::BenchmarkMemoryContext {
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
        }) => {
            assert_eq!(
                output,
                "target/benchmarks/test-memory-context-report.json".to_owned()
            );
            assert_eq!(temp_root, Some("target/benchmarks/tmp-local".to_owned()));
            assert_eq!(history_turns, 96);
            assert_eq!(sliding_window, 12);
            assert_eq!(summary_max_chars, 640);
            assert_eq!(words_per_turn, 18);
            assert_eq!(rebuild_iterations, 3);
            assert_eq!(hot_iterations, 7);
            assert_eq!(warmup_iterations, 2);
            assert_eq!(suite_repetitions, 3);
            assert!(enforce_gate);
            assert!((min_steady_state_speedup_ratio - 1.35).abs() < f64::EPSILON);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn benchmark_memory_context_cli_uses_stable_default_sample_sizes() {
    let cli = try_parse_cli(["loong", "benchmark-memory-context"])
        .expect("benchmark-memory-context CLI should parse with defaults");

    match cli.command {
        Some(Commands::BenchmarkMemoryContext {
            rebuild_iterations,
            hot_iterations,
            warmup_iterations,
            suite_repetitions,
            ..
        }) => {
            assert_eq!(rebuild_iterations, 12);
            assert_eq!(hot_iterations, 32);
            assert_eq!(warmup_iterations, 4);
            assert_eq!(suite_repetitions, 1);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn memory_systems_cli_parses() {
    let cli = try_parse_cli(["loong", "runtime", "memory", "systems"])
        .expect("`runtime memory systems` should parse");

    match cli.command {
        Some(Commands::Runtime {
            command:
                loong_daemon::runtime_cli::RuntimeCommands::Memory {
                    command:
                        loong_daemon::runtime_cli::RuntimeMemoryCommands::Systems(
                            loong_daemon::runtime_cli::RuntimeReadArgs { config, json },
                        ),
                },
        }) => {
            assert!(config.is_none());
            assert!(!json);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_snapshot_cli_parses() {
    let cli = try_parse_cli([
        "loong",
        "runtime",
        "snapshot",
        "--config",
        "/tmp/loong.toml",
        "--json",
        "--output",
        "/tmp/runtime-snapshot.json",
        "--label",
        "baseline",
        "--experiment-id",
        "exp-42",
        "--parent-snapshot-id",
        "snapshot-parent",
    ])
    .expect("`runtime snapshot` should parse");

    match cli.command {
        Some(Commands::Runtime {
            command:
                loong_daemon::runtime_cli::RuntimeCommands::Snapshot(
                    loong_daemon::runtime_cli::RuntimeSnapshotArgs {
                        config,
                        json,
                        output,
                        label,
                        experiment_id,
                        parent_snapshot_id,
                    },
                ),
        }) => {
            assert_eq!(config.as_deref(), Some("/tmp/loong.toml"));
            assert!(json);
            assert_eq!(output.as_deref(), Some("/tmp/runtime-snapshot.json"));
            assert_eq!(label.as_deref(), Some("baseline"));
            assert_eq!(experiment_id.as_deref(), Some("exp-42"));
            assert_eq!(parent_snapshot_id.as_deref(), Some("snapshot-parent"));
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn debug_bundle_cli_parses() {
    let cli = try_parse_cli([
        "loong",
        "debug",
        "--config",
        "/tmp/loong.toml",
        "--json",
        "--session",
        "root-session",
        "bundle",
        "--session-id",
        "target-session",
        "--output",
        "/tmp/debug-bundle.json",
        "--audit-limit",
        "25",
        "--session-event-limit",
        "40",
        "--history-limit",
        "50",
        "--acp-event-limit",
        "120",
        "--include-history",
    ])
    .expect("`debug bundle` should parse");

    match cli.command {
        Some(Commands::Debug {
            config,
            json,
            session,
            command:
                crate::debug_cli::DebugCommands::Bundle {
                    session_id,
                    output,
                    audit_limit,
                    session_event_limit,
                    history_limit,
                    acp_event_limit,
                    include_history,
                },
        }) => {
            assert_eq!(config.as_deref(), Some("/tmp/loong.toml"));
            assert!(json);
            assert_eq!(session, "root-session");
            assert_eq!(session_id.as_deref(), Some("target-session"));
            assert_eq!(output.as_deref(), Some("/tmp/debug-bundle.json"));
            assert_eq!(audit_limit, 25);
            assert_eq!(session_event_limit, 40);
            assert_eq!(history_limit, 50);
            assert_eq!(acp_event_limit, 120);
            assert!(include_history);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn debug_show_cli_parses() {
    let cli = try_parse_cli([
        "loong",
        "debug",
        "--config",
        "/tmp/loong.toml",
        "--session",
        "root-session",
        "show",
        "--artifact",
        "/tmp/debug-bundle.json",
    ])
    .expect("`debug show` should parse");

    match cli.command {
        Some(Commands::Debug {
            config,
            json,
            session,
            command: crate::debug_cli::DebugCommands::Show { artifact },
        }) => {
            assert_eq!(config.as_deref(), Some("/tmp/loong.toml"));
            assert!(!json);
            assert_eq!(session, "root-session");
            assert_eq!(artifact, "/tmp/debug-bundle.json");
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_restore_cli_parses() {
    let cli = try_parse_cli([
        "loong",
        "runtime",
        "restore",
        "--config",
        "/tmp/loong.toml",
        "--snapshot",
        "/tmp/runtime-snapshot.json",
        "--json",
        "--apply",
    ])
    .expect("`runtime restore` should parse");

    match cli.command {
        Some(Commands::Runtime {
            command:
                loong_daemon::runtime_cli::RuntimeCommands::Restore(
                    loong_daemon::runtime_cli::RuntimeRestoreArgs {
                        config,
                        snapshot,
                        json,
                        apply,
                    },
                ),
        }) => {
            assert_eq!(config.as_deref(), Some("/tmp/loong.toml"));
            assert_eq!(snapshot, "/tmp/runtime-snapshot.json");
            assert!(json);
            assert!(apply);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_experiment_cli_parses_restore() {
    let cli = try_parse_cli([
        "loong",
        "runtime",
        "experiment",
        "restore",
        "--run",
        "/tmp/runtime-experiment.json",
        "--stage",
        "result",
        "--config",
        "/tmp/loong.toml",
        "--json",
        "--apply",
    ])
    .expect("`runtime experiment restore` should parse");

    match cli.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Experiment { command },
        }) => match command {
            loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Restore(options) => {
                assert_eq!(options.run, "/tmp/runtime-experiment.json");
                assert_eq!(
                    options.stage,
                    loong_daemon::runtime_experiment_cli::RuntimeExperimentRestoreStage::Result
                );
                assert_eq!(options.config.as_deref(), Some("/tmp/loong.toml"));
                assert!(options.json);
                assert!(options.apply);
            }
            other @ (loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Start(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Finish(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Show(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Compare(
                _,
            )) => {
                panic!("unexpected runtime-experiment subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_experiment_cli_parses_start_finish_and_show() {
    let start = try_parse_cli([
        "loong",
        "runtime",
        "experiment",
        "start",
        "--snapshot",
        "/tmp/runtime-snapshot.json",
        "--output",
        "/tmp/runtime-experiment.json",
        "--mutation-summary",
        "enable browser preview skill",
        "--experiment-id",
        "exp-42",
        "--label",
        "browser-preview-a",
        "--tag",
        "browser",
        "--tag",
        "preview",
        "--json",
    ])
    .expect("`runtime experiment start` should parse");

    match start.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Experiment { command },
        }) => match command {
            loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Start(options) => {
                assert_eq!(options.snapshot, "/tmp/runtime-snapshot.json");
                assert_eq!(options.output, "/tmp/runtime-experiment.json");
                assert_eq!(options.mutation_summary, "enable browser preview skill");
                assert_eq!(options.experiment_id.as_deref(), Some("exp-42"));
                assert_eq!(options.label.as_deref(), Some("browser-preview-a"));
                assert_eq!(
                    options.tag,
                    vec!["browser".to_owned(), "preview".to_owned()]
                );
                assert!(options.json);
            }
            other @ (loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Finish(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Show(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Compare(
                _,
            )
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Restore(
                _,
            )) => {
                panic!("unexpected runtime-experiment subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let finish = try_parse_cli([
        "loong",
        "runtime",
        "experiment",
        "finish",
        "--run",
        "/tmp/runtime-experiment.json",
        "--result-snapshot",
        "/tmp/runtime-snapshot-result.json",
        "--evaluation-summary",
        "task success improved",
        "--metric",
        "task_success=1",
        "--metric",
        "token_delta=0",
        "--decision",
        "promoted",
        "--warning",
        "manual verification only",
        "--status",
        "completed",
        "--json",
    ])
    .expect("`runtime experiment finish` should parse");

    match finish.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Experiment { command },
        }) => match command {
            loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Finish(options) => {
                assert_eq!(options.run, "/tmp/runtime-experiment.json");
                assert_eq!(options.result_snapshot, "/tmp/runtime-snapshot-result.json");
                assert_eq!(options.evaluation_summary, "task success improved");
                assert_eq!(
                    options.metric,
                    vec!["task_success=1".to_owned(), "token_delta=0".to_owned()]
                );
                assert_eq!(options.warning, vec!["manual verification only".to_owned()]);
                assert_eq!(
                    options.decision,
                    loong_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted
                );
                assert_eq!(
                    options.status,
                    loong_daemon::runtime_experiment_cli::RuntimeExperimentFinishStatus::Completed
                );
                assert!(options.json);
            }
            other @ (loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Start(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Show(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Compare(
                _,
            )
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Restore(
                _,
            )) => {
                panic!("unexpected runtime-experiment subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let show = try_parse_cli([
        "loong",
        "runtime",
        "experiment",
        "show",
        "--run",
        "/tmp/runtime-experiment.json",
        "--json",
    ])
    .expect("`runtime experiment show` should parse");

    match show.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Experiment { command },
        }) => match command {
            loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Show(options) => {
                assert_eq!(options.run, "/tmp/runtime-experiment.json");
                assert!(options.json);
            }
            other @ (loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Start(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Finish(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Compare(
                _,
            )
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Restore(
                _,
            )) => {
                panic!("unexpected runtime-experiment subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_experiment_cli_parses_compare() {
    let compare = try_parse_cli([
        "loong",
        "runtime",
        "experiment",
        "compare",
        "--run",
        "/tmp/runtime-experiment.json",
        "--baseline-snapshot",
        "/tmp/runtime-snapshot.json",
        "--result-snapshot",
        "/tmp/runtime-snapshot-result.json",
        "--json",
    ])
    .expect("`runtime experiment compare` should parse");

    match compare.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Experiment { command },
        }) => match command {
            loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Compare(options) => {
                assert_eq!(options.run, "/tmp/runtime-experiment.json");
                assert_eq!(
                    options.baseline_snapshot.as_deref(),
                    Some("/tmp/runtime-snapshot.json")
                );
                assert_eq!(
                    options.result_snapshot.as_deref(),
                    Some("/tmp/runtime-snapshot-result.json")
                );
                assert!(!options.recorded_snapshots);
                assert!(options.json);
            }
            other @ (loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Start(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Finish(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Show(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Restore(
                _,
            )) => {
                panic!("unexpected runtime-experiment subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_experiment_cli_parses_compare_with_recorded_snapshots() {
    let compare = try_parse_cli([
        "loong",
        "runtime",
        "experiment",
        "compare",
        "--run",
        "/tmp/runtime-experiment.json",
        "--recorded-snapshots",
        "--json",
    ])
    .expect("`runtime experiment compare --recorded-snapshots` should parse");

    match compare.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Experiment { command },
        }) => match command {
            loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Compare(options) => {
                assert_eq!(options.run, "/tmp/runtime-experiment.json");
                assert_eq!(options.baseline_snapshot, None);
                assert_eq!(options.result_snapshot, None);
                assert!(options.recorded_snapshots);
                assert!(options.json);
            }
            other @ (loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Start(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Finish(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Show(_)
            | loong_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Restore(
                _,
            )) => {
                panic!("unexpected runtime-experiment subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_experiment_cli_rejects_compare_recorded_snapshots_with_manual_paths() {
    let error = try_parse_cli([
        "loong",
        "runtime",
        "experiment",
        "compare",
        "--run",
        "/tmp/runtime-experiment.json",
        "--recorded-snapshots",
        "--baseline-snapshot",
        "/tmp/runtime-snapshot.json",
        "--result-snapshot",
        "/tmp/runtime-snapshot-result.json",
    ])
    .expect_err("manual snapshot paths should conflict with --recorded-snapshots");

    assert!(error.to_string().contains("--recorded-snapshots"));
}

#[test]
fn runtime_capability_cli_parses_propose_review_show_index_plan_apply_activate_and_rollback() {
    let propose = try_parse_cli([
        "loong",
        "runtime",
        "capability",
        "propose",
        "--run",
        "/tmp/runtime-experiment.json",
        "--output",
        "/tmp/runtime-capability.json",
        "--target",
        "managed-skill",
        "--target-summary",
        "Codify browser preview onboarding as a reusable managed skill",
        "--bounded-scope",
        "Browser preview onboarding and companion readiness checks only",
        "--required-capability",
        "invoke_tool",
        "--required-capability",
        "memory_read",
        "--tag",
        "browser",
        "--tag",
        "onboarding",
        "--label",
        "browser-preview-skill-candidate",
        "--json",
    ])
    .expect("`runtime capability propose` should parse");

    match propose.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Capability { command },
        }) => match command {
            loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Propose(options) => {
                assert_eq!(options.run, "/tmp/runtime-experiment.json");
                assert_eq!(options.output, "/tmp/runtime-capability.json");
                assert_eq!(
                    options.target,
                    loong_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill
                );
                assert_eq!(
                    options.target_summary,
                    "Codify browser preview onboarding as a reusable managed skill"
                );
                assert_eq!(
                    options.bounded_scope,
                    "Browser preview onboarding and companion readiness checks only"
                );
                assert_eq!(
                    options.required_capability,
                    vec!["invoke_tool".to_owned(), "memory_read".to_owned()]
                );
                assert_eq!(
                    options.tag,
                    vec!["browser".to_owned(), "onboarding".to_owned()]
                );
                assert_eq!(
                    options.label.as_deref(),
                    Some("browser-preview-skill-candidate")
                );
                assert!(options.json);
            }
            other @ (loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Review(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Show(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Index(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Plan(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Apply(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Activate(
                _,
            )
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Rollback(
                _,
            )) => {
                panic!("unexpected runtime-capability subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let review = try_parse_cli([
        "loong",
        "runtime",
        "capability",
        "review",
        "--candidate",
        "/tmp/runtime-capability.json",
        "--decision",
        "accepted",
        "--review-summary",
        "Promotion target is bounded and evidence supports manual codification",
        "--warning",
        "still requires manual implementation",
        "--json",
    ])
    .expect("`runtime capability review` should parse");

    match review.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Capability { command },
        }) => match command {
            loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Review(options) => {
                assert_eq!(options.candidate, "/tmp/runtime-capability.json");
                assert_eq!(
                    options.decision,
                    loong_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted
                );
                assert_eq!(
                    options.review_summary,
                    "Promotion target is bounded and evidence supports manual codification"
                );
                assert_eq!(
                    options.warning,
                    vec!["still requires manual implementation".to_owned()]
                );
                assert!(options.json);
            }
            other @ (loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Propose(
                _,
            )
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Show(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Index(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Plan(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Apply(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Activate(
                _,
            )
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Rollback(
                _,
            )) => {
                panic!("unexpected runtime-capability subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let show = try_parse_cli([
        "loong",
        "runtime",
        "capability",
        "show",
        "--candidate",
        "/tmp/runtime-capability.json",
        "--json",
    ])
    .expect("`runtime capability show` should parse");

    match show.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Capability { command },
        }) => match command {
            loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Show(options) => {
                assert_eq!(options.candidate, "/tmp/runtime-capability.json");
                assert!(options.json);
            }
            other @ (loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Propose(
                _,
            )
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Review(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Index(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Plan(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Apply(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Activate(
                _,
            )
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Rollback(
                _,
            )) => {
                panic!("unexpected runtime-capability subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let index = try_parse_cli([
        "loong",
        "runtime",
        "capability",
        "index",
        "--root",
        "/tmp/runtime-capability",
        "--json",
    ])
    .expect("`runtime capability index` should parse");

    match index.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Capability { command },
        }) => match command {
            loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Index(options) => {
                assert_eq!(options.root, "/tmp/runtime-capability");
                assert!(options.json);
            }
            other @ (loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Propose(
                _,
            )
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Review(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Show(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Plan(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Apply(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Activate(
                _,
            )
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Rollback(
                _,
            )) => {
                panic!("unexpected runtime-capability subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let plan = try_parse_cli([
        "loong",
        "runtime",
        "capability",
        "plan",
        "--root",
        "/tmp/runtime-capability",
        "--family-id",
        "family-123",
        "--json",
    ])
    .expect("`runtime capability plan` should parse");

    match plan.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Capability { command },
        }) => match command {
            loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Plan(options) => {
                assert_eq!(options.root, "/tmp/runtime-capability");
                assert_eq!(options.family_id, "family-123");
                assert!(options.json);
            }
            other @ (loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Propose(
                _,
            )
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Review(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Show(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Index(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Apply(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Activate(
                _,
            )
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Rollback(
                _,
            )) => {
                panic!("unexpected runtime-capability subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let apply = try_parse_cli([
        "loong",
        "runtime",
        "capability",
        "apply",
        "--root",
        "/tmp/runtime-capability",
        "--family-id",
        "family-123",
        "--json",
    ])
    .expect("`runtime capability apply` should parse");

    match apply.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Capability { command },
        }) => match command {
            loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Apply(options) => {
                assert_eq!(options.root, "/tmp/runtime-capability");
                assert_eq!(options.family_id, "family-123");
                assert!(options.json);
            }
            other @ (loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Propose(
                _,
            )
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Review(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Show(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Index(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Plan(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Activate(
                _,
            )
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Rollback(
                _,
            )) => {
                panic!("unexpected runtime-capability subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let activate = try_parse_cli([
        "loong",
        "runtime",
        "capability",
        "activate",
        "--config",
        "/tmp/loong.toml",
        "--artifact",
        "/tmp/runtime-capability-apply.json",
        "--apply",
        "--replace",
        "--json",
    ])
    .expect("`runtime capability activate` should parse");

    match activate.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Capability { command },
        }) => match command {
            loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Activate(options) => {
                assert_eq!(options.config.as_deref(), Some("/tmp/loong.toml"));
                assert_eq!(options.artifact, "/tmp/runtime-capability-apply.json");
                assert!(options.apply);
                assert!(options.replace);
                assert!(options.json);
            }
            other @ (loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Propose(
                _,
            )
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Review(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Show(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Index(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Plan(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Apply(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Rollback(
                _,
            )) => {
                panic!("unexpected runtime-capability subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let rollback = try_parse_cli([
        "loong",
        "runtime",
        "capability",
        "rollback",
        "--config",
        "/tmp/loong.toml",
        "--record",
        "/tmp/runtime-capability-activation.json",
        "--apply",
        "--json",
    ])
    .expect("`runtime capability rollback` should parse");

    match rollback.command {
        Some(Commands::Runtime {
            command: loong_daemon::runtime_cli::RuntimeCommands::Capability { command },
        }) => match command {
            loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Rollback(options) => {
                assert_eq!(options.config.as_deref(), Some("/tmp/loong.toml"));
                assert_eq!(options.record, "/tmp/runtime-capability-activation.json");
                assert!(options.apply);
                assert!(options.json);
            }
            other @ (loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Propose(
                _,
            )
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Review(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Show(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Index(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Plan(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Apply(_)
            | loong_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Activate(
                _,
            )) => {
                panic!("unexpected runtime-capability subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn acp_event_summary_cli_rejects_zero_limit() {
    let error = run_acp_event_summary_cli(None, Some("session-a"), 0, false)
        .expect_err("zero limit must be rejected");
    assert!(error.contains(">= 1"));
}

#[test]
fn build_acp_dispatch_address_requires_channel_for_structured_scope() {
    let error =
        build_acp_dispatch_address("opaque-session", None, Some("oc_123"), None, None, None)
            .expect_err("structured scope without channel must be rejected");
    assert!(error.contains("--channel"));
}

#[test]
fn build_acp_dispatch_address_builds_structured_scope() {
    let address = build_acp_dispatch_address(
        "opaque-session",
        Some("Feishu"),
        Some("oc_123"),
        Some("LARK PROD"),
        Some("ou_sender_1"),
        Some("om_thread_1"),
    )
    .expect("structured scope should build");

    assert_eq!(address.session_id, "opaque-session");
    assert_eq!(address.channel_id.as_deref(), Some("feishu"));
    assert_eq!(address.account_id.as_deref(), Some("lark-prod"));
    assert_eq!(address.conversation_id.as_deref(), Some("oc_123"));
    assert_eq!(address.participant_id.as_deref(), Some("ou_sender_1"));
    assert_eq!(address.thread_id.as_deref(), Some("om_thread_1"));
}

#[test]
fn format_u32_rollup_uses_dash_for_empty_map() {
    let rendered = format_u32_rollup(&BTreeMap::new());
    assert_eq!(rendered, "-");
}

#[test]
fn format_acp_event_summary_includes_routing_intent_and_provenance() {
    let rendered = format_acp_event_summary(
        "telegram:42",
        120,
        &mvp::acp::AcpTurnEventSummary {
            turn_event_records: 4,
            final_records: 2,
            done_events: 2,
            error_events: 1,
            text_events: 1,
            usage_update_events: 1,
            turns_succeeded: 1,
            turns_cancelled: 1,
            turns_failed: 0,
            event_type_counts: BTreeMap::from([
                ("done".to_owned(), 2u32),
                ("text".to_owned(), 1u32),
            ]),
            stop_reason_counts: BTreeMap::from([
                ("completed".to_owned(), 1u32),
                ("cancelled".to_owned(), 1u32),
            ]),
            routing_intent_counts: BTreeMap::from([("explicit".to_owned(), 2u32)]),
            routing_origin_counts: BTreeMap::from([("explicit_request".to_owned(), 2u32)]),
            last_backend_id: Some("acpx".to_owned()),
            last_agent_id: Some("codex".to_owned()),
            last_session_key: Some("agent:codex:telegram:42".to_owned()),
            last_conversation_id: Some("telegram:42".to_owned()),
            last_binding_route_session_id: Some("telegram:bot_123456:42".to_owned()),
            last_channel_id: Some("telegram".to_owned()),
            last_account_id: Some("bot_123456".to_owned()),
            last_channel_conversation_id: Some("42".to_owned()),
            last_channel_participant_id: None,
            last_channel_thread_id: None,
            last_routing_intent: Some("explicit".to_owned()),
            last_routing_origin: Some("explicit_request".to_owned()),
            last_trace_id: Some("trace-123".to_owned()),
            last_source_message_id: Some("message-42".to_owned()),
            last_ack_cursor: Some("cursor-9".to_owned()),
            last_turn_state: Some("ready".to_owned()),
            last_stop_reason: Some("cancelled".to_owned()),
            last_error: Some("permission denied".to_owned()),
        },
    );

    assert!(rendered.contains("acp_event_summary session=telegram:42 limit=120"));
    assert!(rendered.contains("routing_intent=explicit"));
    assert!(rendered.contains("routing_origin=explicit_request"));
    assert!(rendered.contains("routing_intents=explicit:2"));
    assert!(rendered.contains("routing_origins=explicit_request:2"));
    assert!(rendered.contains("trace_id=trace-123"));
    assert!(rendered.contains("source_message_id=message-42"));
    assert!(rendered.contains("ack_cursor=cursor-9"));
}

#[test]
fn chat_cli_accepts_acp_runtime_option_flags() {
    let cli = try_parse_cli([
        "loong",
        "chat",
        "--session",
        "telegram:42",
        "--acp",
        "--acp-event-stream",
        "--acp-bootstrap-mcp-server",
        "filesystem",
        "--acp-bootstrap-mcp-server",
        "search",
        "--acp-cwd",
        "/workspace/project",
    ])
    .expect("chat CLI should parse ACP runtime option flags");

    match cli.command {
        Some(Commands::Chat {
            session,
            acp,
            acp_event_stream,
            acp_bootstrap_mcp_server,
            acp_cwd,
            ..
        }) => {
            assert_eq!(session.as_deref(), Some("telegram:42"));
            assert!(acp);
            assert!(acp_event_stream);
            assert_eq!(
                acp_bootstrap_mcp_server,
                vec!["filesystem".to_owned(), "search".to_owned()]
            );
            assert_eq!(acp_cwd.as_deref(), Some("/workspace/project"));
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn feishu_namespace_send_parses_canonical_shape() {
    let cli = try_parse_cli([
        "loong",
        "feishu",
        "send",
        "--receive-id",
        "ou_123",
        "--text",
        "hello",
    ])
    .expect("feishu send CLI should parse through the canonical namespace");

    match cli.command {
        Some(Commands::Feishu {
            command: loong_daemon::feishu_cli::FeishuCommand::Send(args),
        }) => {
            assert_eq!(args.grant.common.account, None);
            assert_eq!(args.receive_id, "ou_123");
            assert_eq!(args.text.as_deref(), Some("hello"));
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn grouped_channel_serve_accepts_account_and_stop_controls() {
    let cli = try_parse_cli([
        "loong",
        "channels",
        "serve",
        "--channel",
        "matrix",
        "--account",
        "ops",
        "--stop",
    ])
    .expect("grouped matrix serve should parse");

    match cli.command {
        Some(Commands::Channels {
            command: Some(loong_daemon::ChannelsCommands::Serve(args)),
            ..
        }) => {
            assert_eq!(args.channel_name.as_deref(), Some("matrix"));
            assert_eq!(args.account.as_deref(), Some("ops"));
            assert!(args.stop);
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

fn fake_send_cli_runner(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let target = args.target.unwrap_or("-");
        Err(format!(
            "config={}|account={}|target={}|target_kind={}|text={}|card={}",
            args.config_path.unwrap_or("-"),
            args.account.unwrap_or("-"),
            target,
            args.target_kind.as_str(),
            args.text,
            args.as_card
        ))
    })
}

fn fake_serve_cli_runner(args: ChannelServeCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        Err(format!(
            "config={}|account={}|once={}|stop={}|stop_duplicates={}|bind={}|path={}",
            args.config_path.unwrap_or("-"),
            args.account.unwrap_or("-"),
            args.once,
            args.stop_requested,
            args.stop_duplicates_requested,
            args.bind_override.unwrap_or("-"),
            args.path_override.unwrap_or("-")
        ))
    })
}

#[test]
fn run_channel_send_cli_forwards_common_arguments_to_runner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build test runtime");
    let error = runtime
        .block_on(run_channel_send_cli(
            ChannelSendCliSpec {
                family: mvp::channel::FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
                run: fake_send_cli_runner,
            },
            ChannelSendCliArgs {
                config_path: Some("/tmp/loong.toml"),
                account: Some("ops"),
                target: Some("om_42"),
                target_kind: mvp::channel::ChannelOutboundTargetKind::MessageReply,
                text: "hello",
                as_card: true,
            },
        ))
        .expect_err("fake runner should surface forwarded arguments");

    assert_eq!(
        error,
        "config=/tmp/loong.toml|account=ops|target=om_42|target_kind=message_reply|text=hello|card=true"
    );
}

#[test]
fn run_channel_serve_cli_forwards_optional_arguments_to_runner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build test runtime");
    let error = runtime
        .block_on(run_channel_serve_cli(
            ChannelServeCliSpec {
                family: mvp::channel::FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
                run: fake_serve_cli_runner,
            },
            ChannelServeCliArgs {
                config_path: Some("/tmp/loong.toml"),
                account: Some("ops"),
                once: true,
                stop_requested: false,
                stop_duplicates_requested: false,
                bind_override: Some("127.0.0.1:8123"),
                path_override: Some("/hooks/feishu"),
            },
        ))
        .expect_err("fake runner should surface forwarded arguments");

    assert_eq!(
        error,
        "config=/tmp/loong.toml|account=ops|once=true|stop=false|stop_duplicates=false|bind=127.0.0.1:8123|path=/hooks/feishu"
    );
}

#[test]
fn gateway_run_cli_parses_channel_account_selection_flags() {
    let cli = try_parse_cli([
        "loong",
        "gateway",
        "run",
        "--session",
        "cli-supervisor",
        "--channel-account",
        "telegram=bot_123456",
        "--channel-account",
        "lark=alerts",
        "--channel-account",
        "matrix=bridge-sync",
        "--channel-account",
        "wecom=robot-prod",
    ])
    .expect("gateway run should parse channel-account selectors");

    match cli.command {
        Some(Commands::Gateway {
            command:
                loong_daemon::gateway::service::GatewayCommand::Run {
                    session,
                    channel_account,
                    ..
                },
        }) => {
            assert_eq!(session.as_deref(), Some("cli-supervisor"));
            assert_eq!(channel_account.len(), 4);
            assert_eq!(channel_account[0].channel_id, "telegram");
            assert_eq!(channel_account[0].account_id, "bot_123456");
            assert_eq!(channel_account[1].channel_id, "feishu");
            assert_eq!(channel_account[1].account_id, "alerts");
            assert_eq!(channel_account[2].channel_id, "matrix");
            assert_eq!(channel_account[2].account_id, "bridge-sync");
            assert_eq!(channel_account[3].channel_id, "wecom");
            assert_eq!(channel_account[3].account_id, "robot-prod");
        }
        other => panic!("unexpected parse result: {other:?}"),
    }
}

#[test]
fn default_channel_send_target_kind_uses_command_family_send_metadata() {
    assert_eq!(
        default_channel_send_target_kind(ChannelSendCliSpec {
            family: mvp::channel::FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
            run: fake_send_cli_runner,
        }),
        mvp::channel::ChannelOutboundTargetKind::ReceiveId
    );
    assert_eq!(
        default_channel_send_target_kind(ChannelSendCliSpec {
            family: mvp::channel::TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
            run: fake_send_cli_runner,
        }),
        mvp::channel::ChannelOutboundTargetKind::Conversation
    );
    assert_eq!(
        default_channel_send_target_kind(ChannelSendCliSpec {
            family: mvp::channel::MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
            run: fake_send_cli_runner,
        }),
        mvp::channel::ChannelOutboundTargetKind::Conversation
    );
    assert_eq!(
        default_channel_send_target_kind(ChannelSendCliSpec {
            family: mvp::channel::WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
            run: fake_send_cli_runner,
        }),
        mvp::channel::ChannelOutboundTargetKind::Conversation
    );
    assert_eq!(
        default_channel_send_target_kind(ChannelSendCliSpec {
            family: mvp::channel::LINE_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
            run: fake_send_cli_runner,
        }),
        mvp::channel::ChannelOutboundTargetKind::Address
    );
    assert_eq!(
        default_channel_send_target_kind(ChannelSendCliSpec {
            family: mvp::channel::MATTERMOST_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
            run: fake_send_cli_runner,
        }),
        mvp::channel::ChannelOutboundTargetKind::Conversation
    );
}
