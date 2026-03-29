use clap::Subcommand;
use kernel::{ToolCoreOutcome, ToolCoreRequest};
use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use serde_json::{Map, Value, json};
use std::{collections::BTreeSet, path::Path};

const DEFAULT_SKILLS_SEARCH_LIMIT: usize = 5;
const DEFAULT_SKILLS_RECOMMEND_LIMIT: usize = 3;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum SkillsCommands {
    /// List discovered external skills across managed, user, and project scopes
    List,
    /// Search the external-skills inventory from a task or capability phrase
    Search {
        query: Vec<String>,
        #[arg(long, default_value_t = DEFAULT_SKILLS_SEARCH_LIMIT)]
        limit: usize,
    },
    /// Recommend the best-fit external skills for an operator goal
    Recommend {
        query: Vec<String>,
        #[arg(long, default_value_t = DEFAULT_SKILLS_RECOMMEND_LIMIT)]
        limit: usize,
    },
    /// Inspect one resolved external skill
    #[command(visible_alias = "inspect")]
    Info { skill_id: String },
    /// Download an external skill package and optionally sync it into the managed runtime
    Fetch {
        url: String,
        #[arg(long)]
        save_as: Option<String>,
        #[arg(long)]
        max_bytes: Option<usize>,
        #[arg(long, default_value_t = false)]
        approve_download: bool,
        #[arg(long, default_value_t = false)]
        install: bool,
        #[arg(long)]
        skill_id: Option<String>,
        #[arg(long, default_value_t = false)]
        replace: bool,
    },
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
        | SkillsCommands::Search { .. }
        | SkillsCommands::Recommend { .. }
        | SkillsCommands::Info { .. }
        | SkillsCommands::Fetch { .. }
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

#[derive(Debug, Clone)]
struct SkillFollowUpRecipe {
    label: String,
    command: String,
}

#[derive(Debug, Clone)]
struct SkillFollowUpGuidance {
    next_steps: Vec<String>,
    recipes: Vec<SkillFollowUpRecipe>,
}

fn execute_non_policy_skills_command(
    resolved_path: &Path,
    config: &mvp::config::LoongClawConfig,
    command: SkillsCommands,
) -> CliResult<ToolCoreOutcome> {
    match command {
        SkillsCommands::List => {
            let tool_runtime_config = tool_runtime_config_for_skills_command(config, resolved_path);
            mvp::tools::external_skills_operator_list_with_config(&tool_runtime_config)
        }
        SkillsCommands::Search { query, limit } => {
            let normalized_query = normalize_skills_discovery_query(query, "skills search")?;
            let tool_runtime_config = tool_runtime_config_for_skills_command(config, resolved_path);
            let request = build_skills_discovery_tool_request(
                "external_skills.search",
                normalized_query.as_str(),
                limit,
            );
            mvp::tools::execute_tool_core_with_config(request, &tool_runtime_config)
        }
        SkillsCommands::Recommend { query, limit } => {
            let normalized_query = normalize_skills_discovery_query(query, "skills recommend")?;
            let tool_runtime_config = tool_runtime_config_for_skills_command(config, resolved_path);
            let request = build_skills_discovery_tool_request(
                "external_skills.recommend",
                normalized_query.as_str(),
                limit,
            );
            mvp::tools::execute_tool_core_with_config(request, &tool_runtime_config)
        }
        SkillsCommands::Info { skill_id } => {
            let tool_runtime_config = tool_runtime_config_for_skills_command(config, resolved_path);
            let inspect_outcome = mvp::tools::external_skills_operator_inspect_with_config(
                &skill_id,
                &tool_runtime_config,
            )?;
            decorate_skill_info_outcome(inspect_outcome, resolved_path)
        }
        SkillsCommands::Fetch {
            url,
            save_as,
            max_bytes,
            approve_download,
            install,
            skill_id,
            replace,
        } => execute_fetch_command(
            resolved_path,
            config,
            &url,
            save_as.as_deref(),
            max_bytes,
            approve_download,
            install,
            skill_id.as_deref(),
            replace,
        ),
        SkillsCommands::InstallBundled { skill_id, replace } => {
            let tool_runtime_config = tool_runtime_config_for_skills_command(config, resolved_path);
            let install_outcome =
                execute_install_bundled_skill_command(resolved_path, config, &skill_id, replace)?;
            decorate_skill_install_outcome(install_outcome, resolved_path, &tool_runtime_config)
        }
        SkillsCommands::Install { .. } => {
            let tool_runtime_config = tool_runtime_config_for_skills_command(config, resolved_path);
            let request = build_skills_tool_request(command)?;
            let install_outcome =
                mvp::tools::execute_tool_core_with_config(request, &tool_runtime_config)?;
            decorate_skill_install_outcome(install_outcome, resolved_path, &tool_runtime_config)
        }
        SkillsCommands::Remove { .. } => {
            let tool_runtime_config = tool_runtime_config_for_skills_command(config, resolved_path);
            let request = build_skills_tool_request(command)?;
            mvp::tools::execute_tool_core_with_config(request, &tool_runtime_config)
        }
        SkillsCommands::Policy { .. } | SkillsCommands::EnableBrowserPreview { .. } => {
            Err("unexpected skills CLI command routed through non-policy execution path".to_owned())
        }
    }
}

fn tool_runtime_config_for_skills_command(
    config: &mvp::config::LoongClawConfig,
    resolved_path: &Path,
) -> mvp::tools::runtime_config::ToolRuntimeConfig {
    mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        config,
        Some(resolved_path),
    )
}

