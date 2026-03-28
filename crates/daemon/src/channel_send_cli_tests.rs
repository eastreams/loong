use super::*;

#[test]
fn email_send_cli_accepts_address_target_kind() {
    let target_kind =
        parse_email_send_target_kind("address").expect("address target kind should be accepted");

    assert_eq!(
        default_email_send_target_kind(),
        mvp::channel::ChannelOutboundTargetKind::Address
    );
    assert_eq!(
        target_kind,
        mvp::channel::ChannelOutboundTargetKind::Address
    );
}

#[test]
fn email_send_cli_rejects_non_address_target_kind() {
    let error = parse_email_send_target_kind("conversation")
        .expect_err("conversation target kind should be rejected");

    assert_eq!(
        error,
        "email --target-kind does not support `conversation`; use `address`"
    );
}

#[tokio::test]
async fn email_send_cli_requires_target() {
    let args = ChannelSendCliArgs {
        config_path: None,
        account: None,
        target: None,
        target_kind: mvp::channel::ChannelOutboundTargetKind::Address,
        text: "hello",
        as_card: false,
    };

    let error = run_email_send_cli_impl(args)
        .await
        .expect_err("missing target should fail");

    assert_eq!(error, "email-send requires --target");
}

#[test]
fn irc_send_cli_accepts_conversation_target_kind() {
    let target_kind = parse_irc_send_target_kind("conversation")
        .expect("irc-send should accept conversation targets");

    assert_eq!(
        default_irc_send_target_kind(),
        mvp::channel::ChannelOutboundTargetKind::Conversation
    );
    assert_eq!(
        target_kind,
        mvp::channel::ChannelOutboundTargetKind::Conversation
    );
}

#[test]
fn irc_send_cli_rejects_non_conversation_target_kind() {
    let rendered =
        parse_irc_send_target_kind("endpoint").expect_err("endpoint targets must be rejected");

    assert!(
        rendered.contains("irc --target-kind does not support `endpoint`; use `conversation`"),
        "unexpected target-kind error: {rendered}"
    );
}

#[test]
fn irc_send_cli_requires_target() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = runtime.block_on(run_irc_send_cli_impl(ChannelSendCliArgs {
        config_path: None,
        account: None,
        target: None,
        target_kind: mvp::channel::ChannelOutboundTargetKind::Conversation,
        text: "hello",
        as_card: false,
    }));

    let error = result.expect_err("missing target should fail before runtime execution");
    assert_eq!(error, "irc-send requires --target");
}

#[test]
fn nostr_send_cli_accepts_address_target_kind() {
    let target_kind =
        parse_nostr_send_target_kind("address").expect("nostr-send should accept address targets");

    assert_eq!(
        default_nostr_send_target_kind(),
        mvp::channel::ChannelOutboundTargetKind::Address
    );
    assert_eq!(
        target_kind,
        mvp::channel::ChannelOutboundTargetKind::Address
    );
}

#[test]
fn twitch_send_cli_accepts_conversation_target_kind() {
    let target_kind = parse_twitch_send_target_kind("conversation")
        .expect("conversation target kind should be accepted");

    assert_eq!(
        default_twitch_send_target_kind(),
        mvp::channel::ChannelOutboundTargetKind::Conversation
    );
    assert_eq!(
        target_kind,
        mvp::channel::ChannelOutboundTargetKind::Conversation
    );
}

#[test]
fn nostr_send_cli_rejects_non_address_target_kind() {
    let error = parse_nostr_send_target_kind("conversation")
        .expect_err("conversation targets must be rejected");

    assert_eq!(
        error,
        "nostr --target-kind does not support `conversation`; use `address`"
    );
}

#[test]
fn twitch_send_cli_rejects_non_conversation_target_kind() {
    let error = parse_twitch_send_target_kind("address")
        .expect_err("address target kind should be rejected");

    assert_eq!(
        error,
        "twitch --target-kind does not support `address`; use `conversation`"
    );
}

#[tokio::test]
async fn twitch_send_cli_requires_target() {
    let args = ChannelSendCliArgs {
        config_path: None,
        account: None,
        target: None,
        target_kind: mvp::channel::ChannelOutboundTargetKind::Conversation,
        text: "hello",
        as_card: false,
    };

    let error = run_twitch_send_cli_impl(args)
        .await
        .expect_err("missing target should fail");

    assert_eq!(error, "twitch-send requires --target");
}
