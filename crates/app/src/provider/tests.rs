use super::*;
use crate::KernelContext;
use crate::config::{LoongClawConfig, ProviderConfig, ReasoningEffort};
use crate::test_support::ScopedEnv;
use loongclaw_contracts::{Capability, ExecutionRoute, HarnessKind, SecretRef};
use loongclaw_kernel::{
    AuditEventKind, FixedClock, InMemoryAuditSink, LoongClawKernel, StaticPolicyEngine,
    VerticalPackManifest,
};
use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Barrier, Notify};

const OPENAI_AUTH_ENV_KEYS: &[&str] = &[
    "OPENAI_CODEX_OAUTH_TOKEN",
    "OPENAI_OAUTH_ACCESS_TOKEN",
    "OPENAI_API_KEY",
];
const VOLCENGINE_AUTH_ENV_KEYS: &[&str] = &["ARK_API_KEY"];

fn build_provider_failover_test_kernel_context(
    agent_id: &str,
) -> (KernelContext, Arc<InMemoryAuditSink>) {
    let audit = Arc::new(InMemoryAuditSink::default());
    let clock = Arc::new(FixedClock::new(1_700_000_321));
    let mut kernel =
        LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone());
    kernel
        .register_pack(VerticalPackManifest {
            pack_id: "provider-test-pack".to_owned(),
            domain: "provider-test".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: BTreeMap::new(),
        })
        .expect("register test pack");
    let token = kernel
        .issue_token("provider-test-pack", agent_id, 3_600)
        .expect("issue test token");
    (
        KernelContext {
            kernel: Arc::new(kernel),
            token,
        },
        audit,
    )
}

fn build_profile_state_policy_for_test(namespace: String) -> ProviderProfileStatePolicy {
    ProviderProfileStatePolicy {
        namespace,
        health_mode: ProviderProfileHealthMode::EnforceUnusableWindows,
        cooldown: Duration::from_secs(30),
        max_cooldown: Duration::from_secs(600),
        auth_reject_disable: Duration::from_secs(3600),
        max_entries: 128,
    }
}

fn next_profile_test_namespace() -> String {
    static NEXT_PROFILE_TEST_NAMESPACE: AtomicUsize = AtomicUsize::new(1);
    let seed = NEXT_PROFILE_TEST_NAMESPACE.fetch_add(1, Ordering::Relaxed);
    format!("provider-profile-test-{seed}")
}

fn next_model_cooldown_test_namespace() -> String {
    static NEXT_MODEL_COOLDOWN_TEST_NAMESPACE: AtomicUsize = AtomicUsize::new(1);
    let seed = NEXT_MODEL_COOLDOWN_TEST_NAMESPACE.fetch_add(1, Ordering::Relaxed);
    format!("model-cooldown-test-{seed}")
}

fn next_temp_path(prefix: &str, extension: &str) -> PathBuf {
    static NEXT_TEMP_PATH_SEED: AtomicUsize = AtomicUsize::new(1);
    let seed = NEXT_TEMP_PATH_SEED.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{seed}.{extension}",
        std::process::id()
    ))
}

fn read_local_provider_request(stream: &mut std::net::TcpStream, deadline: Instant) -> String {
    let timeout = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(1));
    stream
        .set_nonblocking(false)
        .expect("set accepted stream blocking");
    stream
        .set_read_timeout(Some(timeout))
        .expect("set accepted stream read timeout");
    let mut request_buf = [0_u8; 8192];
    let len = stream.read(&mut request_buf).expect("read request");
    String::from_utf8_lossy(&request_buf[..len]).to_string()
}

#[test]
fn read_local_provider_request_accepts_elapsed_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider listener");
    let addr = listener.local_addr().expect("local addr");
    let (written_tx, written_rx) = std::sync::mpsc::channel();
    let client = std::thread::spawn(move || {
        let mut stream = std::net::TcpStream::connect(addr).expect("connect local provider");
        stream
            .write_all(b"GET /models HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect("write local provider request");
        stream.flush().expect("flush local provider request");
        written_tx
            .send(())
            .expect("notify local provider request write");
    });

    let (mut stream, _) = listener.accept().expect("accept local provider");
    written_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("wait for local provider request write");

    let request = read_local_provider_request(&mut stream, Instant::now());
    assert!(request.starts_with("GET /models HTTP/1.1"));

    client.join().expect("join local provider client");
}

#[tokio::test]
async fn provider_auth_ready_accepts_x_api_key_providers() {
    let config = LoongClawConfig {
        provider: ProviderConfig {
            kind: ProviderKind::Anthropic,
            api_key: Some(SecretRef::Inline("anthropic-secret".to_owned())),
            ..ProviderConfig::default()
        },
        ..LoongClawConfig::default()
    };

    assert!(provider_auth_ready(&config).await);
}

#[tokio::test]
async fn provider_auth_ready_accepts_manual_auth_headers_for_custom_provider() {
    let config = LoongClawConfig {
        provider: ProviderConfig {
            kind: ProviderKind::Custom,
            headers: BTreeMap::from([("authorization".to_owned(), "Token manual-auth".to_owned())]),
            ..ProviderConfig::default()
        },
        ..LoongClawConfig::default()
    };

    assert!(provider_auth_ready(&config).await);
}

