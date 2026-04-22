use clap::Args;
use clap::Subcommand;
use loong_spec::CliResult;

const RUNTIME_TRAJECTORY_RUNTIME_COMMAND: &str = "runtime trajectory runtime";

pub const ARTIFACT_MODE_EVENT_PAGE_LIMIT_DEFAULT: usize = 200;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum RuntimeTrajectoryCommands {
    /// Export one runtime trajectory artifact from a live session
    Export(RuntimeTrajectoryExportCommandOptions),
    /// Show one persisted runtime trajectory artifact in text or JSON form
    Show(RuntimeTrajectoryShowCommandOptions),
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTrajectoryExportCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long)]
    pub output: Option<String>,
    #[arg(long)]
    pub turn_limit: Option<usize>,
    #[arg(long, default_value_t = ARTIFACT_MODE_EVENT_PAGE_LIMIT_DEFAULT)]
    pub event_page_limit: usize,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTrajectoryShowCommandOptions {
    #[arg(long)]
    pub artifact: String,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

pub fn execute_runtime_trajectory_command(command: RuntimeTrajectoryCommands) -> CliResult<()> {
    match command {
        RuntimeTrajectoryCommands::Export(options) => run_runtime_trajectory_cli(
            options.config.as_deref(),
            options.session.as_deref(),
            None,
            options.output.as_deref(),
            options.turn_limit,
            options.event_page_limit,
            options.json,
        ),
        RuntimeTrajectoryCommands::Show(options) => run_runtime_trajectory_cli(
            None,
            None,
            Some(options.artifact.as_str()),
            None,
            None,
            ARTIFACT_MODE_EVENT_PAGE_LIMIT_DEFAULT,
            options.json,
        ),
    }
}

pub fn run_runtime_trajectory_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    artifact: Option<&str>,
    output: Option<&str>,
    turn_limit: Option<usize>,
    event_page_limit: usize,
    as_json: bool,
) -> CliResult<()> {
    if matches!(turn_limit, Some(0)) {
        return Err(format!(
            "{RUNTIME_TRAJECTORY_RUNTIME_COMMAND} --turn-limit must be >= 1 when provided"
        ));
    }

    if event_page_limit == 0 {
        return Err(format!(
            "{RUNTIME_TRAJECTORY_RUNTIME_COMMAND} --event-page-limit must be >= 1"
        ));
    }

    validate_runtime_trajectory_arguments(session, artifact, output, turn_limit, event_page_limit)?;

    #[cfg(feature = "memory-sqlite")]
    {
        let artifact_path = artifact
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from);
        let loaded_artifact = if let Some(artifact_path) = artifact_path.as_ref() {
            load_runtime_trajectory_artifact(artifact_path.as_path())?
        } else {
            let (_, config) = crate::mvp::config::load(config_path)?;
            let session_id = session
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    format!("{RUNTIME_TRAJECTORY_RUNTIME_COMMAND} requires --session or --artifact")
                })?;
            let memory_config =
                crate::mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
                    &config.memory,
                );
            let export_options = crate::mvp::session::trajectory::SessionTrajectoryExportOptions {
                turn_limit,
                event_page_limit,
            };
            crate::mvp::session::trajectory::export_session_trajectory(
                session_id,
                &memory_config,
                &export_options,
            )
            .map_err(|error| format!("export session trajectory failed: {error}"))?
        };

        let output_path = output
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from);

        if let Some(output_path) = output_path.as_ref() {
            persist_runtime_trajectory_artifact(output_path.as_path(), &loaded_artifact)?;
        }

        if as_json {
            let pretty = serde_json::to_string_pretty(&loaded_artifact)
                .map_err(|error| format!("serialize session trajectory failed: {error}"))?;
            println!("{pretty}");
            return Ok(());
        }

        let rendered = format_runtime_trajectory_summary(&loaded_artifact);
        let wrapped = wrap_runtime_trajectory_surface("runtime trajectory", rendered);
        if let Some(output_path) = output_path.as_ref() {
            println!("artifact_path={}", output_path.display());
        }
        print!("{wrapped}");
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (
            config_path,
            session,
            artifact,
            output,
            turn_limit,
            event_page_limit,
            as_json,
        );
        Err(format!(
            "{RUNTIME_TRAJECTORY_RUNTIME_COMMAND} requires memory-sqlite feature"
        ))
    }
}

