use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::Mailbox,
    transport::smtp::authentication::Credentials,
};

use crate::{
    CliResult,
    config::{EmailSmtpEndpoint, ResolvedEmailChannelConfig, parse_email_smtp_endpoint},
};

use super::ChannelOutboundTargetKind;

pub(super) async fn run_email_send(
    resolved: &ResolvedEmailChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
) -> CliResult<()> {
    ensure_email_target_kind(target_kind)?;

    let recipient = target_id.trim();
    if recipient.is_empty() {
        return Err("email outbound target address is empty".to_owned());
    }

    let recipient_mailbox = parse_email_mailbox("email recipient", recipient)?;
    let sender_address = resolved.from_address().ok_or_else(|| {
        "email from_address missing (set email.from_address or account override)".to_owned()
    })?;
    let sender_mailbox = parse_email_mailbox("email from_address", sender_address.as_str())?;
    let subject = derive_email_subject(text);
    let message = build_email_message(sender_mailbox, recipient_mailbox, subject.as_str(), text)?;
    let transport = build_email_transport(resolved)?;

    transport
        .send(message)
        .await
        .map_err(|error| format!("email send failed: {error}"))?;

    Ok(())
}

fn ensure_email_target_kind(target_kind: ChannelOutboundTargetKind) -> CliResult<()> {
    if target_kind == ChannelOutboundTargetKind::Address {
        return Ok(());
    }

    Err(format!(
        "email send requires address target kind, got {}",
        target_kind.as_str()
    ))
}

fn parse_email_mailbox(label: &str, raw: &str) -> CliResult<Mailbox> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} is empty"));
    }

    trimmed
        .parse::<Mailbox>()
        .map_err(|error| format!("{label} is invalid: {error}"))
}

fn derive_email_subject(text: &str) -> String {
    let first_line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Loong message");
    first_line.to_owned()
}

fn build_email_message(
    sender_mailbox: Mailbox,
    recipient_mailbox: Mailbox,
    subject: &str,
    text: &str,
) -> CliResult<Message> {
    let builder = Message::builder();
    let builder = builder.from(sender_mailbox);
    let builder = builder.to(recipient_mailbox);
    let builder = builder.subject(subject);

    builder
        .body(text.to_owned())
        .map_err(|error| format!("email message build failed: {error}"))
}

fn build_email_transport(
    resolved: &ResolvedEmailChannelConfig,
) -> CliResult<AsyncSmtpTransport<Tokio1Executor>> {
    let smtp_host = resolved.smtp_host().ok_or_else(|| {
        "email smtp_host missing (set email.smtp_host or account override)".to_owned()
    })?;
    let smtp_endpoint = parse_email_smtp_endpoint(smtp_host.as_str())?;
    let smtp_username = resolved
        .smtp_username()
        .ok_or_else(|| "email smtp_username missing (set email.smtp_username or env)".to_owned())?;
    let smtp_password = resolved
        .smtp_password()
        .ok_or_else(|| "email smtp_password missing (set email.smtp_password or env)".to_owned())?;
    let credentials = Credentials::new(smtp_username, smtp_password);

    let transport_builder = match smtp_endpoint {
        EmailSmtpEndpoint::RelayHost(host) => {
            AsyncSmtpTransport::<Tokio1Executor>::relay(host.as_str())
        }
        EmailSmtpEndpoint::ConnectionUrl(url) => {
            AsyncSmtpTransport::<Tokio1Executor>::from_url(url.as_str())
        }
    }
    .map_err(|error| format!("email smtp transport build failed: {error}"))?;

    let transport = transport_builder.credentials(credentials).build();
    Ok(transport)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_email_target_kind_rejects_non_address_targets() {
        let error = ensure_email_target_kind(ChannelOutboundTargetKind::Conversation)
            .expect_err("conversation target kind should be rejected");

        assert!(
            error.contains("email send requires address target kind"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn derive_email_subject_uses_first_non_empty_line() {
        let subject = derive_email_subject("\n\nStatus update\nSecond line");

        assert_eq!(subject, "Status update");
    }

    #[test]
    fn derive_email_subject_falls_back_for_blank_body() {
        let subject = derive_email_subject(" \n\t ");

        assert_eq!(subject, "Loong message");
    }

    #[test]
    fn parse_email_mailbox_rejects_invalid_mailbox() {
        let error = parse_email_mailbox("email recipient", "not-an-email")
            .expect_err("invalid mailbox should fail");

        assert!(
            error.contains("email recipient is invalid"),
            "unexpected error: {error}"
        );
    }
}
