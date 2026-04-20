use crate::CliResult;

use super::super::types::ChannelCommandFuture;
use super::context::ChannelCommandContext;

#[derive(Debug, Clone, Copy)]
pub(in crate::channel) struct ChannelSendCommandSpec {
    pub(in crate::channel) channel_id: &'static str,
}

pub(in crate::channel) async fn run_channel_send_command<R, F, G>(
    context: ChannelCommandContext<R>,
    spec: ChannelSendCommandSpec,
    send: F,
    render_success: G,
) -> CliResult<()>
where
    F: for<'a> FnOnce(&'a ChannelCommandContext<R>) -> ChannelCommandFuture<'a>,
    G: FnOnce(&ChannelCommandContext<R>) -> String,
{
    context.emit_route_notice(spec.channel_id);
    send(&context).await?;

    #[allow(clippy::print_stdout)]
    {
        println!("{}", render_success(&context));
    }
    Ok(())
}
