#![allow(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks
)]

use super::*;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::MutexGuard,
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

struct SkillsCliEnvironmentGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(String, Option<OsString>)>,
}

impl SkillsCliEnvironmentGuard {
    fn set(pairs: &[(&str, Option<&str>)]) -> Self {
        let lock = super::lock_daemon_test_environment();
        let mut saved = Vec::new();
        for (key, value) in pairs {
            saved.push(((*key).to_owned(), std::env::var_os(key)));
            match value {
                Some(value) => unsafe {
                    std::env::set_var(key, value);
                },
                None => unsafe {
                    std::env::remove_var(key);
                },
            }
        }
        Self { _lock: lock, saved }
    }
}

impl Drop for SkillsCliEnvironmentGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..).rev() {
            match value {
                Some(value) => unsafe {
                    std::env::set_var(&key, value);
                },
                None => unsafe {
                    std::env::remove_var(&key);
                },
            }
        }
    }
}

struct SkillsCliCurrentDirGuard {
    original: PathBuf,
}

impl SkillsCliCurrentDirGuard {
    fn set(path: &Path) -> Self {
        let original = std::env::current_dir().expect("read current dir");
        std::env::set_current_dir(path).expect("set current dir");
        Self { original }
    }
}

impl Drop for SkillsCliCurrentDirGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.original).expect("restore current dir");
    }
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
    let home = unique_temp_dir("loongclaw-skills-cli-install-home");
    let config_path = write_external_skills_config(&root, true);
    fs::create_dir_all(&home).expect("create home root");
    let _env = SkillsCliEnvironmentGuard::set(&[("HOME", Some(home.to_string_lossy().as_ref()))]);
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
    let listed_demo_skill = list.outcome.payload["skills"]
        .as_array()
        .expect("skills should be an array")
        .iter()
        .find(|skill| skill["skill_id"] == "demo-skill")
        .expect("managed skill should appear in CLI list");
    assert_eq!(listed_demo_skill["display_name"], "Demo Skill");

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
    fs::remove_dir_all(&home).ok();
}

#[test]
fn execute_skills_command_list_reports_scopes_and_shadowed_skills() {
    let root = unique_temp_dir("loongclaw-skills-cli-scopes");
    let home = unique_temp_dir("loongclaw-skills-cli-home");
    let config_path = write_external_skills_config(&root, true);
    fs::create_dir_all(&home).expect("create home root");
    write_file(
        &root,
        "source/demo-skill/SKILL.md",
        "# Managed Demo Skill\n\nManaged CLI install should win precedence.\n",
    );
    write_file(
        &root,
        ".agents/skills/demo-skill/SKILL.md",
        "---\nname: demo-skill\ndescription: Project CLI demo skill.\n---\n\nProject CLI copy should be shadowed.\n",
    );
    let _env = SkillsCliEnvironmentGuard::set(&[("HOME", Some(home.to_string_lossy().as_ref()))]);

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

    let list = crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
        config: Some(config_path.display().to_string()),
        json: false,
        command: crate::skills_cli::SkillsCommands::List,
    })
    .expect("skills list should succeed");
    assert_eq!(list.outcome.payload["skills"][0]["skill_id"], "demo-skill");
    assert_eq!(list.outcome.payload["skills"][0]["scope"], "managed");
    assert_eq!(
        list.outcome.payload["shadowed_skills"][0]["scope"],
        "project"
    );
    let rendered =
        crate::skills_cli::render_skills_cli_text(&list).expect("text rendering should succeed");
    assert!(
        rendered.contains("scope=managed"),
        "CLI text should show resolved scope: {rendered}"
    );
    assert!(
        rendered.contains("shadowed skills:"),
        "CLI text should render shadowed entries for operator debugging: {rendered}"
    );

    fs::remove_dir_all(&root).ok();
    fs::remove_dir_all(&home).ok();
}

