use super::{
    ChannelCliCommandFuture, ChannelSendCliArgs, ChannelSendCliSpec, require_channel_send_target,
};
#[cfg(test)]
use super::{default_channel_send_target_kind, parse_channel_send_target_kind};

pub const TLON_SEND_CLI_SPEC: ChannelSendCliSpec = ChannelSendCliSpec {
    family: crate::mvp::channel::TLON_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    run: run_tlon_send_cli_impl,
};

pub fn run_tlon_send_cli_impl(args: ChannelSendCliArgs<'_>) -> ChannelCliCommandFuture<'_> {
    Box::pin(async move {
        let _ = args.as_card;
        let target = require_channel_send_target("channels send tlon", args.target)?;
        crate::mvp::channel::run_tlon_send(
            args.config_path,
            args.account,
            target,
            args.target_kind,
            args.text,
        )
        .await
    })
}

#[cfg(test)]
pub fn default_tlon_send_target_kind() -> crate::mvp::channel::ChannelOutboundTargetKind {
    default_channel_send_target_kind(TLON_SEND_CLI_SPEC)
}

#[cfg(test)]
pub fn parse_tlon_send_target_kind(
    raw: &str,
) -> Result<crate::mvp::channel::ChannelOutboundTargetKind, String> {
    parse_channel_send_target_kind(TLON_SEND_CLI_SPEC, raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tlon_send_cli_accepts_conversation_target_kind() {
        let target_kind = parse_tlon_send_target_kind("conversation")
            .expect("tlon should accept conversation targets");

        assert_eq!(
            default_tlon_send_target_kind(),
            crate::mvp::channel::ChannelOutboundTargetKind::Conversation
        );
        assert_eq!(
            target_kind,
            crate::mvp::channel::ChannelOutboundTargetKind::Conversation
        );
    }

    #[test]
    fn tlon_send_cli_rejects_non_conversation_target_kind() {
        let error =
            parse_tlon_send_target_kind("address").expect_err("address targets should be rejected");

        assert_eq!(
            error,
            "tlon --target-kind does not support `address`; use `conversation`"
        );
    }

    #[tokio::test]
    async fn tlon_send_cli_requires_target() {
        let args = ChannelSendCliArgs {
            config_path: None,
            account: None,
            target: None,
            target_kind: crate::mvp::channel::ChannelOutboundTargetKind::Conversation,
            text: "hello",
            as_card: false,
        };

        let error = run_tlon_send_cli_impl(args)
            .await
            .expect_err("missing target should fail");

        assert_eq!(error, "channels send tlon requires --target");
    }
}