#[cfg(feature = "provider-bedrock")]
#[tokio::test]
async fn provider_auth_ready_accepts_bedrock_sigv4_credentials() {
    let mut env = ScopedEnv::new();
    env.set("AWS_ACCESS_KEY_ID", "test-access-key");
    env.set("AWS_SECRET_ACCESS_KEY", "test-secret-key");
    env.set("AWS_REGION", "us-west-2");
    env.remove("AWS_SESSION_TOKEN");

    let config = LoongClawConfig {
        provider: ProviderConfig {
            kind: ProviderKind::Bedrock,
            ..ProviderConfig::default()
        },
        ..LoongClawConfig::default()
    };

    assert!(provider_auth_ready(&config).await);
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_available_models_rejects_missing_volcengine_credentials_before_network_request() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider listener");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let deadline = Instant::now() + Duration::from_millis(250);
        let mut requests = Vec::new();
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_local_provider_request(&mut stream, deadline);
                    requests.push(request);
                    let body = r#"{"error":{"message":"unexpected request"}}"#;
                    let response = format!(
                        "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write response");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::yield_now();
                }
                Err(error) => panic!("accept local provider request: {error}"),
            }
        }
        requests
    });

    let provider = ProviderConfig {
        kind: ProviderKind::Volcengine,
        base_url: format!("http://{addr}"),
        model: "auto".to_owned(),
        api_key: None,
        api_key_env: None,
        oauth_access_token: None,
        oauth_access_token_env: None,
        ..ProviderConfig::default()
    };
    let mut env = ScopedEnv::new();
    clear_provider_auth_envs(&mut env, VOLCENGINE_AUTH_ENV_KEYS);
    let config = test_config(provider);

    let error = fetch_available_models(&config)
        .await
        .expect_err("missing volcengine credentials should fail before any network request");

    assert!(
        error.contains("provider credentials are missing"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("ARK_API_KEY"),
        "missing-credential guidance should mention the Volcengine env binding: {error}"
    );

    let requests = server.join().expect("join local provider server");
    assert!(
        requests.is_empty(),
        "missing managed credentials should not fall through to an anonymous model-list request: {requests:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_available_models_enriches_volcengine_auth_failures_with_ark_guidance() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider listener");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let deadline = Instant::now() + Duration::from_millis(250);
        let mut requests = Vec::new();
        loop {
            if Instant::now() >= deadline {
                panic!("timed out waiting for local provider request");
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_local_provider_request(&mut stream, deadline);
                    requests.push(request);

                    let body = r#"{"error":{"code":"AuthenticationError","message":"the API key or AK/SK in the request is missing or invalid","type":"Unauthorized"}}"#;
                    let response = format!(
                        "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write response");
                    return requests;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::yield_now();
                }
                Err(error) => panic!("accept local provider request: {error}"),
            }
        }
    });

    let config = test_config(ProviderConfig {
        kind: ProviderKind::VolcengineCoding,
        base_url: format!("http://{addr}"),
        model: "auto".to_owned(),
        api_key: Some(SecretRef::Inline("bad-ark-key".to_owned())),
        api_key_env: None,
        ..ProviderConfig::default()
    });

    let error = fetch_available_models(&config)
        .await
        .expect_err("volcengine auth failures should surface actionable guidance");

    assert!(error.contains("status 401"), "unexpected error: {error}");
    assert!(
        error.contains("Authorization: Bearer <ARK_API_KEY>"),
        "volcengine auth failures should explain the supported auth shape: {error}"
    );
    assert!(
        error.contains("AK/SK request signing is not used"),
        "volcengine auth failures should explain the unsupported auth path clearly: {error}"
    );

    let requests = server.join().expect("join local provider server");
    assert!(
        requests.iter().any(|request| {
            let normalized = request.to_ascii_lowercase();
            request.starts_with("GET /models ")
                && normalized.contains("authorization: bearer bad-ark-key")
        }),
        "the catalog probe should still use the configured bearer secret when credentials exist: {requests:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn request_turn_auto_model_rejects_missing_volcengine_credentials_before_transport() {
    let provider = ProviderConfig {
        kind: ProviderKind::VolcengineCoding,
        base_url: "http://127.0.0.1:1".to_owned(),
        model: "auto".to_owned(),
        api_key: None,
        api_key_env: None,
        oauth_access_token: None,
        oauth_access_token_env: None,
        ..ProviderConfig::default()
    };
    let mut env = ScopedEnv::new();
    clear_provider_auth_envs(&mut env, VOLCENGINE_AUTH_ENV_KEYS);
    let config = test_config(provider);

    let error = request_turn(
        &config,
        "session-provider-test",
        "turn-provider-test",
        &[json!({
            "role": "user",
            "content": "ping"
        })],
        ProviderRuntimeBinding::direct(),
    )
    .await
    .expect_err("auto-model requests should fail on missing managed credentials before transport");

    assert!(
        error.contains("provider credentials are missing"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("ARK_API_KEY"),
        "missing-credential request errors should preserve the provider env hint: {error}"
    );
    assert!(
        !error.contains("Connection refused"),
        "missing managed credentials should fail before a transport attempt: {error}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_available_models_rejects_missing_openai_credentials_before_transport() {
    let provider = ProviderConfig {
        kind: ProviderKind::Openai,
        base_url: "http://127.0.0.1:1".to_owned(),
        model: "auto".to_owned(),
        api_key: None,
        api_key_env: None,
        oauth_access_token: None,
        oauth_access_token_env: None,
        ..ProviderConfig::default()
    };
    let mut env = ScopedEnv::new();
    clear_provider_auth_envs(&mut env, OPENAI_AUTH_ENV_KEYS);
    let config = test_config(provider);

    let error = fetch_available_models(&config)
        .await
        .expect_err("missing OpenAI credentials should fail before transport");

    assert!(
        error.contains("provider credentials are missing"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("OPENAI_CODEX_OAUTH_TOKEN"),
        "openai guidance should preserve the oauth default hint: {error}"
    );
    assert!(
        error.contains("OPENAI_API_KEY"),
        "openai guidance should preserve the api key fallback hint: {error}"
    );
    assert!(
        !error.contains("Connection refused"),
        "missing managed credentials should fail before a transport attempt: {error}"
    );
}

#[test]
fn byteplus_auth_guidance_uses_byteplus_api_key_env() {
    let provider = ProviderConfig {
        kind: ProviderKind::ByteplusCoding,
        ..ProviderConfig::default()
    };

    let hint = provider
        .auth_guidance_hint()
        .expect("byteplus coding should expose auth guidance");

    assert!(hint.contains("BytePlus"));
    assert!(hint.contains("BYTEPLUS_API_KEY"));
    assert!(hint.contains("Authorization: Bearer <BYTEPLUS_API_KEY>"));
}

#[test]
fn custom_missing_auth_configuration_message_mentions_supported_headers() {
    let provider = ProviderConfig {
        kind: ProviderKind::Custom,
        ..ProviderConfig::default()
    };

    let message = provider.missing_auth_configuration_message();

    assert!(message.contains("CUSTOM_PROVIDER_API_KEY"));
    assert!(message.contains("Authorization"));
    assert!(message.contains("provider.headers"));
}

#[test]
fn bedrock_missing_auth_configuration_message_mentions_sigv4_fallback() {
    let provider = ProviderConfig {
        kind: ProviderKind::Bedrock,
        ..ProviderConfig::default()
    };

    let message = provider.missing_auth_configuration_message();

    assert!(message.contains("AWS_BEARER_TOKEN_BEDROCK"));
    assert!(message.contains("AWS_ACCESS_KEY_ID"));
    assert!(message.contains("AWS_SECRET_ACCESS_KEY"));
    assert!(message.contains("AWS_REGION"));
}

fn cleanup_sqlite_artifacts(path: &Path) {
    let _ = std::fs::remove_file(path);
    let wal = format!("{}-wal", path.display());
    let shm = format!("{}-shm", path.display());
    let _ = std::fs::remove_file(wal);
    let _ = std::fs::remove_file(shm);
}

fn clear_provider_auth_envs(env: &mut ScopedEnv, env_keys: &[&'static str]) {
    for key in env_keys {
        env.remove(key);
    }
}

fn test_config(provider: ProviderConfig) -> LoongClawConfig {
    LoongClawConfig {
        provider,
        ..LoongClawConfig::default()
    }
}

#[test]
fn resolve_provider_auth_profiles_prefers_oauth_then_api_key() {
    let provider = ProviderConfig {
        kind: ProviderKind::Ollama,
        oauth_access_token: Some(SecretRef::Inline("oauth-token".to_owned())),
        api_key: Some(SecretRef::Inline("api-key".to_owned())),
        api_key_env: None,
        oauth_access_token_env: None,
        ..ProviderConfig::default()
    };

    let profiles = resolve_provider_auth_profiles(&provider);
    assert_eq!(profiles.len(), 2);
    assert!(profiles[0].id.starts_with("oauth:"));
    assert_eq!(
        profiles[0].authorization_header.as_deref(),
        Some("Bearer oauth-token")
    );
    assert!(profiles[1].id.starts_with("api_key:"));
    assert_eq!(
        profiles[1].authorization_header.as_deref(),
        Some("Bearer api-key")
    );
    assert_eq!(profiles[0].x_api_key_header, None);
    assert_eq!(profiles[1].x_api_key_header, None);
}

#[test]
fn resolve_provider_auth_profiles_expands_delimited_api_key_pool() {
    let provider = ProviderConfig {
        kind: ProviderKind::Ollama,
        api_key: Some(SecretRef::Inline(
            "api-key-a, api-key-b;api-key-c".to_owned(),
        )),
        api_key_env: None,
        ..ProviderConfig::default()
    };

    let profiles = resolve_provider_auth_profiles(&provider);
    assert_eq!(profiles.len(), 3);
    assert_eq!(
        profiles
            .iter()
            .map(|profile| profile.authorization_header.clone())
            .collect::<Vec<_>>(),
        vec![
            Some("Bearer api-key-a".to_owned()),
            Some("Bearer api-key-b".to_owned()),
            Some("Bearer api-key-c".to_owned())
        ]
    );
}

#[test]
fn provider_profile_health_prioritizes_available_profile() {
    let policy = build_profile_state_policy_for_test(next_profile_test_namespace());
    let first = ProviderAuthProfile {
        id: "profile-a".to_owned(),
        authorization_header: Some("Bearer a".to_owned()),
        x_api_key_header: None,
        auth_cache_key: Some("Bearer a".to_owned()),
    };
    let second = ProviderAuthProfile {
        id: "profile-b".to_owned(),
        authorization_header: Some("Bearer b".to_owned()),
        x_api_key_header: None,
        auth_cache_key: Some("Bearer b".to_owned()),
    };

    mark_provider_profile_failure(&policy, &first, ProviderFailoverReason::RateLimited);
    let prioritized = prioritize_provider_auth_profiles_by_health(
        &[first.clone(), second.clone()],
        Some(&policy),
    );

    assert_eq!(prioritized.len(), 2);
    assert_eq!(prioritized[0].id, second.id);
    assert_eq!(prioritized[1].id, first.id);
}

#[test]
fn build_provider_profile_state_policy_uses_provider_config_values() {
    let provider = ProviderConfig {
        kind: ProviderKind::Ollama,
        profile_cooldown_ms: 12_000,
        profile_cooldown_max_ms: 48_000,
        profile_auth_reject_disable_ms: 240_000,
        profile_state_max_entries: 77,
        ..ProviderConfig::default()
    };
    let policy = build_provider_profile_state_policy(
        &provider,
        "https://example.com/v1/chat/completions",
        &reqwest::header::HeaderMap::new(),
    )
    .expect("profile state policy");

    assert_eq!(
        policy.health_mode,
        ProviderProfileHealthMode::EnforceUnusableWindows
    );
    assert_eq!(policy.cooldown, Duration::from_millis(12_000));
    assert_eq!(policy.max_cooldown, Duration::from_millis(48_000));
    assert_eq!(policy.auth_reject_disable, Duration::from_millis(240_000));
    assert_eq!(policy.max_entries, 77);
}

#[test]
fn provider_profile_state_policy_uses_observe_only_mode_for_openrouter() {
    let provider = ProviderConfig {
        kind: ProviderKind::Openrouter,
        ..ProviderConfig::default()
    };
    let policy = build_provider_profile_state_policy(
        &provider,
        "https://openrouter.ai/api/v1/chat/completions",
        &reqwest::header::HeaderMap::new(),
    )
    .expect("profile state policy");

    assert_eq!(policy.health_mode, ProviderProfileHealthMode::ObserveOnly);
}

#[test]
fn provider_profile_state_policy_honors_explicit_health_mode_override() {
    let provider = ProviderConfig {
        kind: ProviderKind::Openrouter,
        profile_health_mode: ProviderProfileHealthModeConfig::Enforce,
        ..ProviderConfig::default()
    };
    let policy = build_provider_profile_state_policy(
        &provider,
        "https://openrouter.ai/api/v1/chat/completions",
        &reqwest::header::HeaderMap::new(),
    )
    .expect("profile state policy");

    assert_eq!(
        policy.health_mode,
        ProviderProfileHealthMode::EnforceUnusableWindows
    );
}

#[test]
fn provider_profile_health_observe_only_mode_bypasses_cooldown_windows() {
    let mut policy = build_profile_state_policy_for_test(next_profile_test_namespace());
    policy.health_mode = ProviderProfileHealthMode::ObserveOnly;
    let profile = ProviderAuthProfile {
        id: "profile-observe".to_owned(),
        authorization_header: Some("Bearer observe".to_owned()),
        x_api_key_header: None,
        auth_cache_key: Some("Bearer observe".to_owned()),
    };

    mark_provider_profile_failure(&policy, &profile, ProviderFailoverReason::RateLimited);

    let state_key =
        build_provider_profile_state_key(policy.namespace.as_str(), profile.id.as_str());
    let snapshot = with_provider_profile_states(|store| {
        store.health_snapshot(state_key.as_str(), Instant::now())
    });
    assert!(snapshot.unusable_until.is_none());

    mark_provider_profile_failure(&policy, &profile, ProviderFailoverReason::AuthRejected);
    let snapshot_after_auth = with_provider_profile_states(|store| {
        store.health_snapshot(state_key.as_str(), Instant::now())
    });
    assert!(snapshot_after_auth.unusable_until.is_none());
}

#[test]
fn provider_profile_state_snapshot_roundtrip_restores_active_entries() {
    let now = Instant::now();
    let mut store = ProviderProfileStateStore::default();
    store.entries.insert(
        "ns::profile-a".to_owned(),
        ProviderProfileStateEntry {
            reason: ProviderFailoverReason::RateLimited,
            failure_count: 2,
            cooldown_until: now.checked_add(Duration::from_secs(30)),
            disabled_until: None,
            last_used_at: now.checked_sub(Duration::from_secs(5)),
        },
    );
    store.entries.insert(
        "ns::profile-b".to_owned(),
        ProviderProfileStateEntry {
            reason: ProviderFailoverReason::AuthRejected,
            failure_count: 1,
            cooldown_until: None,
            disabled_until: now.checked_add(Duration::from_secs(120)),
            last_used_at: None,
        },
    );
    store.order.push_back("ns::profile-a".to_owned());
    store.order.push_back("ns::profile-b".to_owned());

    let snapshot = store.to_snapshot(now);
    let restored = ProviderProfileStateStore::from_snapshot(snapshot, now);

    assert_eq!(restored.entries.len(), 2);
    assert_eq!(restored.order.len(), 2);
    assert_eq!(
        restored.order.iter().cloned().collect::<Vec<_>>(),
        vec!["ns::profile-a".to_owned(), "ns::profile-b".to_owned()]
    );
    let first = restored
        .entries
        .get("ns::profile-a")
        .expect("profile-a state should exist");
    assert_eq!(first.reason, ProviderFailoverReason::RateLimited);
    assert_eq!(first.failure_count, 2);
    assert!(first.cooldown_until.is_some());
    assert!(first.last_used_at.is_some());
    let second = restored
        .entries
        .get("ns::profile-b")
        .expect("profile-b state should exist");
    assert_eq!(second.reason, ProviderFailoverReason::AuthRejected);
    assert!(second.disabled_until.is_some());
}

#[test]
fn provider_profile_state_revision_increments_after_mutations() {
    let now = Instant::now();
    let mut store = ProviderProfileStateStore::default();
    assert_eq!(store.revision, 0);

    store.mark_success("ns::profile-a".to_owned(), now, 16);
    assert_eq!(store.revision, 1);

    let policy = build_profile_state_policy_for_test("ns".to_owned());
    store.mark_failure(
        "ns::profile-a".to_owned(),
        ProviderFailoverReason::RateLimited,
        now,
        &policy,
    );
    assert_eq!(store.revision, 2);

    let snapshot = store.to_snapshot(now);
    assert_eq!(snapshot.revision, 2);
}

#[test]
fn provider_profile_state_snapshot_legacy_payload_defaults_revision() {
    let legacy = json!({
        "version": PROVIDER_PROFILE_STATE_SNAPSHOT_VERSION,
        "generated_at_unix_ms": 1,
        "order": [],
        "entries": [],
    });

    let parsed = serde_json::from_value::<ProviderProfileStateSnapshot>(legacy)
        .expect("legacy snapshot should deserialize");
    assert_eq!(parsed.revision, 0);

    let restored = ProviderProfileStateStore::from_snapshot(parsed, Instant::now());
    assert_eq!(restored.revision, 0);
    assert!(restored.entries.is_empty());
}

#[test]
fn provider_profile_state_snapshot_sorts_entries_by_key_for_stable_persistence() {
    let now = Instant::now();
    let mut store = ProviderProfileStateStore::default();
    store.entries.insert(
        "ns::profile-z".to_owned(),
        ProviderProfileStateEntry {
            reason: ProviderFailoverReason::RateLimited,
            failure_count: 1,
            cooldown_until: now.checked_add(Duration::from_secs(10)),
            disabled_until: None,
            last_used_at: None,
        },
    );
    store.entries.insert(
        "ns::profile-a".to_owned(),
        ProviderProfileStateEntry {
            reason: ProviderFailoverReason::AuthRejected,
            failure_count: 1,
            cooldown_until: None,
            disabled_until: now.checked_add(Duration::from_secs(20)),
            last_used_at: None,
        },
    );

    let snapshot = store.to_snapshot(now);
    let keys = snapshot
        .entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec!["ns::profile-a".to_owned(), "ns::profile-z".to_owned()]
    );
}

#[test]
fn provider_profile_state_backend_defaults_to_in_memory_in_tests() {
    let backend = provider_profile_state_backend();
    let loaded = backend.load_store();
    assert_eq!(loaded.revision, 0);
    assert!(loaded.entries.is_empty());

    let snapshot = ProviderProfileStateSnapshot {
        version: PROVIDER_PROFILE_STATE_SNAPSHOT_VERSION,
        revision: 7,
        generated_at_unix_ms: 0,
        order: Vec::new(),
        entries: Vec::new(),
    };
    backend.persist_snapshot(&snapshot);

    let reloaded = backend.load_store();
    assert_eq!(reloaded.revision, 0);
    assert!(reloaded.entries.is_empty());
}

#[test]
fn provider_profile_state_file_backend_skips_stale_revisions() {
    let path = next_temp_path("provider-profile-state", "json");
    let _ = std::fs::remove_file(&path);
    let backend = FileProviderProfileStateBackend::with_path(path.clone());

    let newest = ProviderProfileStateSnapshot {
        version: PROVIDER_PROFILE_STATE_SNAPSHOT_VERSION,
        revision: 5,
        generated_at_unix_ms: current_unix_timestamp_ms(),
        order: Vec::new(),
        entries: Vec::new(),
    };
    let stale = ProviderProfileStateSnapshot {
        version: PROVIDER_PROFILE_STATE_SNAPSHOT_VERSION,
        revision: 2,
        generated_at_unix_ms: current_unix_timestamp_ms(),
        order: Vec::new(),
        entries: Vec::new(),
    };

    assert_eq!(
        backend.persist_snapshot(&newest),
        ProviderProfileStatePersistOutcome::Persisted
    );
    assert_eq!(
        backend.persist_snapshot(&stale),
        ProviderProfileStatePersistOutcome::StaleSkipped
    );

    let loaded = backend.load_store();
    assert_eq!(loaded.revision, 5);
    let _ = std::fs::remove_file(path);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn provider_profile_state_sqlite_backend_roundtrip_and_stale_guard() {
    let path = next_temp_path("provider-profile-state", "sqlite3");
    cleanup_sqlite_artifacts(&path);
    let backend = SqliteProviderProfileStateBackend::new(path.clone());

    let now = Instant::now();
    let mut store = ProviderProfileStateStore::default();
    let policy = build_profile_state_policy_for_test("provider-profile-sqlite".to_owned());
    store.mark_failure(
        "provider-profile-sqlite::profile-a".to_owned(),
        ProviderFailoverReason::RateLimited,
        now,
        &policy,
    );
    let mut snapshot = store.to_snapshot(now);
    snapshot.revision = 9;

    assert_eq!(
        backend.persist_snapshot(&snapshot),
        ProviderProfileStatePersistOutcome::Persisted
    );
    let loaded = backend.load_store();
    assert_eq!(loaded.revision, 9);
    assert_eq!(loaded.entries.len(), 1);

    let stale = ProviderProfileStateSnapshot {
        version: PROVIDER_PROFILE_STATE_SNAPSHOT_VERSION,
        revision: 3,
        generated_at_unix_ms: current_unix_timestamp_ms(),
        order: Vec::new(),
        entries: Vec::new(),
    };
    assert_eq!(
        backend.persist_snapshot(&stale),
        ProviderProfileStatePersistOutcome::StaleSkipped
    );
    let reloaded = backend.load_store();
    assert_eq!(reloaded.revision, 9);
    assert_eq!(reloaded.entries.len(), 1);

    cleanup_sqlite_artifacts(&path);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn provider_profile_state_sqlite_backend_imports_legacy_json_snapshot() {
    let sqlite_path = next_temp_path("provider-profile-state", "sqlite3");
    let legacy_path = next_temp_path("provider-profile-state-legacy", "json");
    cleanup_sqlite_artifacts(&sqlite_path);
    let _ = std::fs::remove_file(&legacy_path);

    let now = Instant::now();
    let mut store = ProviderProfileStateStore::default();
    let policy = build_profile_state_policy_for_test("provider-profile-legacy".to_owned());
    store.mark_failure(
        "provider-profile-legacy::profile-a".to_owned(),
        ProviderFailoverReason::AuthRejected,
        now,
        &policy,
    );
    let mut snapshot = store.to_snapshot(now);
    snapshot.revision = 11;
    let payload = serde_json::to_vec_pretty(&snapshot).expect("serialize legacy snapshot");
    std::fs::write(&legacy_path, payload).expect("write legacy json snapshot");

    let importing_backend = SqliteProviderProfileStateBackend::with_legacy_fallback(
        sqlite_path.clone(),
        Some(legacy_path.clone()),
    );
    let imported = importing_backend.load_store();
    assert_eq!(imported.revision, 11);
    assert_eq!(imported.entries.len(), 1);

    let sqlite_only_backend = SqliteProviderProfileStateBackend::new(sqlite_path.clone());
    let reloaded = sqlite_only_backend.load_store();
    assert_eq!(reloaded.revision, 11);
    assert_eq!(reloaded.entries.len(), 1);

    cleanup_sqlite_artifacts(&sqlite_path);
    let _ = std::fs::remove_file(legacy_path);
}

#[test]
fn provider_profile_state_persistence_metrics_track_outcomes() {
    let before = provider_profile_state_persistence_metrics_snapshot();
    record_provider_profile_state_persist_outcome(ProviderProfileStatePersistOutcome::Persisted);
    record_provider_profile_state_persist_outcome(ProviderProfileStatePersistOutcome::StaleSkipped);
    record_provider_profile_state_persist_outcome(ProviderProfileStatePersistOutcome::Failed);
    let after = provider_profile_state_persistence_metrics_snapshot();
    assert_eq!(after.persisted, before.persisted + 1);
    assert_eq!(after.stale_skipped, before.stale_skipped + 1);
    assert_eq!(after.failed, before.failed + 1);
}

#[test]
fn provider_profile_state_snapshot_skips_unknown_reason_entries() {
    let snapshot = ProviderProfileStateSnapshot {
        version: PROVIDER_PROFILE_STATE_SNAPSHOT_VERSION,
        revision: 1,
        generated_at_unix_ms: 0,
        order: vec!["unknown".to_owned()],
        entries: vec![ProviderProfileStateSnapshotEntry {
            key: "unknown".to_owned(),
            reason: "unknown_reason".to_owned(),
            failure_count: 1,
            cooldown_remaining_ms: Some(10_000),
            disabled_remaining_ms: None,
            last_used_age_ms: Some(1_000),
        }],
    };

    let restored = ProviderProfileStateStore::from_snapshot(snapshot, Instant::now());
    assert!(restored.entries.is_empty());
    assert!(restored.order.is_empty());
}

#[test]
fn message_builder_includes_system_prompt() {
    let config = test_config(ProviderConfig::default());

    let messages =
        build_messages_for_session(&config, "noop-session", true).expect("build messages");
    assert!(!messages.is_empty());
    assert_eq!(messages[0]["role"], "system");
}

#[test]
fn build_messages_includes_capability_snapshot_block() {
    let config = test_config(ProviderConfig::default());

    let messages =
        build_messages_for_session(&config, "noop-session", true).expect("build messages");
    assert!(!messages.is_empty());
    let system_content = messages[0]["content"].as_str().expect("system content");
    assert!(
        system_content.contains("[tool_discovery_runtime]"),
        "system prompt should contain capability snapshot marker, got: {system_content}"
    );
    assert!(
        system_content.contains("- tool.search: Discover non-core tools"),
        "system prompt should describe tool.search"
    );
    assert!(system_content.contains("- tool.invoke: Invoke a discovered non-core tool"));
    assert!(!system_content.contains("shell.exec"));
    assert!(!system_content.contains("file.read"));
    assert!(!system_content.contains("file.write"));
}

#[test]
fn completion_body_includes_reasoning_effort_when_configured() {
    let mut config = test_config(ProviderConfig::default());
    config.provider.reasoning_effort = Some(ReasoningEffort::High);

    let body = build_completion_request_body(
        &config,
        &[],
        "model-latest",
        CompletionPayloadMode::default_for(&config.provider),
    );
    assert_eq!(body["reasoning_effort"], "high");
}

#[test]
fn responses_completion_body_uses_input_shape_and_responses_specific_fields() {
    let mut config = test_config(ProviderConfig {
        wire_api: crate::config::ProviderWireApi::Responses,
        max_tokens: Some(512),
        ..ProviderConfig::default()
    });
    config.provider.reasoning_effort = Some(ReasoningEffort::High);

    let body = build_completion_request_body(
        &config,
        &[
            json!({
                "role": "system",
                "content": "You are concise."
            }),
            json!({
                "role": "user",
                "content": "ping"
            }),
        ],
        "gpt-5.1-mini",
        CompletionPayloadMode::default_for(&config.provider),
    );
    assert_eq!(body["model"], "gpt-5.1-mini");
    assert_eq!(body["instructions"], "You are concise.");
    assert_eq!(body["input"][0]["role"], "user");
    assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
    assert_eq!(body["input"][0]["content"][0]["text"], "ping");
    assert_eq!(body["max_output_tokens"], 512);
    assert_eq!(body["reasoning"]["effort"], "high");
    assert!(body.get("messages").is_none());
    assert!(body.get("max_completion_tokens").is_none());
    assert!(body.get("reasoning_effort").is_none());
}

#[test]
fn kimi_coding_completion_body_adds_extra_body_thinking() {
    let mut config = test_config(ProviderConfig {
        kind: ProviderKind::KimiCoding,
        ..ProviderConfig::default()
    });
    config.provider.reasoning_effort = Some(ReasoningEffort::High);

    let body = build_completion_request_body(
        &config,
        &[],
        "kimi-for-coding",
        CompletionPayloadMode::default_for(&config.provider),
    );
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(body["extra_body"]["thinking"]["type"], "enabled");
}

#[test]
fn completion_body_model_hint_can_enable_reasoning_extra_body() {
    let mut config = test_config(ProviderConfig {
        kind: ProviderKind::Openai,
        reasoning_extra_body_kimi_model_hints: vec!["thinking-enabled".to_owned()],
        ..ProviderConfig::default()
    });
    config.provider.reasoning_effort = Some(ReasoningEffort::High);

    let body = build_completion_request_body(
        &config,
        &[],
        "gpt-thinking-enabled-v1",
        CompletionPayloadMode::default_for(&config.provider),
    );
    assert_eq!(body["extra_body"]["thinking"]["type"], "enabled");
}

#[test]
fn completion_body_model_hint_can_disable_reasoning_extra_body() {
    let mut config = test_config(ProviderConfig {
        kind: ProviderKind::KimiCoding,
        reasoning_extra_body_omit_model_hints: vec!["coding-lite".to_owned()],
        ..ProviderConfig::default()
    });
    config.provider.reasoning_effort = Some(ReasoningEffort::High);

    let body = build_completion_request_body(
        &config,
        &[],
        "kimi-coding-lite-v1",
        CompletionPayloadMode::default_for(&config.provider),
    );
    assert!(
        body.get("extra_body").is_none(),
        "model hint should suppress extra_body for matching model, got: {body}"
    );
}

#[test]
fn model_catalog_selection_prefers_user_preferences() {
    let config = ProviderConfig {
        model: "auto".to_owned(),
        preferred_models: vec!["model-latest".to_owned(), "model-fallback".to_owned()],
        ..ProviderConfig::default()
    };
    let ranked = rank_model_candidates(
        &config,
        &["model-fallback".to_owned(), "model-latest".to_owned()],
    );
    let selected = ranked.first().expect("model selected");
    assert_eq!(selected, "model-latest");
}

#[test]
fn completion_body_omits_optional_fields_when_not_configured() {
    let config = test_config(ProviderConfig::default());

    let body = build_completion_request_body(
        &config,
        &[],
        "model-latest",
        CompletionPayloadMode::default_for(&config.provider),
    );
    assert!(body.get("max_tokens").is_none());
    assert!(body.get("max_completion_tokens").is_none());
    assert!(body.get("reasoning").is_none());
    assert!(body.get("reasoning_effort").is_none());
}

#[test]
fn anthropic_completion_body_uses_native_messages_shape() {
    let config = LoongClawConfig {
        provider: ProviderConfig {
            kind: ProviderKind::Anthropic,
            max_tokens: Some(2_048),
            ..ProviderConfig::default()
        },
        ..LoongClawConfig::default()
    };
    let messages = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "hello"}),
    ];

    let body = build_completion_request_body(
        &config,
        &messages,
        "claude-test",
        CompletionPayloadMode::default_for(&config.provider),
    );
    assert_eq!(body["system"], "sys");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(body["messages"][0]["content"][0]["text"], "hello");
    assert_eq!(body["max_tokens"], 2_048);
    assert!(body.get("reasoning_effort").is_none());
    assert!(body.get("max_completion_tokens").is_none());
}

#[test]
fn bedrock_completion_body_uses_converse_shape() {
    let config = LoongClawConfig {
        provider: ProviderConfig {
            kind: ProviderKind::Bedrock,
            max_tokens: Some(2_048),
            ..ProviderConfig::default()
        },
        ..LoongClawConfig::default()
    };
    let messages = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "hello"}),
    ];

    let body = build_completion_request_body(
        &config,
        &messages,
        "anthropic.claude-3-7-sonnet-20250219-v1:0",
        CompletionPayloadMode::default_for(&config.provider),
    );
    assert!(body.get("model").is_none());
    assert_eq!(body["system"][0]["text"], "sys");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"][0]["text"], "hello");
    assert_eq!(body["inferenceConfig"]["maxTokens"], 2_048);
    assert_eq!(body["inferenceConfig"]["temperature"], 0.2);
}

