use super::*;

#[test]
fn plugins_bridge_profiles_cli_parses_selected_profile_and_json_flag() {
    let cli = try_parse_cli([
        "loongclaw",
        "plugins",
        "bridge-profiles",
        "--profile",
        "openclaw-ecosystem-balanced",
        "--json",
    ])
    .expect("plugins bridge-profiles CLI should parse");

    match cli.command {
        Some(Commands::Plugins { json, command }) => {
            assert!(json);
            match command {
                loongclaw_daemon::plugins_cli::PluginsCommands::BridgeProfiles(command) => {
                    assert_eq!(
                        command.profiles,
                        vec![
                            loongclaw_daemon::plugins_cli::PluginBridgeProfileArg::OpenclawEcosystemBalanced
                        ]
                    );
                }
                other @ loongclaw_daemon::plugins_cli::PluginsCommands::Init(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Doctor(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Inventory(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeTemplate(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Preflight(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Actions(_) => {
                    panic!("unexpected plugins subcommand parsed: {other:?}");
                }
            }
        }
        other => panic!("unexpected parse result: {other:?}"),
    }
}

#[test]
fn plugins_inventory_cli_parses_bridge_profile_and_examples_flag() {
    let cli = try_parse_cli([
        "loongclaw",
        "plugins",
        "inventory",
        "--root",
        "/tmp/plugins",
        "--query",
        "weather-sdk",
        "--bridge-profile",
        "openclaw-ecosystem-balanced",
        "--include-examples",
        "--json",
    ])
    .expect("plugins inventory CLI should parse");

    match cli.command {
        Some(Commands::Plugins { json, command }) => {
            assert!(json);
            match command {
                loongclaw_daemon::plugins_cli::PluginsCommands::Inventory(command) => {
                    assert_eq!(command.source.roots, vec!["/tmp/plugins".to_owned()]);
                    assert_eq!(command.source.query, "weather-sdk");
                    assert_eq!(command.source.limit, None);
                    assert_eq!(
                        command.source.bridge_profile,
                        Some(
                            loongclaw_daemon::plugins_cli::PluginBridgeProfileArg::OpenclawEcosystemBalanced
                        )
                    );
                    assert!(command.include_ready);
                    assert!(command.include_blocked);
                    assert!(command.include_deferred);
                    assert!(command.include_examples);
                }
                other @ loongclaw_daemon::plugins_cli::PluginsCommands::Init(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Doctor(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeProfiles(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeTemplate(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Preflight(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Actions(_) => {
                    panic!("unexpected plugins subcommand parsed: {other:?}");
                }
            }
        }
        other => panic!("unexpected parse result: {other:?}"),
    }
}

#[test]
fn plugins_doctor_cli_defaults_to_sdk_release_profile() {
    let cli = try_parse_cli([
        "loongclaw",
        "plugins",
        "doctor",
        "--root",
        "/tmp/plugins",
        "--query",
        "weather-sdk",
        "--bridge-profile",
        "openclaw-ecosystem-balanced",
        "--json",
    ])
    .expect("plugins doctor CLI should parse");

    match cli.command {
        Some(Commands::Plugins { json, command }) => {
            assert!(json);
            match command {
                loongclaw_daemon::plugins_cli::PluginsCommands::Doctor(command) => {
                    assert_eq!(command.source.scan.roots, vec!["/tmp/plugins".to_owned()]);
                    assert_eq!(command.source.scan.query, "weather-sdk");
                    assert_eq!(command.source.scan.limit, None);
                    assert_eq!(
                        command.source.scan.bridge_profile,
                        Some(
                            loongclaw_daemon::plugins_cli::PluginBridgeProfileArg::OpenclawEcosystemBalanced
                        )
                    );
                    assert_eq!(
                        command.source.profile,
                        loongclaw_daemon::plugins_cli::PluginPreflightProfileArg::SdkRelease
                    );
                    assert!(command.include_passed);
                    assert!(command.include_warned);
                    assert!(command.include_blocked);
                    assert!(command.include_deferred);
                }
                other @ loongclaw_daemon::plugins_cli::PluginsCommands::Init(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Inventory(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeProfiles(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeTemplate(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Preflight(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Actions(_) => {
                    panic!("unexpected plugins subcommand parsed: {other:?}");
                }
            }
        }
        other => panic!("unexpected parse result: {other:?}"),
    }
}

#[test]
fn plugins_actions_cli_parses_filters_and_global_json_after_subcommand() {
    let cli = try_parse_cli([
        "loongclaw",
        "plugins",
        "actions",
        "--root",
        "/tmp/plugins-a",
        "--root",
        "/tmp/plugins-b",
        "--profile",
        "runtime-activation",
        "--bridge-profile",
        "openclaw-ecosystem-balanced",
        "--surface",
        "plugin-package",
        "--kind",
        "resolve-slot-ownership",
        "--requires-reload",
        "true",
        "--json",
    ])
    .expect("plugins actions CLI should parse");

    match cli.command {
        Some(Commands::Plugins { json, command }) => {
            assert!(json);
            match command {
                loongclaw_daemon::plugins_cli::PluginsCommands::Actions(command) => {
                    assert_eq!(
                        command.source.scan.roots,
                        vec!["/tmp/plugins-a".to_owned(), "/tmp/plugins-b".to_owned()]
                    );
                    assert_eq!(
                        command.source.profile,
                        loongclaw_daemon::plugins_cli::PluginPreflightProfileArg::RuntimeActivation
                    );
                    assert_eq!(
                        command.source.scan.bridge_profile,
                        Some(
                            loongclaw_daemon::plugins_cli::PluginBridgeProfileArg::OpenclawEcosystemBalanced
                        )
                    );
                    assert_eq!(
                        command.surface,
                        vec![loongclaw_daemon::plugins_cli::PluginActionSurfaceArg::PluginPackage]
                    );
                    assert_eq!(
                        command.kind,
                        vec![loongclaw_daemon::plugins_cli::PluginActionKindArg::ResolveSlotOwnership]
                    );
                    assert_eq!(command.requires_reload, Some(true));
                }
                other @ loongclaw_daemon::plugins_cli::PluginsCommands::Init(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Doctor(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Inventory(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeProfiles(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeTemplate(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Preflight(_) => {
                    panic!("unexpected plugins subcommand parsed: {other:?}");
                }
            }
        }
        other => panic!("unexpected parse result: {other:?}"),
    }
}

#[test]
fn plugins_bridge_template_cli_parses_output_and_bridge_profile() {
    let cli = try_parse_cli([
        "loongclaw",
        "plugins",
        "bridge-template",
        "--root",
        "/tmp/plugins",
        "--bridge-profile",
        "openclaw-ecosystem-balanced",
        "--output",
        "/tmp/bridge-support.json",
        "--delta-output",
        "/tmp/bridge-support.delta.json",
        "--json",
    ])
    .expect("plugins bridge-template CLI should parse");

    match cli.command {
        Some(Commands::Plugins { json, command }) => {
            assert!(json);
            match command {
                loongclaw_daemon::plugins_cli::PluginsCommands::BridgeTemplate(command) => {
                    assert_eq!(command.source.scan.roots, vec!["/tmp/plugins".to_owned()]);
                    assert_eq!(
                        command.source.scan.bridge_profile,
                        Some(
                            loongclaw_daemon::plugins_cli::PluginBridgeProfileArg::OpenclawEcosystemBalanced
                        )
                    );
                    assert_eq!(command.output.as_deref(), Some("/tmp/bridge-support.json"));
                    assert_eq!(
                        command.delta_output.as_deref(),
                        Some("/tmp/bridge-support.delta.json")
                    );
                }
                other @ loongclaw_daemon::plugins_cli::PluginsCommands::Init(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Doctor(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Inventory(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeProfiles(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Preflight(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Actions(_) => {
                    panic!("unexpected plugins subcommand parsed: {other:?}");
                }
            }
        }
        other => panic!("unexpected parse result: {other:?}"),
    }
}

#[test]
fn plugins_preflight_cli_parses_bridge_support_delta_selector() {
    let cli = try_parse_cli([
        "loongclaw",
        "plugins",
        "preflight",
        "--root",
        "/tmp/plugins",
        "--bridge-support-delta",
        "/tmp/bridge-support.delta.json",
        "--bridge-support-delta-sha256",
        "abc123",
        "--json",
    ])
    .expect("plugins preflight CLI should parse");

    match cli.command {
        Some(Commands::Plugins { json, command }) => {
            assert!(json);
            match command {
                loongclaw_daemon::plugins_cli::PluginsCommands::Preflight(command) => {
                    assert_eq!(
                        command.source.scan.bridge_support_delta.as_deref(),
                        Some("/tmp/bridge-support.delta.json")
                    );
                    assert_eq!(
                        command.source.scan.bridge_support_delta_sha256.as_deref(),
                        Some("abc123")
                    );
                }
                other @ loongclaw_daemon::plugins_cli::PluginsCommands::Init(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Doctor(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Inventory(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeProfiles(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeTemplate(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Actions(_) => {
                    panic!("unexpected plugins subcommand parsed: {other:?}");
                }
            }
        }
        other => panic!("unexpected parse result: {other:?}"),
    }
}

#[test]
fn plugins_init_cli_parses_manifest_scaffold_request() {
    let cli = try_parse_cli([
        "loongclaw",
        "plugins",
        "init",
        "/tmp/tavily-search",
        "--plugin-id",
        "tavily-search",
        "--provider-id",
        "tavily",
        "--connector-name",
        "tavily-http",
        "--bridge-kind",
        "process_stdio",
        "--source-language",
        "python",
        "--summary",
        "Tavily-backed search package",
        "--json",
    ])
    .expect("plugins init CLI should parse");

    match cli.command {
        Some(Commands::Plugins { json, command }) => {
            assert!(json);
            match command {
                loongclaw_daemon::plugins_cli::PluginsCommands::Init(command) => {
                    assert_eq!(command.package_root, "/tmp/tavily-search");
                    assert_eq!(command.plugin_id, "tavily-search");
                    assert_eq!(command.provider_id.as_deref(), Some("tavily"));
                    assert_eq!(command.connector_name.as_deref(), Some("tavily-http"));
                    assert_eq!(
                        command.bridge_kind,
                        loongclaw_daemon::plugins_cli::PluginInitBridgeKindArg::ProcessStdio
                    );
                    assert_eq!(command.source_language.as_deref(), Some("python"));
                    assert_eq!(
                        command.summary.as_deref(),
                        Some("Tavily-backed search package")
                    );
                }
                other @ loongclaw_daemon::plugins_cli::PluginsCommands::Doctor(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeProfiles(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Inventory(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeTemplate(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Preflight(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Actions(_) => {
                    panic!("unexpected plugins subcommand parsed: {other:?}");
                }
            }
        }
        other => panic!("unexpected parse result: {other:?}"),
    }
}

#[test]
fn plugins_init_cli_parses_channel_bridge_scaffold_request() {
    let cli = try_parse_cli([
        "loongclaw",
        "plugins",
        "init",
        "/tmp/weixin-bridge",
        "--plugin-id",
        "weixin-clawbot-bridge",
        "--bridge-kind",
        "http_json",
        "--channel",
        "weixin",
        "--json",
    ])
    .expect("plugins init CLI should parse channel bridge scaffold request");

    match cli.command {
        Some(Commands::Plugins { json, command }) => {
            assert!(json);
            match command {
                loongclaw_daemon::plugins_cli::PluginsCommands::Init(command) => {
                    assert_eq!(command.package_root, "/tmp/weixin-bridge");
                    assert_eq!(command.plugin_id, "weixin-clawbot-bridge");
                    assert_eq!(
                        command.bridge_kind,
                        loongclaw_daemon::plugins_cli::PluginInitBridgeKindArg::HttpJson
                    );
                    assert_eq!(command.channel.as_deref(), Some("weixin"));
                }
                other @ loongclaw_daemon::plugins_cli::PluginsCommands::Doctor(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeProfiles(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Inventory(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeTemplate(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Preflight(_)
                | other @ loongclaw_daemon::plugins_cli::PluginsCommands::Actions(_) => {
                    panic!("unexpected plugins subcommand parsed: {other:?}");
                }
            }
        }
        other => panic!("unexpected parse result: {other:?}"),
    }
}

#[test]
fn plugins_help_mentions_preflight_and_action_plan() {
    let help = render_cli_help(["plugins"]);
    let help_lists_init_subcommand = help.lines().any(|line| {
        let trimmed_line = line.trim();
        let first_token = trimmed_line.split_whitespace().next();
        first_token == Some("init")
    });

    assert!(help.contains("plugin preflight"), "help: {help}");
    assert!(help.contains("doctor"), "help: {help}");
    assert!(help_lists_init_subcommand, "help: {help}");
    assert!(help.contains("inventory"), "help: {help}");
    assert!(help.contains("bridge-profiles"), "help: {help}");
    assert!(help.contains("bridge-template"), "help: {help}");
    assert!(help.contains("actions"), "help: {help}");
    assert!(help.contains("operator action plan"), "help: {help}");
}

#[test]
fn plugins_inventory_help_mentions_scan_and_bridge_flags_without_governance_profile() {
    let help = render_cli_help(["plugins", "inventory"]);

    assert!(help.contains("--root <ROOT>"), "help: {help}");
    assert!(help.contains("--query <QUERY>"), "help: {help}");
    assert!(help.contains("--limit <LIMIT>"), "help: {help}");
    assert!(
        help.contains("--bridge-profile <BRIDGE_PROFILE>"),
        "help: {help}"
    );
    assert!(
        help.contains("--bridge-support-delta <BRIDGE_SUPPORT_DELTA>"),
        "help: {help}"
    );
    assert!(help.contains("--include-examples"), "help: {help}");
    assert!(!help.contains("--profile <PROFILE>"), "help: {help}");
}

#[test]
fn plugins_doctor_help_mentions_sdk_release_profile_and_scan_flags() {
    let help = render_cli_help(["plugins", "doctor"]);

    assert!(help.contains("--root <ROOT>"), "help: {help}");
    assert!(help.contains("--query <QUERY>"), "help: {help}");
    assert!(help.contains("--limit <LIMIT>"), "help: {help}");
    assert!(help.contains("--profile <PROFILE>"), "help: {help}");
    assert!(help.contains("sdk-release"), "help: {help}");
    assert!(
        help.contains("--bridge-profile <BRIDGE_PROFILE>"),
        "help: {help}"
    );
    assert!(
        help.contains("--bridge-support-delta <BRIDGE_SUPPORT_DELTA>"),
        "help: {help}"
    );
}

#[test]
fn plugins_bridge_profiles_help_mentions_profile_filter() {
    let help = render_cli_help(["plugins", "bridge-profiles"]);

    assert!(help.contains("--profile <PROFILE>"), "help: {help}");
    assert!(help.contains("native-balanced"), "help: {help}");
    assert!(help.contains("openclaw-ecosystem-balanced"), "help: {help}");
}

#[test]
fn plugins_init_help_mentions_bridge_contract_flags() {
    let help = render_cli_help(["plugins", "init"]);

    assert!(help.contains("<PACKAGE_ROOT>"), "help: {help}");
    assert!(help.contains("--plugin-id <PLUGIN_ID>"), "help: {help}");
    assert!(help.contains("--bridge-kind <BRIDGE_KIND>"), "help: {help}");
    assert!(help.contains("--channel <CHANNEL>"), "help: {help}");
    assert!(
        help.contains("--source-language <SOURCE_LANGUAGE>"),
        "help: {help}"
    );
    assert!(help.contains("--provider-id <PROVIDER_ID>"), "help: {help}");
    assert!(
        help.contains("--connector-name <CONNECTOR_NAME>"),
        "help: {help}"
    );
}

#[test]
fn plugins_bridge_template_help_mentions_output_and_root() {
    let help = render_cli_help(["plugins", "bridge-template"]);

    assert!(help.contains("--root <ROOT>"), "help: {help}");
    assert!(
        help.contains("--bridge-profile <BRIDGE_PROFILE>"),
        "help: {help}"
    );
    assert!(
        help.contains("--bridge-support-delta <BRIDGE_SUPPORT_DELTA>"),
        "help: {help}"
    );
    assert!(help.contains("--output <OUTPUT>"), "help: {help}");
    assert!(
        help.contains("--delta-output <DELTA_OUTPUT>"),
        "help: {help}"
    );
}

#[test]
fn plugins_actions_help_mentions_root_and_filters() {
    let help = render_cli_help(["plugins", "actions"]);

    assert!(help.contains("--root <ROOT>"), "help: {help}");
    assert!(
        help.contains("--bridge-profile <BRIDGE_PROFILE>"),
        "help: {help}"
    );
    assert!(
        help.contains("--bridge-support-delta <BRIDGE_SUPPORT_DELTA>"),
        "help: {help}"
    );
    assert!(help.contains("--surface <SURFACE>"), "help: {help}");
    assert!(help.contains("--kind <KIND>"), "help: {help}");
    assert!(help.contains("--requires-reload"), "help: {help}");
}
