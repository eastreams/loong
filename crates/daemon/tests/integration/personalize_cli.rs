use super::*;

#[test]
fn cli_personalize_help_mentions_operator_preferences() {
    let help = render_cli_help(["personalize"]);

    assert!(
        help.contains("Teach Loong your working style"),
        "personalize help should explain the guided setup purpose: {help}"
    );
    assert!(
        help.contains("advisory preferences"),
        "personalize help should explain that saved guidance stays advisory: {help}"
    );
    assert!(
        help.contains("loong onboard"),
        "personalize help should redirect first-time setup back to onboard: {help}"
    );
    assert!(
        help.contains("update or clear"),
        "personalize help should explain that saved preferences can be updated or cleared: {help}"
    );
}

#[test]
fn personalize_cli_accepts_config_flag() {
    let cli = try_parse_cli(["loong", "personalize", "--config", "/tmp/loong.toml"])
        .expect("`personalize --config` should parse");

    match cli.command {
        Some(Commands::Personalize { config }) => {
            assert_eq!(config.as_deref(), Some("/tmp/loong.toml"));
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}