#[test]
fn anthropic_headers_use_native_auth_and_version() {
    let provider = ProviderConfig {
        kind: ProviderKind::Anthropic,
        api_key: Some(SecretRef::Inline("anthropic-test-key".to_owned())),
        ..ProviderConfig::default()
    };
    let headers = transport::build_request_headers(&provider).expect("headers");
    assert_eq!(
        headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok()),
        Some("anthropic-test-key")
    );
    assert_eq!(
        headers
            .get("anthropic-version")
            .and_then(|value| value.to_str().ok()),
        Some("2023-06-01")
    );
    assert!(
        headers.get(reqwest::header::AUTHORIZATION).is_none(),
        "anthropic should not fall back to bearer auth headers"
    );
}

#[test]
fn kimi_coding_request_headers_include_default_user_agent() {
    let provider = ProviderConfig {
        kind: ProviderKind::KimiCoding,
        ..ProviderConfig::default()
    };
    let headers = transport::build_request_headers(&provider).expect("headers");
    let user_agent = headers
        .get(reqwest::header::USER_AGENT)
        .expect("default user-agent")
        .to_str()
        .expect("user-agent value");
    assert_eq!(user_agent, "KimiCLI/LoongClaw");
}

#[test]
fn bailian_coding_request_headers_include_default_user_agent() {
    let provider = ProviderConfig {
        kind: ProviderKind::BailianCoding,
        ..ProviderConfig::default()
    };
    let headers = transport::build_request_headers(&provider).expect("headers");
    let user_agent = headers
        .get(reqwest::header::USER_AGENT)
        .expect("default user-agent")
        .to_str()
        .expect("user-agent value");
    assert_eq!(user_agent, "openclaw");
}