fn normalize_skills_discovery_query(
    query_terms: Vec<String>,
    command_name: &str,
) -> CliResult<String> {
    let trimmed_terms = query_terms
        .into_iter()
        .map(|term| term.trim().to_owned())
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    let joined_query = trimmed_terms.join(" ");
    if joined_query.is_empty() {
        return Err(format!("{command_name} requires a non-empty query"));
    }

    Ok(joined_query)
}

fn decorate_skill_info_outcome(
    mut outcome: ToolCoreOutcome,
    resolved_path: &Path,
) -> CliResult<ToolCoreOutcome> {
    let guidance =
        build_skill_follow_up_guidance_from_info_payload(&outcome.payload, resolved_path)?;
    attach_skill_follow_up_guidance(&mut outcome, guidance);
    Ok(outcome)
}

fn decorate_skill_install_outcome(
    mut outcome: ToolCoreOutcome,
    resolved_path: &Path,
    tool_runtime_config: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> CliResult<ToolCoreOutcome> {
    let payload = outcome
        .payload
        .as_object()
        .ok_or_else(|| "skills install payload must be an object".to_owned())?;
    let skill_id = payload
        .get("skill_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "skills install payload missing `skill_id`".to_owned())?;
    let inspect_outcome =
        mvp::tools::external_skills_operator_inspect_with_config(skill_id, tool_runtime_config)?;
    let mut guidance =
        build_skill_follow_up_guidance_from_info_payload(&inspect_outcome.payload, resolved_path)?;
    let config_path = resolved_path.display().to_string();
    let inspect_subcommand = format!("skills info {skill_id}");
    let inspect_command =
        crate::cli_handoff::format_subcommand_with_config(&inspect_subcommand, &config_path);
    let inspect_step = format!("Inspect the installed skill: {inspect_command}");
    guidance.next_steps.insert(0, inspect_step);
    attach_skill_follow_up_guidance(&mut outcome, guidance);
    Ok(outcome)
}

fn attach_skill_follow_up_guidance(outcome: &mut ToolCoreOutcome, guidance: SkillFollowUpGuidance) {
    let mut recipe_payload = Vec::new();
    for recipe in guidance.recipes {
        let recipe_value = json!({
            "label": recipe.label,
            "command": recipe.command,
        });
        recipe_payload.push(recipe_value);
    }

    if let Some(payload) = outcome.payload.as_object_mut() {
        payload.insert("next_steps".to_owned(), json!(guidance.next_steps));
        payload.insert("recipes".to_owned(), Value::Array(recipe_payload));
    }
}

fn build_skill_follow_up_guidance_from_info_payload(
    payload: &Value,
    resolved_path: &Path,
) -> CliResult<SkillFollowUpGuidance> {
    let skill = payload
        .get("skill")
        .and_then(Value::as_object)
        .ok_or_else(|| "skills info payload missing `skill` object".to_owned())?;
    build_skill_follow_up_guidance(skill, resolved_path)
}

fn build_skill_follow_up_guidance(
    skill: &Map<String, Value>,
    resolved_path: &Path,
) -> CliResult<SkillFollowUpGuidance> {
    let skill_id = skill
        .get("skill_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "skill payload missing `skill_id`".to_owned())?;
    let display_name = skill
        .get("display_name")
        .and_then(Value::as_str)
        .unwrap_or(skill_id);
    let skill_md_path = skill
        .get("skill_md_path")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let visibility = skill
        .get("model_visibility")
        .and_then(Value::as_str)
        .unwrap_or("visible");
    let invocation_policy = skill
        .get("invocation_policy")
        .and_then(Value::as_str)
        .or_else(|| {
            skill
                .get("metadata")
                .and_then(Value::as_object)
                .and_then(|metadata| metadata.get("invocation_policy"))
                .and_then(Value::as_str)
        })
        .unwrap_or("model");
    let eligibility = skill
        .get("eligibility")
        .and_then(Value::as_object)
        .ok_or_else(|| "skill payload missing `eligibility` object".to_owned())?;
    let available = eligibility
        .get("available")
        .and_then(Value::as_bool)
        .or_else(|| eligibility.get("eligible").and_then(Value::as_bool))
        .unwrap_or(true);
    let missing_env = string_array_from_value(eligibility.get("missing_env"));
    let missing_bin = string_array_from_value(eligibility.get("missing_bin"));
    let missing_paths = string_array_from_value(eligibility.get("missing_paths"));
    let missing_config = string_array_from_value(eligibility.get("missing_config"));
    let active = skill.get("active").and_then(Value::as_bool).unwrap_or(true);
    let config_path = resolved_path.display().to_string();
    let mut next_steps = Vec::new();
    let mut recipes = Vec::new();

    let quoted_skill_path = crate::cli_handoff::shell_quote_argument(skill_md_path);
    let review_step = format!("Review the skill instructions at {quoted_skill_path}");
    next_steps.push(review_step);

    if !missing_env.is_empty() {
        let env_fragment = missing_env.join(", ");
        let env_step = format!("Set required environment variables: {env_fragment}");
        next_steps.push(env_step);
    }
    if !missing_bin.is_empty() {
        let bin_fragment = missing_bin.join(", ");
        let bin_step = format!("Install required commands on PATH: {bin_fragment}");
        next_steps.push(bin_step);
    }
    if !missing_paths.is_empty() {
        let path_fragment = missing_paths.join(", ");
        let path_step = format!("Create or point required paths: {path_fragment}");
        next_steps.push(path_step);
    }
    if !missing_config.is_empty() {
        let config_fragment = missing_config.join(", ");
        let config_step = format!("Enable required config gates: {config_fragment}");
        next_steps.push(config_step);
    }

    let hidden_from_model = visibility == "hidden";
    let manual_only = invocation_policy == "manual";
    let can_use_in_ask = active && available && !hidden_from_model && !manual_only;
    if can_use_in_ask {
        let ask_message = format!("Use the {skill_id} skill to help with the current task.");
        let ask_command = crate::cli_handoff::format_ask_with_config(&config_path, &ask_message);
        let ask_step = format!("Try the skill in a conversation: {ask_command}");
        next_steps.push(ask_step);
        let recipe = SkillFollowUpRecipe {
            label: format!("Try {display_name}"),
            command: ask_command,
        };
        recipes.push(recipe);
    } else if hidden_from_model {
        next_steps.push(
            "This skill is hidden from model discovery; keep the workflow operator-driven."
                .to_owned(),
        );
    } else if manual_only {
        next_steps.push(
            "This skill is manual-only and cannot be invoked through `external_skills.invoke`."
                .to_owned(),
        );
    } else if !active {
        next_steps.push(
            "This skill is inactive and cannot be used in a conversation until it is reactivated."
                .to_owned(),
        );
    } else if !available {
        next_steps.push(
            "Resolve the missing prerequisites above before trying this skill in a conversation."
                .to_owned(),
        );
    }

    Ok(SkillFollowUpGuidance {
        next_steps,
        recipes,
    })
}

fn string_array_from_value(value: Option<&Value>) -> Vec<String> {
    let Some(values) = value.and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut strings = Vec::new();
    for value in values {
        let Some(text) = value.as_str() else {
            continue;
        };
        strings.push(text.to_owned());
    }

    strings
}

fn build_skills_discovery_tool_request(
    tool_name: &str,
    query: &str,
    limit: usize,
) -> ToolCoreRequest {
    ToolCoreRequest {
        tool_name: tool_name.to_owned(),
        payload: json!({
            "query": query,
            "limit": limit,
        }),
    }
}

fn build_skills_tool_request(command: SkillsCommands) -> CliResult<ToolCoreRequest> {
    match command {
        SkillsCommands::List => Ok(ToolCoreRequest {
            tool_name: "external_skills.list".to_owned(),
            payload: json!({}),
        }),
        SkillsCommands::Search { .. } | SkillsCommands::Recommend { .. } => {
            Err("skills discovery requests are handled directly by the daemon CLI".to_owned())
        }
        SkillsCommands::Info { skill_id } => Ok(ToolCoreRequest {
            tool_name: "external_skills.inspect".to_owned(),
            payload: json!({
                "skill_id": skill_id,
            }),
        }),
        SkillsCommands::Fetch { .. } => {
            Err("skills fetch requests are handled directly by the daemon CLI".to_owned())
        }
        SkillsCommands::Install {
            path,
            skill_id,
            replace,
        } => Ok(ToolCoreRequest {
            tool_name: "external_skills.install".to_owned(),
            payload: build_install_payload(&path, skill_id.as_deref(), replace),
        }),
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

fn execute_fetch_command(
    resolved_path: &Path,
    config: &mvp::config::LoongClawConfig,
    url: &str,
    save_as: Option<&str>,
    max_bytes: Option<usize>,
    approve_download: bool,
    install: bool,
    skill_id: Option<&str>,
    replace: bool,
) -> CliResult<ToolCoreOutcome> {
    if !install {
        if skill_id.is_some() {
            return Err("skills fetch --skill-id requires --install".to_owned());
        }
        if replace {
            return Err("skills fetch --replace requires --install".to_owned());
        }
    }

    let tool_runtime_config = mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        config,
        Some(resolved_path),
    );
    let fetch_request = build_fetch_tool_request(url, save_as, max_bytes, approve_download);
    let fetch_outcome =
        mvp::tools::execute_tool_core_with_config(fetch_request, &tool_runtime_config)?;
    let fetched = fetch_outcome.payload;

    let mut installed = if install {
        let saved_path = fetched
            .get("saved_path")
            .and_then(Value::as_str)
            .ok_or_else(|| "external skills fetch payload missing `saved_path`".to_owned())?;
        let install_request = build_install_request(saved_path, skill_id, replace);
        Some(
            mvp::tools::execute_tool_core_with_config(install_request, &tool_runtime_config)?
                .payload,
        )
    } else {
        None
    };

    if let Some(installed_payload) = installed.as_mut() {
        let mut installed_outcome = ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: installed_payload.clone(),
        };
        let decorated_outcome =
            decorate_skill_install_outcome(installed_outcome, resolved_path, &tool_runtime_config)?;
        installed_outcome = decorated_outcome;
        *installed_payload = installed_outcome.payload;
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "daemon-cli",
            "tool_name": "skills.fetch",
            "sync_applied": install,
            "fetched": fetched,
            "installed": installed,
        }),
    })
}

