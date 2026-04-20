use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;

use crate::{CLI_COMMAND_NAME, CliResult};

const DEFAULT_RELEASE_REPO: &str = "eastreams/loong";
const GITHUB_RELEASE_API_BASE: &str = "https://api.github.com";
const GITHUB_RELEASE_DOWNLOAD_BASE: &str = "https://github.com";
const UPDATE_USER_AGENT: &str = "Loong-Update";
const UPDATE_RELEASE_REPO_ENV: &str = "LOONG_UPDATE_REPO";
const UPDATE_RELEASE_API_URL_ENV: &str = "LOONG_UPDATE_RELEASE_API_URL";
const UPDATE_RELEASE_BASE_URL_ENV: &str = "LOONG_UPDATE_RELEASE_BASE_URL";
const INSTALL_RELEASE_REPO_ENV: &str = "LOONG_INSTALL_REPO";
const INSTALL_RELEASE_BASE_URL_ENV: &str = "LOONG_INSTALL_RELEASE_BASE_URL";
const TEST_EXECUTABLE_ENV: &str = "LOONG_UPDATE_TEST_EXECUTABLE";

#[derive(Debug, Deserialize)]
struct GitHubReleaseMetadata {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdatePlatform {
    Unix,
    Windows,
}

impl UpdatePlatform {
    fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }

    fn script_name(self) -> &'static str {
        match self {
            Self::Unix => "install.sh",
            Self::Windows => "install.ps1",
        }
    }
}

#[derive(Debug, Clone)]
struct UpdateRuntimeConfig {
    release_repo: String,
    latest_release_api_url: String,
    release_base_url: String,
}

impl UpdateRuntimeConfig {
    fn from_env() -> Self {
        let release_repo = env::var(UPDATE_RELEASE_REPO_ENV)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                env::var(INSTALL_RELEASE_REPO_ENV)
                    .ok()
                    .map(|value| value.trim().to_owned())
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_else(|| DEFAULT_RELEASE_REPO.to_owned());

        let latest_release_api_url = env::var(UPDATE_RELEASE_API_URL_ENV)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| latest_release_api_url(release_repo.as_str()));

        let release_base_url = env::var(UPDATE_RELEASE_BASE_URL_ENV)
            .ok()
            .map(|value| value.trim().trim_end_matches('/').to_owned())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                env::var(INSTALL_RELEASE_BASE_URL_ENV)
                    .ok()
                    .map(|value| value.trim().trim_end_matches('/').to_owned())
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_else(|| format!("{GITHUB_RELEASE_DOWNLOAD_BASE}/{release_repo}/releases"));

        Self {
            release_repo,
            latest_release_api_url,
            release_base_url,
        }
    }
}

pub async fn run_update_cli() -> CliResult<()> {
    let runtime = UpdateRuntimeConfig::from_env();
    let executable_override = env::var_os(TEST_EXECUTABLE_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    run_update_cli_with_runtime(runtime, executable_override).await
}

async fn run_update_cli_with_runtime(
    runtime: UpdateRuntimeConfig,
    current_executable_override: Option<PathBuf>,
) -> CliResult<()> {
    let platform = UpdatePlatform::current();
    let current_executable = resolve_current_executable_path(current_executable_override)?;
    let install_prefix = install_prefix_for_current_executable(current_executable.as_path())?;
    let latest_release = fetch_latest_stable_release(&runtime).await?;
    let script_name = platform.script_name();
    let script_url = stable_release_script_url(
        runtime.release_base_url.as_str(),
        latest_release.tag_name.as_str(),
        script_name,
    );
    let script_path = download_update_script(script_url.as_str(), script_name).await?;

    #[allow(clippy::print_stdout)]
    {
        println!(
            "==> Updating {CLI_COMMAND_NAME} to stable release {}",
            latest_release.tag_name
        );
    }

    let command_result = run_update_script(
        platform,
        script_path.as_path(),
        install_prefix.as_path(),
        latest_release.tag_name.as_str(),
        &runtime,
    );

    let cleanup_result = fs::remove_file(&script_path);
    if let Err(error) = cleanup_result
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            target: "loong.daemon",
            path = %script_path.display(),
            error = %error,
            "failed to remove temporary update script"
        );
    }

    command_result
}

fn resolve_current_executable_path(
    current_executable_override: Option<PathBuf>,
) -> CliResult<PathBuf> {
    if let Some(override_path) = current_executable_override {
        return Ok(override_path);
    }

    env::current_exe()
        .map_err(|error| format!("failed to resolve current executable path: {error}"))
}