#[test]
fn kimi_coding_keeps_explicit_compatible_user_agent() {
    let provider = ProviderConfig {
        kind: ProviderKind::KimiCoding,
        headers: [("User-Agent".to_owned(), "KimiCLI/custom".to_owned())]
            .into_iter()
            .collect(),
        ..ProviderConfig::default()
    };
    let headers = transport::build_request_headers(&provider).expect("headers");
    let user_agent = headers
        .get(reqwest::header::USER_AGENT)
        .expect("explicit user-agent")
        .to_str()
        .expect("user-agent value");
    assert_eq!(user_agent, "KimiCLI/custom");
}

#[cfg(any(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn turn_body_includes_tool_schema_and_auto_choice() {
    let config = test_config(ProviderConfig::default());

    let body = build_turn_request_body(
        &config,
        &[],
        "model-latest",
        CompletionPayloadMode::default_for(&config.provider),
        true,
        &crate::tools::provider_tool_definitions(),
    );
    let tools = body
        .get("tools")
        .and_then(|value| value.as_array())
        .expect("tools array in turn body");
    assert!(!tools.is_empty());
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|item| item.get("function"))
        .filter_map(|function| function.get("name"))
        .filter_map(Value::as_str)
        .collect();

    let expected = vec!["tool_invoke", "tool_search"];

    for expected_name in expected {
        assert!(
            names.contains(&expected_name),
            "turn tool schema is missing expected tool `{expected_name}`; got={names:?}"
        );
    }
    assert_eq!(body["tool_choice"], "auto");
}

#[cfg(any(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn anthropic_turn_body_uses_native_messages_shape_and_tool_schema() {
    let config = LoongClawConfig {
        provider: ProviderConfig {
            kind: ProviderKind::Anthropic,
            ..ProviderConfig::default()
        },
        ..LoongClawConfig::default()
    };
    let messages = vec![
        json!({
            "role": "system",
            "content": "system rules"
        }),
        json!({
            "role": "user",
            "content": "hello"
        }),
        json!({
            "role": "assistant",
            "content": "working"
        }),
        json!({
            "role": "assistant",
            "content": "[tool_result]\n{}"
        }),
        json!({
            "role": "user",
            "content": "continue"
        }),
    ];

    let body = build_turn_request_body(
        &config,
        &messages,
        "claude-3-7-sonnet-latest",
        CompletionPayloadMode::default_for(&config.provider),
        true,
        &crate::tools::provider_tool_definitions(),
    );

    assert_eq!(body["system"], "system rules");
    assert_eq!(body["max_tokens"], 4096);
    let adapted_messages = body["messages"].as_array().expect("anthropic messages");
    assert_eq!(adapted_messages.len(), 3);
    assert_eq!(adapted_messages[0]["role"], "user");
    assert_eq!(adapted_messages[1]["role"], "assistant");
    assert_eq!(
        adapted_messages[1]["content"].as_array().map(Vec::len),
        Some(2)
    );

    let tools = body["tools"].as_array().expect("anthropic tools");
    assert!(!tools.is_empty());
    assert!(tools[0].get("function").is_none());
    assert!(tools[0].get("input_schema").is_some());
    assert_eq!(body["tool_choice"]["type"], "auto");
}

#[cfg(any(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn anthropic_turn_body_converts_tool_schema_to_native_format() {
    let config = LoongClawConfig {
        provider: ProviderConfig {
            kind: ProviderKind::Anthropic,
            ..ProviderConfig::default()
        },
        ..LoongClawConfig::default()
    };

    let body = build_turn_request_body(
        &config,
        &[json!({"role": "user", "content": "inspect README"})],
        "claude-test",
        CompletionPayloadMode::default_for(&config.provider),
        true,
        &crate::tools::provider_tool_definitions(),
    );
    let tools = body
        .get("tools")
        .and_then(Value::as_array)
        .expect("anthropic tools array");
    assert!(!tools.is_empty());
    let first = &tools[0];
    assert!(first.get("function").is_none());
    assert!(first.get("input_schema").is_some());
    assert_eq!(body["tool_choice"]["type"], "auto");
}

#[test]
fn anthropic_turn_body_preserves_native_tool_use_and_tool_result_blocks() {
    let config = LoongClawConfig {
        provider: ProviderConfig {
            kind: ProviderKind::Anthropic,
            ..ProviderConfig::default()
        },
        ..LoongClawConfig::default()
    };

    let body = build_turn_request_body(
        &config,
        &[
            json!({
                "role": "assistant",
                "content": [
                    {
                        "type": "text",
                        "text": "checking"
                    },
                    {
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "file_read",
                        "input": {
                            "path": "README.md"
                        }
                    }
                ]
            }),
            json!({
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "toolu_1",
                        "content": "[ok] {\"path\":\"README.md\"}"
                    },
                    {
                        "type": "text",
                        "text": "Use the tool result above to answer."
                    }
                ]
            }),
        ],
        "claude-test",
        CompletionPayloadMode::default_for(&config.provider),
        true,
        &crate::tools::provider_tool_definitions(),
    );

    let adapted_messages = body["messages"].as_array().expect("anthropic messages");
    assert_eq!(adapted_messages.len(), 2);
    assert_eq!(adapted_messages[0]["role"], "assistant");
    assert_eq!(adapted_messages[0]["content"][1]["type"], "tool_use");
    assert_eq!(adapted_messages[0]["content"][1]["id"], "toolu_1");
    assert_eq!(adapted_messages[1]["role"], "user");
    assert_eq!(adapted_messages[1]["content"][0]["type"], "tool_result");
    assert_eq!(adapted_messages[1]["content"][0]["tool_use_id"], "toolu_1");
    assert_eq!(adapted_messages[1]["content"][1]["type"], "text");
}

#[cfg(any(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn bedrock_turn_body_uses_native_tool_blocks_and_tool_config() {
    let config = LoongClawConfig {
        provider: ProviderConfig {
            kind: ProviderKind::Bedrock,
            ..ProviderConfig::default()
        },
        ..LoongClawConfig::default()
    };

    let body = build_turn_request_body(
        &config,
        &[
            json!({
                "role": "assistant",
                "content": [
                    {
                        "type": "text",
                        "text": "checking"
                    },
                    {
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "file_read",
                        "input": {
                            "path": "README.md"
                        }
                    }
                ]
            }),
            json!({
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "toolu_1",
                        "content": "[ok] README contents"
                    },
                    {
                        "type": "text",
                        "text": "Use the tool result above to answer."
                    }
                ]
            }),
        ],
        "anthropic.claude-3-7-sonnet-20250219-v1:0",
        CompletionPayloadMode::default_for(&config.provider),
        true,
        &crate::tools::provider_tool_definitions(),
    );

    let adapted_messages = body["messages"].as_array().expect("bedrock messages");
    assert_eq!(adapted_messages.len(), 2);
    assert_eq!(adapted_messages[0]["role"], "assistant");
    assert_eq!(
        adapted_messages[0]["content"][1]["toolUse"]["toolUseId"],
        "toolu_1"
    );
    assert_eq!(adapted_messages[1]["role"], "user");
    assert_eq!(
        adapted_messages[1]["content"][0]["toolResult"]["toolUseId"],
        "toolu_1"
    );
    assert_eq!(
        adapted_messages[1]["content"][1]["text"],
        "Use the tool result above to answer."
    );

    let tools = body["toolConfig"]["tools"]
        .as_array()
        .expect("bedrock tools");
    assert!(!tools.is_empty());
    assert!(tools[0].get("toolSpec").is_some());
    assert_eq!(body["toolConfig"]["toolChoice"]["auto"], json!({}));
}

#[test]
fn bedrock_request_endpoint_encodes_model_id_in_path() {
    let provider = ProviderConfig {
        kind: ProviderKind::Bedrock,
        ..ProviderConfig::default()
    };
    let endpoint = transport::resolve_request_endpoint(
        &provider,
        "https://bedrock-runtime.us-west-2.amazonaws.com/model/{modelId}/converse",
        "anthropic.claude-3-7-sonnet-20250219-v1:0",
    );
    assert_eq!(
        endpoint,
        "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.claude-3-7-sonnet-20250219-v1%3A0/converse"
    );
}

#[cfg(any(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn responses_turn_body_keeps_tool_schema_with_responses_input_shape() {
    let config = test_config(ProviderConfig {
        wire_api: crate::config::ProviderWireApi::Responses,
        ..ProviderConfig::default()
    });

    let body = build_turn_request_body(
        &config,
        &[json!({
            "role": "user",
            "content": "read README"
        })],
        "gpt-5.1-mini",
        CompletionPayloadMode::default_for(&config.provider),
        true,
        &crate::tools::provider_tool_definitions(),
    );

    assert_eq!(body["input"][0]["role"], "user");
    assert_eq!(body["input"][0]["content"][0]["text"], "read README");
    assert!(body.get("messages").is_none());
    assert_eq!(body["tool_choice"], "auto");
    assert!(
        body.get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| !tools.is_empty()),
        "responses requests should still carry tool definitions"
    );
}

#[test]
fn responses_turn_body_preserves_native_function_call_roundtrip_items() {
    let config = test_config(ProviderConfig {
        wire_api: crate::config::ProviderWireApi::Responses,
        ..ProviderConfig::default()
    });

    let body = build_turn_request_body(
        &config,
        &[
            json!({
                "role": "assistant",
                "content": "Reading the file now."
            }),
            json!({
                "type": "function_call",
                "name": "file_read",
                "call_id": "call_resp_1",
                "arguments": "{\"path\":\"README.md\"}"
            }),
            json!({
                "type": "function_call_output",
                "call_id": "call_resp_1",
                "output": "[ok] {\"path\":\"README.md\"}"
            }),
            json!({
                "role": "user",
                "content": "Use the tool result above to answer the original request."
            }),
        ],
        "gpt-5.1-mini",
        CompletionPayloadMode::default_for(&config.provider),
        true,
        &crate::tools::provider_tool_definitions(),
    );

    let input = body["input"].as_array().expect("responses input array");
    assert!(
        input.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call")
                && item.get("call_id").and_then(Value::as_str) == Some("call_resp_1")
        }),
        "responses input should preserve function_call items, got: {input:?}"
    );
    assert!(
        input.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call_output")
                && item.get("call_id").and_then(Value::as_str) == Some("call_resp_1")
                && item.get("output").and_then(Value::as_str)
                    == Some("[ok] {\"path\":\"README.md\"}")
        }),
        "responses input should preserve function_call_output items, got: {input:?}"
    );
}

#[test]
fn tool_schema_fallback_detects_unsupported_error_shapes() {
    let runtime_contract = provider_runtime_contract(&ProviderConfig::default());
    let unsupported_tools = json!({
        "error": {
            "code": "unsupported_parameter",
            "param": "tools",
            "message": "Unsupported parameter: tools"
        }
    });
    let unsupported_tool_choice = json!({
        "error": {
            "message": "Function calling is not supported for this model."
        }
    });

    assert!(should_disable_tool_schema_for_error(
        &parse_provider_api_error(&unsupported_tools),
        runtime_contract,
    ));
    assert!(should_disable_tool_schema_for_error(
        &parse_provider_api_error(&unsupported_tool_choice),
        runtime_contract,
    ));
}

