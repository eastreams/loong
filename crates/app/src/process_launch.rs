use std::ffi::OsString;
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(hidden)]
pub struct ResolvedCommandInvocation {
    pub program: OsString,
    pub args: Vec<OsString>,
}

#[doc(hidden)]
pub fn resolve_command_invocation<I, S>(command: &str, args: I) -> ResolvedCommandInvocation
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let collected_args = args
        .into_iter()
        .map(|value| OsString::from(value.as_ref()))
        .collect::<Vec<_>>();

    #[cfg(unix)]
    if let Some(invocation) = resolve_shebang_invocation(command, &collected_args) {
        return invocation;
    }

    ResolvedCommandInvocation {
        program: OsString::from(command),
        args: collected_args,
    }
}

#[cfg(unix)]
fn resolve_shebang_invocation(
    command: &str,
    collected_args: &[OsString],
) -> Option<ResolvedCommandInvocation> {
    let script_path = resolve_existing_command_path(command)?;
    let shebang = read_shebang(script_path.as_path())?;
    let trimmed_shebang = shebang.trim();
    let separator_index = trimmed_shebang.find(char::is_whitespace);
    let interpreter = match separator_index {
        Some(index) => trimmed_shebang.get(..index)?,
        None => trimmed_shebang,
    };
    let mut resolved_args = Vec::new();
    let remainder = separator_index.and_then(|index| trimmed_shebang.get(index..));
    let remainder = remainder.map(str::trim_start);
    if let Some(remainder) = remainder
        && !remainder.is_empty()
    {
        resolved_args.push(OsString::from(remainder));
    }
    let script_arg = script_path.into_os_string();
    resolved_args.push(script_arg);
    for argument in collected_args {
        let cloned_argument = argument.clone();
        resolved_args.push(cloned_argument);
    }

    Some(ResolvedCommandInvocation {
        program: OsString::from(interpreter),
        args: resolved_args,
    })
}

#[cfg(unix)]
fn resolve_existing_command_path(command: &str) -> Option<PathBuf> {
    let direct_path = Path::new(command);
    if direct_path.is_file() {
        return Some(direct_path.to_path_buf());
    }

    which::which(command).ok()
}

#[cfg(unix)]
fn read_shebang(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = [0_u8; 256];
    let count = file.read(&mut buffer).ok()?;
    let prefix = buffer.get(..2)?;
    if count < 2 || prefix != b"#!" {
        return None;
    }

    let header = buffer.get(..count)?;
    let line_end = header
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(count);
    let line = std::str::from_utf8(buffer.get(2..line_end)?).ok()?;
    let trimmed = line.trim().trim_end_matches('\r');
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::path::Path;
    #[cfg(unix)]
    use std::path::PathBuf;

    use super::resolve_command_invocation;

    #[cfg(unix)]
    #[test]
    fn resolve_command_invocation_wraps_shebang_script_with_interpreter() {
        let root = crate::test_support::unique_temp_dir("loongclaw-process-launch-sh");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let script_path = root.join("script.sh");
        crate::test_support::write_executable_script_atomically(
            &script_path,
            "#!/bin/sh\nexit 0\n",
        )
        .expect("write script");

        let resolved =
            resolve_command_invocation(script_path.to_string_lossy().as_ref(), ["--flag", "value"]);

        assert_eq!(resolved.program, std::ffi::OsString::from("/bin/sh"));
        assert_eq!(
            resolved.args,
            vec![
                script_path.into_os_string(),
                std::ffi::OsString::from("--flag"),
                std::ffi::OsString::from("value"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_command_invocation_preserves_non_script_program() {
        let resolved = resolve_command_invocation("/bin/echo", ["hello"]);

        assert_eq!(resolved.program, std::ffi::OsString::from("/bin/echo"));
        assert_eq!(resolved.args, vec![std::ffi::OsString::from("hello")]);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_command_invocation_supports_env_shebang_arguments() {
        let root = crate::test_support::unique_temp_dir("loongclaw-process-launch-env");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let script_path = root.join("script.py");
        crate::test_support::write_executable_script_atomically(
            &script_path,
            "#!/usr/bin/env python3\nprint('ok')\n",
        )
        .expect("write script");

        let resolved = resolve_command_invocation(
            script_path.to_string_lossy().as_ref(),
            Vec::<String>::new(),
        );

        assert_eq!(resolved.program, std::ffi::OsString::from("/usr/bin/env"));
        assert_eq!(
            resolved.args,
            vec![
                std::ffi::OsString::from("python3"),
                PathBuf::from(&script_path).into_os_string(),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_command_invocation_preserves_env_split_arguments_as_one_argument() {
        let root = crate::test_support::unique_temp_dir("loongclaw-process-launch-env-s");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let script_path = root.join("script.py");
        crate::test_support::write_executable_script_atomically(
            &script_path,
            "#!/usr/bin/env -S python3 -u\nprint('ok')\n",
        )
        .expect("write script");

        let resolved = resolve_command_invocation(
            script_path.to_string_lossy().as_ref(),
            Vec::<String>::new(),
        );

        assert_eq!(resolved.program, std::ffi::OsString::from("/usr/bin/env"));
        assert_eq!(
            resolved.args,
            vec![
                std::ffi::OsString::from("-S python3 -u"),
                PathBuf::from(&script_path).into_os_string(),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_command_invocation_uses_resolved_path_for_path_discovered_scripts() {
        let root = crate::test_support::unique_temp_dir("loongclaw-process-launch-path");
        let bin_dir = root.join("bin");
        let script_path = bin_dir.join("path-script");
        std::fs::create_dir_all(&bin_dir).expect("create bin dir");
        crate::test_support::write_executable_script_atomically(
            &script_path,
            "#!/bin/sh\nexit 0\n",
        )
        .expect("write path-discovered script");

        let mut env = crate::test_support::ScopedEnv::new();
        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let mut path_entries = vec![PathBuf::from(&bin_dir)];
        path_entries.extend(std::env::split_paths(Path::new(&original_path)).collect::<Vec<_>>());
        let joined_path = std::env::join_paths(path_entries).expect("join PATH");
        env.set("PATH", joined_path);

        let resolved = resolve_command_invocation("path-script", ["--flag"]);

        assert_eq!(resolved.program, std::ffi::OsString::from("/bin/sh"));
        assert_eq!(
            resolved.args,
            vec![
                script_path.into_os_string(),
                std::ffi::OsString::from("--flag"),
            ]
        );
    }
}
