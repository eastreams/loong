use super::*;

use bech32::Hrp;
use secp256k1::{Keypair, Secp256k1, SecretKey, XOnlyPublicKey};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NostrAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub relay_urls: Option<Vec<String>>,
    #[serde(default)]
    pub relay_urls_env: Option<String>,
    #[serde(default)]
    pub private_key: Option<SecretRef>,
    #[serde(default)]
    pub private_key_env: Option<String>,
    #[serde(default)]
    pub allowed_pubkeys: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNostrChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub relay_urls: Vec<String>,
    pub relay_urls_env: Option<String>,
    pub private_key: Option<SecretRef>,
    pub private_key_env: Option<String>,
    pub allowed_pubkeys: Vec<String>,
}

impl ResolvedNostrChannelConfig {
    pub fn relay_urls(&self) -> Vec<String> {
        resolve_string_list_with_legacy_env(
            Some(self.relay_urls.as_slice()),
            self.relay_urls_env.as_deref(),
        )
    }

    pub fn private_key(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.private_key.as_ref(), self.private_key_env.as_deref())
    }

    pub fn normalized_private_key_hex(&self) -> CliResult<Option<String>> {
        let private_key = self.private_key();
        let Some(private_key) = private_key else {
            return Ok(None);
        };

        let normalized = parse_nostr_private_key_hex(private_key.as_str())?;
        Ok(Some(normalized))
    }

    pub fn public_key_hex(&self) -> CliResult<Option<String>> {
        let private_key_hex = self.normalized_private_key_hex()?;
        let Some(private_key_hex) = private_key_hex else {
            return Ok(None);
        };

        let public_key_hex = derive_nostr_public_key_hex(private_key_hex.as_str())?;
        Ok(Some(public_key_hex))
    }

    pub fn allowed_pubkeys(&self) -> Vec<String> {
        normalize_inline_string_list(self.allowed_pubkeys.as_slice())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct NostrChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub relay_urls: Vec<String>,
    #[serde(default = "default_nostr_relay_urls_env")]
    pub relay_urls_env: Option<String>,
    #[serde(default)]
    pub private_key: Option<SecretRef>,
    #[serde(default = "default_nostr_private_key_env")]
    pub private_key_env: Option<String>,
    #[serde(default)]
    pub allowed_pubkeys: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, NostrAccountConfig>,
}

impl Default for NostrChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            relay_urls: Vec::new(),
            relay_urls_env: default_nostr_relay_urls_env(),
            private_key: None,
            private_key_env: default_nostr_private_key_env(),
            allowed_pubkeys: Vec::new(),
            accounts: BTreeMap::new(),
        }
    }
}

