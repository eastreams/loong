use super::*;

#[tokio::test]
#[cfg(unix)]
async fn runtime_backend_executes_session_turn_and_controls_when_path_is_narrowed() {
    let _lock = lock_acpx_runtime_tests().await;
    let temp_dir = unique_temp_dir("loong-acpx-runtime-narrow-path");
    let log_path = temp_dir.join("calls.log");
    let script_path = write_fake_acpx_script(
        &temp_dir,
        "fake-acpx",
        &log_path,
        r#"
case "$*" in
  "--version")
    echo 'acpx 0.1.16'
    exit 0
    ;;
esac

if printf '%s' "$*" | grep -q 'sessions ensure --name'; then
  echo '{"acpxSessionId":"sess-42","agentSessionId":"agent-42","acpxRecordId":"record-42"}'
  exit 0
fi

if printf '%s' "$*" | grep -q 'prompt --session'; then
  cat >/dev/null
  echo '{"type":"text","content":"hello "}'
  echo '{"type":"text","content":"world"}'
  echo '{"type":"usage_update","used":7,"size":128}'
  echo '{"type":"done"}'
  exit 0
fi

if printf '%s' "$*" | grep -q 'status --session'; then
  echo '{"status":"ready","acpxSessionId":"sess-42","agentSessionId":"agent-42","acpxRecordId":"record-42"}'
  exit 0
fi

exit 0
"#,
    );
    let mut env = crate::test_support::ScopedEnv::new();
    env.set("PATH", &temp_dir);
    let config = fake_acpx_config(&script_path, &temp_dir);
    let backend = AcpxCliProbeBackend;
    let bootstrap = AcpSessionBootstrap {
        session_key: "agent:codex:session-42".to_owned(),
        conversation_id: Some("telegram:42".to_owned()),
        binding: Some(crate::acp::AcpSessionBindingScope {
            route_session_id: "telegram:bot_123456:42".to_owned(),
            channel_id: Some("telegram".to_owned()),
            account_id: Some("bot_123456".to_owned()),
            conversation_id: Some("42".to_owned()),
            participant_id: None,
            thread_id: Some("thread-42".to_owned()),
        }),
        working_directory: Some(temp_dir.clone()),
        initial_prompt: None,
        mode: Some(AcpSessionMode::Interactive),
        mcp_servers: Vec::new(),
        metadata: BTreeMap::new(),
    };

    let handle = backend
        .ensure_session(&config, &bootstrap)
        .await
        .expect("ensure session");
    let result = backend
        .run_turn(
            &config,
            &handle,
            &AcpTurnRequest {
                session_key: bootstrap.session_key.clone(),
                input: "hello runtime".to_owned(),
                working_directory: None,
                metadata: BTreeMap::new(),
            },
        )
        .await
        .expect("run turn");

    assert_eq!(result.output_text, "hello world");
    assert_eq!(
        result.usage,
        Some(serde_json::json!({
            "used": 7,
            "size": 128,
        }))
    );
}

