use clap::Subcommand;
use kernel::{ToolCoreOutcome, ToolCoreRequest};
use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use serde_json::{Map, Value, json};
use std::{collections::BTreeSet, path::Path};

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum SkillsCommands {
    /// List discovered external skills across managed, user, and project scopes
    List,
    /// Inspect one resolved external skill
    #[command(visible_alias = "inspect")]
    Info { skill_id: String },
    /// Install a managed external skill from a local directory or archive
    Install {
        path: String,
        #[arg(long)]
        skill_id: Option<String>,
        #[arg(long, default_value_t = false)]
        replace: bool,
    },
    /// Install a first-party bundled managed external skill
    InstallBundled {
        skill_id: String,
        #[arg(long, default_value_t = false)]
        replace: bool,
    },
    /// Enable the managed browser preview flow and install its bundled helper skill
    EnableBrowserPreview {
        #[arg(long, default_value_t = false)]
        replace: bool,
    },
    /// Remove an installed managed external skill
    Remove { skill_id: String },
    /// Inspect or update persisted runtime policy for external skills
    Policy {
        #[command(subcommand)]
        command: SkillsPolicyCommands,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum SkillsPolicyCommands {
    /// Show the persisted external-skills runtime policy
    #[command(visible_alias = "show")]
    Get,
    /// Persist one or more external-skills runtime policy fields into config
    Set {
        #[arg(long)]
        enabled: Option<bool>,
        #[arg(long)]
        require_download_approval: Option<bool>,
        #[arg(long)]
        auto_expose_installed: Option<bool>,
        #[arg(long = "allow-domain")]
        allowed_domains: Vec<String>,
        #[arg(long, default_value_t = false)]
        clear_allowed_domains: bool,
        #[arg(long = "block-domain")]
        blocked_domains: Vec<String>,
        #[arg(long, default_value_t = false)]
        clear_blocked_domains: bool,
        #[arg(long, default_value_t = false)]
        approve_policy_update: bool,
    },
    /// Reset persisted external-skills policy fields back to config defaults
    Reset {
        #[arg(long, default_value_t = false)]
        approve_policy_update: bool,
    },
}

#[derive(Debug, Clone)]
pub struct SkillsCommandOptions {
    pub config: Option<String>,
    pub json: bool,
    pub command: SkillsCommands,
}

#[derive(Debug)]
pub struct SkillsCommandExecution {
    pub resolved_config_path: String,
    pub outcome: ToolCoreOutcome,
}

pub fn run_skills_cli(options: SkillsCommandOptions) -> CliResult<()> {
    let as_json = options.json;
    let execution = execute_skills_command(options)?;
    if as_json {
        let pretty = serde_json::to_string_pretty(&skills_cli_json(&execution))
            .map_err(|error| format!("serialize skills CLI output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_skills_cli_text(&execution)?);
    Ok(())
}

pub fn execute_skills_command(options: SkillsCommandOptions) -> CliResult<SkillsCommandExecution> {
    let (resolved_path, mut config) = mvp::config::load(options.config.as_deref())?;
    let outcome = match options.command {
        SkillsCommands::Policy { command } => {
            execute_policy_command(&resolved_path, &mut config, command)?
        }
        SkillsCommands::EnableBrowserPreview { replace } => {
            execute_enable_browser_preview_command(&resolved_path, &mut config, replace)?
        }
        command @ (SkillsCommands::List
        | SkillsCommands::Info { .. }
        | SkillsCommands::Install { .. }
        | SkillsCommands::InstallBundled { .. }
        | SkillsCommands::Remove { .. }) => {
            execute_non_policy_skills_command(&resolved_path, &config, command)?
        }
    };
    Ok(SkillsCommandExecution {
        resolved_config_path: resolved_path.display().to_string(),
        outcome,
    })
}

fn execute_non_policy_skills_command(
    resolved_path: &Path,
    config: &mvp::config::LoongClawConfig,
    command: SkillsCommands,
) -> CliResult<ToolCoreOutcome> {
    match command {
        SkillsCommands::InstallBundled { skill_id, replace } => {
            execute_install_bundled_skill_command(resolved_path, config, &skill_id, replace)
        }
        command @ (SkillsCommands::List
        | SkillsCommands::Info { .. }
        | SkillsCommands::Install { .. }
        | SkillsCommands::Remove { .. }) => {
            let tool_runtime_config =
                mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
                    config,
                    Some(resolved_path),
                );
            let request = build_skills_tool_request(command)?;
            mvp::tools::execute_tool_core_with_config(request, &tool_runtime_config)
        }
        SkillsCommands::Policy { .. } | SkillsCommands::EnableBrowserPreview { .. } => {
            Err("unexpected skills CLI command routed through non-policy execution path".to_owned())
        }
    }
}

fn build_skills_tool_request(command: SkillsCommands) -> CliResult<ToolCoreRequest> {
    match command {
        SkillsCommands::List => Ok(ToolCoreRequest {
            tool_name: "external_skills.list".to_owned(),
            payload: json!({}),
        }),
        SkillsCommands::Info { skill_id } => Ok(ToolCoreRequest {
            tool_name: "external_skills.inspect".to_owned(),
            payload: json!({
                "skill_id": skill_id,
            }),
        }),
        SkillsCommands::Install {
            path,
            skill_id,
            replace,
        } => {
            let mut payload = Map::new();
            payload.insert("path".to_owned(), json!(path));
            payload.insert("replace".to_owned(), json!(replace));
            if let Some(skill_id) = skill_id {
                payload.insert("skill_id".to_owned(), json!(skill_id));
            }
            Ok(ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: Value::Object(payload),
            })
        }
        SkillsCommands::InstallBundled { .. } | SkillsCommands::EnableBrowserPreview { .. } => {
            Err("bundled skills install requests are handled directly by the daemon CLI".to_owned())
        }
        SkillsCommands::Remove { skill_id } => Ok(ToolCoreRequest {
            tool_name: "external_skills.remove".to_owned(),
            payload: json!({
                "skill_id": skill_id,
            }),
        }),
        SkillsCommands::Policy { .. } => {
            Err("skills policy requests are handled directly by the daemon CLI".to_owned())
        }
    }
}

fn execute_policy_command(
    resolved_path: &Path,
    config: &mut mvp::config::LoongClawConfig,
    command: SkillsPolicyCommands,
) -> CliResult<ToolCoreOutcome> {
    match command {
        SkillsPolicyCommands::Get => Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "daemon-cli",
                "tool_name": "skills.policy",
                "action": "get",
                "persisted": true,
                "policy": persistent_policy_payload(config),
            }),
        }),
        SkillsPolicyCommands::Reset {
            approve_policy_update,
        } => {
            require_policy_update_approval(approve_policy_update)?;
            let defaults = mvp::config::ExternalSkillsConfig::default();
            config.external_skills.enabled = defaults.enabled;
            config.external_skills.require_download_approval = defaults.require_download_approval;
            config.external_skills.allowed_domains = defaults.allowed_domains;
            config.external_skills.blocked_domains = defaults.blocked_domains;
            config.external_skills.auto_expose_installed = defaults.auto_expose_installed;
            persist_config_update(resolved_path, config)?;
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "adapter": "daemon-cli",
                    "tool_name": "skills.policy",
                    "action": "reset",
                    "persisted": true,
                    "config_updated": true,
                    "policy": persistent_policy_payload(config),
                }),
            })
        }
        SkillsPolicyCommands::Set {
            enabled,
            require_download_approval,
            auto_expose_installed,
            allowed_domains,
            clear_allowed_domains,
            blocked_domains,
            clear_blocked_domains,
            approve_policy_update,
        } => {
            if clear_allowed_domains && !allowed_domains.is_empty() {
                return Err(
                    "skills policy set cannot combine --allow-domain with --clear-allowed-domains"
                        .to_owned(),
                );
            }
            if clear_blocked_domains && !blocked_domains.is_empty() {
                return Err(
                    "skills policy set cannot combine --block-domain with --clear-blocked-domains"
                        .to_owned(),
                );
            }

            let has_mutation = enabled.is_some()
                || require_download_approval.is_some()
                || auto_expose_installed.is_some()
                || clear_allowed_domains
                || !allowed_domains.is_empty()
                || clear_blocked_domains
                || !blocked_domains.is_empty();
            if !has_mutation {
                return Err("skills policy set requires at least one mutation flag".to_owned());
            }
            require_policy_update_approval(approve_policy_update)?;

            if let Some(enabled) = enabled {
                config.external_skills.enabled = enabled;
            }
            if let Some(require_download_approval) = require_download_approval {
                config.external_skills.require_download_approval = require_download_approval;
            }
            if let Some(auto_expose_installed) = auto_expose_installed {
                config.external_skills.auto_expose_installed = auto_expose_installed;
            }
            if clear_allowed_domains {
                config.external_skills.allowed_domains.clear();
            } else if !allowed_domains.is_empty() {
                config.external_skills.allowed_domains =
                    normalize_domain_inputs("--allow-domain", allowed_domains)?;
            }
            if clear_blocked_domains {
                config.external_skills.blocked_domains.clear();
            } else if !blocked_domains.is_empty() {
                config.external_skills.blocked_domains =
                    normalize_domain_inputs("--block-domain", blocked_domains)?;
            }

            persist_config_update(resolved_path, config)?;

            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "adapter": "daemon-cli",
                    "tool_name": "skills.policy",
                    "action": "set",
                    "persisted": true,
                    "config_updated": true,
                    "policy": persistent_policy_payload(config),
                }),
            })
        }
    }
}

