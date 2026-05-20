use super::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

use crate::test_support::ScopedEnv;
use loong_contracts::SecretRef;

fn encode_provider_descriptor(descriptor: &ProviderDescriptorDocument) -> Value {
    serde_json::to_value(descriptor).expect("serialize provider descriptor document")
}

#[test]
fn provider_profile_lookup_matches_kind() {
    for kind in ProviderKind::all_sorted() {
        assert_eq!(kind.profile().kind, *kind);
    }
}

#[test]
fn provider_profile_aliases_are_unique_and_do_not_shadow_ids() {
    let mut provider_ids = BTreeSet::new();
    for kind in ProviderKind::all_sorted() {
        let profile = kind.profile();
        let provider_id = profile.id;
        let inserted = provider_ids.insert(provider_id);
        assert!(inserted, "duplicate provider id detected: {provider_id}");
    }

    let mut alias_owners = BTreeMap::new();
    for kind in ProviderKind::all_sorted() {
        let profile = kind.profile();
        let provider_id = profile.id;
        for alias in profile.aliases {
            let normalized_alias = alias.trim();
            assert!(
                !normalized_alias.is_empty(),
                "provider `{provider_id}` contains an empty alias"
            );
            assert_ne!(
                normalized_alias, provider_id,
                "provider `{provider_id}` repeats its canonical id as an alias"
            );
            assert!(
                !provider_ids.contains(normalized_alias),
                "provider alias `{normalized_alias}` collides with a canonical provider id"
            );

            let previous_owner = alias_owners.insert(normalized_alias.to_owned(), provider_id);
            assert!(
                previous_owner.is_none(),
                "provider alias `{normalized_alias}` is shared by `{}` and `{provider_id}`",
                previous_owner.unwrap_or_default()
            );
        }
    }
}

#[test]
fn provider_profile_aliases_round_trip_through_provider_kind_deserialization() {
    for kind in ProviderKind::all_sorted() {
        let profile = kind.profile();
        let canonical_id = format!("\"{}\"", profile.id);
        let parsed_canonical = serde_json::from_str::<ProviderKind>(canonical_id.as_str())
            .expect("canonical provider id should deserialize");
        assert_eq!(
            parsed_canonical, *kind,
            "canonical provider id should deserialize to its matching provider kind"
        );

        for alias in profile.aliases {
            let raw_alias = format!("\"{alias}\"");
            let parsed_alias = serde_json::from_str::<ProviderKind>(raw_alias.as_str())
                .expect("provider alias should deserialize");
            assert_eq!(
                parsed_alias, *kind,
                "provider alias `{alias}` should deserialize to the same provider kind as parse_provider_kind_id"
            );
        }
    }
}

#[test]
fn provider_kind_serializes_github_copilot_using_canonical_hyphenated_id() {
    let raw = serde_json::to_string(&ProviderKind::GithubCopilot)
        .expect("provider kind should serialize");

    assert_eq!(raw, "\"github-copilot\"");
}

#[test]
fn provider_feature_family_gate_messages_are_stable() {
    let anthropic_message = ProviderFeatureFamily::Anthropic.disabled_message();
    let bedrock_message = ProviderFeatureFamily::Bedrock.disabled_message();
    let volcengine_message = ProviderFeatureFamily::Volcengine.disabled_message();
    let openai_message = ProviderFeatureFamily::OpenAiCompatible.disabled_message();

    assert_eq!(
        anthropic_message,
        "anthropic provider family is disabled (enable feature `provider-anthropic`)"
    );
    assert_eq!(
        bedrock_message,
        "bedrock provider family is disabled (enable feature `provider-bedrock`)"
    );
    assert_eq!(
        volcengine_message,
        "volcengine provider family is disabled (enable feature `provider-volcengine`)"
    );
    assert_eq!(
        openai_message,
        "openai-compatible provider family is disabled (enable feature `provider-openai`)"
    );
}

