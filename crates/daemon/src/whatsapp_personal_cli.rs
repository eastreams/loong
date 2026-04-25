use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, Subcommand};

use crate::CliResult;

#[derive(Subcommand, Debug)]
pub enum WhatsappPersonalCommand {
    /// Operate the bundled WhatsApp Personal bridge helper
    Bridge {
        #[command(subcommand)]
        command: WhatsappPersonalBridgeCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum WhatsappPersonalBridgeCommand {
    /// Start the bundled QR bridge and keep the personal WhatsApp session online
    Run(WhatsappPersonalBridgeRunArgs),
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WhatsappPersonalBridgeRunArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub account: Option<String>,
    #[arg(long, default_value_t = false)]
    pub skip_install: bool,
}

pub async fn run_whatsapp_personal_command(command: WhatsappPersonalCommand) -> CliResult<()> {
    match command {
        WhatsappPersonalCommand::Bridge { command } => match command {
            WhatsappPersonalBridgeCommand::Run(args) => run_whatsapp_personal_bridge_run(args),
        },
    }
}

fn run_whatsapp_personal_bridge_run(args: WhatsappPersonalBridgeRunArgs) -> CliResult<()> {
    let script_path = whatsapp_personal_bridge_script_path()?;
    let mut command = Command::new(script_path);
    if let Some(config) = args.config.as_deref() {
        command.arg("--config").arg(config);
    }
    if let Some(account) = args.account.as_deref() {
        command.arg("--account").arg(account);
    }
    if args.skip_install {
        command.arg("--skip-install");
    }

    let status = command
        .status()
        .map_err(|error| format!("launch whatsapp-personal bridge helper failed: {error}"))?;
    if status.success() {
        return Ok(());
    }

    let code = status
        .code()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "signal".to_owned());
    Err(format!(
        "whatsapp-personal bridge helper exited unsuccessfully ({code})"
    ))
}

fn whatsapp_personal_bridge_script_path() -> CliResult<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Some(repo_root) = manifest_dir.parent().and_then(Path::parent) else {
        return Err("resolve repo root for whatsapp-personal bridge helper failed".to_owned());
    };
    let script_path = repo_root.join("scripts/whatsapp-personal-bridge-run.sh");
    if script_path.is_file() {
        return Ok(script_path);
    }

    Err(format!(
        "whatsapp-personal bridge helper is missing at {}",
        script_path.display()
    ))
}