#[tokio::test]
#[cfg(unix)]
async fn runtime_backend_supports_local_abort_when_path_is_narrowed() {
    let _lock = lock_acpx_runtime_tests().await;
    let temp_dir = unique_temp_dir("loong-acpx-abort-narrow-path");
    let log_path = temp_dir.join("calls.log");
    let script_path = write_fake_acpx_script(
        &temp_dir,
        "fake-acpx",
        &log_path,
        r#"
case "$*" in
  "--version")
    echo 'acpx 0.1.16'
    exit 0
    ;;
esac

if printf '%s' "$*" | grep -q 'sessions ensure --name'; then
  echo '{"acpxSessionId":"sess-abort","agentSessionId":"agent-abort","acpxRecordId":"record-abort"}'
  exit 0
fi

if printf '%s' "$*" | grep -q 'prompt --session'; then
  cat >/dev/null
  sleep 30
  exit 0
fi

exit 0
"#,
    );
    let mut env = crate::test_support::ScopedEnv::new();
    env.set("PATH", &temp_dir);
    let config = fake_acpx_config(&script_path, &temp_dir);
    let backend = AcpxCliProbeBackend;
    let bootstrap = AcpSessionBootstrap {
        session_key: "agent:codex:session-abort".to_owned(),
        conversation_id: Some("telegram:abort".to_owned()),
        binding: None,
        working_directory: Some(temp_dir.clone()),
        initial_prompt: None,
        mode: Some(AcpSessionMode::Interactive),
        mcp_servers: Vec::new(),
        metadata: BTreeMap::new(),
    };
    let handle = backend
        .ensure_session(&config, &bootstrap)
        .await
        .expect("ensure abortable session");

    let abort_controller = crate::acp::AcpAbortController::new();
    let abort_signal = abort_controller.signal();
    let turn_task = {
        let backend = AcpxCliProbeBackend;
        let config = config.clone();
        let handle = handle.clone();
        let session_key = bootstrap.session_key.clone();
        tokio::spawn(async move {
            backend
                .run_turn_with_sink(
                    &config,
                    &handle,
                    &AcpTurnRequest {
                        session_key,
                        input: "abort me".to_owned(),
                        working_directory: None,
                        metadata: BTreeMap::new(),
                    },
                    Some(abort_signal),
                    None,
                )
                .await
        })
    };

    // Give the fake turn a conservative 150ms head start before aborting so the
    // cancellation path is definitely active; the surrounding 2s timeout keeps
    // the test comfortably away from CI timing flake territory.
    tokio::time::sleep(Duration::from_millis(150)).await;
    abort_controller.abort();

    let result = tokio::time::timeout(Duration::from_secs(2), async {
        turn_task
            .await
            .expect("abortable turn join should succeed")
            .expect("abortable turn result should resolve")
    })
    .await
    .expect("aborted prompt should stop promptly");

    assert_eq!(result.stop_reason, Some(AcpTurnStopReason::Cancelled));
    assert_eq!(result.output_text, "");
}

#[tokio::test]
#[cfg(unix)]
async fn ensure_session_falls_back_to_sessions_new_when_path_is_narrowed() {
    let _lock = lock_acpx_runtime_tests().await;
    let temp_dir = unique_temp_dir("loong-acpx-fallback-narrow-path");
    let log_path = temp_dir.join("calls.log");
    let script_path = write_fake_acpx_script(
        &temp_dir,
        "fake-acpx",
        &log_path,
        r#"
case "$*" in
  "--version")
    echo 'acpx 0.1.16'
    exit 0
    ;;
esac

if printf '%s' "$*" | grep -q 'sessions ensure --name'; then
  echo '{}'
  exit 0
fi

if printf '%s' "$*" | grep -q 'sessions new --name'; then
  echo '{"acpxSessionId":"sess-fallback","agentSessionId":"agent-fallback","acpxRecordId":"record-fallback"}'
  exit 0
fi

exit 0
"#,
    );
    let mut env = crate::test_support::ScopedEnv::new();
    env.set("PATH", &temp_dir);
    let config = fake_acpx_config(&script_path, &temp_dir);
    let backend = AcpxCliProbeBackend;

    let handle = backend
        .ensure_session(
            &config,
            &AcpSessionBootstrap {
                session_key: "session-fallback".to_owned(),
                conversation_id: None,
                binding: None,
                working_directory: Some(temp_dir.clone()),
                initial_prompt: None,
                mode: Some(AcpSessionMode::Interactive),
                mcp_servers: Vec::new(),
                metadata: BTreeMap::new(),
            },
        )
        .await
        .expect("fallback ensure");

    assert_eq!(handle.backend_session_id.as_deref(), Some("sess-fallback"));
    assert_eq!(handle.agent_session_id.as_deref(), Some("agent-fallback"));
    let log = std::fs::read_to_string(&log_path).expect("read fake acpx log");
    assert!(
        log.contains("sessions ensure --name session-fallback"),
        "expected ensure attempt in log: {log}"
    );
    assert!(
        log.contains("sessions new --name session-fallback"),
        "expected fallback sessions new in log: {log}"
    );
}
