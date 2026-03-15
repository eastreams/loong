use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}

fn write_file(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directory");
    }
    fs::write(path, content).expect("write fixture");
}

fn write_external_skills_config(root: &Path, enabled: bool) -> PathBuf {
    fs::create_dir_all(root).expect("create fixture root");
    let config_path = root.join("loongclaw.toml");
    let mut config = mvp::config::LoongClawConfig::default();
    config.tools.file_root = Some(root.display().to_string());
    config.external_skills.enabled = enabled;
    config.external_skills.install_root = Some(root.join("managed-skills").display().to_string());
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write config fixture");
    config_path
}

#[test]
fn skills_install_cli_parses_global_flags_after_subcommand() {
    let cli = Cli::try_parse_from([
        "loongclaw",
        "skills",
        "install",
        "source/demo-skill",
        "--skill-id",
        "release-skill",
        "--replace",
        "--json",
        "--config",
        "/tmp/loongclaw.toml",
    ])
    .expect("skills install CLI should parse");

    match cli.command {
        Some(Commands::Skills {
            config,
            json,
            command,
        }) => {
            assert_eq!(config.as_deref(), Some("/tmp/loongclaw.toml"));
            assert!(json);
            match command {
                crate::skills_cli::SkillsCommands::Install {
                    path,
                    skill_id,
                    replace,
                } => {
                    assert_eq!(path, "source/demo-skill");
                    assert_eq!(skill_id.as_deref(), Some("release-skill"));
                    assert!(replace);
                }
                other @ crate::skills_cli::SkillsCommands::List
                | other @ crate::skills_cli::SkillsCommands::Info { .. }
                | other @ crate::skills_cli::SkillsCommands::Remove { .. }
                | other @ crate::skills_cli::SkillsCommands::Policy { .. } => {
                    panic!("unexpected skills subcommand parsed: {other:?}")
                }
            }
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn skills_policy_set_cli_parses_domain_and_approval_flags() {
    let cli = Cli::try_parse_from([
        "loongclaw",
        "skills",
        "policy",
        "set",
        "--enabled",
        "true",
        "--require-download-approval",
        "false",
        "--allow-domain",
        "skills.sh",
        "--allow-domain",
        "clawhub.io",
        "--block-domain",
        "*.evil.example",
        "--approve-policy-update",
    ])
    .expect("skills policy set CLI should parse");

    match cli.command {
        Some(Commands::Skills { command, .. }) => match command {
            crate::skills_cli::SkillsCommands::Policy { command } => match command {
                crate::skills_cli::SkillsPolicyCommands::Set {
                    enabled,
                    require_download_approval,
                    allowed_domains,
                    blocked_domains,
                    approve_policy_update,
                    clear_allowed_domains,
                    clear_blocked_domains,
                } => {
                    assert_eq!(enabled, Some(true));
                    assert_eq!(require_download_approval, Some(false));
                    assert_eq!(allowed_domains, vec!["skills.sh", "clawhub.io"]);
                    assert_eq!(blocked_domains, vec!["*.evil.example"]);
                    assert!(approve_policy_update);
                    assert!(!clear_allowed_domains);
                    assert!(!clear_blocked_domains);
                }
                other @ crate::skills_cli::SkillsPolicyCommands::Get
                | other @ crate::skills_cli::SkillsPolicyCommands::Reset { .. } => {
                    panic!("unexpected policy subcommand parsed: {other:?}")
                }
            },
            other @ crate::skills_cli::SkillsCommands::List
            | other @ crate::skills_cli::SkillsCommands::Info { .. }
            | other @ crate::skills_cli::SkillsCommands::Install { .. }
            | other @ crate::skills_cli::SkillsCommands::Remove { .. } => {
                panic!("unexpected skills subcommand parsed: {other:?}")
            }
        },
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn execute_skills_command_installs_lists_inspects_and_removes_skill() {
    let root = unique_temp_dir("loongclaw-skills-cli-install");
    let config_path = write_external_skills_config(&root, true);
    write_file(
        &root,
        "source/demo-skill/SKILL.md",
        "# Demo Skill\n\nUse this skill when release discipline matters.\n",
    );

    let install =
        crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: crate::skills_cli::SkillsCommands::Install {
                path: "source/demo-skill".to_owned(),
                skill_id: None,
                replace: false,
            },
        })
        .expect("skills install should succeed");
    assert!(
        install.resolved_config_path.ends_with("loongclaw.toml"),
        "resolved config path should be returned for CLI rendering"
    );
    assert_eq!(install.outcome.payload["skill_id"], "demo-skill");
    assert_eq!(install.outcome.payload["display_name"], "Demo Skill");
    assert_eq!(install.outcome.payload["replaced"], false);

    let replace_install =
        crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: crate::skills_cli::SkillsCommands::Install {
                path: "source/demo-skill".to_owned(),
                skill_id: None,
                replace: true,
            },
        })
        .expect("skills replace should succeed");
    assert_eq!(replace_install.outcome.payload["skill_id"], "demo-skill");
    assert_eq!(
        replace_install.outcome.payload["display_name"],
        "Demo Skill"
    );
    assert_eq!(replace_install.outcome.payload["replaced"], true);

    let list = crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
        config: Some(config_path.display().to_string()),
        json: false,
        command: crate::skills_cli::SkillsCommands::List,
    })
    .expect("skills list should succeed");
    assert_eq!(list.outcome.payload["skills"][0]["skill_id"], "demo-skill");
    assert_eq!(
        list.outcome.payload["skills"][0]["display_name"],
        "Demo Skill"
    );

    let info = crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
        config: Some(config_path.display().to_string()),
        json: false,
        command: crate::skills_cli::SkillsCommands::Info {
            skill_id: "demo-skill".to_owned(),
        },
    })
    .expect("skills info should succeed");
    assert_eq!(info.outcome.payload["skill"]["skill_id"], "demo-skill");
    assert!(
        info.outcome.payload["instructions_preview"]
            .as_str()
            .expect("instructions preview should be text")
            .contains("release discipline"),
        "inspect path should surface a preview of SKILL.md"
    );

    let remove =
        crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: crate::skills_cli::SkillsCommands::Remove {
                skill_id: "demo-skill".to_owned(),
            },
        })
        .expect("skills remove should succeed");
    assert_eq!(remove.outcome.payload["removed"], true);

    let list_after_remove =
        crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: crate::skills_cli::SkillsCommands::List,
        })
        .expect("skills list after remove should succeed");
    assert_eq!(
        list_after_remove.outcome.payload["skills"],
        serde_json::json!([])
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn execute_skills_command_policy_round_trips_persisted_config() {
    let root = unique_temp_dir("loongclaw-skills-cli-policy");
    let config_path = write_external_skills_config(&root, false);
    let config_string = config_path.display().to_string();
    let install_root = root.join("managed-skills").display().to_string();

    let initial =
        crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
            config: Some(config_string.clone()),
            json: false,
            command: crate::skills_cli::SkillsCommands::Policy {
                command: crate::skills_cli::SkillsPolicyCommands::Get,
            },
        })
        .expect("policy get should succeed");
    assert_eq!(initial.outcome.payload["persisted"], true);
    assert_eq!(initial.outcome.payload["policy"]["enabled"], false);
    assert_eq!(
        initial.outcome.payload["policy"]["require_download_approval"],
        true
    );
    assert_eq!(
        initial.outcome.payload["policy"]["allowed_domains"],
        serde_json::json!([])
    );
    assert_eq!(
        initial.outcome.payload["policy"]["blocked_domains"],
        serde_json::json!([])
    );
    assert_eq!(
        initial.outcome.payload["policy"]["install_root"],
        install_root
    );

    let set = crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
        config: Some(config_string.clone()),
        json: false,
        command: crate::skills_cli::SkillsCommands::Policy {
            command: crate::skills_cli::SkillsPolicyCommands::Set {
                enabled: Some(true),
                require_download_approval: Some(false),
                allowed_domains: vec![
                    " Skills.SH ".to_owned(),
                    "clawhub.io".to_owned(),
                    "skills.sh".to_owned(),
                ],
                clear_allowed_domains: false,
                blocked_domains: vec!["*.EVIL.example".to_owned(), "*.evil.example".to_owned()],
                clear_blocked_domains: false,
                approve_policy_update: true,
            },
        },
    })
    .expect("policy set should succeed");
    assert_eq!(set.outcome.payload["persisted"], true);
    assert_eq!(set.outcome.payload["config_updated"], true);
    assert_eq!(set.outcome.payload["policy"]["enabled"], true);
    assert_eq!(
        set.outcome.payload["policy"]["require_download_approval"],
        false
    );
    assert_eq!(
        set.outcome.payload["policy"]["allowed_domains"],
        serde_json::json!(["clawhub.io", "skills.sh"])
    );
    assert_eq!(
        set.outcome.payload["policy"]["blocked_domains"],
        serde_json::json!(["*.evil.example"])
    );

    let (_, reloaded) =
        mvp::config::load(Some(config_string.as_str())).expect("reload updated config");
    assert!(reloaded.external_skills.enabled);
    assert!(!reloaded.external_skills.require_download_approval);
    assert_eq!(
        reloaded.external_skills.allowed_domains,
        vec!["clawhub.io".to_owned(), "skills.sh".to_owned()]
    );
    assert_eq!(
        reloaded.external_skills.blocked_domains,
        vec!["*.evil.example".to_owned()]
    );
    assert_eq!(
        reloaded.external_skills.install_root.as_deref(),
        Some(install_root.as_str())
    );
    assert!(reloaded.external_skills.auto_expose_installed);

    let reset =
        crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
            config: Some(config_string.clone()),
            json: false,
            command: crate::skills_cli::SkillsCommands::Policy {
                command: crate::skills_cli::SkillsPolicyCommands::Reset {
                    approve_policy_update: true,
                },
            },
        })
        .expect("policy reset should succeed");
    assert_eq!(reset.outcome.payload["persisted"], true);
    assert_eq!(reset.outcome.payload["config_updated"], true);
    assert_eq!(reset.outcome.payload["policy"]["enabled"], false);
    assert_eq!(
        reset.outcome.payload["policy"]["require_download_approval"],
        true
    );
    assert_eq!(
        reset.outcome.payload["policy"]["allowed_domains"],
        serde_json::json!([])
    );
    assert_eq!(
        reset.outcome.payload["policy"]["blocked_domains"],
        serde_json::json!([])
    );

    let final_get =
        crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
            config: Some(config_string),
            json: false,
            command: crate::skills_cli::SkillsCommands::Policy {
                command: crate::skills_cli::SkillsPolicyCommands::Get,
            },
        })
        .expect("policy get after reset should succeed");
    assert_eq!(final_get.outcome.payload["policy"]["enabled"], false);
    assert_eq!(
        final_get.outcome.payload["policy"]["require_download_approval"],
        true
    );
    assert_eq!(
        final_get.outcome.payload["policy"]["allowed_domains"],
        serde_json::json!([])
    );
    assert_eq!(
        final_get.outcome.payload["policy"]["blocked_domains"],
        serde_json::json!([])
    );

    let (_, reloaded_after_reset) = mvp::config::load(Some(config_path.to_string_lossy().as_ref()))
        .expect("reload reset config");
    assert!(!reloaded_after_reset.external_skills.enabled);
    assert!(
        reloaded_after_reset
            .external_skills
            .require_download_approval
    );
    assert!(
        reloaded_after_reset
            .external_skills
            .allowed_domains
            .is_empty()
    );
    assert!(
        reloaded_after_reset
            .external_skills
            .blocked_domains
            .is_empty()
    );
    assert_eq!(
        reloaded_after_reset.external_skills.install_root.as_deref(),
        Some(install_root.as_str())
    );
    assert!(reloaded_after_reset.external_skills.auto_expose_installed);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn execute_skills_command_policy_set_requires_explicit_approval() {
    let root = unique_temp_dir("loongclaw-skills-cli-policy-approval");
    let config_path = write_external_skills_config(&root, false);
    let config_string = config_path.display().to_string();

    let error =
        crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
            config: Some(config_string.as_str().to_owned()),
            json: false,
            command: crate::skills_cli::SkillsCommands::Policy {
                command: crate::skills_cli::SkillsPolicyCommands::Set {
                    enabled: Some(true),
                    require_download_approval: None,
                    allowed_domains: Vec::new(),
                    clear_allowed_domains: false,
                    blocked_domains: Vec::new(),
                    clear_blocked_domains: false,
                    approve_policy_update: false,
                },
            },
        })
        .expect_err("policy set should require explicit approval");
    assert!(
        error.contains("--approve-policy-update"),
        "approval error should direct operators to the explicit authorization flag: {error}"
    );

    let (_, reloaded) =
        mvp::config::load(Some(config_string.as_str())).expect("reload unchanged config");
    assert!(!reloaded.external_skills.enabled);
    assert!(reloaded.external_skills.require_download_approval);
    assert!(reloaded.external_skills.allowed_domains.is_empty());
    assert!(reloaded.external_skills.blocked_domains.is_empty());

    fs::remove_dir_all(&root).ok();
}