#[test]
fn completion_body_uses_provider_token_field_default() {
    let openai = ProviderConfig {
        kind: ProviderKind::Openai,
        max_tokens: Some(512),
        ..ProviderConfig::default()
    };
    let openai_mode = CompletionPayloadMode::default_for(&openai);
    assert_eq!(
        openai_mode.token_field,
        TokenLimitField::MaxCompletionTokens
    );

    let openrouter = ProviderConfig {
        kind: ProviderKind::Openrouter,
        max_tokens: Some(512),
        ..ProviderConfig::default()
    };
    let openrouter_mode = CompletionPayloadMode::default_for(&openrouter);
    assert_eq!(openrouter_mode.token_field, TokenLimitField::MaxTokens);
}

#[test]
fn provider_runtime_contract_defaults_are_stable() {
    let openai_contract = provider_runtime_contract(&ProviderConfig {
        kind: ProviderKind::Openai,
        ..ProviderConfig::default()
    });
    assert_eq!(
        openai_contract.feature_family,
        ProviderFeatureFamily::OpenAiCompatible
    );
    assert_eq!(
        openai_contract.default_token_field,
        TokenLimitField::MaxCompletionTokens
    );
    assert_eq!(
        openai_contract.default_reasoning_field,
        ReasoningField::ReasoningEffort
    );
    assert_eq!(
        openai_contract.default_temperature_field,
        TemperatureField::Include
    );
    assert_eq!(
        openai_contract.payload_adaptation.token_field_progression,
        [
            TokenLimitField::MaxCompletionTokens,
            TokenLimitField::MaxTokens,
            TokenLimitField::Omit,
            TokenLimitField::Omit,
        ]
    );
    assert_eq!(
        openai_contract
            .payload_adaptation
            .reasoning_field_progression,
        [
            ReasoningField::ReasoningEffort,
            ReasoningField::ReasoningObject,
            ReasoningField::Omit,
        ]
    );
    assert_eq!(
        openai_contract.payload_adaptation.token_error_parameters,
        ["max_output_tokens", "max_tokens", "max_completion_tokens"]
    );
    assert_eq!(
        openai_contract
            .payload_adaptation
            .unsupported_parameter_message_fragments,
        [
            "unknown parameter",
            "unsupported parameter",
            "not supported"
        ]
    );
    assert_eq!(
        openai_contract
            .payload_adaptation
            .reasoning_error_parameters,
        ["reasoning_effort", "reasoning"]
    );
    assert_eq!(
        openai_contract
            .payload_adaptation
            .temperature_error_parameters,
        ["temperature"]
    );
    assert_eq!(
        openai_contract
            .payload_adaptation
            .temperature_default_only_fragments,
        ["only the default"]
    );
    assert_eq!(
        openai_contract.transport_mode,
        ProviderTransportMode::OpenAiChatCompletions
    );
    assert!(!openai_contract.supports_turn_streaming_events());
    assert_eq!(
        openai_contract.profile_health_mode,
        ProviderProfileHealthMode::EnforceUnusableWindows
    );
    assert!(!openai_contract.validation.forbid_kimi_coding_endpoint);
    assert!(
        !openai_contract
            .validation
            .require_kimi_cli_user_agent_prefix
    );
    assert_eq!(
        openai_contract
            .error_classification
            .tool_schema_error_parameters,
        ["tools", "tool_choice"]
    );
    assert_eq!(
        openai_contract.error_classification.model_not_found_codes,
        [
            "model_not_found",
            "unsupported_model",
            "invalid_model",
            "not_found_error",
        ]
    );
    assert!(
        openai_contract
            .error_classification
            .model_mismatch_message_fragments
            .contains(&"/v1/responses")
    );
    assert_eq!(
        openai_contract.capability.tool_schema_mode,
        ProviderToolSchemaMode::EnabledWithDowngradeOnUnsupported
    );
    assert_eq!(
        openai_contract.capability.reasoning_extra_body_mode,
        ProviderReasoningExtraBodyMode::Omit
    );
    assert!(openai_contract.capability.turn_tool_schema_enabled());
    assert!(
        openai_contract
            .capability
            .tool_schema_downgrade_on_unsupported()
    );
    assert!(!openai_contract.capability.include_reasoning_extra_body());

    let anthropic_contract = provider_runtime_contract(&ProviderConfig {
        kind: ProviderKind::Anthropic,
        ..ProviderConfig::default()
    });
    assert_eq!(
        anthropic_contract.feature_family,
        ProviderFeatureFamily::Anthropic
    );
    assert_eq!(
        anthropic_contract.transport_mode,
        ProviderTransportMode::AnthropicMessages
    );
    assert!(anthropic_contract.supports_turn_streaming_events());
    assert_eq!(
        anthropic_contract.default_reasoning_field,
        ReasoningField::Omit
    );

    let responses_contract = provider_runtime_contract(&ProviderConfig {
        kind: ProviderKind::Openai,
        wire_api: crate::config::ProviderWireApi::Responses,
        ..ProviderConfig::default()
    });
    assert_eq!(
        responses_contract.transport_mode,
        ProviderTransportMode::Responses
    );
    assert!(!responses_contract.supports_turn_streaming_events());

    let bedrock_contract = provider_runtime_contract(&ProviderConfig {
        kind: ProviderKind::Bedrock,
        ..ProviderConfig::default()
    });
    assert_eq!(
        bedrock_contract.feature_family,
        ProviderFeatureFamily::Bedrock
    );
    assert_eq!(
        bedrock_contract.transport_mode,
        ProviderTransportMode::BedrockConverse
    );
    assert!(!bedrock_contract.supports_turn_streaming_events());
    assert_eq!(
        bedrock_contract.default_reasoning_field,
        ReasoningField::Omit
    );

    let kimi_coding_contract = provider_runtime_contract(&ProviderConfig {
        kind: ProviderKind::KimiCoding,
        ..ProviderConfig::default()
    });
    assert_eq!(
        kimi_coding_contract.feature_family,
        ProviderFeatureFamily::OpenAiCompatible
    );
    assert_eq!(
        kimi_coding_contract.default_token_field,
        TokenLimitField::MaxTokens
    );
    assert_eq!(
        kimi_coding_contract
            .payload_adaptation
            .token_field_progression,
        [
            TokenLimitField::MaxTokens,
            TokenLimitField::MaxCompletionTokens,
            TokenLimitField::Omit,
            TokenLimitField::Omit,
        ]
    );
    assert_eq!(
        kimi_coding_contract.transport_mode,
        ProviderTransportMode::KimiApi
    );
    assert!(!kimi_coding_contract.supports_turn_streaming_events());
    assert!(!kimi_coding_contract.validation.forbid_kimi_coding_endpoint);
    assert!(
        kimi_coding_contract
            .validation
            .require_kimi_cli_user_agent_prefix
    );
    assert_eq!(
        kimi_coding_contract.profile_health_mode,
        ProviderProfileHealthMode::EnforceUnusableWindows
    );
    assert_eq!(
        kimi_coding_contract.capability.reasoning_extra_body_mode,
        ProviderReasoningExtraBodyMode::KimiThinking
    );
    assert!(
        kimi_coding_contract
            .capability
            .include_reasoning_extra_body()
    );

    let kimi_contract = provider_runtime_contract(&ProviderConfig {
        kind: ProviderKind::Kimi,
        ..ProviderConfig::default()
    });
    assert_eq!(
        kimi_contract.profile_health_mode,
        ProviderProfileHealthMode::EnforceUnusableWindows
    );
    assert!(kimi_contract.validation.forbid_kimi_coding_endpoint);
    assert!(!kimi_contract.validation.require_kimi_cli_user_agent_prefix);
    assert_eq!(
        kimi_contract.capability.reasoning_extra_body_mode,
        ProviderReasoningExtraBodyMode::Omit
    );
    assert!(!kimi_contract.capability.include_reasoning_extra_body());

    let volcengine_contract = provider_runtime_contract(&ProviderConfig {
        kind: ProviderKind::Volcengine,
        ..ProviderConfig::default()
    });
    assert_eq!(
        volcengine_contract.feature_family,
        ProviderFeatureFamily::VolcengineCompatible
    );

    let openrouter_contract = provider_runtime_contract(&ProviderConfig {
        kind: ProviderKind::Openrouter,
        ..ProviderConfig::default()
    });
    assert_eq!(
        openrouter_contract.profile_health_mode,
        ProviderProfileHealthMode::ObserveOnly
    );

    let openrouter_enforced_contract = provider_runtime_contract(&ProviderConfig {
        kind: ProviderKind::Openrouter,
        profile_health_mode: ProviderProfileHealthModeConfig::Enforce,
        ..ProviderConfig::default()
    });
    assert_eq!(
        openrouter_enforced_contract.profile_health_mode,
        ProviderProfileHealthMode::EnforceUnusableWindows
    );

    let openai_observe_only_contract = provider_runtime_contract(&ProviderConfig {
        kind: ProviderKind::Openai,
        profile_health_mode: ProviderProfileHealthModeConfig::ObserveOnly,
        ..ProviderConfig::default()
    });
    assert_eq!(
        openai_observe_only_contract.profile_health_mode,
        ProviderProfileHealthMode::ObserveOnly
    );
}

#[tokio::test(flavor = "current_thread")]
async fn request_turn_streaming_rejects_unsupported_transport_modes() {
    let config = test_config(ProviderConfig {
        kind: ProviderKind::Openai,
        ..ProviderConfig::default()
    });

    assert!(!supports_turn_streaming_events(&config));

    let error = request_turn_streaming(
        &config,
        "session-provider-test",
        "turn-provider-test",
        &[json!({
            "role": "user",
            "content": "turn ping"
        })],
        ProviderRuntimeBinding::direct(),
        None,
    )
    .await
    .expect_err("unsupported transports should be rejected before any request is prepared");

    assert!(
        error.contains("does not support live turn streaming events"),
        "the provider error should explain the unsupported transport: {error}"
    );
}

#[test]
fn plain_kimi_completion_body_skips_kimi_extra_body() {
    let mut config = test_config(ProviderConfig {
        kind: ProviderKind::Kimi,
        ..ProviderConfig::default()
    });
    config.provider.reasoning_effort = Some(ReasoningEffort::High);

    let body = build_completion_request_body(
        &config,
        &[],
        "kimi",
        CompletionPayloadMode::default_for(&config.provider),
    );
    assert!(body.get("extra_body").is_none());
}

#[test]
fn payload_mode_adapts_for_parameter_incompatibility() {
    let provider = ProviderConfig {
        kind: ProviderKind::Openrouter,
        max_tokens: Some(1024),
        reasoning_effort: Some(ReasoningEffort::Medium),
        ..ProviderConfig::default()
    };
    let runtime_contract = provider_runtime_contract(&provider);

    let max_tokens_error = json!({
        "error": {
            "code": "unsupported_parameter",
            "param": "max_tokens",
            "message": "Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens' instead."
        }
    });
    let reasoning_effort_error = json!({
        "error": {
            "code": "unknown_parameter",
            "param": "reasoning_effort",
            "message": "Unknown parameter: 'reasoning_effort'."
        }
    });

    let mut mode = CompletionPayloadMode {
        token_field: TokenLimitField::MaxTokens,
        reasoning_field: ReasoningField::ReasoningEffort,
        temperature_field: TemperatureField::Include,
    };

    mode = adapt_payload_mode_for_error(
        mode,
        &provider,
        runtime_contract,
        &parse_provider_api_error(&max_tokens_error),
    )
    .expect("max_tokens adapt");
    assert_eq!(mode.token_field, TokenLimitField::MaxCompletionTokens);

    mode = adapt_payload_mode_for_error(
        mode,
        &provider,
        runtime_contract,
        &parse_provider_api_error(&reasoning_effort_error),
    )
    .expect("reasoning adapt");
    assert_eq!(mode.reasoning_field, ReasoningField::ReasoningObject);
}

#[test]
fn payload_mode_can_drop_temperature_when_model_rejects_it() {
    let provider = ProviderConfig::default();
    let runtime_contract = provider_runtime_contract(&provider);
    let unsupported_temperature = json!({
        "error": {
            "code": "unsupported_value",
            "param": "temperature",
            "message": "Only the default (1) value is supported."
        }
    });

    let mode = CompletionPayloadMode::default_for(&provider);
    let adapted = adapt_payload_mode_for_error(
        mode,
        &provider,
        runtime_contract,
        &parse_provider_api_error(&unsupported_temperature),
    )
    .expect("temperature adaptation");
    assert_eq!(adapted.temperature_field, TemperatureField::Omit);
}