impl NostrChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "nostr",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_nostr_env_pointer(
            &mut issues,
            "nostr.relay_urls_env",
            self.relay_urls_env.as_deref(),
            "nostr.relay_urls",
        );
        validate_nostr_env_pointer(
            &mut issues,
            "nostr.private_key_env",
            self.private_key_env.as_deref(),
            "nostr.private_key",
        );
        validate_nostr_secret_ref_env_pointer(
            &mut issues,
            "nostr.private_key",
            self.private_key.as_ref(),
        );

        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);

            let relay_urls_field_path = format!("nostr.accounts.{account_id}.relay_urls");
            let relay_urls_env_field_path = format!("{relay_urls_field_path}_env");
            validate_nostr_env_pointer(
                &mut issues,
                relay_urls_env_field_path.as_str(),
                account.relay_urls_env.as_deref(),
                relay_urls_field_path.as_str(),
            );

            let private_key_field_path = format!("nostr.accounts.{account_id}.private_key");
            let private_key_env_field_path = format!("{private_key_field_path}_env");
            validate_nostr_env_pointer(
                &mut issues,
                private_key_env_field_path.as_str(),
                account.private_key_env.as_deref(),
                private_key_field_path.as_str(),
            );
            validate_nostr_secret_ref_env_pointer(
                &mut issues,
                private_key_field_path.as_str(),
                account.private_key.as_ref(),
            );
        }

        issues
    }

    pub fn relay_urls(&self) -> Vec<String> {
        resolve_string_list_with_legacy_env(
            Some(self.relay_urls.as_slice()),
            self.relay_urls_env.as_deref(),
        )
    }

    pub fn private_key(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.private_key.as_ref(), self.private_key_env.as_deref())
    }

    pub fn normalized_private_key_hex(&self) -> CliResult<Option<String>> {
        let private_key = self.private_key();
        let Some(private_key) = private_key else {
            return Ok(None);
        };

        let normalized = parse_nostr_private_key_hex(private_key.as_str())?;
        Ok(Some(normalized))
    }

    pub fn allowed_pubkeys(&self) -> Vec<String> {
        normalize_inline_string_list(self.allowed_pubkeys.as_slice())
    }

    pub fn configured_account_ids(&self) -> Vec<String> {
        let ids = configured_account_ids(self.accounts.keys());
        if ids.is_empty() {
            return vec![self.default_configured_account_id()];
        }
        ids
    }

    pub fn default_configured_account_selection(&self) -> ChannelDefaultAccountSelection {
        resolve_default_configured_account_selection(
            self.accounts.keys(),
            self.default_account.as_deref(),
            self.resolved_account_identity().id.as_str(),
        )
    }

    pub fn default_configured_account_id(&self) -> String {
        self.default_configured_account_selection().id
    }

    pub fn resolved_account_route(
        &self,
        requested_account_id: Option<&str>,
        selected_configured_account_id: &str,
    ) -> ChannelResolvedAccountRoute {
        resolve_channel_account_route(
            self.accounts.keys(),
            self.default_account.as_deref(),
            self.resolved_account_identity().id.as_str(),
            requested_account_id,
            selected_configured_account_id,
        )
    }

    pub fn resolve_account(
        &self,
        requested_account_id: Option<&str>,
    ) -> CliResult<ResolvedNostrChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));
        let has_relay_urls_override = account_override
            .and_then(|account| account.relay_urls.as_ref())
            .is_some();

        let merged = NostrChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            relay_urls: account_override
                .and_then(|account| account.relay_urls.clone())
                .unwrap_or_else(|| self.relay_urls.clone()),
            relay_urls_env: if has_relay_urls_override {
                account_override.and_then(|account| account.relay_urls_env.clone())
            } else {
                account_override
                    .and_then(|account| account.relay_urls_env.clone())
                    .or_else(|| self.relay_urls_env.clone())
            },
            private_key: account_override
                .and_then(|account| account.private_key.clone())
                .or_else(|| self.private_key.clone()),
            private_key_env: account_override
                .and_then(|account| account.private_key_env.clone())
                .or_else(|| self.private_key_env.clone()),
            allowed_pubkeys: account_override
                .and_then(|account| account.allowed_pubkeys.clone())
                .unwrap_or_else(|| self.allowed_pubkeys.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedNostrChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            relay_urls: merged.relay_urls,
            relay_urls_env: merged.relay_urls_env,
            private_key: merged.private_key,
            private_key_env: merged.private_key_env,
            allowed_pubkeys: merged.allowed_pubkeys,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedNostrChannelConfig> {
        resolve_account_for_session_account_id(
            session_account_id,
            || self.resolve_account(session_account_id),
            || self.configured_account_ids(),
            |configured_id| self.resolve_account(Some(configured_id)),
            |resolved| resolved.account.id.as_str(),
        )
    }

    pub fn resolved_account_identity(&self) -> ChannelAccountIdentity {
        if let Some((id, label)) = resolve_configured_account_identity(self.account_id.as_deref()) {
            return ChannelAccountIdentity {
                id,
                label,
                source: ChannelAccountIdentitySource::Configured,
            };
        }

        let private_key_hex = self.normalized_private_key_hex();
        let private_key_hex = match private_key_hex {
            Ok(Some(value)) => value,
            Ok(None) => return default_channel_account_identity(),
            Err(_error) => return default_channel_account_identity(),
        };

        let public_key_hex = derive_nostr_public_key_hex(private_key_hex.as_str());
        let public_key_hex = match public_key_hex {
            Ok(value) => value,
            Err(_error) => return default_channel_account_identity(),
        };
        let short_public_key = public_key_hex.get(..16).unwrap_or(public_key_hex.as_str());
        let account_id = format!("nostr_{short_public_key}");
        let account_label = format!("nostr:{short_public_key}");

        ChannelAccountIdentity {
            id: account_id,
            label: account_label,
            source: ChannelAccountIdentitySource::DerivedCredential,
        }
    }

    fn resolve_configured_account_selection(
        &self,
        requested_account_id: Option<&str>,
    ) -> CliResult<ResolvedConfiguredAccount> {
        resolve_configured_account_selection(
            self.accounts.keys(),
            requested_account_id,
            self.default_account.as_deref(),
            self.resolved_account_identity().id.as_str(),
        )
    }
}

fn default_nostr_relay_urls_env() -> Option<String> {
    Some(NOSTR_RELAY_URLS_ENV.to_owned())
}

