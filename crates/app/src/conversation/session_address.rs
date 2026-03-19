use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

use crate::CliResult;
use crate::config::{normalize_dispatch_account_id, normalize_dispatch_channel_id};

const ROUTE_SESSION_SEGMENT_B64_PREFIX: &str = "~b64~";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationSessionAddress {
    pub session_id: String,
    pub channel_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: Option<String>,
    pub thread_id: Option<String>,
}

impl ConversationSessionAddress {
    pub fn from_session_id(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into().trim().to_owned(),
            ..Self::default()
        }
    }

    pub fn with_channel_scope(
        mut self,
        channel_id: impl Into<String>,
        conversation_id: impl Into<String>,
    ) -> Self {
        self.channel_id = normalize_dispatch_channel_id(channel_id.into().trim());
        self.conversation_id = trimmed_non_empty(conversation_id.into());
        self
    }

    pub fn with_account_id(mut self, account_id: impl Into<String>) -> Self {
        let account_id = account_id.into();
        self.account_id = normalize_dispatch_account_id(account_id.as_str());
        self
    }

    pub fn with_thread_id(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = trimmed_non_empty(thread_id.into());
        self
    }

    pub fn canonical_channel_id(&self) -> Option<String> {
        self.channel_id
            .as_deref()
            .and_then(normalize_dispatch_channel_id)
    }

    pub fn structured_channel_path(&self) -> Vec<String> {
        let mut path = Vec::new();
        if let Some(account_id) = self.account_id.as_ref().and_then(trimmed_non_empty) {
            path.push(account_id);
        }
        if let Some(conversation_id) = self.conversation_id.as_ref().and_then(trimmed_non_empty) {
            path.push(conversation_id);
        }
        if let Some(thread_id) = self.thread_id.as_ref().and_then(trimmed_non_empty) {
            path.push(thread_id);
        }
        path
    }

    pub fn structured_route_session_id(&self) -> Option<String> {
        let channel_id = self.canonical_channel_id()?;
        let path = self.structured_channel_path();
        if path.is_empty() {
            Some(channel_id)
        } else {
            let encoded = path
                .iter()
                .map(|segment| encode_route_session_segment(segment))
                .collect::<Vec<_>>();
            Some(format!("{channel_id}:{}", encoded.join(":")))
        }
    }
}

pub fn encode_route_session_segment(value: &str) -> String {
    let trimmed = value.trim();
    if route_session_segment_needs_encoding(trimmed) {
        format!(
            "{ROUTE_SESSION_SEGMENT_B64_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(trimmed.as_bytes())
        )
    } else {
        trimmed.to_owned()
    }
}

pub fn decode_route_session_segment(value: &str) -> CliResult<String> {
    let trimmed = value.trim();
    let Some(encoded) = trimmed.strip_prefix(ROUTE_SESSION_SEGMENT_B64_PREFIX) else {
        return Ok(trimmed.to_owned());
    };
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|error| format!("invalid route session segment `{trimmed}`: {error}"))?;
    String::from_utf8(decoded)
        .map_err(|error| format!("invalid utf-8 in route session segment `{trimmed}`: {error}"))
}

pub fn parse_route_session_id(value: &str) -> CliResult<Option<(String, Vec<String>)>> {
    let trimmed = value.trim();
    let Some((channel_id, remainder)) = trimmed.split_once(':') else {
        return Ok(None);
    };
    let Some(channel_id) = normalize_dispatch_channel_id(channel_id.trim()) else {
        return Ok(None);
    };

    let mut path = Vec::new();
    for segment in remainder.split(':').map(str::trim) {
        if segment.is_empty() {
            continue;
        }
        path.push(decode_route_session_segment(segment)?);
    }

    Ok(Some((channel_id, path)))
}

fn route_session_segment_needs_encoding(value: &str) -> bool {
    value.contains(':') || value.starts_with(ROUTE_SESSION_SEGMENT_B64_PREFIX)
}

fn trimmed_non_empty(value: impl AsRef<str>) -> Option<String> {
    let value = value.as_ref().trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        ConversationSessionAddress, decode_route_session_segment, encode_route_session_segment,
        parse_route_session_id,
    };

    #[test]
    fn structured_route_session_id_normalizes_channel_and_preserves_scope() {
        let address = ConversationSessionAddress::from_session_id("opaque")
            .with_channel_scope(" Feishu ", "oc_123")
            .with_account_id("lark_cli_a1b2c3")
            .with_thread_id("om_thread_1");

        assert_eq!(
            address.structured_route_session_id().as_deref(),
            Some("feishu:lark_cli_a1b2c3:oc_123:om_thread_1")
        );
    }

    #[test]
    fn route_session_segment_round_trips_colon_delimited_ids() {
        let raw = "!ops:example.org";
        let encoded = encode_route_session_segment(raw);

        assert!(encoded.starts_with("~b64~"));
        assert_eq!(
            decode_route_session_segment(encoded.as_str()).expect("decode route session segment"),
            raw
        );
    }

    #[test]
    fn structured_route_session_id_encodes_segments_with_colons() {
        let address = ConversationSessionAddress::from_session_id("opaque")
            .with_channel_scope("matrix", "!ops:example.org")
            .with_account_id("@bot:example.org")
            .with_thread_id("$thread:example.org");

        let route = address
            .structured_route_session_id()
            .expect("matrix route session id");

        assert!(route.starts_with("matrix:bot-example-org:"));
        assert!(route.contains("~b64~"));

        let parsed = parse_route_session_id(route.as_str())
            .expect("parse route session id")
            .expect("decoded route session id");
        assert_eq!(parsed.0, "matrix");
        assert_eq!(
            parsed.1,
            vec![
                "bot-example-org".to_owned(),
                "!ops:example.org".to_owned(),
                "$thread:example.org".to_owned()
            ]
        );
    }

    #[test]
    fn parse_route_session_id_keeps_raw_matrix_ids_opaque() {
        for raw in [
            "!ops:example.org",
            "$thread:example.org",
            "@bot:example.org",
        ] {
            assert_eq!(
                parse_route_session_id(raw).expect("parse route session id"),
                None,
                "raw matrix id should not be treated as a routed session: {raw}"
            );
        }
    }
}
