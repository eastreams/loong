use loongclaw_app as mvp;

use crate::{
    ChannelCliCommandFuture, ChannelServeCliArgs, ChannelServeCliSpec, CliResult,
    with_graceful_shutdown,
};

pub const FEISHU_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_feishu_serve_cli_impl,
};

pub const LINE_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::LINE_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_line_serve_cli_impl,
};

pub const WHATSAPP_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::WHATSAPP_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_whatsapp_serve_cli_impl,
};

pub const WEBHOOK_SERVE_CLI_SPEC: ChannelServeCliSpec = ChannelServeCliSpec {
    family: mvp::channel::WEBHOOK_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_webhook_serve_cli_impl,
};

fn run_callback_serve_cli_impl<'a, F, Fut>(
    args: ChannelServeCliArgs<'a>,
    run_channel: F,
) -> ChannelCliCommandFuture<'a>
where
    F: FnOnce(Option<&'a str>, Option<&'a str>, Option<&'a str>, Option<&'a str>) -> Fut
        + Send
        + 'a,
    Fut: std::future::Future<Output = CliResult<()>> + Send + 'a,
{
    Box::pin(async move {
        if args.once {
            return Err("`--once` is not supported for callback serve commands".to_owned());
        }
        with_graceful_shutdown(run_channel(
            args.config_path,
            args.account,
            args.bind_override,
            args.path_override,
        ))
        .await
    })
}

pub(crate) fn run_feishu_serve_cli_impl(
    args: ChannelServeCliArgs<'_>,
) -> ChannelCliCommandFuture<'_> {
    run_callback_serve_cli_impl(args, mvp::channel::run_feishu_channel)
}

pub(crate) fn run_line_serve_cli_impl(
    args: ChannelServeCliArgs<'_>,
) -> ChannelCliCommandFuture<'_> {
    run_callback_serve_cli_impl(args, mvp::channel::run_line_channel)
}

pub(crate) fn run_whatsapp_serve_cli_impl(
    args: ChannelServeCliArgs<'_>,
) -> ChannelCliCommandFuture<'_> {
    run_callback_serve_cli_impl(args, mvp::channel::run_whatsapp_channel)
}

pub(crate) fn run_webhook_serve_cli_impl(
    args: ChannelServeCliArgs<'_>,
) -> ChannelCliCommandFuture<'_> {
    run_callback_serve_cli_impl(args, mvp::channel::run_webhook_channel)
}
