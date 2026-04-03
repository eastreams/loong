use crate::mvp;

pub async fn run_tui_cli(config_path: Option<&str>, session: Option<&str>) -> mvp::CliResult<()> {
    run_tui_cli_with_system_message(config_path, session, None).await
}

pub async fn run_existing_config_tui_with_system_message(
    config_path: Option<&str>,
    session: Option<&str>,
    system_message: Option<String>,
) -> mvp::CliResult<()> {
    mvp::chat::run_tui_with_system_message(config_path, session, system_message).await
}

pub async fn run_tui_cli_with_system_message(
    config_path: Option<&str>,
    session: Option<&str>,
    system_message: Option<String>,
) -> mvp::CliResult<()> {
    let resolved_config_path = config_path
        .map(mvp::config::expand_path)
        .unwrap_or_else(mvp::config::default_config_path);
    let config_exists = resolved_config_path.try_exists().map_err(|error| {
        format!(
            "failed to access config path {}: {error}",
            resolved_config_path.display()
        )
    })?;

    if config_exists {
        return run_existing_config_tui_with_system_message(config_path, session, system_message)
            .await;
    }

    let _ = session;

    crate::onboard_cli::run_first_run_fullscreen_onboard(config_path.map(str::to_owned)).await
}