fn default_nostr_private_key_env() -> Option<String> {
    Some(NOSTR_PRIVATE_KEY_ENV.to_owned())
}

fn decode_nostr_bech32_bytes(raw: &str, expected_prefix: &str) -> CliResult<[u8; 32]> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("nostr {expected_prefix} key is empty"));
    }

    let expected_hrp = Hrp::parse(expected_prefix)
        .map_err(|error| format!("invalid nostr prefix `{expected_prefix}`: {error}"))?;
    let decoded = bech32::decode(trimmed)
        .map_err(|error| format!("invalid nostr {expected_prefix} key: {error}"))?;
    let decoded_hrp = decoded.0;
    let decoded_bytes = decoded.1;
    if decoded_hrp != expected_hrp {
        return Err(format!(
            "invalid nostr key prefix `{decoded_hrp}`; expected `{expected_prefix}`"
        ));
    }
    if decoded_bytes.len() != 32 {
        return Err(format!(
            "invalid nostr {expected_prefix} key length {}; expected 32 bytes",
            decoded_bytes.len()
        ));
    }

    let byte_array = <[u8; 32]>::try_from(decoded_bytes.as_slice())
        .map_err(|_conversion_error| format!("invalid nostr {expected_prefix} key length"))?;
    Ok(byte_array)
}

fn decode_nostr_hex_bytes(raw: &str, label: &str) -> CliResult<[u8; 32]> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("nostr {label} key is empty"));
    }

    let decoded =
        hex::decode(trimmed).map_err(|error| format!("invalid nostr {label} hex key: {error}"))?;
    if decoded.len() != 32 {
        return Err(format!(
            "invalid nostr {label} hex key length {}; expected 32 bytes",
            decoded.len()
        ));
    }

    let byte_array = <[u8; 32]>::try_from(decoded.as_slice())
        .map_err(|_conversion_error| format!("invalid nostr {label} hex key length"))?;
    Ok(byte_array)
}

fn normalize_nostr_key_hex(bytes: [u8; 32]) -> String {
    hex::encode(bytes)
}

pub(crate) fn parse_nostr_private_key_hex(raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    let private_key_bytes = if trimmed.starts_with("nsec1") {
        decode_nostr_bech32_bytes(trimmed, "nsec")?
    } else {
        decode_nostr_hex_bytes(trimmed, "private")?
    };
    let secret_key = SecretKey::from_byte_array(private_key_bytes)
        .map_err(|error| format!("invalid nostr private key: {error}"))?;
    let normalized_private_key_bytes = secret_key.secret_bytes();
    Ok(normalize_nostr_key_hex(normalized_private_key_bytes))
}

pub(crate) fn parse_nostr_public_key_hex(raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    let public_key_bytes = if trimmed.starts_with("npub1") {
        decode_nostr_bech32_bytes(trimmed, "npub")?
    } else {
        decode_nostr_hex_bytes(trimmed, "public")?
    };
    let public_key = XOnlyPublicKey::from_byte_array(public_key_bytes)
        .map_err(|error| format!("invalid nostr public key: {error}"))?;
    let normalized_public_key_bytes = public_key.serialize();
    Ok(normalize_nostr_key_hex(normalized_public_key_bytes))
}

fn derive_nostr_public_key_hex(private_key_hex: &str) -> CliResult<String> {
    let private_key_bytes = decode_nostr_hex_bytes(private_key_hex, "private")?;
    let secret_key = SecretKey::from_byte_array(private_key_bytes)
        .map_err(|error| format!("invalid nostr private key: {error}"))?;
    let secp = Secp256k1::new();
    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    let public_key = XOnlyPublicKey::from_keypair(&keypair).0;

    Ok(public_key.to_string())
}

