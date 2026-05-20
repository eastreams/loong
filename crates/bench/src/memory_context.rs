use std::path::{Path, PathBuf};

use self::report::{MemoryContextBenchmarkTempRootSource, ResolvedMemoryContextBenchmarkTempRoot};

use super::CliResult;
#[cfg(test)]
use super::current_epoch_seconds;

#[path = "memory_context_report.rs"]
mod report;

pub use report::{
    MemoryContextBenchmarkReportAugmentContext,
    MemoryContextBenchmarkReportAugmenter,
    MemoryContextBenchmarkSuiteRunner,
    MemoryContextBenchmarkSuiteSamples,
    MemoryContextColdPathPhaseSamples,
    MemoryContextShape,
    run_memory_context_benchmark_cli_with_suite_runner,
};
pub(crate) use report::{
    ProgrammaticPressureGateCheck,
    ProgrammaticPressureScenarioGate,
    ScenarioRunSample,
    SchedulerSnapshot,
};

#[cfg(test)]
fn next_benchmark_temp_suffix() -> u64 {
    static BENCHMARK_TEMP_COUNTER: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    BENCHMARK_TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
fn benchmark_temp_root(prefix: &str, parent: Option<&Path>) -> PathBuf {
    let parent = match parent {
        Some(parent) => parent.to_path_buf(),
        None => std::env::temp_dir(),
    };
    parent.join(format!(
        "{prefix}-{}-{}-{}",
        current_epoch_seconds(),
        std::process::id(),
        next_benchmark_temp_suffix()
    ))
}

fn resolve_memory_context_benchmark_temp_root(
    output_path: &str,
    temp_root: Option<&str>,
) -> CliResult<ResolvedMemoryContextBenchmarkTempRoot> {
    let current_exe = std::env::current_exe().ok();
    resolve_memory_context_benchmark_temp_root_with_exe(
        output_path,
        temp_root,
        current_exe.as_deref(),
    )
}

fn resolve_memory_context_benchmark_temp_root_with_exe(
    output_path: &str,
    temp_root: Option<&str>,
    current_exe: Option<&Path>,
) -> CliResult<ResolvedMemoryContextBenchmarkTempRoot> {
    if let Some(temp_root) = temp_root {
        return Ok(ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from(temp_root),
            source: MemoryContextBenchmarkTempRootSource::Explicit,
        });
    }

    if let Some(current_exe) = current_exe
        && let Some(profile_dir) = current_exe.parent()
        && matches!(
            profile_dir.file_name().and_then(|name| name.to_str()),
            Some("debug" | "release")
        )
        && let Some(target_dir) = profile_dir.parent()
    {
        return Ok(ResolvedMemoryContextBenchmarkTempRoot {
            path: target_dir.join("tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
        });
    }

    let output_path = Path::new(output_path);
    let starts_in_target_dir = matches!(
        output_path.components().next(),
        Some(std::path::Component::Normal(component)) if component == "target"
    );
    if starts_in_target_dir
        && let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        return Ok(ResolvedMemoryContextBenchmarkTempRoot {
            path: parent.join("tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        });
    }

    Ok(ResolvedMemoryContextBenchmarkTempRoot {
        path: std::env::temp_dir(),
        source: MemoryContextBenchmarkTempRootSource::SystemTemp,
    })
}