#[test]
fn provider_profile_static_auth_hints_are_stable() {
    let byteplus_hint = ProviderKind::ByteplusCoding
        .profile()
        .auth_guidance_hint()
        .expect("byteplus coding should expose auth guidance");
    let volcengine_hint = ProviderKind::Volcengine
        .profile()
        .auth_guidance_hint()
        .expect("volcengine should expose auth guidance");
    let bedrock_hint = ProviderKind::Bedrock
        .profile()
        .alternative_auth_configuration_hint()
        .expect("bedrock should expose a SigV4 fallback hint");
    let custom_hint = ProviderKind::Custom
        .profile()
        .alternative_auth_configuration_hint()
        .expect("custom provider should expose header guidance");

    assert!(byteplus_hint.contains("BytePlus"));
    assert!(byteplus_hint.contains("BYTEPLUS_API_KEY"));
    assert!(byteplus_hint.contains("Authorization: Bearer <BYTEPLUS_API_KEY>"));

    assert!(volcengine_hint.contains("Volcengine"));
    assert!(volcengine_hint.contains("ARK_API_KEY"));
    assert!(volcengine_hint.contains("AK/SK request signing is not used"));

    assert!(bedrock_hint.contains("AWS_ACCESS_KEY_ID"));
    assert!(bedrock_hint.contains("AWS_SECRET_ACCESS_KEY"));
    assert!(bedrock_hint.contains("BEDROCK_AWS_REGION"));
    assert!(bedrock_hint.contains("AWS_REGION"));

    assert!(custom_hint.contains("Authorization"));
    assert!(custom_hint.contains("X-API-Key"));
    assert!(custom_hint.contains("provider.headers"));
}

#[test]
fn provider_configuration_hint_prefers_canonical_cross_routing_guidance() {
    let cases = [
        (
            ProviderKind::Byteplus,
            "https://ark.cn-beijing.volces.com/api/coding/v3",
            "kind = \"volcengine_coding\"",
        ),
        (
            ProviderKind::ByteplusCoding,
            "https://ark.cn-beijing.volces.com",
            "kind = \"volcengine\"",
        ),
    ];

    for (kind, base_url, expected_kind_hint) in cases {
        let provider = ProviderConfig {
            kind,
            base_url: base_url.to_owned(),
            base_url_explicit: true,
            ..ProviderConfig::default()
        };
        let hint = provider
            .configuration_hint()
            .expect("cross-routed provider configs should surface a hint");

        assert!(
            hint.contains(expected_kind_hint),
            "expected cross-routing hint for {kind:?} with {base_url}: {hint}"
        );
    }
}

#[test]
fn provider_configuration_hint_requires_coding_path_even_for_proxy_hosts() {
    let cases = [ProviderKind::ByteplusCoding, ProviderKind::VolcengineCoding];

    for kind in cases {
        let provider = ProviderConfig {
            kind,
            base_url: "https://proxy.example.com/openai/v1".to_owned(),
            base_url_explicit: true,
            ..ProviderConfig::default()
        };
        let hint = provider
            .configuration_hint()
            .expect("coding providers should reject proxy urls that drop the coding path");

        assert!(
            hint.contains("/api/coding/v3"),
            "expected coding-path guidance for {kind:?}: {hint}"
        );
    }
}

#[test]
fn provider_configuration_hint_rejects_non_coding_profiles_on_coding_proxy_paths() {
    let cases = [
        (ProviderKind::Byteplus, "kind = \"byteplus_coding\""),
        (ProviderKind::Volcengine, "kind = \"volcengine_coding\""),
    ];

    for (kind, expected_kind_hint) in cases {
        let provider = ProviderConfig {
            kind,
            base_url: "https://proxy.example.com/api/coding/v3".to_owned(),
            base_url_explicit: true,
            ..ProviderConfig::default()
        };
        let hint = provider
            .configuration_hint()
            .expect("non-coding profiles should reject coding proxy paths");

        assert!(
            hint.contains(expected_kind_hint),
            "expected non-coding guidance for {kind:?}: {hint}"
        );
    }
}