#[test]
fn execute_skills_command_list_anchors_project_scope_to_config_directory_when_file_root_is_unset() {
    let root = unique_temp_dir("loongclaw-skills-cli-config-root");
    let outside = unique_temp_dir("loongclaw-skills-cli-outside-root");
    let home = unique_temp_dir("loongclaw-skills-cli-config-home");
    fs::create_dir_all(&root).expect("create project root");
    fs::create_dir_all(&outside).expect("create outside root");
    fs::create_dir_all(&home).expect("create home root");

    let config_path = root.join("loongclaw.toml");
    let mut config = mvp::config::LoongClawConfig::default();
    config.external_skills.enabled = true;
    config.external_skills.install_root = Some(root.join("managed-skills").display().to_string());
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write config fixture");

    write_file(
        &root,
        ".agents/skills/project-skill/SKILL.md",
        "---\nname: project-skill\ndescription: project scoped skill.\n---\n\nproject instructions.\n",
    );
    write_file(
        &outside,
        ".agents/skills/outside-skill/SKILL.md",
        "---\nname: outside-skill\ndescription: outside cwd skill.\n---\n\noutside instructions.\n",
    );

    let _env = SkillsCliEnvironmentGuard::set(&[("HOME", Some(home.to_string_lossy().as_ref()))]);
    let _cwd = SkillsCliCurrentDirGuard::set(&outside);

    let list = crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
        config: Some(config_path.display().to_string()),
        json: false,
        command: crate::skills_cli::SkillsCommands::List,
    })
    .expect("skills list should succeed");
    let skills = list.outcome.payload["skills"]
        .as_array()
        .expect("skills should be an array");
    assert!(
        skills
            .iter()
            .any(|skill| skill["skill_id"] == "project-skill" && skill["scope"] == "project"),
        "project scope should anchor to the config directory, not the caller cwd: {skills:?}"
    );
    assert!(
        skills
            .iter()
            .all(|skill| skill["skill_id"] != "outside-skill"),
        "project discovery should not scan unrelated cwd roots when --config targets a project: {skills:?}"
    );

    fs::remove_dir_all(&root).ok();
    fs::remove_dir_all(&outside).ok();
    fs::remove_dir_all(&home).ok();
}

#[test]
fn execute_skills_command_list_prefers_nearest_project_ancestor_for_duplicate_skill_ids() {
    let root = unique_temp_dir("loongclaw-skills-cli-ancestor-root");
    let home = unique_temp_dir("loongclaw-skills-cli-ancestor-home");
    fs::create_dir_all(&root).expect("create project root");
    fs::create_dir_all(&home).expect("create home root");

    let config_path = root.join("loongclaw.toml");
    let mut config = mvp::config::LoongClawConfig::default();
    config.external_skills.enabled = true;
    config.external_skills.install_root = Some(root.join("managed-skills").display().to_string());
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write config fixture");

    write_file(
        &root,
        ".agents/skills/demo-skill/SKILL.md",
        "---\nname: demo-skill\ndescription: root project skill.\n---\n\nroot instructions.\n",
    );
    write_file(
        &root,
        "workspace/.agents/skills/demo-skill/SKILL.md",
        "---\nname: demo-skill\ndescription: nested workspace skill.\n---\n\nnested instructions.\n",
    );
    fs::create_dir_all(root.join("workspace/subdir")).expect("create nested cwd");

    let _env = SkillsCliEnvironmentGuard::set(&[("HOME", Some(home.to_string_lossy().as_ref()))]);
    let _cwd = SkillsCliCurrentDirGuard::set(&root.join("workspace/subdir"));

    let list = crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
        config: Some(config_path.display().to_string()),
        json: false,
        command: crate::skills_cli::SkillsCommands::List,
    })
    .expect("skills list should succeed");
    assert_eq!(list.outcome.payload["skills"][0]["skill_id"], "demo-skill");
    assert_eq!(list.outcome.payload["skills"][0]["scope"], "project");
    assert!(
        list.outcome.payload["skills"][0]["source_path"]
            .as_str()
            .expect("source path should be text")
            .contains("workspace/.agents/skills/demo-skill"),
        "nearest project ancestor should win within the project scope: {}",
        list.outcome.payload["skills"][0]
    );
    assert!(
        list.outcome.payload["shadowed_skills"]
            .as_array()
            .expect("shadowed skills should be an array")
            .iter()
            .any(|skill| {
                skill["skill_id"] == "demo-skill"
                    && skill["source_path"]
                        .as_str()
                        .is_some_and(|path| path.ends_with(".agents/skills/demo-skill"))
                    && !skill["source_path"]
                        .as_str()
                        .is_some_and(|path| path.contains("workspace/.agents/skills/demo-skill"))
            }),
        "project ancestor duplicates should remain inspectable as shadowed skills"
    );

    fs::remove_dir_all(&root).ok();
    fs::remove_dir_all(&home).ok();
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
    assert!(!reloaded.external_skills.auto_expose_installed);

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
    assert!(!reloaded_after_reset.external_skills.auto_expose_installed);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn execute_skills_command_policy_set_normalizes_domain_rules_for_persistence() {
    let root = unique_temp_dir("loongclaw-skills-cli-policy-domain-rules");
    let config_path = write_external_skills_config(&root, false);
    let config_string = config_path.display().to_string();

    let set = crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
        config: Some(config_string.clone()),
        json: false,
        command: crate::skills_cli::SkillsCommands::Policy {
            command: crate::skills_cli::SkillsPolicyCommands::Set {
                enabled: Some(true),
                require_download_approval: None,
                allowed_domains: vec!["https://Skills.SH/catalog".to_owned()],
                clear_allowed_domains: false,
                blocked_domains: vec!["HTTPS://evil.example/download".to_owned()],
                clear_blocked_domains: false,
                approve_policy_update: true,
            },
        },
    })
    .expect("policy set should normalize domain rules before writing config");

    assert_eq!(
        set.outcome.payload["policy"]["allowed_domains"],
        serde_json::json!(["skills.sh"])
    );
    assert_eq!(
        set.outcome.payload["policy"]["blocked_domains"],
        serde_json::json!(["evil.example"])
    );

    let (_, reloaded) =
        mvp::config::load(Some(config_string.as_str())).expect("reload normalized config");
    assert_eq!(
        reloaded.external_skills.allowed_domains,
        vec!["skills.sh".to_owned()]
    );
    assert_eq!(
        reloaded.external_skills.blocked_domains,
        vec!["evil.example".to_owned()]
    );

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

