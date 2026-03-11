use std::path::Path;

use loongclaw_app as mvp;
use loongclaw_spec::CliResult;

#[derive(Debug, Clone)]
pub(crate) struct ImportClawCommandOptions {
    pub input: String,
    pub output: Option<String>,
    pub source: Option<String>,
    pub force: bool,
}

pub(crate) fn parse_legacy_claw_source(raw: &str) -> Option<mvp::migration::LegacyClawSource> {
    mvp::migration::LegacyClawSource::from_id(raw)
}

pub(crate) fn run_import_claw_cli(options: ImportClawCommandOptions) -> CliResult<()> {
    let input_path = mvp::config::expand_path(&options.input);
    let output_path = options
        .output
        .as_deref()
        .map(mvp::config::expand_path)
        .unwrap_or_else(mvp::config::default_config_path);

    if output_path.exists() && !options.force {
        return Err(format!(
            "config {} already exists (use --force to overwrite)",
            output_path.display()
        ));
    }

    let hint = if let Some(raw) = options.source.as_deref() {
        let parsed = parse_legacy_claw_source(raw).ok_or_else(|| {
            format!(
                "unsupported --source value \"{raw}\". supported: {}",
                supported_legacy_source_list()
            )
        })?;
        if parsed == mvp::migration::LegacyClawSource::Unknown {
            None
        } else {
            Some(parsed)
        }
    } else {
        None
    };

    let plan = mvp::migration::plan_import_from_path(&input_path, hint)?;
    let mut config = load_or_default_config(&output_path, output_path.exists())?;
    mvp::migration::apply_import_plan(&mut config, &plan);

    let output_string = output_path.display().to_string();
    let written = mvp::config::write(Some(&output_string), &config, options.force)?;

    #[cfg(feature = "memory-sqlite")]
    let memory_path = {
        let mem_config =
            mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        mvp::memory::ensure_memory_db_ready(Some(config.memory.resolved_sqlite_path()), &mem_config)
            .map_err(|error| format!("failed to bootstrap sqlite memory: {error}"))?
    };

    println!("import complete");
    println!("- source: {}", legacy_claw_source_id(plan.source));
    println!("- input: {}", input_path.display());
    println!("- config: {}", written.display());
    println!(
        "- prompt pack: {}",
        config
            .cli
            .prompt_pack_id()
            .unwrap_or(mvp::prompt::DEFAULT_PROMPT_PACK_ID)
    );
    println!(
        "- memory profile: {}",
        memory_profile_id(config.memory.profile)
    );
    println!(
        "- imported prompt addendum: {}",
        yes_no(config.cli.system_prompt_addendum.is_some())
    );
    println!(
        "- imported profile note: {}",
        yes_no(config.memory.profile_note.is_some())
    );
    #[cfg(feature = "memory-sqlite")]
    println!("- sqlite memory: {}", memory_path.display());
    for warning in &plan.warnings {
        println!("- warning: {warning}");
    }
    println!("next step: loongclawd chat --config {}", written.display());
    Ok(())
}

fn load_or_default_config(path: &Path, exists: bool) -> CliResult<mvp::config::LoongClawConfig> {
    if !exists {
        return Ok(mvp::config::LoongClawConfig::default());
    }
    let path_string = path.display().to_string();
    let (_, config) = mvp::config::load(Some(&path_string))?;
    Ok(config)
}

fn legacy_claw_source_id(source: mvp::migration::LegacyClawSource) -> &'static str {
    source.as_id()
}

fn supported_legacy_source_list() -> &'static str {
    "auto, nanobot, openclaw, picoclaw, zeroclaw, nanoclaw"
}

fn memory_profile_id(profile: mvp::config::MemoryProfile) -> &'static str {
    match profile {
        mvp::config::MemoryProfile::WindowOnly => "window_only",
        mvp::config::MemoryProfile::WindowPlusSummary => "window_plus_summary",
        mvp::config::MemoryProfile::ProfilePlusWindow => "profile_plus_window",
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