#[test]
fn provider_configuration_hint_allows_proxy_coding_routes_that_keep_the_required_path() {
    let cases = [ProviderKind::ByteplusCoding, ProviderKind::VolcengineCoding];

    for kind in cases {
        let provider = ProviderConfig {
            kind,
            base_url: "https://proxy.example.com/api/coding/v3".to_owned(),
            base_url_explicit: true,
            ..ProviderConfig::default()
        };
        let hint = provider.configuration_hint();

        assert!(
            hint.is_none(),
            "proxy coding routes should stay valid for {kind:?}: {hint:?}"
        );
    }
}

#[test]
fn provider_configuration_hint_checks_explicit_endpoints_before_current_base_url() {
    let provider = ProviderConfig {
        kind: ProviderKind::ByteplusCoding,
        base_url: "https://ark.ap-southeast.bytepluses.com/api/coding/v3".to_owned(),
        base_url_explicit: true,
        endpoint: Some("https://ark.cn-beijing.volces.com/api/v3/chat/completions".to_owned()),
        endpoint_explicit: true,
        ..ProviderConfig::default()
    };
    let hint = provider
        .configuration_hint()
        .expect("explicit cross-routed endpoints should surface a hint");

    assert!(hint.contains("kind = \"volcengine\""));
}

#[test]
fn provider_configuration_hint_requires_coding_path_for_explicit_endpoint_overrides() {
    let cases = [ProviderKind::ByteplusCoding, ProviderKind::VolcengineCoding];

    for kind in cases {
        let provider = ProviderConfig {
            kind,
            endpoint: Some("https://proxy.example.com/openai/v1/chat/completions".to_owned()),
            endpoint_explicit: true,
            ..ProviderConfig::default()
        };
        let hint = provider
            .configuration_hint()
            .expect("explicit endpoint overrides should keep the coding path");

        assert!(
            hint.contains("/api/coding/v3"),
            "expected coding-path guidance for {kind:?}: {hint}"
        );
    }
}

#[test]
fn provider_configuration_hint_requires_coding_path_for_explicit_models_endpoints() {
    let cases = [ProviderKind::ByteplusCoding, ProviderKind::VolcengineCoding];

    for kind in cases {
        let provider = ProviderConfig {
            kind,
            models_endpoint: Some("https://proxy.example.com/openai/v1/models".to_owned()),
            models_endpoint_explicit: true,
            ..ProviderConfig::default()
        };
        let hint = provider
            .configuration_hint()
            .expect("explicit models endpoints should keep the coding path");

        assert!(
            hint.contains("/api/coding/v3"),
            "expected coding-path guidance for {kind:?}: {hint}"
        );
    }
}

#[test]
fn provider_configuration_hint_ignores_non_canonical_host_only_proxy_paths() {
    let provider = ProviderConfig {
        kind: ProviderKind::Byteplus,
        base_url: "https://ark.cn-beijing.volces.com/custom/proxy".to_owned(),
        base_url_explicit: true,
        ..ProviderConfig::default()
    };
    let hint = provider.configuration_hint();

    assert!(
        hint.is_none(),
        "non-canonical host-only proxy paths should not claim a canonical cross-route: {hint:?}"
    );
}

#[test]
fn provider_feature_support_facts_are_stable() {
    let facts = ProviderFeatureFamily::Volcengine.support_facts();

    assert_eq!(facts.family, ProviderFeatureFamily::Volcengine);
    assert_eq!(facts.gate_name, "provider-volcengine");
    assert_eq!(
        facts.enabled_in_build,
        ProviderFeatureFamily::Volcengine.is_enabled_in_build()
    );
    assert_eq!(
        facts.disabled_message,
        "volcengine provider family is disabled (enable feature `provider-volcengine`)"
    );
}

