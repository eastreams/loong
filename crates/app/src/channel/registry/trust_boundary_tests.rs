use super::*;

#[test]
fn signal_status_blocks_default_private_service_url_without_override() {
    let mut config = LoongClawConfig::default();
    config.signal.enabled = true;

    let snapshots = channel_status_snapshots(&config);
    let signal = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "signal")
        .expect("signal snapshot");
    let send = signal.operation("send").expect("signal send operation");
    let serve = signal.operation("serve").expect("signal serve operation");

    assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
    assert!(
        send.issues
            .iter()
            .any(|issue| issue.contains("account is missing")),
        "send issues should require a signal account"
    );
    assert!(
        send.issues
            .iter()
            .any(|issue| issue.contains("private or special-use")),
        "default signal service URL should be blocked until the operator widens outbound_http"
    );
    assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
    assert_eq!(
        signal.api_base_url.as_deref(),
        Some("http://127.0.0.1:8080")
    );
    assert!(serve.runtime.is_none());
}

#[test]
fn signal_status_rejects_non_http_service_url() {
    let mut config = LoongClawConfig::default();
    config.signal.enabled = true;
    config.signal.signal_account = Some("+15550001111".to_owned());
    config.signal.service_url = Some("file:///tmp/signal-api".to_owned());

    let snapshots = channel_status_snapshots(&config);
    let signal = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "signal")
        .expect("signal snapshot");
    let send = signal.operation("send").expect("signal send operation");

    assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
    assert!(
        send.issues
            .iter()
            .any(|issue| issue.contains("requires http or https")),
        "send issues should reject non-http signal service urls"
    );
}

#[test]
fn signal_status_allows_private_service_url_when_outbound_http_override_is_enabled() {
    let mut config = LoongClawConfig::default();
    config.signal.enabled = true;
    config.signal.signal_account = Some("+15550001111".to_owned());
    config.outbound_http.allow_private_hosts = true;

    let snapshots = channel_status_snapshots(&config);
    let signal = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "signal")
        .expect("signal snapshot");
    let send = signal.operation("send").expect("signal send operation");

    assert_eq!(send.health, ChannelOperationHealth::Ready);
    assert!(
        send.issues.is_empty(),
        "widened outbound_http policy should allow the default local signal bridge"
    );
}

#[test]
fn google_chat_status_rejects_credential_bearing_webhook_url() {
    let mut config = LoongClawConfig::default();
    config.google_chat.enabled = true;
    config.google_chat.webhook_url = Some(loongclaw_contracts::SecretRef::Inline(
        "https://user:pass@chat.googleapis.com/v1/spaces/AAAA/messages".to_owned(),
    ));

    let snapshots = channel_status_snapshots(&config);
    let google_chat = snapshots
        .iter()
        .find(|snapshot| snapshot.id == "google-chat")
        .expect("google chat snapshot");
    let send = google_chat
        .operation("send")
        .expect("google chat send operation");

    assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
    assert!(
        send.issues
            .iter()
            .any(|issue| issue.contains("must not embed credentials")),
        "send issues should reject credential-bearing google chat webhook urls"
    );
}
