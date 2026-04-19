use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use loong_app as mvp;
use loong_spec::CliResult;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum MemoryCommands {
    Current,
    Switch {
        topic: Option<String>,
        #[arg(long, default_value_t = false)]
        shared: bool,
    },
    List,
}

#[derive(Debug, Clone)]
pub struct MemoryCommandOptions {
    pub config: Option<String>,
    pub json: bool,
    pub command: MemoryCommands,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryCommandExecution {
    pub action: String,
    pub resolved_config_path: String,
    pub active_topic: Option<String>,
    pub active_scope: String,
    pub sqlite_path: String,
    pub memory_workspace_root: Option<String>,
    pub discovered_topics: Vec<String>,
}

pub fn run_memory_cli(options: MemoryCommandOptions) -> CliResult<()> {
    let json = options.json;
    let execution = execute_memory_command(options)?;

    if json {
        let rendered = serde_json::to_string_pretty(&execution)
            .map_err(|error| format!("serialize memory cli output failed: {error}"))?;
        println!("{rendered}");
        return Ok(());
    }

    let rendered = render_memory_command_text(&execution);
    println!("{rendered}");
    Ok(())
}

pub(crate) fn execute_memory_command(
    options: MemoryCommandOptions,
) -> CliResult<MemoryCommandExecution> {
    match options.command {
        MemoryCommands::Current => {
            let snapshot = load_memory_cli_snapshot(options.config.as_deref())?;
            Ok(MemoryCommandExecution {
                action: "current".to_owned(),
                resolved_config_path: snapshot.resolved_config_path,
                active_topic: snapshot.active_topic,
                active_scope: snapshot.active_scope,
                sqlite_path: snapshot.sqlite_path,
                memory_workspace_root: snapshot.memory_workspace_root,
                discovered_topics: snapshot.discovered_topics,
            })
        }
        MemoryCommands::List => {
            let snapshot = load_memory_cli_snapshot(options.config.as_deref())?;
            Ok(MemoryCommandExecution {
                action: "list".to_owned(),
                resolved_config_path: snapshot.resolved_config_path,
                active_topic: snapshot.active_topic,
                active_scope: snapshot.active_scope,
                sqlite_path: snapshot.sqlite_path,
                memory_workspace_root: snapshot.memory_workspace_root,
                discovered_topics: snapshot.discovered_topics,
            })
        }
        MemoryCommands::Switch { topic, shared } => {
            let (resolved_path, mut config) = mvp::config::load(options.config.as_deref())?;
            let next_topic = resolve_switch_topic(topic.as_deref(), shared)?;
            config.memory.agent_id = next_topic;
            let resolved_path_string = resolved_path.display().to_string();
            mvp::config::write(Some(resolved_path_string.as_str()), &config, true)?;
            let snapshot = load_memory_cli_snapshot(Some(resolved_path_string.as_str()))?;
            create_scoped_memory_roots(&snapshot)?;
            Ok(MemoryCommandExecution {
                action: "switch".to_owned(),
                resolved_config_path: snapshot.resolved_config_path,
                active_topic: snapshot.active_topic,
                active_scope: snapshot.active_scope,
                sqlite_path: snapshot.sqlite_path,
                memory_workspace_root: snapshot.memory_workspace_root,
                discovered_topics: snapshot.discovered_topics,
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoryCliSnapshot {
    resolved_config_path: String,
    active_topic: Option<String>,
    active_scope: String,
    sqlite_path: String,
    memory_workspace_root: Option<String>,
    discovered_topics: Vec<String>,
}

fn resolve_switch_topic(topic: Option<&str>, shared: bool) -> CliResult<Option<String>> {
    if shared {
        if topic.is_some() {
            return Err("memory switch accepts either <topic> or --shared, not both".to_owned());
        }
        return Ok(None);
    }

    let Some(topic) = topic.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err("memory switch requires a topic or --shared".to_owned());
    };

    Ok(Some(topic.to_owned()))
}

fn load_memory_cli_snapshot(config_path: Option<&str>) -> CliResult<MemoryCliSnapshot> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let tool_runtime_config = mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
        &config,
        Some(&resolved_path),
    );
    let active_topic = memory_config.agent_id.clone();
    let active_scope = active_topic.clone().unwrap_or_else(|| "shared".to_owned());
    let sqlite_path = memory_config.resolved_sqlite_path().display().to_string();
    let memory_workspace_root = tool_runtime_config
        .effective_memory_workspace_root()
        .map(|path| path.display().to_string());
    let discovered_topics =
        collect_discovered_topics(&config, &resolved_path, active_topic.as_deref());

    Ok(MemoryCliSnapshot {
        resolved_config_path: resolved_path.display().to_string(),
        active_topic,
        active_scope,
        sqlite_path,
        memory_workspace_root,
        discovered_topics,
    })
}

fn collect_discovered_topics(
    config: &mvp::config::LoongConfig,
    resolved_path: &Path,
    active_topic: Option<&str>,
) -> Vec<String> {
    let mut topics = BTreeSet::new();
    topics.insert("shared".to_owned());
    if let Some(active_topic) = active_topic {
        topics.insert(active_topic.to_owned());
    }

    let base_sqlite_path = std::env::var("LOONG_SQLITE_PATH")
        .ok()
        .map(|value| mvp::config::expand_path(&value))
        .unwrap_or_else(|| mvp::config::expand_path(&config.memory.sqlite_path));
    let agents_dir = base_sqlite_path
        .parent()
        .map(|parent| parent.join("agents"))
        .unwrap_or_else(|| PathBuf::from("agents"));
    collect_topic_names_from_directory(agents_dir.as_path(), &mut topics);

    let tool_runtime_config = mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
        config,
        Some(resolved_path),
    );
    if let Some(base_workspace_root) = tool_runtime_config.effective_workspace_root() {
        let workspace_agents_dir = base_workspace_root.join(".loong").join("agents");
        collect_topic_names_from_directory(workspace_agents_dir.as_path(), &mut topics);
    }

    topics.into_iter().collect()
}

fn collect_topic_names_from_directory(directory: &Path, topics: &mut BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let topic = entry.file_name().to_string_lossy().trim().to_owned();
        if topic.is_empty() {
            continue;
        }
        topics.insert(topic);
    }
}

fn create_scoped_memory_roots(snapshot: &MemoryCliSnapshot) -> CliResult<()> {
    let sqlite_path = PathBuf::from(&snapshot.sqlite_path);
    if let Some(parent) = sqlite_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create memory sqlite parent directory {} failed: {error}",
                parent.display()
            )
        })?;
    }

    if let Some(memory_workspace_root) = snapshot.memory_workspace_root.as_deref() {
        let memory_workspace_root = Path::new(memory_workspace_root);
        fs::create_dir_all(memory_workspace_root).map_err(|error| {
            format!(
                "create scoped memory workspace root {} failed: {error}",
                memory_workspace_root.display()
            )
        })?;
    }

    Ok(())
}

