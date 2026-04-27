use axum::Router;

use crate::CliResult;

use super::ChannelServeStopHandle;

pub(crate) struct ChannelHttpServeSpec<'a> {
    pub(crate) bind_error_context: &'a str,
    pub(crate) serve_error_context: &'a str,
}

pub(crate) async fn serve_channel_http_router(
    bind: &str,
    router: Router,
    stop: ChannelServeStopHandle,
    spec: ChannelHttpServeSpec<'_>,
) -> CliResult<()> {
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|error| format!("bind {} failed: {error}", spec.bind_error_context))?;

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            stop.wait().await;
        })
        .await
        .map_err(|error| format!("{} stopped: {error}", spec.serve_error_context))
}
