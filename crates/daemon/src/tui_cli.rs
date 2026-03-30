use crate::mvp;

pub async fn run_tui_cli(config_path: Option<&str>, session: Option<&str>) -> mvp::CliResult<()> {
    mvp::chat::run_tui(config_path, session).await
}