fn render_memory_command_text(execution: &MemoryCommandExecution) -> String {
    let mut lines = Vec::new();
    lines.push(format!("action: {}", execution.action));
    lines.push(format!("config: {}", execution.resolved_config_path));
    lines.push(format!("active_scope: {}", execution.active_scope));
    lines.push(format!("sqlite_path: {}", execution.sqlite_path));
    if let Some(memory_workspace_root) = execution.memory_workspace_root.as_deref() {
        lines.push(format!("memory_workspace_root: {memory_workspace_root}"));
    }
    if !execution.discovered_topics.is_empty() {
        lines.push(format!(
            "topics: {}",
            execution.discovered_topics.join(", ")
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_memory_cli_config(path: &Path, workspace_root: &Path) {
        let mut config = mvp::config::LoongConfig::default();
        config.tools.file_root = Some(workspace_root.display().to_string());
        config.memory.sqlite_path = path
            .parent()
            .expect("config parent")
            .join("memory.sqlite3")
            .display()
            .to_string();
        let path_string = path.display().to_string();
        mvp::config::write(Some(path_string.as_str()), &config, true).expect("write config");
    }

    #[test]
    fn execute_memory_command_switch_persists_topic_scope() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let config_path = temp_dir.path().join("config.toml");
        let workspace_root = temp_dir.path().join("workspace");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        write_memory_cli_config(&config_path, &workspace_root);

        let execution = execute_memory_command(MemoryCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: MemoryCommands::Switch {
                topic: Some("health".to_owned()),
                shared: false,
            },
        })
        .expect("switch command should succeed");

        let (_, config) = mvp::config::load(Some(config_path.display().to_string().as_str()))
            .expect("reload config");

        assert_eq!(config.memory.agent_id.as_deref(), Some("health"));
        assert_eq!(execution.active_topic.as_deref(), Some("health"));
        assert!(
            execution
                .memory_workspace_root
                .as_deref()
                .is_some_and(|value| value.ends_with("/.loong/agents/health")),
            "execution={execution:?}"
        );
    }

    #[test]
    fn execute_memory_command_list_discovers_scoped_topics() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let config_path = temp_dir.path().join("config.toml");
        let workspace_root = temp_dir.path().join("workspace");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        write_memory_cli_config(&config_path, &workspace_root);

        let sqlite_agents_dir = temp_dir.path().join("agents").join("investment");
        std::fs::create_dir_all(&sqlite_agents_dir).expect("create sqlite agents dir");
        let workspace_agents_dir = workspace_root.join(".loong/agents/health");
        std::fs::create_dir_all(&workspace_agents_dir).expect("create workspace agents dir");

        let execution = execute_memory_command(MemoryCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: MemoryCommands::List,
        })
        .expect("list command should succeed");

        assert_eq!(
            execution.discovered_topics,
            vec![
                "health".to_owned(),
                "investment".to_owned(),
                "shared".to_owned()
            ]
        );
    }

    #[test]
    fn execute_memory_command_current_defaults_to_shared_scope() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let config_path = temp_dir.path().join("config.toml");
        let workspace_root = temp_dir.path().join("workspace");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        write_memory_cli_config(&config_path, &workspace_root);

        let execution = execute_memory_command(MemoryCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: MemoryCommands::Current,
        })
        .expect("current command should succeed");

        assert_eq!(execution.active_topic, None);
        assert_eq!(execution.active_scope, "shared");
    }

    #[test]
    fn execute_memory_command_switch_shared_clears_topic_scope() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let config_path = temp_dir.path().join("config.toml");
        let workspace_root = temp_dir.path().join("workspace");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        write_memory_cli_config(&config_path, &workspace_root);

        let _ = execute_memory_command(MemoryCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: MemoryCommands::Switch {
                topic: Some("health".to_owned()),
                shared: false,
            },
        })
        .expect("switch topic command should succeed");

        let execution = execute_memory_command(MemoryCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: MemoryCommands::Switch {
                topic: None,
                shared: true,
            },
        })
        .expect("switch shared command should succeed");

        let (_, config) = mvp::config::load(Some(config_path.display().to_string().as_str()))
            .expect("reload config");

        assert_eq!(config.memory.agent_id, None);
        assert_eq!(execution.active_topic, None);
        assert_eq!(execution.active_scope, "shared");
    }
}
