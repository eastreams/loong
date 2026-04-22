use super::*;

#[test]
fn build_channels_cli_json_payload_includes_operation_requirement_metadata() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");
    let surfaces = encoded["channel_surfaces"]
        .as_array()
        .expect("channel surfaces array");

    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("telegram")
                    && entry
                        .get("operations")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|operations| operations.first())
                        .and_then(|operation| operation.get("requirements"))
                        .and_then(serde_json::Value::as_array)
                        .map(|requirements| {
                            requirements
                                .iter()
                                .filter_map(|item| item.get("id"))
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["enabled", "bot_token"])
            })
    );

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("feishu")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("operations"))
                .and_then(serde_json::Value::as_array)
                .and_then(|operations| operations.get(1))
                .and_then(|operation| operation.get("requirements"))
                .and_then(serde_json::Value::as_array)
                .map(|requirements| {
                    requirements
                        .iter()
                        .filter_map(|item| item.get("id"))
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(vec![
                    "enabled",
                    "app_id",
                    "app_secret",
                    "mode",
                    "allowed_chat_ids",
                    "allowed_sender_ids",
                    "verification_token",
                    "encrypt_key",
                ])
    }));
}

#[test]
fn build_channels_cli_json_payload_includes_structured_channel_access_policy_summaries() {
    let mut config = mvp::config::LoongConfig::default();
    config.matrix.enabled = true;
    config.matrix.access_token = Some(loong_contracts::SecretRef::Inline(
        "matrix-token".to_owned(),
    ));
    config.matrix.base_url = Some("https://matrix.example.org".to_owned());
    config.matrix.allowed_room_ids = vec!["!ops:example.org".to_owned()];
    config.matrix.allowed_sender_ids = vec!["@alice:example.org".to_owned()];

    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");
    let access_policies = encoded["channel_access_policies"]
        .as_array()
        .expect("channel access policies array");

    assert!(access_policies.iter().any(|policy| {
        policy.get("channel_id").and_then(serde_json::Value::as_str) == Some("matrix")
            && policy
                .get("conversation_config_key")
                .and_then(serde_json::Value::as_str)
                == Some("allowed_room_ids")
            && policy
                .get("sender_config_key")
                .and_then(serde_json::Value::as_str)
                == Some("allowed_sender_ids")
            && policy
                .get("conversation_mode")
                .and_then(serde_json::Value::as_str)
                == Some("exact_allowlist")
            && policy
                .get("sender_mode")
                .and_then(serde_json::Value::as_str)
                == Some("exact_allowlist")
    }));
}

#[test]
fn build_channels_cli_json_payload_includes_onboarding_metadata() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("telegram")
                    && entry
                        .get("onboarding")
                        .and_then(|onboarding| onboarding.get("strategy"))
                        .and_then(serde_json::Value::as_str)
                        == Some("manual_config")
                    && entry
                        .get("onboarding")
                        .and_then(|onboarding| onboarding.get("status_command"))
                        .and_then(serde_json::Value::as_str)
                        == Some("loong doctor")
                    && entry
                        .get("onboarding")
                        .and_then(|onboarding| onboarding.get("repair_command"))
                        .and_then(serde_json::Value::as_str)
                        == Some("loong doctor --fix")
            })
    );

    assert!(
        encoded["channel_surfaces"]
            .as_array()
            .expect("channel surfaces array")
            .iter()
            .any(|surface| {
                surface
                    .get("catalog")
                    .and_then(|catalog| catalog.get("id"))
                    .and_then(serde_json::Value::as_str)
                    == Some("discord")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("onboarding"))
                        .and_then(|onboarding| onboarding.get("strategy"))
                        .and_then(serde_json::Value::as_str)
                        == Some("manual_config")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("onboarding"))
                        .and_then(|onboarding| onboarding.get("status_command"))
                        .and_then(serde_json::Value::as_str)
                        == Some("loong doctor")
                    && surface
                        .get("catalog")
                        .and_then(|catalog| catalog.get("onboarding"))
                        .and_then(|onboarding| onboarding.get("repair_command"))
                        .and_then(serde_json::Value::as_str)
                        == Some("loong doctor --fix")
            })
    );
}