#[test]
fn provider_support_facts_preserve_auth_guidance_and_missing_auth_message() {
    let provider = ProviderConfig {
        kind: ProviderKind::ByteplusCoding,
        ..ProviderConfig::default()
    };

    let support_facts = provider.support_facts();
    let auth_support = support_facts.auth;
    let guidance_hint = auth_support
        .guidance_hint
        .expect("byteplus coding should expose auth guidance");

    assert!(auth_support.requires_explicit_configuration);
    assert!(
        auth_support
            .hint_env_names
            .contains(&"BYTEPLUS_API_KEY".to_owned())
    );
    assert!(guidance_hint.contains("BytePlus"));
    assert!(guidance_hint.contains("BYTEPLUS_API_KEY"));
    assert!(guidance_hint.contains("Authorization: Bearer <BYTEPLUS_API_KEY>"));
    assert!(
        auth_support
            .missing_configuration_message
            .contains("BYTEPLUS_API_KEY")
    );
    assert!(
        auth_support
            .missing_configuration_message
            .contains("BytePlus")
    );
}

#[test]
fn auth_hint_env_names_respect_explicit_api_key_env_binding_precedence() {
    let provider = ProviderConfig {
        kind: ProviderKind::Openai,
        api_key: Some(SecretRef::Env {
            env: "TEAM_OPENAI_KEY".to_owned(),
        }),
        ..ProviderConfig::default()
    };

    let env_names = provider.auth_hint_env_names();

    assert_eq!(env_names, vec!["TEAM_OPENAI_KEY".to_owned()]);
}

#[test]
fn auth_hint_env_names_keep_api_key_fallback_after_explicit_oauth_env_binding() {
    let provider = ProviderConfig {
        kind: ProviderKind::Openai,
        oauth_access_token: Some(SecretRef::Env {
            env: "TEAM_OPENAI_OAUTH".to_owned(),
        }),
        ..ProviderConfig::default()
    };

    let env_names = provider.auth_hint_env_names();

    assert_eq!(
        env_names,
        vec!["TEAM_OPENAI_OAUTH".to_owned(), "OPENAI_API_KEY".to_owned(),]
    );
}

#[test]
fn provider_support_facts_preserve_region_endpoint_hints() {
    let provider = ProviderConfig {
        kind: ProviderKind::Minimax,
        ..ProviderConfig::default()
    };

    let support_facts = provider.support_facts();
    let region_endpoint_support = support_facts.region_endpoint;
    let note = region_endpoint_support
        .note
        .expect("minimax should expose a region endpoint note");
    let catalog_failure_hint = region_endpoint_support
        .catalog_failure_hint
        .expect("minimax should expose a catalog failure hint");
    let request_failure_hint = region_endpoint_support
        .request_failure_hint
        .expect("minimax should expose a request failure hint");

    assert!(note.contains("MiniMax region endpoint"));
    assert!(note.contains("https://api.minimaxi.com"));
    assert!(note.contains("https://api.minimax.io"));
    assert!(catalog_failure_hint.contains("https://api.minimaxi.com"));
    assert!(catalog_failure_hint.contains("https://api.minimax.io"));
    assert!(request_failure_hint.contains("https://api.minimaxi.com"));
    assert!(request_failure_hint.contains("https://api.minimax.io"));
}

