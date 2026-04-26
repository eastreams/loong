use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use loong_spec::CliResult;
use serde_json::{Value, json};

use crate::mvp;
use crate::weixin_onboarding::onboard_via_qr_registration;

#[derive(Subcommand, Debug)]
pub enum WeixinCommand {
    /// Start the Weixin / iLink QR onboarding flow and save the resulting bridge contract
    Onboard(WeixinOnboardArgs),
}

#[derive(Args, Debug, Clone)]
pub struct WeixinCommonArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub account: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct WeixinOnboardArgs {
    #[command(flatten)]
    pub common: WeixinCommonArgs,
    #[arg(long)]
    pub timeout_s: Option<u64>,
}

pub fn run_weixin_command(
    command: WeixinCommand,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = CliResult<()>> + Send>> {
    Box::pin(async move {
        match command {
            WeixinCommand::Onboard(args) => {
                let payload = execute_weixin_onboard(&args).await?;
                print_weixin_payload(&payload, args.common.json, render_onboard_text)?;
            }
        }
        Ok(())
    })
}

pub async fn execute_weixin_onboard(args: &WeixinOnboardArgs) -> CliResult<Value> {
    ensure_weixin_onboard_config_exists(args.common.config.as_deref())?;

    let result = onboard_via_qr_registration(
        args.common.config.as_deref(),
        args.common.account.as_deref(),
        args.timeout_s,
    )
    .await?;

    let serve_command = if result.configured_account_id == "default" {
        "loong channels serve weixin".to_owned()
    } else {
        format!(
            "loong channels serve weixin --account {}",
            result.configured_account_id
        )
    };
    let mut notes = vec![
        "run `loong doctor` to verify the saved Weixin bridge contract and managed bridge discovery"
            .to_owned(),
        "QR onboarding writes the iLink bridge_url and bot_token directly into loong.toml; the long-lived reply loop still stays in the external bridge or managed plugin"
            .to_owned(),
    ];
    if result.owner_contact_bootstrap_applied {
        if let Some(user_id) = result.user_id.as_deref() {
            notes.push(format!(
                "defaulted `allowed_contact_ids = [\"{user_id}\"]` so the onboarding account can start a direct chat immediately"
            ));
        }
    } else {
        notes.push(
            "keep `allowed_contact_ids` explicit before running the long-lived bridge reply loop in production"
                .to_owned(),
        );
    }
    notes.push(
        "if several compatible managed bridges are installed, pin `weixin.managed_bridge_plugin_id` before production serve"
            .to_owned(),
    );

    Ok(json!({
        "account_id": result.runtime_account_id,
        "configured_account": result.configured_account_label,
        "configured_account_id": result.configured_account_id,
        "config": result.config_path,
        "credential_source": "qr_registration",
        "bridge_url": result.bridge_url,
        "bot_id": result.bot_id,
        "user_id": result.user_id,
        "qr_url": result.qr_url,
        "qr_rendered": result.qr_rendered,
        "owner_contact_bootstrap_applied": result.owner_contact_bootstrap_applied,
        "serve_command": serve_command,
        "status_command": "loong doctor",
        "notes": notes,
    }))
}

#[allow(clippy::print_stdout)]
fn print_weixin_payload(
    payload: &Value,
    as_json: bool,
    render_text: fn(&Value) -> CliResult<String>,
) -> CliResult<()> {
    if as_json {
        let encoded = serde_json::to_string_pretty(payload)
            .map_err(|error| format!("serialize weixin command output failed: {error}"))?;
        println!("{encoded}");
        return Ok(());
    }
    println!("{}", render_text(payload)?);
    Ok(())
}

fn render_onboard_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "weixin onboard".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!("config: {}", required_json_string(payload, "config")?),
        format!(
            "credential_source: {}",
            required_json_string(payload, "credential_source")?
        ),
        format!(
            "bridge_url: {}",
            required_json_string(payload, "bridge_url")?
        ),
    ]);
    if let Some(bot_id) = payload.get("bot_id").and_then(Value::as_str) {
        lines.push(format!("bot_id: {bot_id}"));
    }
    if let Some(user_id) = payload.get("user_id").and_then(Value::as_str) {
        lines.push(format!("user_id: {user_id}"));
    }
    if let Some(qr_url) = payload.get("qr_url").and_then(Value::as_str) {
        lines.push(format!("qr_url: {qr_url}"));
    }
    lines.push(format!(
        "serve_command: {}",
        required_json_string(payload, "serve_command")?
    ));
    lines.push(format!(
        "status_command: {}",
        required_json_string(payload, "status_command")?
    ));
    if let Some(notes) = payload.get("notes").and_then(Value::as_array) {
        for note in notes.iter().filter_map(Value::as_str) {
            lines.push(format!("note: {note}"));
        }
    }
    Ok(lines.join("\n"))
}

fn required_json_string<'a>(payload: &'a Value, key: &str) -> CliResult<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("weixin command output missing `{key}`"))
}

fn active_cli_command_name() -> &'static str {
    mvp::config::active_cli_command_name()
}

fn ensure_weixin_onboard_config_exists(raw: Option<&str>) -> CliResult<PathBuf> {
    let path = raw
        .map(PathBuf::from)
        .unwrap_or_else(mvp::config::default_config_path);
    verify_weixin_onboard_config_exists(&path)
}

fn verify_weixin_onboard_config_exists(path: &Path) -> CliResult<PathBuf> {
    if path.exists() {
        return Ok(path.to_path_buf());
    }

    let cli = active_cli_command_name();
    Err(format!(
        "config file {} not found; run `{cli} onboard` to complete initial configuration before running `{cli} weixin onboard`",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_onboard_text_includes_qr_registration_summary() {
        let payload = json!({
            "account_id": "bot-ops",
            "configured_account": "ops",
            "config": "/tmp/loong.toml",
            "credential_source": "qr_registration",
            "bridge_url": "https://bridge.example.test",
            "bot_id": "bot-ops",
            "user_id": "wxid-ops",
            "qr_url": "https://scan.example/qr",
            "serve_command": "loong channels serve weixin --account ops",
            "status_command": "loong doctor",
            "notes": [
                "run `loong doctor`",
                "defaulted `allowed_contact_ids`"
            ]
        });

        let rendered = render_onboard_text(&payload).expect("render onboard text");

        assert!(rendered.contains("weixin onboard"));
        assert!(rendered.contains("account: bot-ops"));
        assert!(rendered.contains("bridge_url: https://bridge.example.test"));
        assert!(rendered.contains("serve_command: loong channels serve weixin --account ops"));
        assert!(rendered.contains("note: run `loong doctor`"));
    }

    #[test]
    fn verify_weixin_onboard_config_exists_reports_bootstrap_hint() {
        let missing = PathBuf::from("/tmp/loong-weixin-missing.toml");
        let error =
            verify_weixin_onboard_config_exists(&missing).expect_err("missing config should fail");

        assert!(error.contains("loong onboard"));
        assert!(error.contains("loong weixin onboard"));
    }
}
