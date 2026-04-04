use super::*;

#[test]
fn validate_config_logging_redacts_raw_config_path_from_tracing_fields() {
    let missing_path = std::env::temp_dir().join(format!(
        "loongclaw-private-config-{}-missing.toml",
        std::process::id()
    ));
    let missing_path = missing_path.display().to_string();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_loongclaw"))
        .args(["validate-config", "--config", missing_path.as_str()])
        .env("LOONGCLAW_LOG", "debug")
        .env("LOONGCLAW_LOG_FORMAT", "compact")
        .output()
        .expect("run loongclaw validate-config");

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr should include one user-facing error"
    );

    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf8");
    let path_mentions = stderr.matches(missing_path.as_str()).count();

    assert_eq!(
        path_mentions, 1,
        "raw config path should only appear in the final user-facing error, got: {stderr}"
    );
    assert!(
        stderr.contains("command_kind=validate_config"),
        "expected sanitized command kind in logs, got: {stderr}"
    );
    assert!(
        stderr.contains("error_code="),
        "expected stable error code field in logs, got: {stderr}"
    );
}
