use super::*;
use serde_json::json;

#[test]
fn runtime_snapshot_restore_managed_skills_keeps_entries_without_display_metadata() {
    let mut warnings = Vec::new();
    let spec = build_runtime_snapshot_restore_managed_skills_spec(
        &RuntimeSnapshotExternalSkillsState {
            policy: mvp::tools::runtime_config::ExternalSkillsRuntimePolicy::default(),
            override_active: false,
            inventory_status: RuntimeSnapshotInventoryStatus::Ok,
            inventory_error: None,
            inventory: json!({
                "skills": [{
                    "scope": "managed",
                    "skill_id": "demo-skill",
                    "source_kind": "directory",
                    "source_path": "/tmp/demo-skill",
                    "sha256": "deadbeef"
                }]
            }),
            resolved_skill_count: 1,
            shadowed_skill_count: 0,
        },
        &mut warnings,
    );

    assert!(warnings.is_empty());
    assert_eq!(spec.skills.len(), 1);
    assert_eq!(spec.skills[0].skill_id, "demo-skill");
    assert!(spec.skills[0].display_name.is_empty());
    assert!(spec.skills[0].summary.is_empty());
}

#[test]
fn runtime_snapshot_provider_header_safety_uses_explicit_safe_names_only() {
    assert!(runtime_snapshot_provider_header_is_safe_to_persist(
        mvp::config::ProviderKind::Anthropic,
        "anthropic-version",
        "2023-06-01",
    ));
    assert!(runtime_snapshot_provider_header_is_safe_to_persist(
        mvp::config::ProviderKind::Deepseek,
        "anthropic-version",
        "2023-06-01",
    ));
    assert!(runtime_snapshot_provider_header_is_safe_to_persist(
        mvp::config::ProviderKind::Anthropic,
        "anthropic-beta",
        "prompt-caching-2024-07-31",
    ));
    assert!(runtime_snapshot_provider_header_is_safe_to_persist(
        mvp::config::ProviderKind::Openai,
        "openai-beta",
        "assistants=v2",
    ));
    assert!(runtime_snapshot_provider_header_is_safe_to_persist(
        mvp::config::ProviderKind::Deepseek,
        "x-goog-api-key",
        "${GOOGLE_API_KEY}",
    ));
    assert!(!runtime_snapshot_provider_header_is_safe_to_persist(
        mvp::config::ProviderKind::Deepseek,
        "x-secret-beta",
        "literal-secret",
    ));
    assert!(!runtime_snapshot_provider_header_is_safe_to_persist(
        mvp::config::ProviderKind::Deepseek,
        "x-secret-version",
        "literal-secret",
    ));
}

#[test]
fn runtime_snapshot_restore_normalization_moves_provider_env_name_fields_into_secret_refs() {
    let mut warnings = Vec::new();
    let mut profile = mvp::config::ProviderProfileConfig {
        default_for_kind: true,
        provider: mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Openai,
            model: "openai/gpt-5.1-codex".to_owned(),
            api_key_env: Some("OPENAI_API_KEY".to_owned()),
            oauth_access_token_env: Some("OPENAI_CODEX_OAUTH_TOKEN".to_owned()),
            ..Default::default()
        },
    };

    normalize_runtime_snapshot_restore_provider_profile("openai-main", &mut profile, &mut warnings);

    assert_eq!(
        profile.provider.api_key,
        Some(SecretRef::Env {
            env: "OPENAI_API_KEY".to_owned(),
        })
    );
    assert_eq!(profile.provider.api_key_env, None);
    assert_eq!(
        profile.provider.oauth_access_token,
        Some(SecretRef::Env {
            env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
        })
    );
    assert_eq!(profile.provider.oauth_access_token_env, None);
    assert!(warnings.is_empty());
}

#[test]
fn runtime_snapshot_restore_normalization_canonicalizes_matching_explicit_env_reference() {
    let mut warnings = Vec::new();
    let mut profile = mvp::config::ProviderProfileConfig {
        default_for_kind: true,
        provider: mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Openai,
            model: "openai/gpt-5.1-codex".to_owned(),
            api_key: Some(SecretRef::Inline("${INLINE_OPENAI_API_KEY}".to_owned())),
            api_key_env: Some(" INLINE_OPENAI_API_KEY ".to_owned()),
            oauth_access_token: Some(SecretRef::Inline("$INLINE_OPENAI_OAUTH_TOKEN".to_owned())),
            oauth_access_token_env: Some("INLINE_OPENAI_OAUTH_TOKEN".to_owned()),
            ..Default::default()
        },
    };

    normalize_runtime_snapshot_restore_provider_profile("openai-main", &mut profile, &mut warnings);

    assert_eq!(
        profile.provider.api_key,
        Some(SecretRef::Env {
            env: "INLINE_OPENAI_API_KEY".to_owned(),
        })
    );
    assert_eq!(profile.provider.api_key_env, None);
    assert_eq!(
        profile.provider.oauth_access_token,
        Some(SecretRef::Env {
            env: "INLINE_OPENAI_OAUTH_TOKEN".to_owned(),
        })
    );
    assert_eq!(profile.provider.oauth_access_token_env, None);
    assert!(warnings.is_empty());
}