fn validate_nostr_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("relay_urls_env") {
        NOSTR_RELAY_URLS_ENV
    } else {
        NOSTR_PRIVATE_KEY_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_nostr_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: NOSTR_PRIVATE_KEY_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    fn deterministic_test_nostr_private_key_bytes() -> [u8; 32] {
        [0x11_u8; 32]
    }

    fn deterministic_test_nostr_private_key_nsec() -> String {
        let hrp = Hrp::parse("nsec").expect("valid nsec hrp");
        let private_key_bytes = deterministic_test_nostr_private_key_bytes();
        bech32::encode::<bech32::Bech32>(hrp, &private_key_bytes)
            .expect("encode deterministic test nsec")
    }

    #[test]
    fn nostr_resolves_relay_urls_and_nsec_private_key_from_env_pointers() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set(
            "TEST_NOSTR_RELAY_URLS",
            "wss://relay-one.example.test,wss://relay-two.example.test",
        );
        let deterministic_nostr_private_key = deterministic_test_nostr_private_key_nsec();
        env.set(
            "TEST_NOSTR_PRIVATE_KEY",
            deterministic_nostr_private_key.as_str(),
        );

        let config_value = json!({
            "enabled": true,
            "account_id": "Nostr Primary",
            "relay_urls_env": "TEST_NOSTR_RELAY_URLS",
            "private_key_env": "TEST_NOSTR_PRIVATE_KEY"
        });
        let config: NostrChannelConfig =
            serde_json::from_value(config_value).expect("deserialize nostr config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default nostr account");
        let relay_urls = resolved.relay_urls();
        let private_key_hex = resolved
            .normalized_private_key_hex()
            .expect("normalize private key");

        assert_eq!(resolved.configured_account_id, "nostr-primary");
        assert_eq!(resolved.account.id, "nostr-primary");
        assert_eq!(resolved.account.label, "Nostr Primary");
        assert_eq!(
            relay_urls,
            vec![
                "wss://relay-one.example.test".to_owned(),
                "wss://relay-two.example.test".to_owned(),
            ]
        );
        let private_key_hex = private_key_hex.expect("nostr nsec should normalize to hex");
        assert_eq!(private_key_hex.len(), 64);
        assert!(
            private_key_hex
                .chars()
                .all(|value| value.is_ascii_hexdigit())
                && private_key_hex == private_key_hex.to_ascii_lowercase(),
            "normalized nostr key should be lowercase hex: {private_key_hex}"
        );
    }

    #[test]
    fn nostr_partial_deserialization_keeps_default_env_pointers() {
        let config: NostrChannelConfig = serde_json::from_value(json!({
            "enabled": true
        }))
        .expect("deserialize nostr config");

        assert_eq!(config.relay_urls_env.as_deref(), Some(NOSTR_RELAY_URLS_ENV));
        assert_eq!(
            config.private_key_env.as_deref(),
            Some(NOSTR_PRIVATE_KEY_ENV)
        );
    }

    #[test]
    fn nostr_default_config_keeps_default_env_pointers_when_table_is_omitted() {
        let config = crate::config::LoongClawConfig::default();

        assert_eq!(
            config.nostr.relay_urls_env.as_deref(),
            Some(NOSTR_RELAY_URLS_ENV)
        );
        assert_eq!(
            config.nostr.private_key_env.as_deref(),
            Some(NOSTR_PRIVATE_KEY_ENV)
        );
    }

    #[test]
    fn nostr_account_omission_inherits_top_level_env_pointer_names() {
        let config: NostrChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "relay_urls_env": "CUSTOM_NOSTR_RELAY_URLS",
            "private_key_env": "CUSTOM_NOSTR_PRIVATE_KEY",
            "accounts": {
                "ops": {
                    "enabled": true
                }
            }
        }))
        .expect("deserialize nostr config");

        let resolved = config
            .resolve_account(Some("ops"))
            .expect("resolve nostr account");

        assert_eq!(
            resolved.relay_urls_env.as_deref(),
            Some("CUSTOM_NOSTR_RELAY_URLS")
        );
        assert_eq!(
            resolved.private_key_env.as_deref(),
            Some("CUSTOM_NOSTR_PRIVATE_KEY")
        );
    }

    #[test]
    fn nostr_account_explicit_empty_relay_urls_do_not_fall_back_to_top_level_env() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("CUSTOM_NOSTR_RELAY_URLS", "wss://relay.example.test");

        let config: NostrChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "relay_urls_env": "CUSTOM_NOSTR_RELAY_URLS",
            "accounts": {
                "ops": {
                    "relay_urls": []
                }
            }
        }))
        .expect("deserialize nostr config");

        let resolved = config
            .resolve_account(Some("ops"))
            .expect("resolve nostr account");
        let relay_urls = resolved.relay_urls();

        assert!(relay_urls.is_empty());
        assert_eq!(resolved.relay_urls_env, None);
    }

    #[test]
    fn nostr_private_key_parser_rejects_zero_scalar() {
        let zero_private_key = "00".repeat(32);
        let error = parse_nostr_private_key_hex(zero_private_key.as_str())
            .expect_err("zero scalar should be rejected");

        assert!(error.contains("invalid nostr private key"));
    }

    #[test]
    fn nostr_public_key_parser_rejects_invalid_curve_point() {
        let invalid_public_key = "ff".repeat(32);
        let error = parse_nostr_public_key_hex(invalid_public_key.as_str())
            .expect_err("invalid curve point should be rejected");

        assert!(error.contains("invalid nostr public key"));
    }
}