fn install_prefix_for_current_executable(current_executable: &Path) -> CliResult<PathBuf> {
    current_executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "failed to determine install prefix from current executable {}",
                current_executable.display()
            )
        })
}

async fn fetch_latest_stable_release(
    runtime: &UpdateRuntimeConfig,
) -> CliResult<GitHubReleaseMetadata> {
    let response = reqwest::Client::new()
        .get(runtime.latest_release_api_url.as_str())
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header(reqwest::header::USER_AGENT, UPDATE_USER_AGENT)
        .send()
        .await
        .map_err(|error| {
            format!(
                "failed to contact GitHub release API for {}: {error}",
                runtime.release_repo
            )
        })?;
    let response = response.error_for_status().map_err(|error| {
        format!(
            "failed to resolve latest stable GitHub release for {}: {error}",
            runtime.release_repo
        )
    })?;
    let body = response.text().await.map_err(|error| {
        format!(
            "failed to read latest stable GitHub release response for {}: {error}",
            runtime.release_repo
        )
    })?;
    parse_latest_stable_release_response(body.as_str())
}

fn parse_latest_stable_release_response(response_body: &str) -> CliResult<GitHubReleaseMetadata> {
    let release =
        serde_json::from_str::<GitHubReleaseMetadata>(response_body).map_err(|error| {
            format!("failed to parse latest stable GitHub release response: {error}")
        })?;
    let tag_name = release.tag_name.trim();
    if tag_name.is_empty() {
        return Err("latest stable GitHub release response did not include tag_name".to_owned());
    }
    if release.draft {
        return Err(format!(
            "latest stable GitHub release `{tag_name}` is still marked as a draft"
        ));
    }
    if release.prerelease {
        return Err(format!(
            "latest stable GitHub release `{tag_name}` unexpectedly resolved to a pre-release"
        ));
    }
    Ok(release)
}

async fn download_update_script(script_url: &str, script_name: &str) -> CliResult<PathBuf> {
    let response = reqwest::Client::new()
        .get(script_url)
        .header(reqwest::header::USER_AGENT, UPDATE_USER_AGENT)
        .send()
        .await
        .map_err(|error| {
            format!("failed to download update installer from {script_url}: {error}")
        })?;
    let response = response
        .error_for_status()
        .map_err(|error| format!("update installer download failed for {script_url}: {error}"))?;
    let script_bytes = response
        .bytes()
        .await
        .map_err(|error| format!("failed to read update installer from {script_url}: {error}"))?;
    write_temp_update_script(script_name, script_bytes.as_ref())
}

fn write_temp_update_script(script_name: &str, script_bytes: &[u8]) -> CliResult<PathBuf> {
    let process_id = std::process::id();
    let unix_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("failed to derive temporary update path timestamp: {error}"))?
        .as_nanos();
    let path = env::temp_dir().join(format!(
        "loong-update-{process_id}-{unix_nanos}-{script_name}"
    ));

    fs::write(&path, script_bytes).map_err(|error| {
        format!(
            "failed to write temporary update installer to {}: {error}",
            path.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&path)
            .map_err(|error| {
                format!(
                    "failed to inspect temporary update installer {}: {error}",
                    path.display()
                )
            })?
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).map_err(|error| {
            format!(
                "failed to mark temporary update installer {} executable: {error}",
                path.display()
            )
        })?;
    }

    Ok(path)
}

fn run_update_script(
    platform: UpdatePlatform,
    script_path: &Path,
    install_prefix: &Path,
    release_tag: &str,
    runtime: &UpdateRuntimeConfig,
) -> CliResult<()> {
    let mut command =
        build_update_script_command(platform, script_path, install_prefix, release_tag, runtime)?;
    let rendered_command = render_update_command(&command);
    let status = command.status().map_err(|error| {
        format!("failed to launch update installer `{rendered_command}`: {error}")
    })?;

    if status.success() {
        return Ok(());
    }

    match status.code() {
        Some(code) => Err(format!(
            "update installer `{rendered_command}` exited with status code {code}"
        )),
        None => Err(format!(
            "update installer `{rendered_command}` terminated without an exit code"
        )),
    }
}

