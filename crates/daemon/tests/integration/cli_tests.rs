use super::*;

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
    let mut command = Cli::command();
    let mut rendered = Vec::new();
    command
        .write_long_help(&mut rendered)
        .expect("render root help");
    let help = String::from_utf8(rendered).expect("help is valid utf-8");

    assert!(help.contains("onboarding"));
    assert!(
        !help
            .lines()
            .any(|line| line.trim_start().starts_with("setup ")),
        "root help should not advertise a standalone `setup` subcommand: {help}"
    );
}

#[test]
fn setup_subcommand_is_removed() {
    let error = Cli::try_parse_from(["loongclaw", "setup"])
        .expect_err("`setup` should no longer parse as a valid subcommand");
    assert!(
        error
            .to_string()
            .contains("unrecognized subcommand 'setup'")
    );
}

#[test]
fn safe_lane_summary_cli_rejects_zero_limit() {
    let error = run_safe_lane_summary_cli(None, Some("session-a"), 0, false)
        .expect_err("zero limit must be rejected");
    assert!(error.contains(">= 1"));
}

#[test]
fn onboard_cli_accepts_generic_api_key_flag() {
    let cli = Cli::try_parse_from([
        "loongclaw",
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
    let cli = Cli::try_parse_from([
        "loongclaw",
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
fn onboard_cli_accepts_personality_flag() {
    let cli = Cli::try_parse_from([
        "loongclaw",
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
    let cli = Cli::try_parse_from([
        "loongclaw",
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
    let cli = Cli::try_parse_from([
        "loongclaw",
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
    let cli = Cli::try_parse_from(["loongclaw", "benchmark-memory-context"])
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
    let cli = Cli::try_parse_from(["loongclaw", "list-memory-systems"])
        .expect("`list-memory-systems` should parse");

    match cli.command {
        Some(Commands::ListMemorySystems { config, json }) => {
            assert!(config.is_none());
            assert!(!json);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_snapshot_cli_parses() {
    let cli = Cli::try_parse_from([
        "loongclaw",
        "runtime-snapshot",
        "--config",
        "/tmp/loongclaw.toml",
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
    .expect("`runtime-snapshot` should parse");

    match cli.command {
        Some(Commands::RuntimeSnapshot {
            config,
            json,
            output,
            label,
            experiment_id,
            parent_snapshot_id,
        }) => {
            assert_eq!(config.as_deref(), Some("/tmp/loongclaw.toml"));
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
fn runtime_restore_cli_parses() {
    let cli = Cli::try_parse_from([
        "loongclaw",
        "runtime-restore",
        "--config",
        "/tmp/loongclaw.toml",
        "--snapshot",
        "/tmp/runtime-snapshot.json",
        "--json",
        "--apply",
    ])
    .expect("`runtime-restore` should parse");

    match cli.command {
        Some(Commands::RuntimeRestore {
            config,
            snapshot,
            json,
            apply,
        }) => {
            assert_eq!(config.as_deref(), Some("/tmp/loongclaw.toml"));
            assert_eq!(snapshot, "/tmp/runtime-snapshot.json");
            assert!(json);
            assert!(apply);
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_experiment_cli_parses_restore() {
    let cli = Cli::try_parse_from([
        "loongclaw",
        "runtime-experiment",
        "restore",
        "--run",
        "/tmp/runtime-experiment.json",
        "--stage",
        "result",
        "--config",
        "/tmp/loongclaw.toml",
        "--json",
        "--apply",
    ])
    .expect("`runtime-experiment restore` should parse");

    match cli.command {
        Some(Commands::RuntimeExperiment { command }) => match command {
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Restore(
                options,
            ) => {
                assert_eq!(options.run, "/tmp/runtime-experiment.json");
                assert_eq!(
                    options.stage,
                    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentRestoreStage::Result
                );
                assert_eq!(options.config.as_deref(), Some("/tmp/loongclaw.toml"));
                assert!(options.json);
                assert!(options.apply);
            }
            other @ (loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Start(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Finish(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Show(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Compare(_)) => {
                panic!("unexpected runtime-experiment subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_experiment_cli_parses_start_finish_and_show() {
    let start = Cli::try_parse_from([
        "loongclaw",
        "runtime-experiment",
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
    .expect("`runtime-experiment start` should parse");

    match start.command {
        Some(Commands::RuntimeExperiment { command }) => match command {
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Start(options) => {
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
            other @ (loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Finish(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Show(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Compare(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Restore(_)) => {
                panic!("unexpected runtime-experiment subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let finish = Cli::try_parse_from([
        "loongclaw",
        "runtime-experiment",
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
    .expect("`runtime-experiment finish` should parse");

    match finish.command {
        Some(Commands::RuntimeExperiment { command }) => match command {
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Finish(
                options,
            ) => {
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
                    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentDecision::Promoted
                );
                assert_eq!(
                    options.status,
                    loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentFinishStatus::Completed
                );
                assert!(options.json);
            }
            other
            @ (loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Start(
                _,
            )
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Show(
                _,
            )
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Compare(
                _,
            )
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Restore(
                _,
            )) => {
                panic!("unexpected runtime-experiment subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let show = Cli::try_parse_from([
        "loongclaw",
        "runtime-experiment",
        "show",
        "--run",
        "/tmp/runtime-experiment.json",
        "--json",
    ])
    .expect("`runtime-experiment show` should parse");

    match show.command {
        Some(Commands::RuntimeExperiment { command }) => match command {
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Show(options) => {
                assert_eq!(options.run, "/tmp/runtime-experiment.json");
                assert!(options.json);
            }
            other @ (loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Start(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Finish(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Compare(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Restore(_)) => {
                panic!("unexpected runtime-experiment subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_experiment_cli_parses_compare() {
    let compare = Cli::try_parse_from([
        "loongclaw",
        "runtime-experiment",
        "compare",
        "--run",
        "/tmp/runtime-experiment.json",
        "--baseline-snapshot",
        "/tmp/runtime-snapshot.json",
        "--result-snapshot",
        "/tmp/runtime-snapshot-result.json",
        "--json",
    ])
    .expect("`runtime-experiment compare` should parse");

    match compare.command {
        Some(Commands::RuntimeExperiment { command }) => match command {
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Compare(
                options,
            ) => {
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
            other @ (loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Start(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Finish(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Show(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Restore(_)) => {
                panic!("unexpected runtime-experiment subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_experiment_cli_parses_compare_with_recorded_snapshots() {
    let compare = Cli::try_parse_from([
        "loongclaw",
        "runtime-experiment",
        "compare",
        "--run",
        "/tmp/runtime-experiment.json",
        "--recorded-snapshots",
        "--json",
    ])
    .expect("`runtime-experiment compare --recorded-snapshots` should parse");

    match compare.command {
        Some(Commands::RuntimeExperiment { command }) => match command {
            loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Compare(
                options,
            ) => {
                assert_eq!(options.run, "/tmp/runtime-experiment.json");
                assert_eq!(options.baseline_snapshot, None);
                assert_eq!(options.result_snapshot, None);
                assert!(options.recorded_snapshots);
                assert!(options.json);
            }
            other @ (loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Start(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Finish(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Show(_)
            | loongclaw_daemon::runtime_experiment_cli::RuntimeExperimentCommands::Restore(_)) => {
                panic!("unexpected runtime-experiment subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn runtime_experiment_cli_rejects_compare_recorded_snapshots_with_manual_paths() {
    let error = Cli::try_parse_from([
        "loongclaw",
        "runtime-experiment",
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
fn runtime_capability_cli_parses_propose_review_show_index_and_plan() {
    let propose = Cli::try_parse_from([
        "loongclaw",
        "runtime-capability",
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
    .expect("`runtime-capability propose` should parse");

    match propose.command {
        Some(Commands::RuntimeCapability { command }) => match command {
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Propose(
                options,
            ) => {
                assert_eq!(options.run, "/tmp/runtime-experiment.json");
                assert_eq!(options.output, "/tmp/runtime-capability.json");
                assert_eq!(
                    options.target,
                    loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityTarget::ManagedSkill
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
            other @ (loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Review(
                _,
            )
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Show(_)
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Index(_)
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Plan(_)) => {
                panic!("unexpected runtime-capability subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let review = Cli::try_parse_from([
        "loongclaw",
        "runtime-capability",
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
    .expect("`runtime-capability review` should parse");

    match review.command {
        Some(Commands::RuntimeCapability { command }) => match command {
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Review(
                options,
            ) => {
                assert_eq!(options.candidate, "/tmp/runtime-capability.json");
                assert_eq!(
                    options.decision,
                    loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityReviewDecision::Accepted
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
            other @ (loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Propose(
                _,
            )
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Show(_)
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Index(_)
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Plan(_)) => {
                panic!("unexpected runtime-capability subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let show = Cli::try_parse_from([
        "loongclaw",
        "runtime-capability",
        "show",
        "--candidate",
        "/tmp/runtime-capability.json",
        "--json",
    ])
    .expect("`runtime-capability show` should parse");

    match show.command {
        Some(Commands::RuntimeCapability { command }) => match command {
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Show(options) => {
                assert_eq!(options.candidate, "/tmp/runtime-capability.json");
                assert!(options.json);
            }
            other @ (loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Propose(
                _,
            )
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Review(
                _,
            )
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Index(_)
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Plan(_)) => {
                panic!("unexpected runtime-capability subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let index = Cli::try_parse_from([
        "loongclaw",
        "runtime-capability",
        "index",
        "--root",
        "/tmp/runtime-capability",
        "--json",
    ])
    .expect("`runtime-capability index` should parse");

    match index.command {
        Some(Commands::RuntimeCapability { command }) => match command {
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Index(options) => {
                assert_eq!(options.root, "/tmp/runtime-capability");
                assert!(options.json);
            }
            other @ (loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Propose(
                _,
            )
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Review(_)
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Show(_)
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Plan(_)) => {
                panic!("unexpected runtime-capability subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }

    let plan = Cli::try_parse_from([
        "loongclaw",
        "runtime-capability",
        "plan",
        "--root",
        "/tmp/runtime-capability",
        "--family-id",
        "family-123",
        "--json",
    ])
    .expect("`runtime-capability plan` should parse");

    match plan.command {
        Some(Commands::RuntimeCapability { command }) => match command {
            loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Plan(options) => {
                assert_eq!(options.root, "/tmp/runtime-capability");
                assert_eq!(options.family_id, "family-123");
                assert!(options.json);
            }
            other @ (loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Propose(
                _,
            )
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Review(_)
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Show(_)
            | loongclaw_daemon::runtime_capability_cli::RuntimeCapabilityCommands::Index(_)) => {
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
    let error = build_acp_dispatch_address("opaque-session", None, Some("oc_123"), None, None)
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
        Some("om_thread_1"),
    )
    .expect("structured scope should build");

    assert_eq!(address.session_id, "opaque-session");
    assert_eq!(address.channel_id.as_deref(), Some("feishu"));
    assert_eq!(address.account_id.as_deref(), Some("lark-prod"));
    assert_eq!(address.conversation_id.as_deref(), Some("oc_123"));
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
    let cli = Cli::try_parse_from([
        "loongclaw",
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
fn feishu_send_cli_accepts_generic_target_and_target_kind() {
    let cli = Cli::try_parse_from([
        "loongclaw",
        channel_send_command("feishu"),
        "--target",
        "om_123",
        "--target-kind",
        "message_reply",
        "--text",
        "hello",
    ])
    .expect("generic feishu target flags should parse");

    match cli.command {
        Some(Commands::FeishuSend {
            target,
            target_kind,
            text,
            ..
        }) => {
            assert_eq!(target, "om_123");
            assert_eq!(
                target_kind,
                mvp::channel::ChannelOutboundTargetKind::MessageReply
            );
            assert_eq!(text.as_deref(), Some("hello"));
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn feishu_send_cli_keeps_receive_id_alias() {
    let cli = Cli::try_parse_from([
        "loongclaw",
        channel_send_command("feishu"),
        "--receive-id",
        "ou_123",
        "--text",
        "hello",
    ])
    .expect("legacy receive-id alias should still parse");

    match cli.command {
        Some(Commands::FeishuSend {
            target,
            target_kind,
            text,
            ..
        }) => {
            assert_eq!(target, "ou_123");
            assert_eq!(
                target_kind,
                mvp::channel::ChannelOutboundTargetKind::ReceiveId
            );
            assert_eq!(text.as_deref(), Some("hello"));
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn feishu_send_cli_rejects_unsupported_conversation_target_kind() {
    let error = Cli::try_parse_from([
        "loongclaw",
        channel_send_command("feishu"),
        "--target",
        "oc_123",
        "--target-kind",
        "conversation",
        "--text",
        "hello",
    ])
    .expect_err("conversation target kind should be rejected");

    assert!(
        error
            .to_string()
            .contains("use `receive_id` or `message_reply`")
    );
}

#[test]
fn feishu_send_cli_defaults_target_kind_from_catalog_metadata() {
    let cli = Cli::try_parse_from([
        "loongclaw",
        channel_send_command("feishu"),
        "--target",
        "ou_123",
        "--text",
        "hello",
    ])
    .expect("default feishu target kind should parse from catalog metadata");

    match cli.command {
        Some(Commands::FeishuSend { target_kind, .. }) => {
            assert_eq!(target_kind, channel_default_send_target_kind("feishu"));
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn telegram_send_cli_accepts_generic_target_and_defaults_to_conversation() {
    let cli = Cli::try_parse_from([
        "loongclaw",
        channel_send_command("telegram"),
        "--target",
        "123:topic:7",
        "--text",
        "hello",
    ])
    .expect("telegram send CLI should parse");

    match cli.command {
        Some(Commands::TelegramSend {
            target,
            target_kind,
            text,
            ..
        }) => {
            assert_eq!(target, "123:topic:7");
            assert_eq!(target_kind, channel_default_send_target_kind("telegram"));
            assert_eq!(text, "hello");
        }
        other => panic!("unexpected command parse result: {other:?}"),
    }
}

#[test]
fn telegram_send_cli_rejects_non_conversation_target_kind() {
    let error = Cli::try_parse_from([
        "loongclaw",
        channel_send_command("telegram"),
        "--target",
        "123",
        "--target-kind",
        "message_reply",
        "--text",
        "hello",
    ])
    .expect_err("telegram send should reject non-conversation kinds");

    assert!(
        error.to_string().contains(
            "telegram --target-kind does not support `message_reply`; use `conversation`"
        )
    );
}

fn fake_send_cli_runner(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        Err(format!(
            "config={}|account={}|target={}|target_kind={}|text={}|card={}",
            args.config_path.unwrap_or("-"),
            args.account.unwrap_or("-"),
            args.target,
            args.target_kind.as_str(),
            args.text,
            args.as_card
        ))
    })
}

fn fake_serve_cli_runner(args: ChannelServeCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        Err(format!(
            "config={}|account={}|once={}|bind={}|path={}",
            args.config_path.unwrap_or("-"),
            args.account.unwrap_or("-"),
            args.once,
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
                family: mvp::channel::FEISHU_COMMAND_FAMILY_DESCRIPTOR,
                run: fake_send_cli_runner,
            },
            ChannelSendCliArgs {
                config_path: Some("/tmp/loongclaw.toml"),
                account: Some("ops"),
                target: "om_42",
                target_kind: mvp::channel::ChannelOutboundTargetKind::MessageReply,
                text: "hello",
                as_card: true,
            },
        ))
        .expect_err("fake runner should surface forwarded arguments");

    assert_eq!(
        error,
        "config=/tmp/loongclaw.toml|account=ops|target=om_42|target_kind=message_reply|text=hello|card=true"
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
                family: mvp::channel::FEISHU_COMMAND_FAMILY_DESCRIPTOR,
                run: fake_serve_cli_runner,
            },
            ChannelServeCliArgs {
                config_path: Some("/tmp/loongclaw.toml"),
                account: Some("ops"),
                once: true,
                bind_override: Some("127.0.0.1:8123"),
                path_override: Some("/hooks/feishu"),
            },
        ))
        .expect_err("fake runner should surface forwarded arguments");

    assert_eq!(
        error,
        "config=/tmp/loongclaw.toml|account=ops|once=true|bind=127.0.0.1:8123|path=/hooks/feishu"
    );
}

#[test]
fn default_channel_send_target_kind_uses_command_family_send_metadata() {
    assert_eq!(
        default_channel_send_target_kind(ChannelSendCliSpec {
            family: mvp::channel::FEISHU_COMMAND_FAMILY_DESCRIPTOR,
            run: fake_send_cli_runner,
        }),
        mvp::channel::ChannelOutboundTargetKind::ReceiveId
    );
    assert_eq!(
        default_channel_send_target_kind(ChannelSendCliSpec {
            family: mvp::channel::TELEGRAM_COMMAND_FAMILY_DESCRIPTOR,
            run: fake_send_cli_runner,
        }),
        mvp::channel::ChannelOutboundTargetKind::Conversation
    );
}
