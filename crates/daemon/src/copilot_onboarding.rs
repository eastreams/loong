use std::path::{Path, PathBuf};

use loongclaw_contracts::SecretRef;

use crate::provider_credential_policy;
use crate::{CliResult, mvp};

pub(crate) async fn finalize_github_copilot_onboard_credentials(
    provider: &mut mvp::config::ProviderConfig,
    output_path: &Path,
    non_interactive: bool,
) -> CliResult<()> {
    let available_binding =
        provider_credential_policy::provider_available_credential_env_binding(provider);
    if let Some(binding) = available_binding
        && binding.field == provider_credential_policy::ProviderCredentialEnvField::OAuthAccessToken
    {
        provider_credential_policy::apply_provider_credential_env_binding(provider, &binding);
        return Ok(());
    }

    let resolved_token = provider.oauth_access_token();
    if resolved_token.is_some() {
        return Ok(());
    }

    if non_interactive {
        let env_name = provider
            .configured_oauth_access_token_env_override()
            .or_else(|| {
                provider
                    .kind
                    .default_oauth_access_token_env()
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "an OAuth token environment variable".to_owned());
        let message = format!(
            "GitHub Copilot onboarding needs an OAuth token in --non-interactive mode; set {env_name} or configure provider.oauth_access_token first"
        );
        return Err(message);
    }

    tracing::warn!("GitHub Copilot uses an undocumented API. It may break without notice.");

    let token = mvp::provider::copilot_device_code_login().await?;

    persist_github_copilot_oauth_token(provider, output_path, token.as_str())?;

    Ok(())
}

fn persist_github_copilot_oauth_token(
    provider: &mut mvp::config::ProviderConfig,
    output_path: &Path,
    token: &str,
) -> CliResult<()> {
    let trimmed_token = token.trim();
    if trimmed_token.is_empty() {
        return Err("GitHub Copilot OAuth token was empty after device login.".to_owned());
    }

    let secret_path = github_copilot_oauth_token_path(output_path);

    write_github_copilot_oauth_token_file(secret_path.as_path(), trimmed_token)?;

    provider.api_key = None;
    provider.clear_api_key_env_binding();
    provider.clear_oauth_access_token_env_binding();
    provider.oauth_access_token = Some(SecretRef::File { file: secret_path });

    Ok(())
}

fn github_copilot_oauth_token_path(output_path: &Path) -> PathBuf {
    let parent_dir = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(mvp::config::default_loongclaw_home);

    parent_dir
        .join("secrets")
        .join("github-copilot-oauth-token")
}

fn write_github_copilot_oauth_token_file(path: &Path, token: &str) -> CliResult<()> {
    create_github_copilot_oauth_token_parent_dir(path)?;
    harden_github_copilot_oauth_token_parent_dir(path)?;
    std::fs::write(path, token).map_err(|error| {
        format!(
            "write GitHub Copilot OAuth token file failed for {}: {error}",
            path.display()
        )
    })?;
    harden_github_copilot_oauth_token_file(path)?;

    Ok(())
}

fn create_github_copilot_oauth_token_parent_dir(path: &Path) -> CliResult<()> {
    let parent = path.parent();
    let Some(parent) = parent else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }

    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "create GitHub Copilot OAuth token parent directory failed for {}: {error}",
            parent.display()
        )
    })
}

#[cfg(unix)]
fn harden_github_copilot_oauth_token_parent_dir(path: &Path) -> CliResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let parent = path.parent();
    let Some(parent) = parent else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() || !parent.exists() {
        return Ok(());
    }

    let metadata = std::fs::metadata(parent).map_err(|error| {
        format!(
            "read GitHub Copilot OAuth token directory metadata failed for {}: {error}",
            parent.display()
        )
    })?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(parent, permissions).map_err(|error| {
        format!(
            "set GitHub Copilot OAuth token directory permissions failed for {}: {error}",
            parent.display()
        )
    })
}

#[cfg(not(unix))]
fn harden_github_copilot_oauth_token_parent_dir(_path: &Path) -> CliResult<()> {
    Ok(())
}

#[cfg(unix)]
fn harden_github_copilot_oauth_token_file(path: &Path) -> CliResult<()> {
    use std::os::unix::fs::PermissionsExt;

    if !path.exists() {
        return Ok(());
    }

    let metadata = std::fs::metadata(path).map_err(|error| {
        format!(
            "read GitHub Copilot OAuth token file metadata failed for {}: {error}",
            path.display()
        )
    })?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o600);
    std::fs::set_permissions(path, permissions).map_err(|error| {
        format!(
            "set GitHub Copilot OAuth token file permissions failed for {}: {error}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn harden_github_copilot_oauth_token_file(_path: &Path) -> CliResult<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn persist_github_copilot_oauth_token_writes_file_secret() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let temp_dir =
            std::env::temp_dir().join(format!("loongclaw-github-copilot-secret-{unique_suffix}"));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let output_path = temp_dir.join("loongclaw.toml");
        let mut provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::GithubCopilot,
            api_key: Some(SecretRef::Inline("stale-api-key".to_owned())),
            oauth_access_token: Some(SecretRef::Env {
                env: "STALE_GITHUB_COPILOT_OAUTH_TOKEN".to_owned(),
            }),
            ..mvp::config::ProviderConfig::default()
        };

        persist_github_copilot_oauth_token(&mut provider, &output_path, "ghu_copilot_token")
            .expect("persist GitHub Copilot token");

        let Some(SecretRef::File { file }) = provider.oauth_access_token.as_ref() else {
            panic!("GitHub Copilot token should persist as a file secret");
        };

        let stored_token =
            std::fs::read_to_string(file).expect("read persisted GitHub Copilot token");

        assert_eq!(stored_token, "ghu_copilot_token");
        assert_eq!(provider.api_key, None);
        assert_eq!(provider.api_key_env, None);
        assert_eq!(provider.oauth_access_token_env, None);

        std::fs::remove_dir_all(&temp_dir).expect("remove temp dir");
    }
}
