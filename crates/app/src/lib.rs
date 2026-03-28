pub mod acp;
mod advisory_prompt;
pub mod channel;
pub mod chat;
pub mod config;
pub mod context;
pub mod conversation;
pub mod crypto;
#[cfg(feature = "feishu-integration")]
pub mod feishu;
pub mod memory;
pub mod migration;
pub mod presentation;
pub mod prompt;
pub mod provider;
pub mod runtime_env;
mod runtime_identity;
mod runtime_self;
mod runtime_self_continuity;
mod secrets;
pub mod session;
pub mod tools;
pub mod tui_surface;

mod process_env;
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::missing_panics_doc
)]
#[doc(hidden)]
pub mod test_support;

pub use context::KernelContext;
/// Result type for MVP CLI operations.
pub type CliResult<T> = Result<T, String>;

#[cfg(test)]
mod secret_runtime_tests {
    use std::fs;

    use loongclaw_contracts::{SecretRef, SecretResolver};

    use crate::test_support::unique_temp_dir;

    #[test]
    fn default_secret_resolver_reads_file_secret_and_trims_trailing_newline() {
        let temp_dir = unique_temp_dir("secret-resolver-file");
        fs::create_dir_all(&temp_dir).expect("create temp dir");

        let secret_path = temp_dir.join("token.txt");
        fs::write(&secret_path, "file-secret-value\n").expect("write secret file");

        let resolver = crate::secrets::DefaultSecretResolver::default();
        let secret = resolver
            .resolve(&SecretRef::File {
                file: secret_path.clone(),
            })
            .expect("file secret should resolve")
            .expect("file secret should not be empty");

        assert_eq!(secret.expose(), "file-secret-value");

        fs::remove_file(&secret_path).ok();
        fs::remove_dir_all(&temp_dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn default_secret_resolver_reads_exec_secret_output() {
        let resolver = crate::secrets::DefaultSecretResolver::default();
        let secret = resolver
            .resolve(&SecretRef::Exec {
                exec: vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "printf 'exec-secret-value\\n'".to_owned(),
                ],
            })
            .expect("exec secret should resolve")
            .expect("exec secret should not be empty");

        assert_eq!(secret.expose(), "exec-secret-value");
    }
}