#[test]
fn step_plan_provider_support_facts_expose_region_endpoint_variants() {
    let provider = ProviderConfig {
        kind: ProviderKind::StepPlan,
        ..ProviderConfig::default()
    };

    let support_facts = provider.support_facts();
    let region_endpoint_support = support_facts.region_endpoint;
    let note = region_endpoint_support
        .note
        .expect("step plan should expose a region endpoint note");
    let catalog_failure_hint = region_endpoint_support
        .catalog_failure_hint
        .expect("step plan should expose a catalog failure hint");
    let request_failure_hint = region_endpoint_support
        .request_failure_hint
        .expect("step plan should expose a request failure hint");

    assert!(note.contains("Stepfun"));
    assert!(note.contains("https://api.stepfun.com"));
    assert!(note.contains("https://api.stepfun.ai"));
    assert!(catalog_failure_hint.contains("https://api.stepfun.com"));
    assert!(catalog_failure_hint.contains("https://api.stepfun.ai"));
    assert!(request_failure_hint.contains("https://api.stepfun.com"));
    assert!(request_failure_hint.contains("https://api.stepfun.ai"));
}

#[test]
fn provider_descriptor_document_preserves_x_api_key_contract_facts() {
    let provider = ProviderConfig {
        kind: ProviderKind::Anthropic,
        ..ProviderConfig::default()
    };

    let descriptor = provider.descriptor_document();
    let encoded = encode_provider_descriptor(&descriptor);
    let hint_env_names = encoded["auth"]["hint_env_names"]
        .as_array()
        .expect("hint env names should be an array");

    assert_eq!(
        encoded["schema"]["version"],
        json!(PROVIDER_DESCRIPTOR_SCHEMA_VERSION)
    );
    assert_eq!(encoded["schema"]["surface"], json!("provider_descriptor"));
    assert_eq!(encoded["schema"]["purpose"], json!("internal_sdk_contract"));
    assert_eq!(encoded["kind"], json!("anthropic"));
    assert_eq!(encoded["display_name"], json!("Anthropic"));
    assert_eq!(encoded["protocol_family"], json!("anthropic_messages"));
    assert_eq!(encoded["feature"]["family"], json!("anthropic"));
    assert_eq!(encoded["auth"]["scheme"], json!("x_api_key"));
    assert_eq!(
        encoded["auth"]["default_api_key_env"],
        json!("ANTHROPIC_API_KEY")
    );
    assert_eq!(
        encoded["auth"]["requires_explicit_configuration"],
        json!(true)
    );
    assert!(
        hint_env_names.contains(&json!("ANTHROPIC_API_KEY")),
        "anthropic descriptor should surface the canonical x-api-key env hint"
    );
    assert_eq!(
        encoded["default_headers"][0]["name"],
        json!("anthropic-version")
    );
}

#[test]
fn provider_descriptor_document_marks_auth_optional_profiles_without_required_envs() {
    let provider = ProviderConfig {
        kind: ProviderKind::Ollama,
        ..ProviderConfig::default()
    };

    let descriptor = provider.descriptor_document();
    let encoded = encode_provider_descriptor(&descriptor);

    assert_eq!(encoded["kind"], json!("ollama"));
    assert_eq!(encoded["auth"]["scheme"], json!("bearer"));
    assert_eq!(encoded["auth"]["auth_optional"], json!(true));
    assert_eq!(encoded["auth"]["model_probe_auth_optional"], json!(true));
    assert_eq!(
        encoded["auth"]["requires_explicit_configuration"],
        json!(false)
    );
    assert_eq!(encoded["auth"]["hint_env_names"], json!([]));
    assert_eq!(encoded["region_endpoint"]["variants"], json!([]));
}

#[test]
fn provider_descriptor_document_prefers_dynamic_configuration_hint() {
    let provider = ProviderConfig {
        kind: ProviderKind::Custom,
        ..ProviderConfig::default()
    };

    let descriptor = provider.descriptor_document();
    let encoded = encode_provider_descriptor(&descriptor);
    let configuration_hint = encoded["configuration_hint"]
        .as_str()
        .expect("custom descriptor should expose a configuration hint");

    assert!(configuration_hint.contains("tenant-scoped base_url configuration"));
    assert!(configuration_hint.contains("current template"));
    assert!(configuration_hint.contains("https://<openai-compatible-host>/v1"));
}