#[test]
fn payload_mode_adaptation_progression_is_monotonic_without_cycles() {
    let provider = ProviderConfig {
        kind: ProviderKind::Openrouter,
        max_tokens: Some(1024),
        reasoning_effort: Some(ReasoningEffort::Medium),
        ..ProviderConfig::default()
    };
    let runtime_contract = provider_runtime_contract(&provider);

    let unsupported_max_tokens = parse_provider_api_error(&json!({
        "error": {
            "param": "max_tokens",
            "message": "Unsupported parameter: 'max_tokens'."
        }
    }));
    let unsupported_max_completion_tokens = parse_provider_api_error(&json!({
        "error": {
            "param": "max_completion_tokens",
            "message": "Unsupported parameter: 'max_completion_tokens'."
        }
    }));
    let unsupported_reasoning_effort = parse_provider_api_error(&json!({
        "error": {
            "param": "reasoning_effort",
            "message": "Unsupported parameter: 'reasoning_effort'."
        }
    }));
    let unsupported_reasoning_object = parse_provider_api_error(&json!({
        "error": {
            "param": "reasoning",
            "message": "Unsupported parameter: 'reasoning'."
        }
    }));

    let mut mode = CompletionPayloadMode::default_for_contract(&provider, runtime_contract);
    assert_eq!(mode.token_field, TokenLimitField::MaxTokens);
    assert_eq!(mode.reasoning_field, ReasoningField::ReasoningEffort);

    mode = adapt_payload_mode_for_error(mode, &provider, runtime_contract, &unsupported_max_tokens)
        .expect("token fallback to max_completion_tokens");
    assert_eq!(mode.token_field, TokenLimitField::MaxCompletionTokens);
    mode = adapt_payload_mode_for_error(
        mode,
        &provider,
        runtime_contract,
        &unsupported_max_completion_tokens,
    )
    .expect("token fallback to omit");
    assert_eq!(mode.token_field, TokenLimitField::Omit);
    assert!(
        adapt_payload_mode_for_error(mode, &provider, runtime_contract, &unsupported_max_tokens)
            .is_none()
    );

    mode = adapt_payload_mode_for_error(
        mode,
        &provider,
        runtime_contract,
        &unsupported_reasoning_effort,
    )
    .expect("reasoning fallback to object");
    assert_eq!(mode.reasoning_field, ReasoningField::ReasoningObject);
    mode = adapt_payload_mode_for_error(
        mode,
        &provider,
        runtime_contract,
        &unsupported_reasoning_object,
    )
    .expect("reasoning fallback to omit");
    assert_eq!(mode.reasoning_field, ReasoningField::Omit);
    assert!(
        adapt_payload_mode_for_error(
            mode,
            &provider,
            runtime_contract,
            &unsupported_reasoning_effort,
        )
        .is_none()
    );
}

#[test]
fn ranking_model_candidates_keeps_preferences_then_catalog() {
    let config = ProviderConfig {
        model: "auto".to_owned(),
        preferred_models: vec![
            "model-z".to_owned(),
            "MODEL-A".to_owned(),
            "model-z".to_owned(),
        ],
        ..ProviderConfig::default()
    };

    let ranked = rank_model_candidates(
        &config,
        &[
            "model-a".to_owned(),
            "model-b".to_owned(),
            "model-z".to_owned(),
        ],
    );
    assert_eq!(ranked, vec!["model-z", "model-a", "model-b"]);
}

#[test]
fn model_error_parser_detects_endpoint_mismatch() {
    let runtime_contract = provider_runtime_contract(&ProviderConfig::default());
    let body = json!({
        "error": {
            "message": "The model `gpt-5-pro` only supports /v1/responses and not this endpoint."
        }
    });
    let parsed = parse_provider_api_error(&body);
    assert!(should_try_next_model_on_error(&parsed, runtime_contract));
}

#[tokio::test(flavor = "current_thread")]
async fn responses_completion_falls_back_to_chat_completions_for_compatible_endpoints() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider listener");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept local provider request");
            let mut request_buf = [0_u8; 8192];
            let len = stream.read(&mut request_buf).expect("read request");
            let request = String::from_utf8_lossy(&request_buf[..len]).to_string();
            requests.push(request.clone());

            let (status_line, body) = if request.starts_with("POST /v1/responses ") {
                (
                    "HTTP/1.1 400 Bad Request",
                    r#"{"error":{"code":"unsupported_parameter","param":"input","message":"This compatibility endpoint expects `messages`; unknown parameter `input`. Retry with /v1/chat/completions."}}"#.to_owned(),
                )
            } else if request.starts_with("POST /v1/chat/completions ") {
                (
                    "HTTP/1.1 200 OK",
                    r#"{"choices":[{"message":{"role":"assistant","content":"fallback ok"}}]}"#
                        .to_owned(),
                )
            } else {
                (
                    "HTTP/1.1 404 Not Found",
                    r#"{"error":{"message":"unexpected request"}}"#.to_owned(),
                )
            };

            let response = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        }
        requests
    });

    let config = test_config(ProviderConfig {
        kind: ProviderKind::Deepseek,
        base_url: format!("http://{addr}"),
        model: "deepseek-chat".to_owned(),
        wire_api: crate::config::ProviderWireApi::Responses,
        api_key: Some(SecretRef::Inline("deepseek-test-key".to_owned())),
        ..ProviderConfig::default()
    });

    let completion = request_completion(
        &config,
        &[json!({
            "role": "user",
            "content": "ping"
        })],
        ProviderRuntimeBinding::direct(),
    )
    .await
    .expect("compatible responses transport should retry chat-completions automatically");
    assert_eq!(completion, "fallback ok");

    let requests = server.join().expect("join local provider server");
    assert!(
        requests.iter().any(|request| {
            request.starts_with("POST /v1/responses ")
                && request.contains("\"input\"")
                && !request.contains("\"messages\"")
        }),
        "first attempt should use Responses input shape: {requests:#?}"
    );
    assert!(
        requests.iter().any(|request| {
            request.starts_with("POST /v1/chat/completions ")
                && request.contains("\"messages\"")
                && !request.contains("\"input\"")
        }),
        "fallback attempt should switch to chat-completions payload shape: {requests:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn responses_turn_falls_back_to_chat_completions_for_compatible_endpoints() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider listener");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept local provider request");
            let mut request_buf = [0_u8; 8192];
            let len = stream.read(&mut request_buf).expect("read request");
            let request = String::from_utf8_lossy(&request_buf[..len]).to_string();
            requests.push(request.clone());

            let (status_line, body) = if request.starts_with("POST /v1/responses ") {
                (
                    "HTTP/1.1 422 Unprocessable Entity",
                    r#"{"error":{"code":"invalid_request_error","param":"input","message":"Missing required parameter: `messages`. This provider expects /v1/chat/completions instead of Responses input."}}"#.to_owned(),
                )
            } else if request.starts_with("POST /v1/chat/completions ") {
                (
                    "HTTP/1.1 200 OK",
                    r#"{"choices":[{"message":{"role":"assistant","content":"turn fallback ok"}}]}"#
                        .to_owned(),
                )
            } else {
                (
                    "HTTP/1.1 404 Not Found",
                    r#"{"error":{"message":"unexpected request"}}"#.to_owned(),
                )
            };

            let response = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        }
        requests
    });

    let config = test_config(ProviderConfig {
        kind: ProviderKind::Deepseek,
        base_url: format!("http://{addr}"),
        model: "deepseek-chat".to_owned(),
        wire_api: crate::config::ProviderWireApi::Responses,
        api_key: Some(SecretRef::Inline("deepseek-test-key".to_owned())),
        ..ProviderConfig::default()
    });

    let turn = request_turn(
        &config,
        "session-provider-test",
        "turn-provider-test",
        &[json!({
            "role": "user",
            "content": "turn ping"
        })],
        ProviderRuntimeBinding::direct(),
    )
    .await
    .expect("turn requests should retry chat-completions when Responses is rejected");
    assert_eq!(turn.assistant_text, "turn fallback ok");

    let requests = server.join().expect("join local provider server");
    assert!(
        requests.iter().any(|request| {
            request.starts_with("POST /v1/responses ")
                && request.contains("\"input\"")
                && request.contains("\"tools\"")
        }),
        "turn flow should first attempt Responses with tool schema: {requests:#?}"
    );
    assert!(
        requests.iter().any(|request| {
            request.starts_with("POST /v1/chat/completions ")
                && request.contains("\"messages\"")
                && request.contains("\"tools\"")
        }),
        "turn flow fallback should preserve tool schema on chat-completions: {requests:#?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn responses_turn_does_not_fallback_for_generic_gateway_failures() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider listener");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().expect("accept local provider request");
            let mut request_buf = [0_u8; 8192];
            let len = stream.read(&mut request_buf).expect("read request");
            let request = String::from_utf8_lossy(&request_buf[..len]).to_string();
            requests.push(request.clone());

            let body =
                r#"{"error":{"message":"temporary backend outage while handling the request"}}"#;
            let response = format!(
                "HTTP/1.1 502 Bad Gateway\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        }
        requests
    });

    let config = test_config(ProviderConfig {
        kind: ProviderKind::Deepseek,
        base_url: format!("http://{addr}"),
        model: "deepseek-chat".to_owned(),
        wire_api: crate::config::ProviderWireApi::Responses,
        api_key: Some(SecretRef::Inline("deepseek-test-key".to_owned())),
        ..ProviderConfig::default()
    });

    let error = request_turn(
        &config,
        "session-provider-test",
        "turn-provider-test",
        &[json!({
            "role": "user",
            "content": "turn ping"
        })],
        ProviderRuntimeBinding::direct(),
    )
    .await
    .expect_err("generic gateway failures should stay on the same transport and eventually fail");

    assert!(
        error.contains("status 502"),
        "the surfaced error should still report the gateway failure: {error}"
    );

    let requests = server.join().expect("join local provider server");
    assert!(
        requests
            .iter()
            .all(|request| request.starts_with("POST /v1/responses ")),
        "generic gateway failures should not trigger chat-completions fallback: {requests:#?}"
    );
    assert!(
        !requests.is_empty(),
        "the test server should observe at least the initial responses request"
    );
}

#[test]
fn model_request_error_includes_parseable_failover_snapshot_payload() {
    let error = build_model_request_error(
        "provider returned status 429 for model `model-z`".to_owned(),
        false,
        ProviderFailoverReason::RateLimited,
        ProviderFailoverStage::StatusFailure,
        "model-z",
        2,
        3,
        Some(429),
        None,
    );
    assert_eq!(error.reason, ProviderFailoverReason::RateLimited);
    assert_eq!(error.snapshot.stage, ProviderFailoverStage::StatusFailure);

    let (_, payload) = error
        .message
        .split_once("provider_failover=")
        .expect("message should embed structured failover payload");
    let parsed: serde_json::Value =
        serde_json::from_str(payload).expect("failover payload should be valid json");
    assert_eq!(parsed["reason"], "rate_limited");
    assert_eq!(parsed["stage"], "status_failure");
    assert_eq!(parsed["model"], "model-z");
    assert_eq!(parsed["attempt"], 2);
    assert_eq!(parsed["max_attempts"], 3);
    assert_eq!(parsed["status_code"], 429);
}

#[test]
fn model_request_error_omits_status_code_for_transport_failures() {
    let error = build_model_request_error(
        "provider request failed for model `model-a`".to_owned(),
        false,
        ProviderFailoverReason::TransportFailure,
        ProviderFailoverStage::TransportFailure,
        "model-a",
        1,
        3,
        None,
        None,
    );
    let (_, payload) = error
        .message
        .split_once("provider_failover=")
        .expect("message should embed structured failover payload");
    let parsed: serde_json::Value =
        serde_json::from_str(payload).expect("failover payload should be valid json");
    assert_eq!(parsed["reason"], "transport_failure");
    assert_eq!(parsed["stage"], "transport_failure");
    assert!(parsed.get("status_code").is_none());
}

#[test]
fn provider_failover_audit_event_records_structured_payload() {
    let (kernel_ctx, audit) = build_provider_failover_test_kernel_context("provider-agent");
    let snapshot = ProviderFailoverSnapshot {
        reason: ProviderFailoverReason::RateLimited,
        stage: ProviderFailoverStage::StatusFailure,
        model: "model-z".to_owned(),
        attempt: 2,
        max_attempts: 3,
        status_code: Some(429),
    };
    let provider = ProviderConfig {
        kind: ProviderKind::KimiCoding,
        ..ProviderConfig::default()
    };

    record_provider_failover_audit_event(
        ProviderRuntimeBinding::kernel(&kernel_ctx),
        &provider,
        &snapshot,
        true,
        true,
        1,
        4,
        false,
    );

    let failover_event = audit
        .snapshot()
        .into_iter()
        .find_map(|event| {
            if let AuditEventKind::ProviderFailover {
                pack_id,
                provider_id,
                reason,
                stage,
                model,
                attempt,
                max_attempts,
                status_code,
                try_next_model,
                auto_model_mode,
                candidate_index,
                candidate_count,
            } = event.kind
            {
                Some((
                    pack_id,
                    provider_id,
                    reason,
                    stage,
                    model,
                    attempt,
                    max_attempts,
                    status_code,
                    try_next_model,
                    auto_model_mode,
                    candidate_index,
                    candidate_count,
                ))
            } else {
                None
            }
        })
        .expect("provider failover event should be recorded");

    assert_eq!(failover_event.0, "provider-test-pack");
    assert_eq!(failover_event.1, "kimi_coding");
    assert_eq!(failover_event.2, "rate_limited");
    assert_eq!(failover_event.3, "status_failure");
    assert_eq!(failover_event.4, "model-z");
    assert_eq!(failover_event.5, 2);
    assert_eq!(failover_event.6, 3);
    assert_eq!(failover_event.7, Some(429));
    assert!(failover_event.8);
    assert!(failover_event.9);
    assert_eq!(failover_event.10, 1);
    assert_eq!(failover_event.11, 4);
}

