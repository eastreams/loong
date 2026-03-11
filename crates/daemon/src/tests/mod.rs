use super::*;
use clap::CommandFactory;

fn approval_test_operation(tool_name: &str, payload: Value) -> OperationSpec {
    OperationSpec::ToolCore {
        tool_name: tool_name.to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload,
        core: None,
    }
}

fn write_temp_risk_profile(path: &Path, body: &str) {
    fs::create_dir_all(
        path.parent()
            .expect("temp risk profile path should have parent directory"),
    )
    .expect("create temp risk profile directory");
    fs::write(path, body).expect("write temp risk profile");
}

fn sign_security_scan_profile_for_test(profile: &SecurityScanProfile) -> (String, String) {
    use ed25519_dalek::{Signer, SigningKey};

    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let signature = signing_key.sign(&security_scan_profile_message(profile));
    let public_key_base64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
    let signature_base64 = BASE64_STANDARD.encode(signature.to_bytes());
    (public_key_base64, signature_base64)
}

mod architecture;
mod import_claw_cli;
mod onboard_cli;
mod programmatic;
mod spec_runtime;
mod spec_runtime_bridge;

#[test]
fn clap_command_name_is_loongclaw() {
    let command = Cli::command();
    assert_eq!(command.get_name(), "loongclaw");
}

#[test]
fn resolve_validate_output_defaults_to_text() {
    let resolved = resolve_validate_output(false, None).expect("resolve default output");
    assert_eq!(resolved, ValidateConfigOutput::Text);
}

#[test]
fn resolve_validate_output_uses_json_flag_legacy_alias() {
    let resolved = resolve_validate_output(true, None).expect("resolve json output");
    assert_eq!(resolved, ValidateConfigOutput::Json);
}

#[test]
fn resolve_validate_output_accepts_explicit_problem_json() {
    let resolved = resolve_validate_output(false, Some(ValidateConfigOutput::ProblemJson))
        .expect("resolve problem-json output");
    assert_eq!(resolved, ValidateConfigOutput::ProblemJson);
}

#[test]
fn resolve_validate_output_rejects_conflicting_json_and_output_flags() {
    let error = resolve_validate_output(true, Some(ValidateConfigOutput::Json))
        .expect_err("conflicting flags should fail");
    assert!(error.contains("conflicts"));
}

#[test]
fn render_channel_snapshots_text_reports_aliases_and_operation_health() {
    let mut config = mvp::config::LoongClawConfig::default();
    config.feishu.enabled = true;
    config.feishu.app_id = Some("cli_a1b2c3".to_owned());
    config.feishu.app_secret = Some("app-secret".to_owned());

    let snapshots = mvp::channel::channel_status_snapshots(&config);
    let rendered = render_channel_snapshots_text("/tmp/loongclaw.toml", &snapshots);

    assert!(rendered.contains("config=/tmp/loongclaw.toml"));
    assert!(rendered.contains("Feishu/Lark [feishu]"));
    assert!(rendered.contains("aliases=lark"));
    assert!(rendered.contains("account=feishu:cli_a1b2c3"));
    assert!(rendered.contains("op send (feishu-send) ready"));
    assert!(rendered.contains("op serve (feishu-serve) misconfigured"));
    assert!(rendered.contains("running=false"));
}

#[test]
fn render_channel_snapshots_text_reports_configured_accounts_for_multi_account_channels() {
    let config: mvp::config::LoongClawConfig = serde_json::from_value(serde_json::json!({
        "telegram": {
            "enabled": true,
            "default_account": "Work Bot",
            "allowed_chat_ids": [1001],
            "accounts": {
                "Work Bot": {
                    "account_id": "Ops-Bot",
                    "bot_token": "123456:token-work",
                    "allowed_chat_ids": [2002]
                },
                "Personal": {
                    "bot_token": "654321:token-personal",
                    "allowed_chat_ids": [3003]
                }
            }
        }
    }))
    .expect("deserialize multi-account config");

    let snapshots = mvp::channel::channel_status_snapshots(&config);
    let rendered = render_channel_snapshots_text("/tmp/loongclaw.toml", &snapshots);

    assert!(rendered.contains("configured_account=work-bot"));
    assert!(rendered.contains("configured_account=personal"));
}

#[test]
fn render_channel_snapshots_text_reports_default_account_marker() {
    let config: mvp::config::LoongClawConfig = serde_json::from_value(serde_json::json!({
        "telegram": {
            "enabled": true,
            "default_account": "Work Bot",
            "allowed_chat_ids": [1001],
            "accounts": {
                "Work Bot": {
                    "account_id": "Ops-Bot",
                    "bot_token": "123456:token-work",
                    "allowed_chat_ids": [2002]
                },
                "Personal": {
                    "bot_token": "654321:token-personal",
                    "allowed_chat_ids": [3003]
                }
            }
        }
    }))
    .expect("deserialize multi-account config");

    let snapshots = mvp::channel::channel_status_snapshots(&config);
    let rendered = render_channel_snapshots_text("/tmp/loongclaw.toml", &snapshots);

    assert!(rendered.contains("configured_account=work-bot"));
    assert!(rendered.contains("default_account=true"));
    assert!(rendered.contains("default_source=explicit_default"));
}