#[test]
fn provider_descriptor_document_preserves_region_endpoint_variants_and_hints() {
    let provider = ProviderConfig {
        kind: ProviderKind::Minimax,
        ..ProviderConfig::default()
    };

    let descriptor = provider.descriptor_document();
    let encoded = encode_provider_descriptor(&descriptor);
    let note = encoded["region_endpoint"]["note"]
        .as_str()
        .expect("minimax descriptor should expose a region note");
    let catalog_failure_hint = encoded["region_endpoint"]["catalog_failure_hint"]
        .as_str()
        .expect("minimax descriptor should expose a catalog failure hint");
    let request_failure_hint = encoded["region_endpoint"]["request_failure_hint"]
        .as_str()
        .expect("minimax descriptor should expose a request failure hint");

    assert_eq!(encoded["region_endpoint"]["family_label"], json!("MiniMax"));
    assert_eq!(
        encoded["region_endpoint"]["variants"][0]["label"],
        json!("CN")
    );
    assert_eq!(
        encoded["region_endpoint"]["variants"][0]["base_url"],
        json!("https://api.minimaxi.com")
    );
    assert_eq!(
        encoded["region_endpoint"]["variants"][1]["label"],
        json!("Global")
    );
    assert_eq!(
        encoded["region_endpoint"]["variants"][1]["base_url"],
        json!("https://api.minimax.io")
    );
    assert!(note.contains("MiniMax region endpoint"));
    assert!(catalog_failure_hint.contains("https://api.minimaxi.com"));
    assert!(catalog_failure_hint.contains("https://api.minimax.io"));
    assert!(request_failure_hint.contains("https://api.minimaxi.com"));
    assert!(request_failure_hint.contains("https://api.minimax.io"));
}

#[test]
fn step_plan_descriptor_document_preserves_region_endpoint_variants_and_hints() {
    let provider = ProviderConfig {
        kind: ProviderKind::StepPlan,
        ..ProviderConfig::default()
    };

    let descriptor = provider.descriptor_document();
    let encoded = encode_provider_descriptor(&descriptor);
    let note = encoded["region_endpoint"]["note"]
        .as_str()
        .expect("step plan descriptor should expose a region note");
    let catalog_failure_hint = encoded["region_endpoint"]["catalog_failure_hint"]
        .as_str()
        .expect("step plan descriptor should expose a catalog failure hint");
    let request_failure_hint = encoded["region_endpoint"]["request_failure_hint"]
        .as_str()
        .expect("step plan descriptor should expose a request failure hint");

    assert_eq!(encoded["region_endpoint"]["family_label"], json!("Stepfun"));
    assert_eq!(
        encoded["region_endpoint"]["variants"][0]["label"],
        json!("CN")
    );
    assert_eq!(
        encoded["region_endpoint"]["variants"][0]["base_url"],
        json!("https://api.stepfun.com")
    );
    assert_eq!(
        encoded["region_endpoint"]["variants"][1]["label"],
        json!("Global")
    );
    assert_eq!(
        encoded["region_endpoint"]["variants"][1]["base_url"],
        json!("https://api.stepfun.ai")
    );
    assert!(note.contains("https://api.stepfun.com"));
    assert!(note.contains("https://api.stepfun.ai"));
    assert!(catalog_failure_hint.contains("https://api.stepfun.com"));
    assert!(catalog_failure_hint.contains("https://api.stepfun.ai"));
    assert!(request_failure_hint.contains("https://api.stepfun.com"));
    assert!(request_failure_hint.contains("https://api.stepfun.ai"));
}

#[test]
fn custom_models_endpoint_avoids_double_v1_suffix() {
    let config = ProviderConfig {
        kind: ProviderKind::Custom,
        base_url: "https://example.test/v1".to_owned(),
        ..ProviderConfig::default()
    };

    assert_eq!(
        config.endpoint(),
        "https://example.test/v1/chat/completions"
    );
    assert_eq!(config.models_endpoint(), "https://example.test/v1/models");
}