fn require_policy_update_approval(approved: bool) -> CliResult<()> {
    if approved {
        return Ok(());
    }
    Err(
        "skills policy update requires explicit authorization; pass --approve-policy-update"
            .to_owned(),
    )
}

fn persist_config_update(
    resolved_path: &Path,
    config: &mvp::config::LoongClawConfig,
) -> CliResult<()> {
    let path = resolved_path.to_string_lossy();
    mvp::config::write(Some(path.as_ref()), config, true).map(|_| ())
}

fn execute_install_bundled_skill_command(
    resolved_path: &Path,
    config: &mvp::config::LoongClawConfig,
    skill_id: &str,
    replace: bool,
) -> CliResult<ToolCoreOutcome> {
    let tool_runtime_config = mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        config,
        Some(resolved_path),
    );
    let request = ToolCoreRequest {
        tool_name: "external_skills.install".to_owned(),
        payload: json!({
            "bundled_skill_id": skill_id,
            "replace": replace,
        }),
    };
    mvp::tools::execute_tool_core_with_config(request, &tool_runtime_config)
}

fn execute_enable_browser_preview_command(
    resolved_path: &Path,
    config: &mut mvp::config::LoongClawConfig,
    replace: bool,
) -> CliResult<ToolCoreOutcome> {
    let mut updated_config = config.clone();
    let config_updated = crate::browser_preview::ensure_browser_preview_config(&mut updated_config);
    if crate::browser_preview::shell_policy_explicitly_denies_command(
        &updated_config,
        mvp::tools::BROWSER_COMPANION_COMMAND,
    ) {
        return Err(
            "browser preview cannot be enabled while [tools].shell_deny blocks `agent-browser`; remove that entry and retry"
                .to_owned(),
        );
    }

    if config_updated {
        persist_config_update(resolved_path, &updated_config)?;
    }
    let install_result = execute_install_bundled_skill_command(
        resolved_path,
        &updated_config,
        crate::browser_preview::BROWSER_PREVIEW_SKILL_ID,
        replace
            || crate::browser_preview::inspect_browser_preview_state(&updated_config)
                .skill_installed,
    );
    let mut outcome = match install_result {
        Ok(outcome) => outcome,
        Err(error) => {
            if config_updated {
                persist_config_update(resolved_path, config).map_err(|rollback_error| {
                    format!("{error}; browser preview config rollback failed: {rollback_error}")
                })?;
            }
            return Err(error);
        }
    };
    *config = updated_config;

    if let Some(payload) = outcome.payload.as_object_mut() {
        payload.insert(
            "tool_name".to_owned(),
            json!("skills.enable-browser-preview"),
        );
        payload.insert("config_updated".to_owned(), json!(config_updated));
        payload.insert("browser_preview_enabled".to_owned(), json!(true));
        payload.insert(
            "runtime_binary_available".to_owned(),
            json!(crate::browser_preview::inspect_browser_preview_state(config).runtime_available),
        );
    }

    Ok(outcome)
}