fn wrap_runtime_trajectory_surface(title: &str, body: String) -> String {
    crate::render_operator_shell_surface_from_body(title, "runtime trajectory", body)
}

fn validate_runtime_trajectory_arguments(
    session: Option<&str>,
    artifact: Option<&str>,
    output: Option<&str>,
    turn_limit: Option<usize>,
    event_page_limit: usize,
) -> CliResult<()> {
    let session = session.map(str::trim).filter(|value| !value.is_empty());
    let artifact = artifact.map(str::trim).filter(|value| !value.is_empty());
    let output = output.map(str::trim).filter(|value| !value.is_empty());

    if session.is_none() && artifact.is_none() {
        return Err(format!(
            "{RUNTIME_TRAJECTORY_RUNTIME_COMMAND} requires --session or --artifact"
        ));
    }

    if session.is_some() && artifact.is_some() {
        return Err(format!(
            "{RUNTIME_TRAJECTORY_RUNTIME_COMMAND} cannot combine --session with --artifact"
        ));
    }

    if artifact.is_some() && output.is_some() {
        return Err(format!(
            "{RUNTIME_TRAJECTORY_RUNTIME_COMMAND} cannot combine --artifact with --output"
        ));
    }

    if artifact.is_some() && turn_limit.is_some() {
        return Err(format!(
            "{RUNTIME_TRAJECTORY_RUNTIME_COMMAND} cannot combine --artifact with --turn-limit"
        ));
    }

    if artifact.is_some() && event_page_limit != ARTIFACT_MODE_EVENT_PAGE_LIMIT_DEFAULT {
        return Err(format!(
            "{RUNTIME_TRAJECTORY_RUNTIME_COMMAND} cannot combine --artifact with --event-page-limit"
        ));
    }

    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn persist_runtime_trajectory_artifact(
    output_path: &std::path::Path,
    artifact: &crate::mvp::session::trajectory::SessionTrajectoryArtifact,
) -> CliResult<()> {
    // Reuse the shared JSON artifact writer so this path inherits the
    // cross-platform temp-write-and-rename behavior used by the other
    // persisted daemon artifacts.
    let output_path_string = output_path.to_string_lossy().into_owned();
    let payload = serde_json::to_value(artifact)
        .map_err(|error| format!("serialize runtime trajectory artifact failed: {error}"))?;

    crate::persist_json_artifact(
        output_path_string.as_str(),
        &payload,
        "runtime trajectory artifact",
    )
}

#[cfg(feature = "memory-sqlite")]
fn load_runtime_trajectory_artifact(
    artifact_path: &std::path::Path,
) -> CliResult<crate::mvp::session::trajectory::SessionTrajectoryArtifact> {
    let encoded = std::fs::read_to_string(artifact_path).map_err(|error| {
        format!(
            "read runtime trajectory artifact {} failed: {error}",
            artifact_path.display()
        )
    })?;
    let artifact =
        serde_json::from_str::<crate::mvp::session::trajectory::SessionTrajectoryArtifact>(
            &encoded,
        )
        .map_err(|error| {
            format!(
                "decode runtime trajectory artifact {} failed: {error}",
                artifact_path.display()
            )
        })?;
    validate_runtime_trajectory_artifact_schema(&artifact)?;
    Ok(artifact)
}

#[cfg(feature = "memory-sqlite")]
fn validate_runtime_trajectory_artifact_schema(
    artifact: &crate::mvp::session::trajectory::SessionTrajectoryArtifact,
) -> CliResult<()> {
    let schema = &artifact.schema;
    let expected_version =
        crate::mvp::session::trajectory::SESSION_TRAJECTORY_ARTIFACT_JSON_SCHEMA_VERSION;
    if schema.version != expected_version {
        return Err(format!(
            "runtime trajectory artifact schema version {} does not match expected version {}",
            schema.version, expected_version
        ));
    }

    let expected_surface = crate::mvp::session::trajectory::SESSION_TRAJECTORY_ARTIFACT_SURFACE;
    if schema.surface != expected_surface {
        return Err(format!(
            "runtime trajectory artifact surface `{}` does not match expected surface `{}`",
            schema.surface, expected_surface
        ));
    }

    let expected_purpose = crate::mvp::session::trajectory::SESSION_TRAJECTORY_ARTIFACT_PURPOSE;
    if schema.purpose != expected_purpose {
        return Err(format!(
            "runtime trajectory artifact purpose `{}` does not match expected purpose `{}`",
            schema.purpose, expected_purpose
        ));
    }

    Ok(())
}

#[cfg(feature = "memory-sqlite")]
pub fn format_runtime_trajectory_summary(
    artifact: &crate::mvp::session::trajectory::SessionTrajectoryArtifact,
) -> String {
    let terminal_status = artifact
        .terminal_outcome
        .as_ref()
        .map(|outcome| outcome.status.as_str())
        .unwrap_or("none");
    let turns_truncated = if artifact.turns_truncated {
        "true"
    } else {
        "false"
    };

    let mut rendered = String::new();
    rendered.push_str("runtime_trajectory ");
    rendered.push_str("session=");
    rendered.push_str(&artifact.session.session_id);
    rendered.push_str(" kind=");
    rendered.push_str(&artifact.session.kind);
    rendered.push_str(" state=");
    rendered.push_str(&artifact.session.state);
    rendered.push_str(" lineage_root=");
    rendered.push_str(artifact.lineage.root_session_id.as_deref().unwrap_or("-"));
    rendered.push_str(" lineage_depth=");
    rendered.push_str(&artifact.lineage.depth.to_string());
    rendered.push('\n');

    rendered.push_str("counts ");
    rendered.push_str("total_turns=");
    rendered.push_str(&artifact.session.turn_count.to_string());
    rendered.push_str(" exported_turns=");
    rendered.push_str(&artifact.exported_turn_count.to_string());
    rendered.push_str(" turns_truncated=");
    rendered.push_str(turns_truncated);
    rendered.push_str(" canonical_records=");
    rendered.push_str(&artifact.canonical_record_count.to_string());
    rendered.push_str(" events=");
    rendered.push_str(&artifact.event_count.to_string());
    rendered.push_str(" approvals=");
    rendered.push_str(&artifact.approval_request_count.to_string());
    rendered.push_str(" terminal_status=");
    rendered.push_str(terminal_status);
    rendered.push('\n');

    rendered.push_str("export ");
    rendered.push_str("schema_version=");
    rendered.push_str(&artifact.schema.version.to_string());
    rendered.push_str(" exported_at=");
    rendered.push_str(&artifact.exported_at);
    rendered.push_str(" event_page_limit=");
    rendered.push_str(&artifact.event_page_limit.to_string());
    rendered.push('\n');

    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_trajectory_runtime_errors_use_grouped_namespace() {
        let turn_limit_error = run_runtime_trajectory_cli(
            None,
            None,
            None,
            None,
            Some(0),
            ARTIFACT_MODE_EVENT_PAGE_LIMIT_DEFAULT,
            false,
        )
        .expect_err("zero turn limit should fail before feature-specific logic");
        assert_eq!(
            turn_limit_error,
            "runtime trajectory runtime --turn-limit must be >= 1 when provided"
        );

        let missing_selector_error = validate_runtime_trajectory_arguments(
            None,
            None,
            None,
            None,
            ARTIFACT_MODE_EVENT_PAGE_LIMIT_DEFAULT,
        )
        .expect_err("missing selector should fail");
        assert_eq!(
            missing_selector_error,
            "runtime trajectory runtime requires --session or --artifact"
        );
    }
}