#[test]
fn explicit_api_key_binding_beats_default_oauth_fallback() {
    let mut env = ScopedEnv::new();
    env.set("OPENAI_API_KEY", "api-key-wins");
    env.set("OPENAI_CODEX_OAUTH_TOKEN", "oauth-fallback-should-not-win");

    let config = ProviderConfig {
        kind: ProviderKind::Openai,
        api_key: Some(SecretRef::Inline("${OPENAI_API_KEY}".to_owned())),
        ..ProviderConfig::default()
    };

    assert_eq!(config.oauth_access_token(), None);
    assert_eq!(config.api_key().as_deref(), Some("api-key-wins"));
    assert_eq!(
        config.resolved_auth_secret().as_deref(),
        Some("api-key-wins")
    );
    assert_eq!(
        config.authorization_header().as_deref(),
        Some("Bearer api-key-wins")
    );
}

#[test]
fn explicit_api_key_env_field_beats_default_oauth_fallback() {
    let mut env = ScopedEnv::new();
    env.set("OPENAI_API_KEY", "api-key-wins");
    env.set("OPENAI_CODEX_OAUTH_TOKEN", "oauth-fallback-should-not-win");

    let config: ProviderConfig = toml::from_str(
        r#"
kind = "openai"
api_key_env = "OPENAI_API_KEY"
"#,
    )
    .expect("deserialize provider config");

    assert_eq!(config.oauth_access_token(), None);
    assert_eq!(config.api_key().as_deref(), Some("api-key-wins"));
    assert_eq!(
        config.resolved_auth_secret().as_deref(),
        Some("api-key-wins")
    );
    assert_eq!(
        config.authorization_header().as_deref(),
        Some("Bearer api-key-wins")
    );
}

#[test]
fn normalized_for_persistence_canonicalizes_legacy_api_key_env_binding() {
    let mut config = ProviderConfig::fresh_for_kind(ProviderKind::Openai);
    config.set_api_key_env(Some("OPENAI_API_KEY".to_owned()));

    let normalized = config.normalized_for_persistence();

    assert_eq!(
        normalized.api_key,
        Some(SecretRef::Env {
            env: "OPENAI_API_KEY".to_owned(),
        })
    );
    assert_eq!(normalized.api_key_env, None);
    assert_eq!(normalized.oauth_access_token, None);
    assert_eq!(normalized.oauth_access_token_env, None);
}

#[test]
fn normalized_for_persistence_keeps_secret_ref_env_binding_canonical() {
    let mut config = ProviderConfig::fresh_for_kind(ProviderKind::Openai);
    config.api_key = Some(SecretRef::Env {
        env: "OPENAI_API_KEY".to_owned(),
    });

    let normalized = config.normalized_for_persistence();

    assert_eq!(
        normalized.api_key,
        Some(SecretRef::Env {
            env: "OPENAI_API_KEY".to_owned(),
        })
    );
    assert_eq!(normalized.api_key_env, None);
}

#[test]
fn normalized_for_persistence_keeps_implicit_provider_auth_defaults_unset() {
    let config = ProviderConfig::fresh_for_kind(ProviderKind::Openai);

    let normalized = config.normalized_for_persistence();

    assert_eq!(normalized.api_key, None);
    assert_eq!(normalized.api_key_env, None);
    assert_eq!(normalized.oauth_access_token, None);
    assert_eq!(normalized.oauth_access_token_env, None);
}

#[test]
fn canonicalize_configured_auth_env_bindings_rewrites_inline_env_templates() {
    let mut config = ProviderConfig::fresh_for_kind(ProviderKind::Openai);
    config.set_api_key_env(Some("OPENAI_API_KEY".to_owned()));
    config.api_key = Some(SecretRef::Inline("${OPENAI_API_KEY}".to_owned()));

    config.canonicalize_configured_auth_env_bindings();

    assert_eq!(
        config.api_key,
        Some(SecretRef::Env {
            env: "OPENAI_API_KEY".to_owned(),
        })
    );
    assert_eq!(config.api_key_env, None);
}