fn persistent_policy_payload(config: &mvp::config::LoongClawConfig) -> Value {
    json!({
        "enabled": config.external_skills.enabled,
        "require_download_approval": config.external_skills.require_download_approval,
        "allowed_domains": config.external_skills.normalized_allowed_domains(),
        "blocked_domains": config.external_skills.normalized_blocked_domains(),
        "install_root": config
            .external_skills
            .resolved_install_root()
            .map(|path| path.display().to_string()),
        "auto_expose_installed": config.external_skills.auto_expose_installed,
    })
}

fn normalize_domain_inputs(flag: &str, entries: Vec<String>) -> CliResult<Vec<String>> {
    let mut normalized = BTreeSet::new();
    for entry in entries {
        let value = mvp::tools::normalize_external_skill_domain_rule(entry.as_str())
            .map_err(|error| format!("invalid domain rule for {flag}: {error}"))?;
        normalized.insert(value);
    }
    Ok(normalized.into_iter().collect())
}

pub fn skills_cli_json(execution: &SkillsCommandExecution) -> Value {
    json!({
        "config": execution.resolved_config_path,
        "status": execution.outcome.status,
        "result": execution.outcome.payload,
    })
}

pub fn render_skills_cli_text(execution: &SkillsCommandExecution) -> CliResult<String> {
    let payload = &execution.outcome.payload;
    let tool_name = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("external_skills");
    let mut lines = vec![format!("config={}", execution.resolved_config_path)];

    match tool_name {
        "external_skills.list" => {
            let skills = payload
                .get("skills")
                .and_then(Value::as_array)
                .ok_or_else(|| "skills list payload missing `skills` array".to_owned())?;
            if skills.is_empty() {
                lines.push("skills: (none)".to_owned());
            } else {
                lines.push("skills:".to_owned());
                for skill in skills {
                    lines.push(format!("- {}", render_skill_summary_line(skill)));
                }
            }
            let shadowed = payload
                .get("shadowed_skills")
                .and_then(Value::as_array)
                .ok_or_else(|| "skills list payload missing `shadowed_skills` array".to_owned())?;
            if !shadowed.is_empty() {
                lines.push("shadowed skills:".to_owned());
                for skill in shadowed {
                    lines.push(format!("- {}", render_skill_summary_line(skill)));
                }
            }
        }
        "external_skills.inspect" => {
            let skill = payload
                .get("skill")
                .and_then(Value::as_object)
                .ok_or_else(|| "skills info payload missing `skill` object".to_owned())?;
            lines.push(format!(
                "skill_id={}",
                skill.get("skill_id").and_then(Value::as_str).unwrap_or("-")
            ));
            lines.push(format!(
                "display_name={}",
                skill
                    .get("display_name")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            ));
            lines.push(format!(
                "scope={}",
                skill.get("scope").and_then(Value::as_str).unwrap_or("-")
            ));
            lines.push(format!(
                "active={}",
                skill.get("active").and_then(Value::as_bool).unwrap_or(true)
            ));
            lines.push(format!(
                "source_path={}",
                skill
                    .get("source_path")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            ));
            lines.push(format!(
                "install_path={}",
                skill
                    .get("install_path")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            ));
            lines.push(format!(
                "skill_md_path={}",
                skill
                    .get("skill_md_path")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            ));
            lines.push(format!(
                "sha256={}",
                skill.get("sha256").and_then(Value::as_str).unwrap_or("-")
            ));
            lines.push("instructions_preview:".to_owned());
            lines.push(
                payload
                    .get("instructions_preview")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_owned(),
            );
            let shadowed = payload
                .get("shadowed_skills")
                .and_then(Value::as_array)
                .ok_or_else(|| "skills info payload missing `shadowed_skills` array".to_owned())?;
            if !shadowed.is_empty() {
                lines.push("shadowed skills:".to_owned());
                for shadowed_skill in shadowed {
                    lines.push(format!("- {}", render_skill_summary_line(shadowed_skill)));
                }
            }
        }
        "external_skills.install" | "skills.enable-browser-preview" => {
            lines.push(format!(
                "installed skill_id={}",
                payload
                    .get("skill_id")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            ));
            lines.push(format!(
                "display_name={}",
                payload
                    .get("display_name")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            ));
            lines.push(format!(
                "source_path={}",
                payload
                    .get("source_path")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            ));
            lines.push(format!(
                "install_path={}",
                payload
                    .get("install_path")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            ));
            lines.push(format!(
                "replaced={}",
                payload
                    .get("replaced")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            ));
            if tool_name == "skills.enable-browser-preview" {
                lines.push(format!(
                    "config_updated={}",
                    payload
                        .get("config_updated")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                ));
                lines.push(format!(
                    "runtime_binary_available={}",
                    payload
                        .get("runtime_binary_available")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                ));
            }
        }
        "external_skills.remove" => {
            lines.push(format!(
                "removed skill_id={}",
                payload
                    .get("skill_id")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            ));
        }
        "external_skills.policy" | "skills.policy" => {
            let policy = payload
                .get("policy")
                .and_then(Value::as_object)
                .ok_or_else(|| "skills policy payload missing `policy` object".to_owned())?;
            lines.push(format!(
                "policy_action={}",
                payload
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("get")
            ));
            lines.push(format!(
                "persisted={}",
                payload
                    .get("persisted")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
            ));
            lines.push(format!(
                "config_updated={}",
                payload
                    .get("config_updated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            ));
            lines.push(format!(
                "enabled={}",
                policy
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            ));
            lines.push(format!(
                "require_download_approval={}",
                policy
                    .get("require_download_approval")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
            ));
            lines.push(format!(
                "allowed_domains={}",
                render_string_list(policy.get("allowed_domains"))
            ));
            lines.push(format!(
                "blocked_domains={}",
                render_string_list(policy.get("blocked_domains"))
            ));
            lines.push(format!(
                "install_root={}",
                policy
                    .get("install_root")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            ));
            lines.push(format!(
                "auto_expose_installed={}",
                policy
                    .get("auto_expose_installed")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
            ));
        }
        other => {
            lines.push(format!("tool={other}"));
            lines.push(payload.to_string());
        }
    }

    Ok(lines.join("\n"))
}

fn render_string_list(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_array)
        .map(|items| {
            let rendered = items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",");
            if rendered.is_empty() {
                "-".to_owned()
            } else {
                rendered
            }
        })
        .unwrap_or_else(|| "-".to_owned())
}

fn render_skill_summary_line(skill: &Value) -> String {
    let skill_id = skill
        .get("skill_id")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let active = if skill.get("active").and_then(Value::as_bool).unwrap_or(true) {
        "active"
    } else {
        "inactive"
    };
    let scope = skill.get("scope").and_then(Value::as_str).unwrap_or("-");
    let display_name = skill
        .get("display_name")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let summary = skill.get("summary").and_then(Value::as_str).unwrap_or("-");
    format!("{skill_id} [{active}] scope={scope} display_name={display_name} summary={summary}")
}