fn build_fetch_tool_request(
    url: &str,
    save_as: Option<&str>,
    max_bytes: Option<usize>,
    approve_download: bool,
) -> ToolCoreRequest {
    let mut payload = Map::new();
    payload.insert("url".to_owned(), json!(url));
    if let Some(save_as) = save_as {
        payload.insert("save_as".to_owned(), json!(save_as));
    }
    if let Some(max_bytes) = max_bytes {
        payload.insert("max_bytes".to_owned(), json!(max_bytes));
    }
    if approve_download {
        payload.insert("approval_granted".to_owned(), json!(true));
    }
    ToolCoreRequest {
        tool_name: "external_skills.fetch".to_owned(),
        payload: Value::Object(payload),
    }
}

fn build_install_payload(path: &str, skill_id: Option<&str>, replace: bool) -> Value {
    let mut payload = Map::new();
    payload.insert("path".to_owned(), json!(path));
    payload.insert("replace".to_owned(), json!(replace));
    if let Some(skill_id) = skill_id {
        payload.insert("skill_id".to_owned(), json!(skill_id));
    }
    Value::Object(payload)
}

fn build_install_request(path: &str, skill_id: Option<&str>, replace: bool) -> ToolCoreRequest {
    ToolCoreRequest {
        tool_name: "external_skills.install".to_owned(),
        payload: build_install_payload(path, skill_id, replace),
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
    let resolved_config_path = resolved_path.display().to_string();
    let runtime_available =
        crate::browser_preview::inspect_browser_preview_state(config).runtime_available;
    let cli_enabled = config.cli.enabled;
    let recipes = if cli_enabled {
        crate::browser_preview::browser_preview_recipe_commands(&resolved_config_path)
    } else {
        Vec::new()
    };
    let doctor_command =
        crate::cli_handoff::format_subcommand_with_config("doctor", &resolved_config_path);
    let mut next_steps = if runtime_available {
        let mut steps = Vec::new();
        if cli_enabled && let Some(first_recipe) = recipes.first() {
            steps.push(format!(
                "Try browser companion preview: {}",
                first_recipe.command
            ));
        }
        steps.push(format!("Run diagnostics: {doctor_command}"));
        steps
    } else {
        vec![
            crate::browser_preview::browser_preview_install_step(),
            crate::browser_preview::browser_preview_verify_step(),
            format!("Run diagnostics: {doctor_command}"),
        ]
    };
    if !cli_enabled {
        next_steps.push("Re-enable `cli.enabled` before running the preview recipes.".to_owned());
    }

    if let Some(payload) = outcome.payload.as_object_mut() {
        payload.insert(
            "tool_name".to_owned(),
            json!("skills.enable-browser-preview"),
        );
        payload.insert("config_updated".to_owned(), json!(config_updated));
        payload.insert("browser_preview_enabled".to_owned(), json!(true));
        payload.insert(
            "runtime_binary_available".to_owned(),
            json!(runtime_available),
        );
        payload.insert("cli_enabled".to_owned(), json!(cli_enabled));
        payload.insert("next_steps".to_owned(), json!(next_steps));
        payload.insert(
            "recipes".to_owned(),
            json!(
                recipes
                    .into_iter()
                    .map(|recipe| json!({
                        "label": recipe.label,
                        "command": recipe.command,
                    }))
                    .collect::<Vec<_>>()
            ),
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
        "skills.search"
        | "skills.recommend"
        | "external_skills.search"
        | "external_skills.recommend" => {
            lines.push(format!(
                "query={}",
                payload.get("query").and_then(Value::as_str).unwrap_or("-")
            ));
            let inventory_summary = payload
                .get("inventory_summary")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    "skills discovery payload missing `inventory_summary` object".to_owned()
                })?;
            lines.push(format!(
                "visible_skill_count={}",
                inventory_summary
                    .get("visible_skill_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            ));
            lines.push(format!(
                "shadowed_skill_count={}",
                inventory_summary
                    .get("shadowed_skill_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            ));
            lines.push(format!(
                "blocked_skill_count={}",
                inventory_summary
                    .get("blocked_skill_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            ));
            let is_recommendation =
                matches!(tool_name, "skills.recommend" | "external_skills.recommend");
            let results_heading = if is_recommendation {
                "recommended skills:"
            } else {
                "results:"
            };
            render_skill_discovery_results_section(
                &mut lines,
                results_heading,
                payload.get("results"),
                execution.resolved_config_path.as_str(),
                "skills discovery payload missing `results` array",
            )?;
            render_skill_discovery_results_section(
                &mut lines,
                "shadowed matches:",
                payload.get("shadowed_results"),
                execution.resolved_config_path.as_str(),
                "skills discovery payload missing `shadowed_results` array",
            )?;
            render_blocked_skill_discovery_results_section(
                &mut lines,
                "blocked matches:",
                payload.get("blocked_results"),
                "skills discovery payload missing `blocked_results` array",
            )?;
        }
        "skills.fetch" => {
            let fetched = payload
                .get("fetched")
                .and_then(Value::as_object)
                .ok_or_else(|| "skills fetch payload missing `fetched` object".to_owned())?;
            lines.push(format!(
                "saved_path={}",
                fetched
                    .get("saved_path")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            ));
            lines.push(format!(
                "bytes_downloaded={}",
                fetched
                    .get("bytes_downloaded")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            ));
            lines.push(format!(
                "sha256={}",
                fetched.get("sha256").and_then(Value::as_str).unwrap_or("-")
            ));
            lines.push(format!(
                "approval_required={}",
                fetched
                    .get("approval_required")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            ));
            lines.push(format!(
                "approval_granted={}",
                fetched
                    .get("approval_granted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            ));
            let sync_applied = payload
                .get("sync_applied")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            lines.push(format!("sync_applied={sync_applied}"));
            if sync_applied {
                let installed = payload
                    .get("installed")
                    .and_then(Value::as_object)
                    .ok_or_else(|| "skills fetch payload missing `installed` object".to_owned())?;
                lines.push(format!(
                    "installed skill_id={}",
                    installed
                        .get("skill_id")
                        .and_then(Value::as_str)
                        .unwrap_or("-")
                ));
                lines.push(format!(
                    "display_name={}",
                    installed
                        .get("display_name")
                        .and_then(Value::as_str)
                        .unwrap_or("-")
                ));
                lines.push(format!(
                    "install_path={}",
                    installed
                        .get("install_path")
                        .and_then(Value::as_str)
                        .unwrap_or("-")
                ));
                lines.push(format!(
                    "replaced={}",
                    installed
                        .get("replaced")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                ));
                render_optional_string_section(
                    &mut lines,
                    "next steps:",
                    installed.get("next_steps"),
                )?;
                render_optional_recipe_section(&mut lines, "recipes:", installed.get("recipes"))?;
            }
        }
        "external_skills.inspect" => {
            let skill = payload
                .get("skill")
                .and_then(Value::as_object)
                .ok_or_else(|| "skills info payload missing `skill` object".to_owned())?;
            let metadata = skill.get("metadata").and_then(Value::as_object);
            let eligibility = skill.get("eligibility").and_then(Value::as_object);
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
                "model_visibility={}",
                skill
                    .get("model_visibility")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        metadata
                            .and_then(|value| value.get("model_visibility"))
                            .and_then(Value::as_str)
                    })
                    .unwrap_or("visible")
            ));
            lines.push(format!(
                "eligible={}",
                eligibility
                    .and_then(|value| {
                        value
                            .get("available")
                            .and_then(Value::as_bool)
                            .or_else(|| value.get("eligible").and_then(Value::as_bool))
                    })
                    .unwrap_or(true)
            ));
            lines.push(format!(
                "required_env={}",
                render_string_list(
                    skill
                        .get("required_env")
                        .or_else(|| metadata.and_then(|value| value.get("required_env")))
                )
            ));
            lines.push(format!(
                "required_bin={}",
                render_string_list(
                    skill
                        .get("required_bin")
                        .or_else(|| metadata.and_then(|value| value.get("required_bins")))
                )
            ));
            lines.push(format!(
                "required_paths={}",
                render_string_list(
                    skill
                        .get("required_paths")
                        .or_else(|| metadata.and_then(|value| value.get("required_paths")))
                )
            ));
            lines.push(format!(
                "missing_env={}",
                render_string_list(eligibility.and_then(|value| value.get("missing_env")))
            ));
            lines.push(format!(
                "missing_bin={}",
                render_string_list(eligibility.and_then(|value| value.get("missing_bin")))
            ));
            lines.push(format!(
                "missing_paths={}",
                render_string_list(eligibility.and_then(|value| value.get("missing_paths")))
            ));
            lines.push(format!(
                "missing_config={}",
                render_string_list(eligibility.and_then(|value| value.get("missing_config")))
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
            lines.push(format!(
                "invocation_policy={}",
                skill
                    .get("invocation_policy")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        metadata
                            .and_then(|value| value.get("invocation_policy"))
                            .and_then(Value::as_str)
                    })
                    .unwrap_or("model")
            ));
            lines.push("instructions_preview:".to_owned());
            lines.push(
                payload
                    .get("instructions_preview")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_owned(),
            );
            render_string_section(
                &mut lines,
                "eligibility_issues:",
                eligibility.and_then(|value| value.get("issues")),
                "skills info payload missing `skill.eligibility.issues` array",
            )?;
            render_string_section(
                &mut lines,
                "required_env:",
                skill
                    .get("required_env")
                    .or_else(|| metadata.and_then(|value| value.get("required_env"))),
                "skills info payload missing `skill.required_env` array",
            )?;
            render_string_section(
                &mut lines,
                "required_bins:",
                skill
                    .get("required_bin")
                    .or_else(|| metadata.and_then(|value| value.get("required_bins"))),
                "skills info payload missing `skill.required_bin` array",
            )?;
            render_string_section(
                &mut lines,
                "required_paths:",
                skill
                    .get("required_paths")
                    .or_else(|| metadata.and_then(|value| value.get("required_paths"))),
                "skills info payload missing `skill.required_paths` array",
            )?;
            render_string_section(
                &mut lines,
                "required_config:",
                skill
                    .get("required_config")
                    .or_else(|| metadata.and_then(|value| value.get("required_config"))),
                "skills info payload missing `skill.required_config` array",
            )?;
            render_string_section(
                &mut lines,
                "allowed_tools:",
                skill
                    .get("allowed_tools")
                    .or_else(|| metadata.and_then(|value| value.get("allowed_tools"))),
                "skills info payload missing `skill.allowed_tools` array",
            )?;
            render_string_section(
                &mut lines,
                "blocked_tools:",
                skill
                    .get("blocked_tools")
                    .or_else(|| metadata.and_then(|value| value.get("blocked_tools"))),
                "skills info payload missing `skill.blocked_tools` array",
            )?;
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
            render_optional_string_section(&mut lines, "next steps:", payload.get("next_steps"))?;
            render_optional_recipe_section(&mut lines, "recipes:", payload.get("recipes"))?;
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
                lines.push("browser preview enabled via bundled helper skill".to_owned());
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
            render_optional_string_section(&mut lines, "next steps:", payload.get("next_steps"))?;
            render_optional_recipe_section(&mut lines, "recipes:", payload.get("recipes"))?;
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

fn render_string_section(
    lines: &mut Vec<String>,
    heading: &str,
    value: Option<&Value>,
    missing_error: &str,
) -> CliResult<()> {
    let items = value
        .and_then(Value::as_array)
        .ok_or_else(|| missing_error.to_owned())?;
    if items.is_empty() {
        return Ok(());
    }

    lines.push(heading.to_owned());
    for item in items {
        let rendered = item
            .as_str()
            .ok_or_else(|| format!("{missing_error}: entries must be strings"))?;
        lines.push(format!("- {rendered}"));
    }
    Ok(())
}

fn render_optional_string_section(
    lines: &mut Vec<String>,
    heading: &str,
    value: Option<&Value>,
) -> CliResult<()> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Ok(());
    };
    if items.is_empty() {
        return Ok(());
    }

    lines.push(heading.to_owned());
    for item in items {
        let rendered = item
            .as_str()
            .ok_or_else(|| format!("{heading} entries must be strings"))?;
        lines.push(format!("- {rendered}"));
    }
    Ok(())
}

fn render_recipe_section(
    lines: &mut Vec<String>,
    heading: &str,
    value: Option<&Value>,
) -> CliResult<()> {
    let recipes = value
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{heading} payload missing recipe array"))?;
    if recipes.is_empty() {
        return Ok(());
    }

    lines.push(heading.to_owned());
    for recipe in recipes {
        let label = recipe
            .get("label")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{heading} recipe is missing `label`"))?;
        let command = recipe
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{heading} recipe is missing `command`"))?;
        lines.push(format!("- {label}: {command}"));
    }
    Ok(())
}

fn render_optional_recipe_section(
    lines: &mut Vec<String>,
    heading: &str,
    value: Option<&Value>,
) -> CliResult<()> {
    let Some(recipes) = value.and_then(Value::as_array) else {
        return Ok(());
    };
    if recipes.is_empty() {
        return Ok(());
    }

    render_recipe_section(lines, heading, value)
}

fn render_skill_discovery_results_section(
    lines: &mut Vec<String>,
    heading: &str,
    value: Option<&Value>,
    resolved_config_path: &str,
    missing_error: &str,
) -> CliResult<()> {
    let results = value
        .and_then(Value::as_array)
        .ok_or_else(|| missing_error.to_owned())?;
    if results.is_empty() {
        return Ok(());
    }

    lines.push(heading.to_owned());
    for result in results {
        let skill_id = result
            .get("skill_id")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let resolution = result
            .get("resolution")
            .and_then(Value::as_str)
            .unwrap_or("active");
        lines.push(format!(
            "- {} resolution={resolution}",
            render_skill_summary_line(result)
        ));
        render_optional_string_section(lines, "  match reasons:", result.get("match_reasons"))?;
        render_optional_string_section(lines, "  limitations:", result.get("limitations"))?;
        if resolution == "active" {
            let inspect_subcommand = format!("skills info {skill_id}");
            let inspect_command = crate::cli_handoff::format_subcommand_with_config(
                &inspect_subcommand,
                resolved_config_path,
            );
            lines.push(format!("  inspect={inspect_command}"));
        } else {
            let skill_md_path = result
                .get("skill_md_path")
                .and_then(Value::as_str)
                .unwrap_or("-");
            lines.push(format!("  skill_md_path={skill_md_path}"));
        }
    }

    Ok(())
}

fn render_blocked_skill_discovery_results_section(
    lines: &mut Vec<String>,
    heading: &str,
    value: Option<&Value>,
    missing_error: &str,
) -> CliResult<()> {
    let results = value
        .and_then(Value::as_array)
        .ok_or_else(|| missing_error.to_owned())?;
    if results.is_empty() {
        return Ok(());
    }

    lines.push(heading.to_owned());
    for result in results {
        let skill_id = result
            .get("skill_id")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let error = result.get("error").and_then(Value::as_str).unwrap_or("-");
        lines.push(format!("- skill_id={skill_id} blocked_error={error}"));
        render_optional_string_section(lines, "  match reasons:", result.get("match_reasons"))?;
    }

    Ok(())
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
    let model_visibility = skill
        .get("model_visibility")
        .and_then(Value::as_str)
        .or_else(|| {
            skill
                .get("metadata")
                .and_then(|value| value.get("model_visibility"))
                .and_then(Value::as_str)
        })
        .unwrap_or("visible");
    let eligible = skill
        .get("eligibility")
        .and_then(|value| {
            value
                .get("available")
                .and_then(Value::as_bool)
                .or_else(|| value.get("eligible").and_then(Value::as_bool))
        })
        .unwrap_or(true);
    let invocation_policy = skill
        .get("invocation_policy")
        .and_then(Value::as_str)
        .or_else(|| {
            skill
                .get("metadata")
                .and_then(|value| value.get("invocation_policy"))
                .and_then(Value::as_str)
        })
        .unwrap_or("model");
    format!(
        "{skill_id} [{active}] scope={scope} model_visibility={model_visibility} eligible={eligible} invocation_policy={invocation_policy} display_name={display_name} summary={summary}"
    )
}