#[test]
fn canonicalize_configured_auth_env_bindings_treats_blank_inline_secret_as_absent() {
    let mut config = ProviderConfig::fresh_for_kind(ProviderKind::Openai);
    config.set_api_key_env(Some("OPENAI_API_KEY".to_owned()));
    config.api_key = Some(SecretRef::Inline("   ".to_owned()));

    config.canonicalize_configured_auth_env_bindings();

    assert_eq!(
        config.api_key,
        Some(SecretRef::Env {
            env: "OPENAI_API_KEY".to_owned(),
        })
    );
    assert_eq!(config.api_key_env, None);
}

#[test]
fn fresh_minimax_provider_does_not_seed_hidden_preferred_models() {
    let config = ProviderConfig::fresh_for_kind(ProviderKind::Minimax);

    assert_eq!(config.model, "auto");
    assert!(
        config.preferred_models.is_empty(),
        "provider defaults should not inject hidden runtime fallback models: {config:#?}"
    );
}

#[test]
fn configured_auto_model_candidates_require_explicit_preferred_models() {
    let config = ProviderConfig {
        kind: ProviderKind::Minimax,
        model: "auto".to_owned(),
        ..ProviderConfig::default()
    };

    assert!(
        config.configured_auto_model_candidates().is_empty(),
        "auto-model fallback candidates should only exist when the operator configured preferred_models explicitly"
    );
}

#[test]
fn only_reviewed_providers_expose_onboarding_models() {
    assert_eq!(
        ProviderKind::Deepseek.recommended_onboarding_model(),
        Some("deepseek-chat")
    );
    assert_eq!(
        ProviderKind::Minimax.recommended_onboarding_model(),
        Some("MiniMax-M2.7")
    );
    assert_eq!(
        ProviderKind::Xiaomi.recommended_onboarding_model(),
        Some("mimo-v2-pro")
    );
    assert_eq!(
        ProviderKind::KimiCoding.recommended_onboarding_model(),
        None
    );
    assert_eq!(ProviderKind::Openai.recommended_onboarding_model(), None);
}

#[test]
fn model_catalog_probe_recovery_requires_explicit_model_for_reviewed_auto_provider() {
    let config = ProviderConfig {
        kind: ProviderKind::Deepseek,
        model: "auto".to_owned(),
        ..ProviderConfig::default()
    };

    assert_eq!(
        config.model_catalog_probe_recovery(),
        ModelCatalogProbeRecovery::RequiresExplicitModel {
            recommended_onboarding_model: Some("deepseek-chat"),
        }
    );
}

#[test]
fn model_catalog_probe_recovery_prefers_explicit_runtime_configuration() {
    let explicit = ProviderConfig {
        kind: ProviderKind::Deepseek,
        model: "deepseek-chat".to_owned(),
        ..ProviderConfig::default()
    };
    assert_eq!(
        explicit.model_catalog_probe_recovery(),
        ModelCatalogProbeRecovery::ExplicitModel("deepseek-chat".to_owned())
    );

    let preferred = ProviderConfig {
        kind: ProviderKind::Deepseek,
        model: "auto".to_owned(),
        preferred_models: vec![
            "deepseek-chat".to_owned(),
            "deepseek-chat".to_owned(),
            "deepseek-reasoner".to_owned(),
        ],
        ..ProviderConfig::default()
    };
    assert_eq!(
        preferred.model_catalog_probe_recovery(),
        ModelCatalogProbeRecovery::ConfiguredPreferredModels(vec![
            "deepseek-chat".to_owned(),
            "deepseek-reasoner".to_owned(),
        ])
    );
}