#[test]
fn build_channels_cli_json_payload_includes_full_channel_catalog() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

    assert_eq!(
        encoded.get("config").and_then(serde_json::Value::as_str),
        Some("/tmp/loong.toml")
    );
    assert_eq!(
        encoded
            .get("schema")
            .and_then(|schema| schema.get("version"))
            .and_then(serde_json::Value::as_u64),
        Some(u64::from(CHANNELS_CLI_JSON_SCHEMA_VERSION))
    );
    assert_eq!(
        encoded
            .get("schema")
            .and_then(|schema| schema.get("primary_channel_view"))
            .and_then(serde_json::Value::as_str),
        Some("channel_surfaces")
    );
    assert_eq!(
        encoded
            .get("schema")
            .and_then(|schema| schema.get("catalog_view"))
            .and_then(serde_json::Value::as_str),
        Some("channel_catalog")
    );
    assert_eq!(
        encoded
            .get("schema")
            .and_then(|schema| schema.get("legacy_channel_views"))
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec!["channels", "catalog_only_channels"])
    );
    assert_eq!(
        encoded
            .get("summary")
            .and_then(|summary| summary.get("total_surface_count"))
            .and_then(serde_json::Value::as_u64),
        Some(inventory.channel_surfaces.len() as u64)
    );
    assert_eq!(
        encoded
            .get("summary")
            .and_then(|summary| summary.get("plugin_backed_surface_count"))
            .and_then(serde_json::Value::as_u64),
        Some(
            inventory
                .channel_surfaces
                .iter()
                .filter(|surface| {
                    surface.catalog.implementation_status
                        == mvp::channel::ChannelCatalogImplementationStatus::PluginBacked
                })
                .count() as u64
        )
    );
    assert_eq!(
        encoded
            .get("channel_catalog")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(inventory.channel_catalog.len())
    );
    assert_eq!(
        encoded
            .get("catalog_only_channels")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(inventory.catalog_only_channels.len())
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("telegram")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("runtime_backed")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("matrix")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("runtime_backed")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("wecom")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("runtime_backed")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("discord")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
                    && entry
                        .get("operations")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|item| item.get("availability"))
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["implemented", "stub"])
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(40)
                    && entry
                        .get("selection_label")
                        .and_then(serde_json::Value::as_str)
                        == Some("community server bot")
                    && entry
                        .get("blurb")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|value| value.contains("config-backed direct sends"))
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("slack")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(50)
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("whatsapp")
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("runtime_backed")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["address"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(90)
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("webhook")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["endpoint"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(110)
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("google-chat")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["endpoint"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(120)
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("teams")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["endpoint", "conversation"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(140)
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("nextcloud-talk")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(160)
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("imessage")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["conversation"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(180)
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("synology-chat")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["address"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(165)
                    && entry
                        .get("implementation_status")
                        .and_then(serde_json::Value::as_str)
                        == Some("config_backed")
            })
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                entry.get("id").and_then(serde_json::Value::as_str) == Some("signal")
                    && entry
                        .get("supported_target_kinds")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                        })
                        == Some(vec!["address"])
                    && entry
                        .get("selection_order")
                        .and_then(serde_json::Value::as_u64)
                        == Some(130)
            })
    );
}

#[test]
fn build_channels_cli_json_payload_includes_grouped_channel_surfaces() {
    let _env = super::MigrationEnvironmentGuard::set(&[("TELEGRAM_BOT_TOKEN", None)]);

    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload = build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize payload");

    assert_eq!(
        encoded
            .get("channel_surfaces")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(inventory.channel_surfaces.len())
    );

    let surfaces = encoded["channel_surfaces"]
        .as_array()
        .expect("channel surfaces array");

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("telegram")
            && surface
                .get("default_configured_account_id")
                .and_then(serde_json::Value::as_str)
                == Some("default")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("telegram"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("telegram"))
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("slack")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("slack"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("slack"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(50)
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("whatsapp")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("whatsapp"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("whatsapp"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(90)
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("signal")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("signal"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("signal"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(130)
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("wecom")
            && surface
                .get("default_configured_account_id")
                .and_then(serde_json::Value::as_str)
                == Some("default")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("wecom"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("wecom"))
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("matrix")
            && surface
                .get("default_configured_account_id")
                .and_then(serde_json::Value::as_str)
                == Some("default")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("matrix"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("matrix"))
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("discord")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("capabilities"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_capability_ids("discord"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("discord"))
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(40)
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("webhook")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("webhook"))
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(1)
    }));

    assert!(surfaces.iter().any(|surface| {
        surface
            .get("catalog")
            .and_then(|catalog| catalog.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some("webchat")
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("selection_order"))
                .and_then(serde_json::Value::as_u64)
                == Some(230)
            && surface
                .get("catalog")
                .and_then(|catalog| catalog.get("supported_target_kinds"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                })
                == Some(channel_supported_target_kinds("webchat"))
            && surface
                .get("configured_accounts")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                == Some(0)
    }));
}