fn build_update_script_command(
    platform: UpdatePlatform,
    script_path: &Path,
    install_prefix: &Path,
    release_tag: &str,
    runtime: &UpdateRuntimeConfig,
) -> CliResult<Command> {
    let mut command = match platform {
        UpdatePlatform::Windows => {
            let shell = ["pwsh", "powershell"]
                .into_iter()
                .find(|candidate| command_exists(candidate))
                .ok_or_else(|| {
                    "failed to find PowerShell (`pwsh` or `powershell`) for `loong update`"
                        .to_owned()
                })?;
            let mut command = Command::new(shell);
            command
                .arg("-NoLogo")
                .arg("-NoProfile")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(script_path)
                .arg("-Prefix")
                .arg(install_prefix)
                .arg("-Version")
                .arg(release_tag);
            command
        }
        UpdatePlatform::Unix => {
            if !command_exists("bash") {
                return Err("failed to find `bash` for `loong update`".to_owned());
            }

            let mut command = Command::new("bash");
            command
                .arg(script_path)
                .arg("--prefix")
                .arg(install_prefix)
                .arg("--version")
                .arg(release_tag);
            command
        }
    };
    command.env(INSTALL_RELEASE_REPO_ENV, runtime.release_repo.as_str());
    command.env(
        INSTALL_RELEASE_BASE_URL_ENV,
        runtime.release_base_url.as_str(),
    );
    Ok(command)
}

fn render_update_command(command: &Command) -> String {
    let program = command.get_program().to_string_lossy();
    let args = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if args.is_empty() {
        return program.into_owned();
    }
    format!("{program} {}", args.join(" "))
}

fn command_exists(program: &str) -> bool {
    env::var_os("PATH").is_some_and(|path| {
        env::split_paths(&path).any(|directory| {
            let candidate = directory.join(program);
            if candidate.is_file() {
                return true;
            }

            #[cfg(windows)]
            {
                return ["exe", "cmd", "bat"]
                    .iter()
                    .any(|extension| directory.join(format!("{program}.{extension}")).is_file());
            }

            #[cfg(not(windows))]
            {
                false
            }
        })
    })
}

fn latest_release_api_url(release_repo: &str) -> String {
    format!("{GITHUB_RELEASE_API_BASE}/repos/{release_repo}/releases/latest")
}