#[test]
fn provider_failover_audit_event_is_noop_without_kernel_context() {
    let (_kernel_ctx, audit) = build_provider_failover_test_kernel_context("provider-agent");
    let snapshot = ProviderFailoverSnapshot {
        reason: ProviderFailoverReason::TransportFailure,
        stage: ProviderFailoverStage::TransportFailure,
        model: "model-a".to_owned(),
        attempt: 1,
        max_attempts: 3,
        status_code: None,
    };
    let provider = ProviderConfig::default();
    let before = audit.snapshot().len();

    record_provider_failover_audit_event(
        ProviderRuntimeBinding::direct(),
        &provider,
        &snapshot,
        false,
        false,
        0,
        1,
        true,
    );

    let after = audit.snapshot().len();
    assert_eq!(after, before);
}

#[test]
fn provider_failover_metrics_record_even_without_kernel_context() {
    let before = provider_failover_metrics_snapshot();
    let snapshot = ProviderFailoverSnapshot {
        reason: ProviderFailoverReason::TransportFailure,
        stage: ProviderFailoverStage::TransportFailure,
        model: "model-a".to_owned(),
        attempt: 1,
        max_attempts: 3,
        status_code: None,
    };
    let provider = ProviderConfig::default();

    record_provider_failover_audit_event(
        ProviderRuntimeBinding::direct(),
        &provider,
        &snapshot,
        false,
        false,
        0,
        1,
        true,
    );

    let after = provider_failover_metrics_snapshot();
    let reason_before = before
        .by_reason
        .get("transport_failure")
        .copied()
        .unwrap_or(0);
    let reason_after = after
        .by_reason
        .get("transport_failure")
        .copied()
        .unwrap_or(0);
    let stage_before = before
        .by_stage
        .get("transport_failure")
        .copied()
        .unwrap_or(0);
    let stage_after = after
        .by_stage
        .get("transport_failure")
        .copied()
        .unwrap_or(0);
    let provider_before = before.by_provider.get("openai").copied().unwrap_or(0);
    let provider_after = after.by_provider.get("openai").copied().unwrap_or(0);

    assert!(after.total_events > before.total_events);
    assert!(after.exhausted_events > before.exhausted_events);
    assert!(reason_after > reason_before);
    assert!(stage_after > stage_before);
    assert!(provider_after > provider_before);
}

#[test]
fn provider_failover_metrics_track_continue_path() {
    let before = provider_failover_metrics_snapshot();
    let snapshot = ProviderFailoverSnapshot {
        reason: ProviderFailoverReason::RateLimited,
        stage: ProviderFailoverStage::StatusFailure,
        model: "model-z".to_owned(),
        attempt: 2,
        max_attempts: 4,
        status_code: Some(429),
    };
    let provider = ProviderConfig {
        kind: ProviderKind::KimiCoding,
        ..ProviderConfig::default()
    };

    record_provider_failover_audit_event(
        ProviderRuntimeBinding::direct(),
        &provider,
        &snapshot,
        true,
        true,
        1,
        4,
        false,
    );

    let after = provider_failover_metrics_snapshot();
    let reason_before = before.by_reason.get("rate_limited").copied().unwrap_or(0);
    let reason_after = after.by_reason.get("rate_limited").copied().unwrap_or(0);
    let stage_before = before.by_stage.get("status_failure").copied().unwrap_or(0);
    let stage_after = after.by_stage.get("status_failure").copied().unwrap_or(0);
    let provider_before = before.by_provider.get("kimi_coding").copied().unwrap_or(0);
    let provider_after = after.by_provider.get("kimi_coding").copied().unwrap_or(0);

    assert!(after.total_events > before.total_events);
    assert!(after.continued_events > before.continued_events);
    assert!(reason_after > reason_before);
    assert!(stage_after > stage_before);
    assert!(provider_after > provider_before);
}

#[test]
fn model_request_status_plan_prefers_retry_when_retryable_status_has_budget() {
    let provider = ProviderConfig::default();
    let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
    let runtime_contract = provider_runtime_contract(&provider);
    let mut headers = HeaderMap::new();
    headers.insert(RETRY_AFTER, HeaderValue::from_static("2"));

    let plan = plan_model_request_status(
        429,
        &headers,
        &ProviderApiError::default(),
        1,
        &request_policy,
        100,
        true,
        runtime_contract,
    );
    assert_eq!(
        plan,
        ModelRequestStatusPlan::Retry {
            delay_ms: 2_000,
            next_backoff_ms: 3_000
        }
    );
}

#[test]
fn model_request_status_plan_switches_model_when_auto_and_model_mismatch() {
    let provider = ProviderConfig::default();
    let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
    let runtime_contract = provider_runtime_contract(&provider);
    let api_error = ProviderApiError {
        message: Some("model does not exist".to_owned()),
        ..ProviderApiError::default()
    };
    let plan = plan_model_request_status(
        400,
        &HeaderMap::new(),
        &api_error,
        1,
        &request_policy,
        100,
        true,
        runtime_contract,
    );
    assert_eq!(plan, ModelRequestStatusPlan::TryNextModel);
}

#[test]
fn model_request_status_plan_does_not_switch_model_on_server_status() {
    let provider = ProviderConfig::default();
    let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
    let runtime_contract = provider_runtime_contract(&provider);
    let api_error = ProviderApiError {
        message: Some("model does not exist".to_owned()),
        ..ProviderApiError::default()
    };
    let plan = plan_model_request_status(
        503,
        &HeaderMap::new(),
        &api_error,
        request_policy.max_attempts,
        &request_policy,
        100,
        true,
        runtime_contract,
    );
    assert_eq!(plan, ModelRequestStatusPlan::Fail);
}

#[test]
fn model_request_status_plan_fails_when_auto_mode_disabled() {
    let provider = ProviderConfig::default();
    let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
    let runtime_contract = provider_runtime_contract(&provider);
    let api_error = ProviderApiError {
        message: Some("model does not exist".to_owned()),
        ..ProviderApiError::default()
    };
    let plan = plan_model_request_status(
        400,
        &HeaderMap::new(),
        &api_error,
        1,
        &request_policy,
        100,
        false,
        runtime_contract,
    );
    assert_eq!(plan, ModelRequestStatusPlan::Fail);
}

#[test]
fn model_request_status_plan_fails_when_retry_budget_exhausted() {
    let provider = ProviderConfig::default();
    let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
    let runtime_contract = provider_runtime_contract(&provider);
    let mut headers = HeaderMap::new();
    headers.insert(RETRY_AFTER, HeaderValue::from_static("5"));

    let plan = plan_model_request_status(
        503,
        &headers,
        &ProviderApiError::default(),
        request_policy.max_attempts,
        &request_policy,
        100,
        true,
        runtime_contract,
    );
    assert_eq!(plan, ModelRequestStatusPlan::Fail);
}

#[test]
fn status_failure_reason_classifies_rate_limit_and_overload() {
    let runtime_contract = provider_runtime_contract(&ProviderConfig::default());
    assert_eq!(
        classify_model_status_failure_reason(429, &ProviderApiError::default(), runtime_contract),
        ProviderFailoverReason::RateLimited
    );
    assert_eq!(
        classify_model_status_failure_reason(503, &ProviderApiError::default(), runtime_contract),
        ProviderFailoverReason::ProviderOverloaded
    );
}

#[test]
fn status_failure_reason_classifies_auth_rejection() {
    let runtime_contract = provider_runtime_contract(&ProviderConfig::default());
    assert_eq!(
        classify_model_status_failure_reason(401, &ProviderApiError::default(), runtime_contract),
        ProviderFailoverReason::AuthRejected
    );
    assert_eq!(
        classify_model_status_failure_reason(403, &ProviderApiError::default(), runtime_contract),
        ProviderFailoverReason::AuthRejected
    );
}

#[test]
fn status_failure_reason_classifies_payload_incompatibility() {
    let runtime_contract = provider_runtime_contract(&ProviderConfig::default());
    let api_error = ProviderApiError {
        code: Some("unsupported_parameter".to_owned()),
        param: Some("max_tokens".to_owned()),
        message: Some("unsupported parameter".to_owned()),
    };
    assert_eq!(
        classify_model_status_failure_reason(400, &api_error, runtime_contract),
        ProviderFailoverReason::PayloadIncompatible
    );
}

#[test]
fn status_failure_reason_classifies_tool_schema_incompatibility() {
    let runtime_contract = provider_runtime_contract(&ProviderConfig::default());
    let api_error = parse_provider_api_error(&json!({
        "error": {
            "code": "unsupported_parameter",
            "param": "tools",
            "message": "Unsupported parameter: 'tools'"
        }
    }));

    assert_eq!(
        classify_model_status_failure_reason(400, &api_error, runtime_contract),
        ProviderFailoverReason::PayloadIncompatible
    );
}

#[test]
fn payload_adaptation_axis_honors_contract_temperature_fragments() {
    let provider = ProviderConfig::default();
    let runtime_contract = provider_runtime_contract(&provider);
    let temperature_default_only = parse_provider_api_error(&json!({
        "error": {
            "param": "temperature",
            "message": "Only the default value is supported."
        }
    }));

    assert_eq!(
        classify_payload_adaptation_axis(
            &temperature_default_only,
            &runtime_contract.payload_adaptation,
        ),
        Some(PayloadAdaptationAxis::TemperatureField)
    );
}

#[test]
fn status_failure_reason_keeps_server_errors_from_model_mismatch_switches() {
    let runtime_contract = provider_runtime_contract(&ProviderConfig::default());
    let api_error = ProviderApiError {
        message: Some("model does not exist".to_owned()),
        ..ProviderApiError::default()
    };
    assert_eq!(
        classify_model_status_failure_reason(503, &api_error, runtime_contract),
        ProviderFailoverReason::ProviderOverloaded
    );
}

#[test]
fn model_catalog_cache_honors_ttl_and_prunes_expired_entries() {
    let mut cache = ModelCatalogCache::default();
    let now = Instant::now();
    cache.put(
        "catalog-key".to_owned(),
        vec!["model-a".to_owned()],
        now,
        Duration::from_millis(500),
        Duration::from_millis(0),
        MODEL_CATALOG_CACHE_MAX_ENTRIES,
    );

    assert_eq!(
        cache.lookup("catalog-key", now),
        Some(ModelCatalogCacheLookup::Fresh(vec!["model-a".to_owned()]))
    );
    assert_eq!(
        cache.lookup("catalog-key", now + Duration::from_millis(501)),
        None
    );
    assert!(cache.entries.is_empty());
}

#[test]
fn model_catalog_cache_serves_stale_entry_within_grace_window() {
    let mut cache = ModelCatalogCache::default();
    let now = Instant::now();
    cache.put(
        "catalog-key".to_owned(),
        vec!["model-a".to_owned(), "model-b".to_owned()],
        now,
        Duration::from_millis(500),
        Duration::from_millis(700),
        MODEL_CATALOG_CACHE_MAX_ENTRIES,
    );

    assert_eq!(
        cache.lookup("catalog-key", now + Duration::from_millis(499)),
        Some(ModelCatalogCacheLookup::Fresh(vec![
            "model-a".to_owned(),
            "model-b".to_owned()
        ]))
    );
    assert_eq!(
        cache.lookup("catalog-key", now + Duration::from_millis(501)),
        Some(ModelCatalogCacheLookup::Stale(vec![
            "model-a".to_owned(),
            "model-b".to_owned()
        ]))
    );
    assert_eq!(
        cache.lookup("catalog-key", now + Duration::from_millis(1_201)),
        None
    );
}

