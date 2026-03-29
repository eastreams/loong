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
                other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeTemplate(_)
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
                        command.source.roots,
                        vec!["/tmp/plugins-a".to_owned(), "/tmp/plugins-b".to_owned()]
                    );
                    assert_eq!(
                        command.source.profile,
                        loongclaw_daemon::plugins_cli::PluginPreflightProfileArg::RuntimeActivation
                    );
                    assert_eq!(
                        command.source.bridge_profile,
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
                other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeProfiles(_)
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
                    assert_eq!(command.source.roots, vec!["/tmp/plugins".to_owned()]);
                    assert_eq!(
                        command.source.bridge_profile,
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
                other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeProfiles(_)
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
                        command.source.bridge_support_delta.as_deref(),
                        Some("/tmp/bridge-support.delta.json")
                    );
                    assert_eq!(
                        command.source.bridge_support_delta_sha256.as_deref(),
                        Some("abc123")
                    );
                }
                other @ loongclaw_daemon::plugins_cli::PluginsCommands::BridgeProfiles(_)
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
fn plugins_help_mentions_preflight_and_action_plan() {
    let help = render_cli_help(["plugins"]);

    assert!(help.contains("plugin preflight"), "help: {help}");
    assert!(help.contains("bridge-profiles"), "help: {help}");
    assert!(help.contains("bridge-template"), "help: {help}");
    assert!(help.contains("actions"), "help: {help}");
    assert!(help.contains("operator action plan"), "help: {help}");
}

#[test]
fn plugins_bridge_profiles_help_mentions_profile_filter() {
    let help = render_cli_help(["plugins", "bridge-profiles"]);

    assert!(help.contains("--profile <PROFILE>"), "help: {help}");
    assert!(help.contains("native-balanced"), "help: {help}");
    assert!(help.contains("openclaw-ecosystem-balanced"), "help: {help}");
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