#[test]
fn execute_skills_command_policy_set_rejects_invalid_domain_rules() {
    let root = unique_temp_dir("loongclaw-skills-cli-policy-invalid-domains");
    let config_path = write_external_skills_config(&root, false);
    let config_string = config_path.display().to_string();

    let error =
        crate::skills_cli::execute_skills_command(crate::skills_cli::SkillsCommandOptions {
            config: Some(config_string.clone()),
            json: false,
            command: crate::skills_cli::SkillsCommands::Policy {
                command: crate::skills_cli::SkillsPolicyCommands::Set {
                    enabled: Some(true),
                    require_download_approval: None,
                    allowed_domains: vec!["not-a-domain".to_owned()],
                    clear_allowed_domains: false,
                    blocked_domains: Vec::new(),
                    clear_blocked_domains: false,
                    approve_policy_update: true,
                },
            },
        })
        .expect_err("policy set should reject invalid domain rules");
    assert!(
        error.contains("invalid domain rule for --allow-domain"),
        "invalid domain error should point operators at the malformed rule: {error}"
    );

    let (_, reloaded) =
        mvp::config::load(Some(config_string.as_str())).expect("reload unchanged config");
    assert!(!reloaded.external_skills.enabled);
    assert!(reloaded.external_skills.allowed_domains.is_empty());
    assert!(reloaded.external_skills.blocked_domains.is_empty());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn render_skills_cli_text_surfaces_operator_install_summary() {
    let rendered =
        crate::skills_cli::render_skills_cli_text(&crate::skills_cli::SkillsCommandExecution {
            resolved_config_path: "/tmp/loongclaw.toml".to_owned(),
            outcome: kernel::ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: serde_json::json!({
                    "tool_name": "external_skills.install",
                    "skill_id": "demo-skill",
                    "display_name": "Demo Skill",
                    "source_path": "/tmp/source/demo-skill",
                    "install_path": "/tmp/managed/demo-skill",
                    "replaced": true
                }),
            },
        })
        .expect("install payload should render");

    assert!(rendered.contains("config=/tmp/loongclaw.toml"));
    assert!(rendered.contains("installed skill_id=demo-skill"));
    assert!(rendered.contains("display_name=Demo Skill"));
    assert!(rendered.contains("replaced=true"));
}

#[test]
fn skills_cli_json_wraps_config_status_and_result_payload() {
    let rendered = crate::skills_cli::skills_cli_json(&crate::skills_cli::SkillsCommandExecution {
        resolved_config_path: "/tmp/loongclaw.toml".to_owned(),
        outcome: kernel::ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: serde_json::json!({
                "tool_name": "skills.policy",
                "action": "get",
                "policy": {
                    "enabled": true
                }
            }),
        },
    });

    assert_eq!(rendered["config"], "/tmp/loongclaw.toml");
    assert_eq!(rendered["status"], "ok");
    assert_eq!(rendered["result"]["tool_name"], "skills.policy");
    assert_eq!(rendered["result"]["policy"]["enabled"], true);
}