fn stable_release_script_url(release_base_url: &str, tag_name: &str, script_name: &str) -> String {
    format!(
        "{}/download/{tag_name}/{script_name}",
        release_base_url.trim_end_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::Mutex,
        thread,
    };

    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn parse_latest_stable_release_response_accepts_stable_release() {
        let release = parse_latest_stable_release_response(
            r#"{"tag_name":"v0.4.2","draft":false,"prerelease":false}"#,
        )
        .expect("stable release response should parse");

        assert_eq!(release.tag_name, "v0.4.2");
        assert!(!release.draft);
        assert!(!release.prerelease);
    }

    #[test]
    fn parse_latest_stable_release_response_rejects_prerelease() {
        let error = parse_latest_stable_release_response(
            r#"{"tag_name":"v0.4.2-alpha.1","draft":false,"prerelease":true}"#,
        )
        .expect_err("pre-release response must be rejected");

        assert!(error.contains("pre-release"));
    }

    #[test]
    fn parse_latest_stable_release_response_rejects_draft_release() {
        let error = parse_latest_stable_release_response(r#"{"tag_name":"v0.4.2","draft":true}"#)
            .expect_err("draft release must be rejected");

        assert!(error.contains("draft"));
    }

    #[test]
    fn parse_latest_stable_release_response_requires_tag_name() {
        let error = parse_latest_stable_release_response(r#"{"tag_name":"   "}"#)
            .expect_err("blank tag name must be rejected");

        assert!(error.contains("tag_name"));
    }

    #[test]
    fn stable_release_script_url_points_at_release_download_asset() {
        let url = stable_release_script_url(
            "https://github.com/eastreams/loong/releases/",
            "v1.2.3",
            "install.sh",
        );

        assert_eq!(
            url,
            "https://github.com/eastreams/loong/releases/download/v1.2.3/install.sh"
        );
    }

    #[test]
    fn install_prefix_for_current_executable_uses_parent_directory() {
        let prefix = install_prefix_for_current_executable(Path::new("/tmp/bin/loong"))
            .expect("parent directory should resolve");

        assert_eq!(prefix, PathBuf::from("/tmp/bin"));
    }

    #[cfg(unix)]
    #[test]
    fn run_update_cli_downloads_latest_stable_installer_and_preserves_prefix() {
        let _env_lock = ENV_LOCK.lock().expect("env lock");
        let temp_dir = TempDir::new().expect("temp dir");
        let prefix_dir = temp_dir.path().join("bin");
        fs::create_dir_all(&prefix_dir).expect("prefix dir");
        let capture_file = temp_dir.path().join("installer-args.txt");
        let env_capture_file = temp_dir.path().join("installer-env.txt");
        let (release_api_url, release_base_url, server_handle) = spawn_release_test_server(
            "v9.9.9".to_owned(),
            capture_file.clone(),
            env_capture_file.clone(),
        );
        let runtime = UpdateRuntimeConfig {
            release_repo: DEFAULT_RELEASE_REPO.to_owned(),
            latest_release_api_url: release_api_url,
            release_base_url: release_base_url.clone(),
        };
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let result = tokio_runtime.block_on(run_update_cli_with_runtime(
            runtime,
            Some(prefix_dir.join("loong")),
        ));
        server_handle.join().expect("server should exit cleanly");

        result.expect("update command should execute the downloaded installer");
        let captured = fs::read_to_string(&capture_file).expect("captured installer args");
        let expected_prefix = prefix_dir.display().to_string();
        assert_eq!(
            captured,
            format!("--prefix\n{expected_prefix}\n--version\nv9.9.9\n")
        );
        let env_capture =
            fs::read_to_string(&env_capture_file).expect("captured installer environment");
        let expected_repo_line = format!("repo={DEFAULT_RELEASE_REPO}\n");
        let expected_base_line = format!("base={release_base_url}\n");
        assert_eq!(
            env_capture,
            format!("{expected_repo_line}{expected_base_line}")
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_update_cli_propagates_runtime_release_overrides_to_installer() {
        let _env_lock = ENV_LOCK.lock().expect("env lock");
        let temp_dir = TempDir::new().expect("temp dir");
        let prefix_dir = temp_dir.path().join("bin");
        fs::create_dir_all(&prefix_dir).expect("prefix dir");
        let capture_file = temp_dir.path().join("installer-args.txt");
        let env_capture_file = temp_dir.path().join("installer-env.txt");
        let release_repo = "example/loong-fork".to_owned();
        let (release_api_url, release_base_url, server_handle) =
            spawn_release_test_server("v2.3.4".to_owned(), capture_file, env_capture_file.clone());
        let runtime = UpdateRuntimeConfig {
            release_repo: release_repo.clone(),
            latest_release_api_url: release_api_url,
            release_base_url: release_base_url.clone(),
        };
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let result = tokio_runtime.block_on(run_update_cli_with_runtime(
            runtime,
            Some(prefix_dir.join("loong")),
        ));
        server_handle.join().expect("server should exit cleanly");

        result.expect("update command should propagate release overrides");
        let env_capture =
            fs::read_to_string(&env_capture_file).expect("captured installer environment");
        let expected_repo_line = format!("repo={release_repo}\n");
        let expected_base_line = format!("base={release_base_url}\n");
        assert_eq!(
            env_capture,
            format!("{expected_repo_line}{expected_base_line}")
        );
    }

    #[cfg(unix)]
    fn spawn_release_test_server(
        stable_tag: String,
        capture_file: PathBuf,
        env_capture_file: PathBuf,
    ) -> (String, String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("local test server addr");
        let handle = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request_buffer = [0_u8; 4096];
                let read_len = stream.read(&mut request_buffer).expect("read request");
                let request = String::from_utf8_lossy(&request_buffer[..read_len]).into_owned();
                let request_line = request.lines().next().unwrap_or_default();

                let (status_line, response_body, content_type) = if request_line
                    .starts_with("GET /repos/eastreams/loong/releases/latest ")
                {
                    (
                        "HTTP/1.1 200 OK",
                        format!(
                            r#"{{"tag_name":"{stable_tag}","draft":false,"prerelease":false}}"#
                        ),
                        "application/json",
                    )
                } else if request_line
                    .starts_with(&format!("GET /releases/download/{stable_tag}/install.sh "))
                {
                    (
                        "HTTP/1.1 200 OK",
                        format!(
                            "#!/usr/bin/env bash\nset -euo pipefail\nprintf 'repo=%s\\n' \"${{LOONG_INSTALL_REPO:-}}\" > \"{}\"\nprintf 'base=%s\\n' \"${{LOONG_INSTALL_RELEASE_BASE_URL:-}}\" >> \"{}\"\nprintf '%s\\n' \"$@\" > \"{}\"\n",
                            env_capture_file.display(),
                            env_capture_file.display(),
                            capture_file.display()
                        ),
                        "text/plain",
                    )
                } else {
                    (
                        "HTTP/1.1 404 Not Found",
                        "not found".to_owned(),
                        "text/plain",
                    )
                };

                let response = format!(
                    "{status_line}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });

        (
            format!("http://{address}/repos/eastreams/loong/releases/latest"),
            format!("http://{address}/releases"),
            handle,
        )
    }
}