#[test]
fn runtime_snapshot_restore_normalization_prefers_explicit_env_reference_over_legacy_env_field() {
    let mut warnings = Vec::new();
    let mut profile = mvp::config::ProviderProfileConfig {
        default_for_kind: true,
        provider: mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Openai,
            model: "openai/gpt-5.1-codex".to_owned(),
            api_key: Some(SecretRef::Inline("${INLINE_OPENAI_API_KEY}".to_owned())),
            api_key_env: Some("CONFIGURED_OPENAI_API_KEY".to_owned()),
            oauth_access_token: Some(SecretRef::Inline("$INLINE_OPENAI_OAUTH_TOKEN".to_owned())),
            oauth_access_token_env: Some("CONFIGURED_OPENAI_OAUTH_TOKEN".to_owned()),
            ..Default::default()
        },
    };

    normalize_runtime_snapshot_restore_provider_profile("openai-main", &mut profile, &mut warnings);

    assert_eq!(
        profile.provider.api_key,
        Some(SecretRef::Env {
            env: "INLINE_OPENAI_API_KEY".to_owned(),
        })
    );
    assert_eq!(profile.provider.api_key_env, None);
    assert_eq!(
        profile.provider.oauth_access_token,
        Some(SecretRef::Env {
            env: "INLINE_OPENAI_OAUTH_TOKEN".to_owned(),
        })
    );
    assert_eq!(profile.provider.oauth_access_token_env, None);
    assert!(warnings.is_empty());
}

#[test]
fn runtime_snapshot_restore_normalization_treats_blank_inline_secret_as_absent() {
    let mut warnings = Vec::new();
    let mut profile = mvp::config::ProviderProfileConfig {
        default_for_kind: true,
        provider: mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Openai,
            model: "openai/gpt-5.1-codex".to_owned(),
            api_key: Some(SecretRef::Inline("   ".to_owned())),
            api_key_env: Some("OPENAI_API_KEY".to_owned()),
            oauth_access_token: Some(SecretRef::Inline("   ".to_owned())),
            oauth_access_token_env: Some("OPENAI_CODEX_OAUTH_TOKEN".to_owned()),
            ..Default::default()
        },
    };

    normalize_runtime_snapshot_restore_provider_profile("openai-main", &mut profile, &mut warnings);

    assert_eq!(
        profile.provider.api_key,
        Some(SecretRef::Env {
            env: "OPENAI_API_KEY".to_owned(),
        })
    );
    assert_eq!(profile.provider.api_key_env, None);
    assert_eq!(
        profile.provider.oauth_access_token,
        Some(SecretRef::Env {
            env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
        })
    );
    assert_eq!(profile.provider.oauth_access_token_env, None);
    assert!(warnings.is_empty());
}

#[test]
fn runtime_snapshot_tool_runtime_json_reports_browser_execution_tiers() {
    let mut runtime = mvp::tools::runtime_config::ToolRuntimeConfig::default();
    runtime.browser_companion.enabled = true;
    runtime.browser_companion.ready = true;
    runtime.browser_companion.command = Some("browser-companion".to_owned());

    let json = runtime_snapshot_tool_runtime_json(&runtime);

    assert_eq!(json["browser"]["execution_tier"], json!("restricted"));
    assert_eq!(
        json["browser_companion"]["execution_tier"],
        json!("balanced")
    );
    assert_eq!(json["web_search"]["enabled"], json!(true));
    assert_eq!(json["web_search"]["default_provider"], json!("duckduckgo"));
    assert_eq!(json["web_search"]["credential_ready"], json!(true));
    assert_eq!(
        json["web_search"]["separation_note"],
        json!(RUNTIME_WEB_ACCESS_SEPARATION_NOTE)
    );
    assert_eq!(
        json["web_access"]["ordinary_network_access_enabled"],
        json!(true)
    );
    assert_eq!(json["web_access"]["query_search_enabled"], json!(true));
    assert_eq!(
        json["web_access"]["query_search_default_provider"],
        json!("duckduckgo")
    );
    assert_eq!(
        json["web_access"]["query_search_credential_ready"],
        json!(true)
    );
}

#[test]
fn runtime_web_access_summary_distinguishes_network_from_query_search_credentials() {
    let mut runtime = mvp::tools::runtime_config::ToolRuntimeConfig::default();
    runtime.web_fetch.enabled = false;
    runtime.web_search.default_provider = mvp::config::WEB_SEARCH_PROVIDER_BRAVE.to_owned();
    runtime.web_search.brave_api_key = None;

    let summary = runtime_web_access_summary(&runtime);

    assert!(!summary.ordinary_network_access_enabled);
    assert!(summary.query_search_enabled);
    assert_eq!(
        summary.query_search_default_provider,
        mvp::config::WEB_SEARCH_PROVIDER_BRAVE
    );
    assert!(!summary.query_search_credential_ready);
}

#[test]
fn runtime_web_access_summary_accepts_firecrawl_credentials() {
    let mut runtime = mvp::tools::runtime_config::ToolRuntimeConfig::default();
    runtime.web_search.default_provider = mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL.to_owned();
    runtime.web_search.firecrawl_api_key = Some("firecrawl-secret".to_owned());

    let summary = runtime_web_access_summary(&runtime);

    assert!(summary.ordinary_network_access_enabled);
    assert!(summary.query_search_enabled);
    assert_eq!(
        summary.query_search_default_provider,
        mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL
    );
    assert!(summary.query_search_credential_ready);
}