#[test]
fn model_catalog_cache_evicts_oldest_entry_when_capacity_exceeded() {
    let mut cache = ModelCatalogCache::default();
    let now = Instant::now();

    for idx in 0..(MODEL_CATALOG_CACHE_MAX_ENTRIES + 2) {
        cache.put(
            format!("cache-key-{idx}"),
            vec![format!("model-{idx}")],
            now,
            Duration::from_secs(60),
            Duration::from_secs(30),
            MODEL_CATALOG_CACHE_MAX_ENTRIES,
        );
    }

    assert_eq!(cache.entries.len(), MODEL_CATALOG_CACHE_MAX_ENTRIES);
    assert!(cache.lookup("cache-key-0", now).is_none());
    assert!(cache.lookup("cache-key-1", now).is_none());
    assert!(cache.lookup("cache-key-2", now).is_some());
}

#[test]
fn model_catalog_cache_capacity_uses_runtime_limit() {
    let mut cache = ModelCatalogCache::default();
    let now = Instant::now();

    for idx in 0..4 {
        cache.put(
            format!("cache-key-{idx}"),
            vec![format!("model-{idx}")],
            now,
            Duration::from_secs(60),
            Duration::from_secs(30),
            2,
        );
    }

    assert_eq!(cache.entries.len(), 2);
    assert!(cache.lookup("cache-key-0", now).is_none());
    assert!(cache.lookup("cache-key-1", now).is_none());
    assert!(cache.lookup("cache-key-2", now).is_some());
    assert!(cache.lookup("cache-key-3", now).is_some());
}

#[test]
fn model_catalog_cache_key_includes_endpoint_auth_and_headers() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "x-provider",
        reqwest::header::HeaderValue::from_static("foo"),
    );
    let first = build_model_catalog_cache_key("https://api.example.com/v1/models", &headers, None);
    let second = build_model_catalog_cache_key(
        "https://api.example.com/v1/models",
        &headers,
        Some("Bearer abc"),
    );
    let mut headers_with_extra = headers.clone();
    headers_with_extra.insert("x-extra", reqwest::header::HeaderValue::from_static("bar"));
    let third = build_model_catalog_cache_key(
        "https://api.example.com/v1/models",
        &headers_with_extra,
        None,
    );

    assert_ne!(first, second);
    assert_ne!(first, third);
}

#[test]
fn model_catalog_cache_key_does_not_expose_raw_secret_values() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "x-provider-token",
        reqwest::header::HeaderValue::from_static("secret-header-value"),
    );
    let key = build_model_catalog_cache_key(
        "https://api.example.com/v1/models",
        &headers,
        Some("Bearer secret-auth-token"),
    );
    assert!(!key.contains("secret-auth-token"));
    assert!(!key.contains("secret-header-value"));
    assert!(key.starts_with("provider-model-catalog::"));
}

#[test]
fn model_candidate_cooldown_reorders_candidates_with_active_cooldown() {
    let policy = ModelCandidateCooldownPolicy {
        namespace: next_model_cooldown_test_namespace(),
        cooldown: Duration::from_secs(60),
        max_cooldown: Duration::from_secs(600),
        max_entries: MODEL_CANDIDATE_COOLDOWN_CACHE_MAX_ENTRIES,
    };
    register_model_candidate_cooldown(&policy, "model-a", ProviderFailoverReason::ModelMismatch);

    let ordered = prioritize_model_candidates_by_cooldown(
        vec![
            "model-a".to_owned(),
            "model-b".to_owned(),
            "model-c".to_owned(),
        ],
        Some(&policy),
    );
    assert_eq!(ordered, vec!["model-b", "model-c", "model-a"]);
}

#[test]
fn model_candidate_cooldown_ignores_non_model_replacement_failures() {
    let policy = ModelCandidateCooldownPolicy {
        namespace: next_model_cooldown_test_namespace(),
        cooldown: Duration::from_secs(60),
        max_cooldown: Duration::from_secs(600),
        max_entries: MODEL_CANDIDATE_COOLDOWN_CACHE_MAX_ENTRIES,
    };
    register_model_candidate_cooldown(&policy, "model-a", ProviderFailoverReason::RequestRejected);

    let ordered = prioritize_model_candidates_by_cooldown(
        vec!["model-a".to_owned(), "model-b".to_owned()],
        Some(&policy),
    );
    assert_eq!(ordered, vec!["model-a", "model-b"]);
}

#[test]
fn model_candidate_cooldown_backoff_is_exponential_and_capped() {
    let mut cache = ModelCandidateCooldownCache::default();
    let key = "provider-model-candidate-cooldown::test";
    let base = Duration::from_millis(100);
    let max = Duration::from_millis(250);
    let now = Instant::now();

    cache.put(
        key.to_owned(),
        ProviderFailoverReason::ModelMismatch,
        now,
        base,
        max,
        MODEL_CANDIDATE_COOLDOWN_CACHE_MAX_ENTRIES,
    );
    let first = cache
        .lookup_active(key, now)
        .expect("first cooldown should exist")
        .clone();
    assert_eq!(first.failure_count, 1);
    assert_eq!(
        first.expires_at.duration_since(now),
        Duration::from_millis(100)
    );

    let second_now = now + Duration::from_millis(1);
    cache.put(
        key.to_owned(),
        ProviderFailoverReason::ModelMismatch,
        second_now,
        base,
        max,
        MODEL_CANDIDATE_COOLDOWN_CACHE_MAX_ENTRIES,
    );
    let second = cache
        .lookup_active(key, second_now)
        .expect("second cooldown should exist")
        .clone();
    assert_eq!(second.failure_count, 2);
    assert_eq!(
        second.expires_at.duration_since(second_now),
        Duration::from_millis(200)
    );

    let third_now = now + Duration::from_millis(2);
    cache.put(
        key.to_owned(),
        ProviderFailoverReason::ModelMismatch,
        third_now,
        base,
        max,
        MODEL_CANDIDATE_COOLDOWN_CACHE_MAX_ENTRIES,
    );
    let third = cache
        .lookup_active(key, third_now)
        .expect("third cooldown should exist")
        .clone();
    assert_eq!(third.failure_count, 3);
    assert_eq!(
        third.expires_at.duration_since(third_now),
        Duration::from_millis(250)
    );
}

#[test]
fn model_candidate_cooldown_resets_after_expiry() {
    let mut cache = ModelCandidateCooldownCache::default();
    let key = "provider-model-candidate-cooldown::expiry";
    let base = Duration::from_millis(100);
    let max = Duration::from_millis(400);
    let now = Instant::now();

    cache.put(
        key.to_owned(),
        ProviderFailoverReason::ModelMismatch,
        now,
        base,
        max,
        MODEL_CANDIDATE_COOLDOWN_CACHE_MAX_ENTRIES,
    );
    let after_expiry = now + Duration::from_millis(101);
    cache.put(
        key.to_owned(),
        ProviderFailoverReason::ModelMismatch,
        after_expiry,
        base,
        max,
        MODEL_CANDIDATE_COOLDOWN_CACHE_MAX_ENTRIES,
    );
    let refreshed = cache
        .lookup_active(key, after_expiry)
        .expect("refreshed cooldown should exist")
        .clone();
    assert_eq!(refreshed.failure_count, 1);
    assert_eq!(refreshed.expires_at.duration_since(after_expiry), base);
}

#[tokio::test(flavor = "current_thread")]
async fn model_catalog_singleflight_deduplicates_concurrent_fetches() {
    const CACHE_KEY: &str = "singleflight-key";
    clear_model_catalog_singleflight_slot(CACHE_KEY);

    let run_count = Arc::new(AtomicUsize::new(0));
    let leader_started = Arc::new(Notify::new());
    let leader_release = Arc::new(Notify::new());
    let follower_ready = Arc::new(AtomicUsize::new(0));

    let follower_count = 10usize;
    let start_barrier = Arc::new(Barrier::new(follower_count + 2));
    let leader_task = {
        let run_count = run_count.clone();
        let leader_started = leader_started.clone();
        let leader_release = leader_release.clone();
        let start_barrier = start_barrier.clone();
        tokio::spawn(async move {
            fetch_model_catalog_singleflight(CACHE_KEY, || async move {
                run_count.fetch_add(1, Ordering::SeqCst);
                leader_started.notify_waiters();
                start_barrier.wait().await;
                leader_release.notified().await;
                Ok(vec!["model-a".to_owned()])
            })
            .await
        })
    };
    leader_started.notified().await;

    let mut followers = Vec::with_capacity(follower_count);
    for _ in 0..follower_count {
        let run_count = run_count.clone();
        let start_barrier = start_barrier.clone();
        let follower_ready = follower_ready.clone();
        followers.push(tokio::spawn(async move {
            start_barrier.wait().await;
            follower_ready.fetch_add(1, Ordering::SeqCst);
            fetch_model_catalog_singleflight(CACHE_KEY, || async move {
                run_count.fetch_add(1, Ordering::SeqCst);
                Ok(vec!["model-a".to_owned()])
            })
            .await
        }));
    }
    start_barrier.wait().await;
    while follower_ready.load(Ordering::SeqCst) < follower_count {
        tokio::task::yield_now().await;
    }
    tokio::task::yield_now().await;
    leader_release.notify_waiters();

    let leader_models = leader_task
        .await
        .expect("join leader singleflight task")
        .expect("leader singleflight result");
    assert_eq!(leader_models, vec!["model-a"]);

    for task in followers {
        let models = task
            .await
            .expect("join follower singleflight task")
            .expect("singleflight result");
        assert_eq!(models, vec!["model-a"]);
    }
    assert_eq!(run_count.load(Ordering::SeqCst), 1);

    clear_model_catalog_singleflight_slot(CACHE_KEY);
}

#[tokio::test(flavor = "current_thread")]
async fn model_catalog_singleflight_recovers_when_leader_panics() {
    const CACHE_KEY: &str = "panic-recovery-key";
    clear_model_catalog_singleflight_slot(CACHE_KEY);

    let leader = tokio::spawn(async {
        let _ = fetch_model_catalog_singleflight(CACHE_KEY, || async {
            sleep(Duration::from_millis(80)).await;
            panic!("synthetic singleflight leader panic");
        })
        .await;
    });
    sleep(Duration::from_millis(10)).await;

    let follower = tokio::spawn(async {
        fetch_model_catalog_singleflight(CACHE_KEY, || async {
            Ok(vec!["model-recovered".to_owned()])
        })
        .await
    });

    let leader_err = leader.await.expect_err("leader should panic");
    assert!(leader_err.is_panic());

    let recovered = follower
        .await
        .expect("join follower")
        .expect("follower should retry and recover");
    assert_eq!(recovered, vec!["model-recovered"]);
    assert!(
        !has_model_catalog_singleflight_slot(CACHE_KEY),
        "panic recovery should clear the singleflight slot for the recovered cache key"
    );
    clear_model_catalog_singleflight_slot(CACHE_KEY);
}

#[tokio::test(flavor = "current_thread")]
async fn model_catalog_singleflight_recovers_when_leader_stalls() {
    const CACHE_KEY: &str = "stalled-recovery-key";
    clear_model_catalog_singleflight_slot(CACHE_KEY);

    let run_count = Arc::new(AtomicUsize::new(0));
    let leader_started = Arc::new(Notify::new());
    let leader = {
        let run_count = run_count.clone();
        let leader_started = leader_started.clone();
        tokio::spawn(async move {
            fetch_model_catalog_singleflight_with_timeouts(
                CACHE_KEY,
                Duration::from_secs(30),
                Duration::from_secs(30),
                || async move {
                    run_count.fetch_add(1, Ordering::SeqCst);
                    leader_started.notify_waiters();
                    sleep(Duration::from_millis(220)).await;
                    Ok(vec!["model-from-stalled-leader".to_owned()])
                },
            )
            .await
        })
    };
    leader_started.notified().await;

    let follower_started_at = Instant::now();
    let follower_result = fetch_model_catalog_singleflight_with_timeouts(
        CACHE_KEY,
        Duration::from_millis(20),
        Duration::from_millis(60),
        || {
            let run_count = run_count.clone();
            async move {
                run_count.fetch_add(1, Ordering::SeqCst);
                Ok(vec!["model-from-recovery-follower".to_owned()])
            }
        },
    )
    .await
    .expect("follower should recover from stale slot");
    assert_eq!(follower_result, vec!["model-from-recovery-follower"]);
    assert!(follower_started_at.elapsed() < Duration::from_millis(180));

    let leader_result = leader
        .await
        .expect("join stalled leader")
        .expect("leader should still finish");
    assert_eq!(leader_result, vec!["model-from-stalled-leader"]);
    assert_eq!(run_count.load(Ordering::SeqCst), 2);

    clear_model_catalog_singleflight_slot(CACHE_KEY);
}
