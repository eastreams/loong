    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::fs::OpenOptions;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::kernel::{
        AuditEvent, AuditEventKind, AuditSink, Capability, CapabilityToken, ExecutionPlane,
        PlaneTier,
    };
    use crate::test_support::ScopedEnv;

    use super::*;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let counter = UNIQUE_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{counter}", std::process::id()))
    }

    fn write_audit_config_with_mode(
        root: &Path,
        journal_path: &Path,
        mode: crate::mvp::config::AuditMode,
    ) -> PathBuf {
        fs::create_dir_all(root).expect("create config root");
        let config_path = root.join("loong.toml");
        let mut config = crate::mvp::config::LoongConfig::default();
        config.audit.mode = mode;
        config.audit.path = journal_path.display().to_string();
        crate::mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
            .expect("write audit config");
        config_path
    }

    fn write_audit_config(root: &Path, journal_path: &Path) -> PathBuf {
        write_audit_config_with_mode(root, journal_path, crate::mvp::config::AuditMode::Fanout)
    }

    fn sample_audit_event(
        event_id: &str,
        timestamp_epoch_s: u64,
        agent_id: Option<&str>,
        kind: AuditEventKind,
    ) -> AuditEvent {
        AuditEvent {
            event_id: event_id.to_owned(),
            timestamp_epoch_s,
            agent_id: agent_id.map(str::to_owned),
            kind,
        }
    }

    fn write_journal(path: &Path, events: &[AuditEvent]) {
        let parent = path.parent().expect("journal path should have parent");
        fs::create_dir_all(parent).expect("create journal parent");
        let encoded = events
            .iter()
            .map(|event| serde_json::to_string(event).expect("serialize audit event"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, format!("{encoded}\n")).expect("write audit journal");
    }

    #[test]
    fn audit_recent_execution_keeps_last_events_in_order() {
        let root = unique_temp_dir("loong-audit-cli-recent");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_001,
                    Some("agent-a"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-1".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_002,
                    Some("agent-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-2".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_003,
                    Some("agent-c"),
                    AuditEventKind::PlaneInvoked {
                        pack_id: "sales-intel".to_owned(),
                        plane: ExecutionPlane::Tool,
                        tier: PlaneTier::Core,
                        primary_adapter: "mvp-tools".to_owned(),
                        delegated_core_adapter: None,
                        operation: "tool.call".to_owned(),
                        required_capabilities: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 2,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent");

        assert_eq!(execution.journal_path, journal_path.display().to_string());
        match execution.result {
            AuditCommandResult::Recent { limit, events } => {
                assert_eq!(limit, 2);
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-2", "evt-3"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_recent_json_includes_loaded_events_and_journal_path() {
        let root = unique_temp_dir("loong-audit-cli-recent-json");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[sample_audit_event(
                "evt-json",
                1_700_010_010,
                Some("agent-json"),
                AuditEventKind::TokenRevoked {
                    token_id: "token-json".to_owned(),
                },
            )],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: true,
            command: AuditCommands::Recent {
                limit: 5,
                since_epoch_s: Some(1_700_010_000),
                until_epoch_s: Some(1_700_010_050),
                pack_id: None,
                agent_id: None,
                event_id: Some("evt-json".to_owned()),
                token_id: Some("token-json".to_owned()),
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent");
        let payload = audit_cli_json(&execution);

        assert_eq!(payload["journal_path"], journal_path.display().to_string());
        assert_eq!(payload["limit"], 5);
        assert_eq!(payload["since_epoch_s_filter"], 1_700_010_000_u64);
        assert_eq!(payload["until_epoch_s_filter"], 1_700_010_050_u64);
        assert_eq!(payload["event_id_filter"], "evt-json");
        assert_eq!(payload["token_id_filter"], "token-json");
        assert_eq!(payload["loaded_events"], 1);
        assert_eq!(payload["events"][0]["event_id"], "evt-json");
    }

    #[test]
    fn audit_recent_waits_for_existing_audit_journal_lock_before_reading() {
        let root = unique_temp_dir("loong-audit-cli-recent-lock");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[sample_audit_event(
                "evt-lock",
                1_700_010_020,
                Some("agent-lock"),
                AuditEventKind::TokenRevoked {
                    token_id: "token-lock".to_owned(),
                },
            )],
        );

        let external_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&journal_path)
            .expect("open external audit journal handle");
        external_lock
            .lock()
            .expect("hold external audit journal lock");

        let (tx, rx) = mpsc::channel();
        let config = config_path.display().to_string();
        let handle = thread::spawn(move || {
            let result = execute_audit_command(AuditCommandOptions {
                config: Some(config),
                json: false,
                command: AuditCommands::Recent {
                    limit: 10,
                    since_epoch_s: None,
                    until_epoch_s: None,
                    pack_id: None,
                    agent_id: None,
                    event_id: None,
                    token_id: None,
                    kind: None,
                    triage_label: None,
                    query_contains: None,
                    trust_tier: None,
                },
            });
            tx.send(result).expect("send audit recent result");
        });

        match rx.recv_timeout(Duration::from_millis(100)) {
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Ok(result) => {
                panic!("audit recent should block on external journal lock, got {result:?}")
            }
            Err(error) => panic!("audit recent channel closed unexpectedly: {error:?}"),
        }

        external_lock
            .unlock()
            .expect("release external audit journal lock");
        let execution = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("audit recent should finish after lock release")
            .expect("audit recent should succeed after lock release");
        handle.join().expect("join audit recent reader");

        match execution.result {
            AuditCommandResult::Recent { events, .. } => {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].event_id, "evt-lock");
            }
            other => panic!("unexpected audit command result after lock release: {other:?}"),
        }
    }

    #[test]
    fn audit_recent_rejects_zero_limit() {
        let mut env = ScopedEnv::new();
        env.set("HOME", unique_temp_dir("loong-audit-cli-missing-home"));

        let error = execute_audit_command(AuditCommandOptions {
            config: None,
            json: false,
            command: AuditCommands::Recent {
                limit: 0,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect_err("zero recent limit should fail");

        assert!(error.contains("audit recent limit must be between 1 and 10000"));
    }

    #[test]
    fn audit_recent_rejects_excessive_limit() {
        let mut env = ScopedEnv::new();
        env.set("HOME", unique_temp_dir("loong-audit-cli-large-limit-home"));

        let error = execute_audit_command(AuditCommandOptions {
            config: None,
            json: false,
            command: AuditCommands::Recent {
                limit: 10_001,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect_err("excessive recent limit should fail");

        assert!(error.contains("audit recent limit must be between 1 and 10000"));
    }

    #[test]
    fn audit_recent_text_renders_tool_search_trust_conflict_details() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loong.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_000),
            until_epoch_s_filter: Some(1_700_010_060),
            pack_id_filter: None,
            agent_id_filter: None,
            event_id_filter: Some("evt-tool-search".to_owned()),
            token_id_filter: None,
            kind_filter: None,
            triage_label_filter: None,
            query_contains_filter: Some("trust:official".to_owned()),
            trust_tier_filter: Some("official".to_owned()),
            result: AuditCommandResult::Recent {
                limit: 5,
                events: vec![sample_audit_event(
                    "evt-tool-search",
                    1_700_010_021,
                    Some("agent-search"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: vec!["verified-community".to_owned()],
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: true,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("official".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                )],
            },
        };

        let rendered = render_audit_cli_text(&execution).expect("render audit recent");

        assert!(rendered.contains("since_epoch_s=1700010000"));
        assert!(rendered.contains("until_epoch_s=1700010060"));
        assert!(rendered.contains(
            "pack_id=- agent_id=- event_id=evt-tool-search token_id=- kind=- triage_label=-"
        ));
        assert!(rendered.contains("query_contains=trust:official"));
        assert!(rendered.contains("trust_tier=official"));
        assert!(rendered.contains("kind=ToolSearchEvaluated"));
        assert!(rendered.contains("query=\"trust:official search\""));
        assert!(rendered.contains("returned=0"));
        assert!(rendered.contains("trust_scope=-"));
        assert!(rendered.contains("conflicting_requested_tiers=true"));
        assert!(rendered.contains("filtered_out_candidates=2"));
        assert!(rendered.contains("filtered_out_tier_counts=official=1,verified-community=1"));
    }

    #[test]
    fn audit_recent_filters_by_kind_and_uses_filtered_window_limit() {
        let root = unique_temp_dir("loong-audit-cli-recent-kind-filter");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_030,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_031,
                    Some("agent-b"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: Vec::new(),
                        structured_requested_tiers: vec!["official".to_owned()],
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 1,
                        filtered_out_tier_counts: BTreeMap::from([(
                            "verified-community".to_owned(),
                            1_usize,
                        )]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_032,
                    Some("agent-c"),
                    AuditEventKind::PluginTrustEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        scanned_plugins: 1,
                        official_plugins: 0,
                        verified_community_plugins: 0,
                        unverified_plugins: 1,
                        high_risk_plugins: 1,
                        high_risk_unverified_plugins: 1,
                        blocked_auto_apply_plugins: 1,
                        review_required_plugin_ids: vec!["stdio-review".to_owned()],
                        review_required_bridges: vec!["process_stdio".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_033,
                    Some("agent-d"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: vec!["verified-community".to_owned()],
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: true,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("official".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 1,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: Some("ToolSearchEvaluated".to_owned()),
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute filtered audit recent");

        assert_eq!(
            execution.kind_filter.as_deref(),
            Some("ToolSearchEvaluated")
        );
        assert_eq!(execution.triage_label_filter, None);
        match execution.result {
            AuditCommandResult::Recent { limit, events } => {
                assert_eq!(limit, 1);
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].event_id, "evt-4");
                assert!(matches!(
                    events[0].kind,
                    AuditEventKind::ToolSearchEvaluated { .. }
                ));
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_recent_filters_by_query_contains_and_trust_tier() {
        let root = unique_temp_dir("loong-audit-cli-recent-tool-search-filter");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_034,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_035,
                    Some("agent-b"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official search".to_owned(),
                        returned: 1,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: Vec::new(),
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 0,
                        filtered_out_tier_counts: BTreeMap::new(),
                        top_provider_ids: vec!["official-search".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_036,
                    Some("agent-c"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official search".to_owned(),
                        returned: 1,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["verified-community".to_owned()],
                        structured_requested_tiers: Vec::new(),
                        effective_tiers: vec!["verified-community".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 0,
                        filtered_out_tier_counts: BTreeMap::new(),
                        top_provider_ids: vec!["community-search".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_037,
                    Some("agent-d"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "catalog search".to_owned(),
                        returned: 1,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: Vec::new(),
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 0,
                        filtered_out_tier_counts: BTreeMap::new(),
                        top_provider_ids: vec!["official-search".to_owned()],
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: Some("ToolSearchEvaluated".to_owned()),
                triage_label: None,
                query_contains: Some("trust:official".to_owned()),
                trust_tier: Some("official".to_owned()),
            },
        })
        .expect("execute audit recent with tool search filters");

        assert_eq!(
            execution.query_contains_filter.as_deref(),
            Some("trust:official")
        );
        assert_eq!(execution.trust_tier_filter.as_deref(), Some("official"));
        match execution.result {
            AuditCommandResult::Recent { events, .. } => {
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-2"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_recent_filters_by_time_window_inclusively() {
        let root = unique_temp_dir("loong-audit-cli-recent-time-window");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_030,
                    Some("agent-a"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-1".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_031,
                    Some("agent-b"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-2".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_032,
                    Some("agent-c"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-3".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_033,
                    Some("agent-d"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-4".to_owned(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: Some(1_700_010_031),
                until_epoch_s: Some(1_700_010_032),
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent with time window");

        assert_eq!(execution.since_epoch_s_filter, Some(1_700_010_031));
        assert_eq!(execution.until_epoch_s_filter, Some(1_700_010_032));
        match execution.result {
            AuditCommandResult::Recent { events, .. } => {
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-2", "evt-3"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_recent_filters_by_pack_id_and_agent_id() {
        let root = unique_temp_dir("loong-audit-cli-recent-pack-agent-filter");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_035,
                    Some("agent-a"),
                    AuditEventKind::TokenIssued {
                        token: CapabilityToken {
                            token_id: "token-1".to_owned(),
                            pack_id: "sales-intel".to_owned(),
                            agent_id: "agent-a".to_owned(),
                            allowed_capabilities: Default::default(),
                            issued_at_epoch_s: 1_700_010_035,
                            expires_at_epoch_s: 1_700_010_135,
                            generation: 0,
                        },
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_036,
                    Some("agent-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "ops-pack".to_owned(),
                        token_id: "token-2".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_037,
                    Some("agent-c"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-3".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_038,
                    Some("agent-a"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "search".to_owned(),
                        returned: 1,
                        trust_filter_applied: false,
                        query_requested_tiers: Vec::new(),
                        structured_requested_tiers: Vec::new(),
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 0,
                        filtered_out_tier_counts: BTreeMap::new(),
                        top_provider_ids: vec!["official-search".to_owned()],
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: Some("sales-intel".to_owned()),
                agent_id: Some("agent-a".to_owned()),
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent with pack and agent filters");

        assert_eq!(execution.pack_id_filter.as_deref(), Some("sales-intel"));
        assert_eq!(execution.agent_id_filter.as_deref(), Some("agent-a"));
        match execution.result {
            AuditCommandResult::Recent { events, .. } => {
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-1", "evt-4"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_recent_filters_by_event_id_and_token_id() {
        let root = unique_temp_dir("loong-audit-cli-recent-event-token-filter");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_039,
                    Some("agent-a"),
                    AuditEventKind::TokenIssued {
                        token: CapabilityToken {
                            token_id: "token-shared".to_owned(),
                            pack_id: "sales-intel".to_owned(),
                            agent_id: "agent-a".to_owned(),
                            allowed_capabilities: Default::default(),
                            issued_at_epoch_s: 1_700_010_039,
                            expires_at_epoch_s: 1_700_010_139,
                            generation: 0,
                        },
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_040,
                    Some("agent-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-shared".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_041,
                    Some("agent-c"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-shared".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_042,
                    Some("agent-d"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-other".to_owned(),
                    },
                ),
            ],
        );

        let by_event = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: Some("evt-2".to_owned()),
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent with event id filter");

        let by_token = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: Some("token-shared".to_owned()),
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent with token id filter");

        let combined = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: Some("evt-3".to_owned()),
                token_id: Some("token-shared".to_owned()),
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent with event and token filters");

        let mismatch = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: Some("evt-4".to_owned()),
                token_id: Some("token-shared".to_owned()),
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent with mismatched event and token filters");

        assert_eq!(by_event.event_id_filter.as_deref(), Some("evt-2"));
        assert_eq!(by_event.token_id_filter, None);
        match by_event.result {
            AuditCommandResult::Recent { events, .. } => {
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-2"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }

        assert_eq!(by_token.event_id_filter, None);
        assert_eq!(by_token.token_id_filter.as_deref(), Some("token-shared"));
        match by_token.result {
            AuditCommandResult::Recent { events, .. } => {
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-1", "evt-2", "evt-3"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }

        assert_eq!(combined.event_id_filter.as_deref(), Some("evt-3"));
        assert_eq!(combined.token_id_filter.as_deref(), Some("token-shared"));
        match combined.result {
            AuditCommandResult::Recent { events, .. } => {
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-3"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }

        match mismatch.result {
            AuditCommandResult::Recent { events, .. } => {
                assert!(events.is_empty());
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_token_trail_filters_token_events_and_summarizes_lifecycle() {
        let root = unique_temp_dir("loong-audit-cli-token-trail");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_060,
                    Some("agent-issue"),
                    AuditEventKind::TokenIssued {
                        token: CapabilityToken {
                            token_id: "token-shared".to_owned(),
                            pack_id: "sales-intel".to_owned(),
                            agent_id: "agent-issue".to_owned(),
                            allowed_capabilities: BTreeSet::from([
                                Capability::InvokeTool,
                                Capability::NetworkEgress,
                            ]),
                            issued_at_epoch_s: 1_700_010_060,
                            expires_at_epoch_s: 1_700_010_160,
                            generation: 3,
                        },
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_061,
                    Some("agent-deny-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-shared".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_062,
                    Some("agent-deny-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-shared".to_owned(),
                        reason: "network egress denied".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_063,
                    Some("agent-revoke"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-shared".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-5",
                    1_700_010_064,
                    Some("agent-other"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-other".to_owned(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::TokenTrail {
                token_id: "token-shared".to_owned(),
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
            },
        })
        .expect("execute audit token trail");

        assert_eq!(execution.token_id_filter.as_deref(), Some("token-shared"));
        match execution.result {
            AuditCommandResult::TokenTrail {
                limit,
                token_id,
                loaded_events,
                total_matching_events,
                truncated_matching_events,
                event_kind_counts,
                issued_event_id,
                issued_timestamp_epoch_s,
                issued_pack_id,
                issued_agent_id,
                issued_generation,
                issued_expires_at_epoch_s,
                issued_capability_count,
                issued_capabilities,
                authorization_denied_count,
                authorization_denied_reason_counts,
                last_denied_event_id,
                last_denied_timestamp_epoch_s,
                last_denied_pack_id,
                last_denied_agent_id,
                last_denied_reason,
                revoked_event_id,
                revoked_timestamp_epoch_s,
                revoked_agent_id,
                timeline,
                ..
            } => {
                assert_eq!(limit, 10);
                assert_eq!(token_id, "token-shared");
                assert_eq!(loaded_events, 4);
                assert_eq!(total_matching_events, 4);
                assert_eq!(truncated_matching_events, 0);
                assert_eq!(
                    event_kind_counts,
                    BTreeMap::from([
                        ("AuthorizationDenied".to_owned(), 2_usize),
                        ("TokenIssued".to_owned(), 1_usize),
                        ("TokenRevoked".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(issued_event_id.as_deref(), Some("evt-1"));
                assert_eq!(issued_timestamp_epoch_s, Some(1_700_010_060));
                assert_eq!(issued_pack_id.as_deref(), Some("sales-intel"));
                assert_eq!(issued_agent_id.as_deref(), Some("agent-issue"));
                assert_eq!(issued_generation, Some(3));
                assert_eq!(issued_expires_at_epoch_s, Some(1_700_010_160));
                assert_eq!(issued_capability_count, Some(2));
                assert_eq!(
                    issued_capabilities,
                    vec!["invoke_tool".to_owned(), "network_egress".to_owned()]
                );
                assert_eq!(authorization_denied_count, 2);
                assert_eq!(
                    authorization_denied_reason_counts,
                    BTreeMap::from([
                        ("missing capability".to_owned(), 1_usize),
                        ("network egress denied".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(last_denied_event_id.as_deref(), Some("evt-3"));
                assert_eq!(last_denied_timestamp_epoch_s, Some(1_700_010_062));
                assert_eq!(last_denied_pack_id.as_deref(), Some("sales-intel"));
                assert_eq!(last_denied_agent_id.as_deref(), Some("agent-deny-b"));
                assert_eq!(last_denied_reason.as_deref(), Some("network egress denied"));
                assert_eq!(revoked_event_id.as_deref(), Some("evt-4"));
                assert_eq!(revoked_timestamp_epoch_s, Some(1_700_010_063));
                assert_eq!(revoked_agent_id.as_deref(), Some("agent-revoke"));
                let ids = timeline
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-1", "evt-2", "evt-3", "evt-4"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_token_trail_reports_truncated_matching_events() {
        let root = unique_temp_dir("loong-audit-cli-token-trail-truncated");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_070,
                    Some("agent-issue"),
                    AuditEventKind::TokenIssued {
                        token: CapabilityToken {
                            token_id: "token-shared".to_owned(),
                            pack_id: "sales-intel".to_owned(),
                            agent_id: "agent-issue".to_owned(),
                            allowed_capabilities: Default::default(),
                            issued_at_epoch_s: 1_700_010_070,
                            expires_at_epoch_s: 1_700_010_170,
                            generation: 0,
                        },
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_071,
                    Some("agent-deny-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-shared".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_072,
                    Some("agent-deny-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-shared".to_owned(),
                        reason: "network egress denied".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_073,
                    Some("agent-revoke"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-shared".to_owned(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::TokenTrail {
                token_id: "token-shared".to_owned(),
                limit: 2,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
            },
        })
        .expect("execute truncated audit token trail");

        match execution.result {
            AuditCommandResult::TokenTrail {
                loaded_events,
                total_matching_events,
                truncated_matching_events,
                issued_event_id,
                revoked_event_id,
                timeline,
                ..
            } => {
                assert_eq!(loaded_events, 2);
                assert_eq!(total_matching_events, 4);
                assert_eq!(truncated_matching_events, 2);
                assert_eq!(issued_event_id, None);
                assert_eq!(revoked_event_id.as_deref(), Some("evt-4"));
                let ids = timeline
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-3", "evt-4"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_token_trail_text_and_json_render_lifecycle() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loong.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_500),
            until_epoch_s_filter: Some(1_700_010_599),
            pack_id_filter: Some("sales-intel".to_owned()),
            agent_id_filter: Some("agent-issue".to_owned()),
            event_id_filter: None,
            token_id_filter: Some("token-shared".to_owned()),
            kind_filter: None,
            triage_label_filter: None,
            query_contains_filter: None,
            trust_tier_filter: None,
            result: AuditCommandResult::TokenTrail {
                limit: 25,
                token_id: "token-shared".to_owned(),
                loaded_events: 3,
                total_matching_events: 4,
                truncated_matching_events: 1,
                event_kind_counts: BTreeMap::from([
                    ("AuthorizationDenied".to_owned(), 1_usize),
                    ("TokenIssued".to_owned(), 1_usize),
                    ("TokenRevoked".to_owned(), 1_usize),
                ]),
                first_timestamp_epoch_s: Some(1_700_010_500),
                last_event_id: Some("evt-3".to_owned()),
                last_timestamp_epoch_s: Some(1_700_010_502),
                last_agent_id: Some("agent-revoke".to_owned()),
                issued_event_id: Some("evt-1".to_owned()),
                issued_timestamp_epoch_s: Some(1_700_010_500),
                issued_pack_id: Some("sales-intel".to_owned()),
                issued_agent_id: Some("agent-issue".to_owned()),
                issued_generation: Some(2),
                issued_expires_at_epoch_s: Some(1_700_010_800),
                issued_capability_count: Some(2),
                issued_capabilities: vec!["invoke_tool".to_owned(), "network_egress".to_owned()],
                authorization_denied_count: 1,
                authorization_denied_reason_counts: BTreeMap::from([(
                    "missing capability".to_owned(),
                    1_usize,
                )]),
                last_denied_event_id: Some("evt-2".to_owned()),
                last_denied_timestamp_epoch_s: Some(1_700_010_501),
                last_denied_pack_id: Some("sales-intel".to_owned()),
                last_denied_agent_id: Some("agent-deny".to_owned()),
                last_denied_reason: Some("missing capability".to_owned()),
                revoked_event_id: Some("evt-3".to_owned()),
                revoked_timestamp_epoch_s: Some(1_700_010_502),
                revoked_agent_id: Some("agent-revoke".to_owned()),
                timeline: vec![
                    sample_audit_event(
                        "evt-1",
                        1_700_010_500,
                        Some("agent-issue"),
                        AuditEventKind::TokenIssued {
                            token: CapabilityToken {
                                token_id: "token-shared".to_owned(),
                                pack_id: "sales-intel".to_owned(),
                                agent_id: "agent-issue".to_owned(),
                                allowed_capabilities: BTreeSet::from([
                                    Capability::InvokeTool,
                                    Capability::NetworkEgress,
                                ]),
                                issued_at_epoch_s: 1_700_010_500,
                                expires_at_epoch_s: 1_700_010_800,
                                generation: 2,
                            },
                        },
                    ),
                    sample_audit_event(
                        "evt-2",
                        1_700_010_501,
                        Some("agent-deny"),
                        AuditEventKind::AuthorizationDenied {
                            pack_id: "sales-intel".to_owned(),
                            token_id: "token-shared".to_owned(),
                            reason: "missing capability".to_owned(),
                        },
                    ),
                    sample_audit_event(
                        "evt-3",
                        1_700_010_502,
                        Some("agent-revoke"),
                        AuditEventKind::TokenRevoked {
                            token_id: "token-shared".to_owned(),
                        },
                    ),
                ],
            },
        };

        let rendered = render_audit_cli_text(&execution).expect("render audit token trail");
        let payload = audit_cli_json(&execution);

        assert!(rendered.contains("audit token-trail"));
        assert!(rendered.contains("token_id=token-shared"));
        assert!(
            rendered
                .contains("loaded_events=3 total_matching_events=4 truncated_matching_events=1")
        );
        assert!(rendered.contains("since_epoch_s=1700010500"));
        assert!(rendered.contains("until_epoch_s=1700010599"));
        assert!(rendered.contains(
            "pack_id=sales-intel agent_id=agent-issue event_id=- token_id=token-shared kind=- triage_label=-"
        ));
        assert!(
            rendered
                .contains("event_kind_counts=AuthorizationDenied=1,TokenIssued=1,TokenRevoked=1")
        );
        assert!(rendered.contains("issued_event_id=evt-1"));
        assert!(
            rendered.contains(
                "issued_capability_count=2 issued_capabilities=invoke_tool,network_egress"
            )
        );
        assert!(rendered.contains(
            "authorization_denied_count=1 authorization_denied_reason_counts=missing capability=1"
        ));
        assert!(rendered.contains("revoked_event_id=evt-3 revoked_timestamp_epoch_s=1700010502 revoked_agent_id=agent-revoke"));
        assert!(rendered.contains("timeline:"));
        assert!(rendered.contains("- ts=1700010502 event_id=evt-3 agent_id=agent-revoke kind=TokenRevoked token_id=token-shared"));

        assert_eq!(payload["command"], "token-trail");
        assert_eq!(payload["token_id"], "token-shared");
        assert_eq!(payload["token_id_filter"], "token-shared");
        assert_eq!(payload["pack_id_filter"], "sales-intel");
        assert_eq!(payload["agent_id_filter"], "agent-issue");
        assert_eq!(payload["loaded_events"], 3);
        assert_eq!(payload["total_matching_events"], 4);
        assert_eq!(payload["truncated_matching_events"], 1);
        assert_eq!(payload["event_kind_counts"]["TokenIssued"], 1);
        assert_eq!(payload["issued_generation"], 2);
        assert_eq!(payload["issued_capability_count"], 2);
        assert_eq!(payload["authorization_denied_count"], 1);
        assert_eq!(
            payload["authorization_denied_reason_counts"]["missing capability"],
            1
        );
        assert_eq!(payload["last_denied_reason"], "missing capability");
        assert_eq!(payload["revoked_event_id"], "evt-3");
        assert_eq!(payload["timeline"][0]["event_id"], "evt-1");
        assert_eq!(payload["timeline"][2]["event_id"], "evt-3");
    }

    #[test]
    fn audit_summary_filters_by_triage_label() {
        let root = unique_temp_dir("loong-audit-cli-summary-triage-filter");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_040,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_041,
                    Some("agent-b"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: Vec::new(),
                        structured_requested_tiers: vec!["official".to_owned()],
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 1,
                        filtered_out_tier_counts: BTreeMap::from([(
                            "verified-community".to_owned(),
                            1_usize,
                        )]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_042,
                    Some("agent-c"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: vec!["verified-community".to_owned()],
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: true,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("official".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: Some("tool_search_trust_conflict".to_owned()),
                group_by: None,
            },
        })
        .expect("execute filtered audit summary");

        assert_eq!(execution.kind_filter, None);
        assert_eq!(
            execution.triage_label_filter.as_deref(),
            Some("tool_search_trust_conflict")
        );
        match execution.result {
            AuditCommandResult::Summary {
                loaded_events,
                event_kind_counts,
                triage_counts,
                last_triage_event_id,
                last_triage_label,
                ..
            } => {
                assert_eq!(loaded_events, 1);
                assert_eq!(
                    event_kind_counts,
                    BTreeMap::from([("ToolSearchEvaluated".to_owned(), 1_usize)])
                );
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([("tool_search_trust_conflict".to_owned(), 1_usize)])
                );
                assert_eq!(last_triage_event_id.as_deref(), Some("evt-3"));
                assert_eq!(
                    last_triage_label.as_deref(),
                    Some("tool_search_trust_conflict")
                );
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_summary_filters_by_agent_id() {
        let root = unique_temp_dir("loong-audit-cli-summary-agent-filter");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_045,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_046,
                    Some("agent-b"),
                    AuditEventKind::ProviderFailover {
                        pack_id: "sales-intel".to_owned(),
                        provider_id: "openai".to_owned(),
                        reason: "rate_limited".to_owned(),
                        stage: "response".to_owned(),
                        model: "gpt-5.1".to_owned(),
                        attempt: 1,
                        max_attempts: 3,
                        status_code: Some(429),
                        request_id: Some("req-audit-1".to_owned()),
                        cf_ray: None,
                        auth_error: None,
                        auth_error_code: Some("token_expired".to_owned()),
                        try_next_model: true,
                        auto_model_mode: true,
                        candidate_index: 0,
                        candidate_count: 2,
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_047,
                    Some("agent-b"),
                    AuditEventKind::SecurityScanEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        scanned_plugins: 1,
                        total_findings: 2,
                        high_findings: 1,
                        medium_findings: 1,
                        low_findings: 0,
                        blocked: true,
                        block_reason: Some("unsigned plugin".to_owned()),
                        categories: vec!["signature".to_owned()],
                        finding_ids: vec!["finding-1".to_owned()],
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: Some("agent-b".to_owned()),
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: None,
            },
        })
        .expect("execute audit summary with agent filter");

        assert_eq!(execution.agent_id_filter.as_deref(), Some("agent-b"));
        match execution.result {
            AuditCommandResult::Summary {
                loaded_events,
                event_kind_counts,
                triage_counts,
                last_event_id,
                ..
            } => {
                assert_eq!(loaded_events, 2);
                assert_eq!(
                    event_kind_counts,
                    BTreeMap::from([
                        ("ProviderFailover".to_owned(), 1_usize),
                        ("SecurityScanEvaluated".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([
                        ("provider_failover".to_owned(), 1_usize),
                        ("security_scan_blocked".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(last_event_id.as_deref(), Some("evt-3"));
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_discovery_filters_tool_search_events_and_rolls_up_trust_context() {
        let root = unique_temp_dir("loong-audit-cli-discovery");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_050,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_051,
                    Some("agent-b"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "catalog search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: Vec::new(),
                        structured_requested_tiers: vec!["official".to_owned()],
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("unverified".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_052,
                    Some("agent-c"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official catalog search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: vec!["verified-community".to_owned()],
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: true,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("official".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_053,
                    Some("agent-d"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:verified-community catalog search".to_owned(),
                        returned: 1,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["verified-community".to_owned()],
                        structured_requested_tiers: Vec::new(),
                        effective_tiers: vec!["verified-community".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 1,
                        filtered_out_tier_counts: BTreeMap::from([(
                            "unverified".to_owned(),
                            1_usize,
                        )]),
                        top_provider_ids: vec!["community-search".to_owned()],
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Discovery {
                limit: 10,
                since_epoch_s: Some(1_700_010_051),
                until_epoch_s: Some(1_700_010_052),
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                triage_label: None,
                query_contains: Some("catalog".to_owned()),
                trust_tier: Some("official".to_owned()),
                group_by: None,
            },
        })
        .expect("execute audit discovery");

        assert_eq!(execution.since_epoch_s_filter, Some(1_700_010_051));
        assert_eq!(execution.until_epoch_s_filter, Some(1_700_010_052));
        assert_eq!(
            execution.kind_filter.as_deref(),
            Some("ToolSearchEvaluated")
        );
        assert_eq!(execution.triage_label_filter, None);
        assert_eq!(execution.query_contains_filter.as_deref(), Some("catalog"));
        assert_eq!(execution.trust_tier_filter.as_deref(), Some("official"));
        match execution.result {
            AuditCommandResult::Discovery {
                limit,
                loaded_events,
                triage_counts,
                query_requested_tier_counts,
                structured_requested_tier_counts,
                effective_tier_counts,
                filtered_out_tier_counts,
                trust_filter_applied_events,
                conflicting_requested_tier_events,
                trust_filtered_empty_events,
                last_event_id,
                last_pack_id,
                last_query,
                last_returned,
                last_trust_filter_applied,
                last_conflicting_requested_tiers,
                last_query_requested_tiers,
                last_structured_requested_tiers,
                last_effective_tiers,
                last_filtered_out_candidates,
                last_filtered_out_tier_counts,
                last_top_provider_ids,
                last_triage_event_id,
                last_triage_label,
                last_triage_summary,
                last_triage_hint,
                ..
            } => {
                assert_eq!(limit, 10);
                assert_eq!(loaded_events, 2);
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([
                        ("tool_search_trust_conflict".to_owned(), 1_usize),
                        ("tool_search_trust_empty".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    query_requested_tier_counts,
                    BTreeMap::from([("official".to_owned(), 1_usize)])
                );
                assert_eq!(
                    structured_requested_tier_counts,
                    BTreeMap::from([
                        ("official".to_owned(), 1_usize),
                        ("verified-community".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    effective_tier_counts,
                    BTreeMap::from([("official".to_owned(), 1_usize)])
                );
                assert_eq!(
                    filtered_out_tier_counts,
                    BTreeMap::from([
                        ("official".to_owned(), 1_usize),
                        ("unverified".to_owned(), 1_usize),
                        ("verified-community".to_owned(), 2_usize),
                    ])
                );
                assert_eq!(trust_filter_applied_events, 2);
                assert_eq!(conflicting_requested_tier_events, 1);
                assert_eq!(trust_filtered_empty_events, 1);
                assert_eq!(last_event_id.as_deref(), Some("evt-3"));
                assert_eq!(last_pack_id.as_deref(), Some("sales-intel"));
                assert_eq!(last_query.as_deref(), Some("trust:official catalog search"));
                assert_eq!(last_returned, Some(0));
                assert_eq!(last_trust_filter_applied, Some(true));
                assert_eq!(last_conflicting_requested_tiers, Some(true));
                assert_eq!(last_query_requested_tiers, vec!["official".to_owned()]);
                assert_eq!(
                    last_structured_requested_tiers,
                    vec!["verified-community".to_owned()]
                );
                assert_eq!(last_effective_tiers, Vec::<String>::new());
                assert_eq!(last_filtered_out_candidates, Some(2));
                assert_eq!(
                    last_filtered_out_tier_counts,
                    BTreeMap::from([
                        ("official".to_owned(), 1_usize),
                        ("verified-community".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(last_top_provider_ids, Vec::<String>::new());
                assert_eq!(last_triage_event_id.as_deref(), Some("evt-3"));
                assert_eq!(
                    last_triage_label.as_deref(),
                    Some("tool_search_trust_conflict")
                );
                assert_eq!(
                    last_triage_summary.as_deref(),
                    Some(
                        "query=\"trust:official catalog search\" trust_scope=- conflicting_requested_tiers=true filtered_out_candidates=2 top_provider_ids=-"
                    )
                );
                assert_eq!(
                    last_triage_hint.as_deref(),
                    Some(
                        "align query trust prefixes with structured trust_tiers before retrying discovery"
                    )
                );
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_discovery_rejects_excessive_limit() {
        let mut env = ScopedEnv::new();
        env.set(
            "HOME",
            unique_temp_dir("loong-audit-cli-large-discovery-limit-home"),
        );

        let error = execute_audit_command(AuditCommandOptions {
            config: None,
            json: false,
            command: AuditCommands::Discovery {
                limit: 10_001,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
                group_by: None,
            },
        })
        .expect_err("excessive discovery limit should fail");

        assert!(error.contains("audit discovery limit must be between 1 and 10000"));
    }

    #[test]
    fn audit_discovery_rejects_until_before_since() {
        let mut env = ScopedEnv::new();
        env.set(
            "HOME",
            unique_temp_dir("loong-audit-cli-invalid-time-range-home"),
        );

        let error = execute_audit_command(AuditCommandOptions {
            config: None,
            json: false,
            command: AuditCommands::Discovery {
                limit: 10,
                since_epoch_s: Some(1_700_010_100),
                until_epoch_s: Some(1_700_010_099),
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
                group_by: None,
            },
        })
        .expect_err("invalid discovery time range should fail");

        assert!(error.contains(
            "audit discovery until_epoch_s must be greater than or equal to since_epoch_s"
        ));
    }

    #[test]
    fn audit_discovery_groups_by_agent() {
        let root = unique_temp_dir("loong-audit-cli-discovery-group-by-agent");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-0",
                    1_700_010_299,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-a".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-1",
                    1_700_010_300,
                    Some("agent-a"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "catalog".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: Vec::new(),
                        structured_requested_tiers: vec!["official".to_owned()],
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 1,
                        filtered_out_tier_counts: BTreeMap::from([(
                            "verified-community".to_owned(),
                            1_usize,
                        )]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_301,
                    Some("agent-a"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official catalog".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: vec!["verified-community".to_owned()],
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: true,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("official".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_302,
                    Some("agent-b"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "ops-pack".to_owned(),
                        query: "trust:verified-community search".to_owned(),
                        returned: 1,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["verified-community".to_owned()],
                        structured_requested_tiers: Vec::new(),
                        effective_tiers: vec!["verified-community".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 1,
                        filtered_out_tier_counts: BTreeMap::from([(
                            "unverified".to_owned(),
                            1_usize,
                        )]),
                        top_provider_ids: vec!["community-search".to_owned()],
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Discovery {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
                group_by: Some("agent".to_owned()),
            },
        })
        .expect("execute grouped audit discovery");
        let rendered = render_audit_cli_text(&execution).expect("render grouped audit discovery");
        let payload = audit_cli_json(&execution);

        match execution.result {
            AuditCommandResult::Discovery {
                group_by, groups, ..
            } => {
                assert_eq!(group_by.as_deref(), Some("agent"));
                assert_eq!(groups.len(), 2);

                assert_eq!(groups[0].group_value.as_deref(), Some("agent-a"));
                assert_eq!(groups[0].loaded_events, 2);
                assert_eq!(
                    groups[0].triage_counts,
                    BTreeMap::from([
                        ("tool_search_trust_conflict".to_owned(), 1_usize),
                        ("tool_search_trust_empty".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    groups[0].query_requested_tier_counts,
                    BTreeMap::from([("official".to_owned(), 1_usize)])
                );
                assert_eq!(
                    groups[0].structured_requested_tier_counts,
                    BTreeMap::from([
                        ("official".to_owned(), 1_usize),
                        ("verified-community".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(groups[0].trust_filter_applied_events, 2);
                assert_eq!(groups[0].conflicting_requested_tier_events, 1);
                assert_eq!(groups[0].trust_filtered_empty_events, 1);
                assert_eq!(groups[0].last_pack_id.as_deref(), Some("sales-intel"));
                assert_eq!(
                    groups[0].last_query.as_deref(),
                    Some("trust:official catalog")
                );
                assert_eq!(groups[0].last_returned, Some(0));
                assert_eq!(
                    groups[0]
                        .correlated_summary
                        .as_ref()
                        .expect("agent-a correlated summary should exist")
                        .loaded_events,
                    3
                );
                assert_eq!(
                    groups[0]
                        .correlated_summary
                        .as_ref()
                        .expect("agent-a correlated summary should exist")
                        .event_kind_counts,
                    BTreeMap::from([
                        ("AuthorizationDenied".to_owned(), 1_usize),
                        ("ToolSearchEvaluated".to_owned(), 2_usize),
                    ])
                );
                assert_eq!(
                    groups[0]
                        .correlated_summary
                        .as_ref()
                        .expect("agent-a correlated summary should exist")
                        .triage_counts,
                    BTreeMap::from([
                        ("authorization_denied".to_owned(), 1_usize),
                        ("tool_search_trust_conflict".to_owned(), 1_usize),
                        ("tool_search_trust_empty".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(groups[0].correlated_additional_events, 1);
                assert_eq!(
                    groups[0].correlated_non_discovery_event_kind_counts,
                    BTreeMap::from([("AuthorizationDenied".to_owned(), 1_usize)])
                );
                assert_eq!(
                    groups[0].correlated_non_discovery_triage_counts,
                    BTreeMap::from([("authorization_denied".to_owned(), 1_usize)])
                );
                assert_eq!(
                    groups[0].correlated_attention_hint.as_deref(),
                    Some("adjacent_triage=authorization_denied=1")
                );
                assert_eq!(
                    groups[0].correlated_remediation_hint.as_deref(),
                    Some(
                        "grant the required capability or retry with a token scoped for the requested operation"
                    )
                );

                assert_eq!(groups[1].group_value.as_deref(), Some("agent-b"));
                assert_eq!(groups[1].loaded_events, 1);
                assert_eq!(
                    groups[1].effective_tier_counts,
                    BTreeMap::from([("verified-community".to_owned(), 1_usize)])
                );
                assert_eq!(groups[1].last_pack_id.as_deref(), Some("ops-pack"));
                assert_eq!(groups[1].last_returned, Some(1));
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }

        assert!(rendered.contains("group_by=agent group_count=2"));
        assert!(rendered.contains("group[agent]=agent-a loaded_events=2"));
        assert!(rendered.contains(
            "group[agent]=agent-b loaded_events=1 triage_counts=- query_requested_tier_counts=verified-community=1"
        ));
        assert!(
            rendered
                .contains("group_drill_down[agent]=agent-a command=loong audit recent --config")
        );
        assert!(rendered.contains(
            "group_correlated_preview[agent]=agent-a loaded_events=3 event_kind_counts=AuthorizationDenied=1,ToolSearchEvaluated=2 triage_counts=authorization_denied=1,tool_search_trust_conflict=1,tool_search_trust_empty=1"
        ));
        assert!(rendered.contains(
            "group_correlated_focus[agent]=agent-a additional_events=1 non_discovery_event_kind_counts=AuthorizationDenied=1 non_discovery_triage_counts=authorization_denied=1 attention_hint=adjacent_triage=authorization_denied=1 remediation_hint=grant the required capability or retry with a token scoped for the requested operation"
        ));
        assert!(rendered.contains(
            "group_correlated_summary[agent]=agent-a command=loong audit summary --config"
        ));
        assert!(rendered.contains(
            "group_correlated_remediation[agent]=agent-a command=loong audit summary --config"
        ));
        assert!(rendered.contains("--agent-id 'agent-a'"));
        assert!(rendered.contains("--kind 'ToolSearchEvaluated'"));

        assert_eq!(payload["group_by"], "agent");
        assert_eq!(payload["groups"][0]["group_value"], "agent-a");
        assert_eq!(payload["groups"][0]["loaded_events"], 2);
        assert_eq!(payload["groups"][0]["trust_filter_applied_events"], 2);
        assert_eq!(
            payload["groups"][0]["drill_down_command"],
            json!(format!(
                "loong audit recent --config '{}' --limit 10 --agent-id 'agent-a' --kind 'ToolSearchEvaluated'",
                config_path.display()
            ))
        );
        assert_eq!(
            payload["groups"][0]["correlated_summary_command"],
            json!(format!(
                "loong audit summary --config '{}' --limit 10 --agent-id 'agent-a'",
                config_path.display()
            ))
        );
        assert_eq!(payload["groups"][0]["correlated_additional_events"], 1);
        assert_eq!(
            payload["groups"][0]["correlated_non_discovery_event_kind_counts"]["AuthorizationDenied"],
            1
        );
        assert_eq!(
            payload["groups"][0]["correlated_non_discovery_triage_counts"]["authorization_denied"],
            1
        );
        assert_eq!(
            payload["groups"][0]["correlated_attention_hint"],
            "adjacent_triage=authorization_denied=1"
        );
        assert_eq!(
            payload["groups"][0]["correlated_remediation_hint"],
            "grant the required capability or retry with a token scoped for the requested operation"
        );
        assert_eq!(
            payload["groups"][0]["correlated_remediation_command"],
            json!(format!(
                "loong audit summary --config '{}' --limit 10 --agent-id 'agent-a' --triage-label 'authorization_denied' --group-by 'token'",
                config_path.display()
            ))
        );
        assert_eq!(
            payload["groups"][0]["correlated_summary"]["loaded_events"],
            3
        );
        assert_eq!(
            payload["groups"][0]["correlated_summary"]["event_kind_counts"]["AuthorizationDenied"],
            1
        );
        assert_eq!(
            payload["groups"][0]["correlated_summary"]["triage_counts"]["authorization_denied"],
            1
        );
        assert_eq!(
            payload["groups"][1]["effective_tier_counts"]["verified-community"],
            1
        );
    }

    #[test]
    fn audit_discovery_group_drill_down_command_preserves_filters() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loong.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_400),
            until_epoch_s_filter: Some(1_700_010_499),
            pack_id_filter: Some("sales-intel".to_owned()),
            agent_id_filter: None,
            event_id_filter: Some("evt-2".to_owned()),
            token_id_filter: None,
            kind_filter: Some("ToolSearchEvaluated".to_owned()),
            triage_label_filter: Some("tool_search_trust_conflict".to_owned()),
            query_contains_filter: Some("trust:official".to_owned()),
            trust_tier_filter: Some("official".to_owned()),
            result: AuditCommandResult::Recent {
                limit: 1,
                events: Vec::new(),
            },
        };
        let group = AuditDiscoveryGroup {
            group_value: Some("agent-b".to_owned()),
            loaded_events: 2,
            triage_counts: BTreeMap::new(),
            query_requested_tier_counts: BTreeMap::new(),
            structured_requested_tier_counts: BTreeMap::new(),
            effective_tier_counts: BTreeMap::new(),
            filtered_out_tier_counts: BTreeMap::new(),
            trust_filter_applied_events: 0,
            conflicting_requested_tier_events: 0,
            trust_filtered_empty_events: 0,
            first_timestamp_epoch_s: Some(1_700_010_400),
            last_event_id: Some("evt-2".to_owned()),
            last_timestamp_epoch_s: Some(1_700_010_401),
            last_agent_id: Some("agent-b".to_owned()),
            last_pack_id: Some("sales-intel".to_owned()),
            last_query: Some("trust:official search".to_owned()),
            last_returned: Some(0),
            correlated_summary: None,
            correlated_additional_events: 0,
            correlated_non_discovery_event_kind_counts: BTreeMap::new(),
            correlated_non_discovery_triage_counts: BTreeMap::new(),
            correlated_attention_hint: None,
            correlated_remediation_hint: None,
        };

        let command = discovery_group_drill_down_command(&execution, 25, Some("agent"), &group)
            .expect("group drill-down command should render");

        assert_eq!(
            command,
            "loong audit recent --config '/tmp/loong.toml' --limit 25 --since-epoch-s 1700010400 --until-epoch-s 1700010499 --pack-id 'sales-intel' --agent-id 'agent-b' --event-id 'evt-2' --kind 'ToolSearchEvaluated' --triage-label 'tool_search_trust_conflict' --query-contains 'trust:official' --trust-tier 'official'"
        );
    }

    #[test]
    fn audit_discovery_group_correlated_summary_command_broadens_to_workload_window() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loong.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_400),
            until_epoch_s_filter: Some(1_700_010_499),
            pack_id_filter: Some("sales-intel".to_owned()),
            agent_id_filter: None,
            event_id_filter: Some("evt-2".to_owned()),
            token_id_filter: Some("token-1".to_owned()),
            kind_filter: Some("ToolSearchEvaluated".to_owned()),
            triage_label_filter: Some("tool_search_trust_conflict".to_owned()),
            query_contains_filter: Some("trust:official".to_owned()),
            trust_tier_filter: Some("official".to_owned()),
            result: AuditCommandResult::Recent {
                limit: 1,
                events: Vec::new(),
            },
        };
        let group = AuditDiscoveryGroup {
            group_value: Some("agent-b".to_owned()),
            loaded_events: 2,
            triage_counts: BTreeMap::new(),
            query_requested_tier_counts: BTreeMap::new(),
            structured_requested_tier_counts: BTreeMap::new(),
            effective_tier_counts: BTreeMap::new(),
            filtered_out_tier_counts: BTreeMap::new(),
            trust_filter_applied_events: 0,
            conflicting_requested_tier_events: 0,
            trust_filtered_empty_events: 0,
            first_timestamp_epoch_s: Some(1_700_010_400),
            last_event_id: Some("evt-2".to_owned()),
            last_timestamp_epoch_s: Some(1_700_010_401),
            last_agent_id: Some("agent-b".to_owned()),
            last_pack_id: Some("sales-intel".to_owned()),
            last_query: Some("trust:official search".to_owned()),
            last_returned: Some(0),
            correlated_summary: None,
            correlated_additional_events: 0,
            correlated_non_discovery_event_kind_counts: BTreeMap::new(),
            correlated_non_discovery_triage_counts: BTreeMap::new(),
            correlated_attention_hint: None,
            correlated_remediation_hint: None,
        };

        let command =
            discovery_group_correlated_summary_command(&execution, 25, Some("agent"), &group)
                .expect("group correlated summary command should render");

        assert_eq!(
            command,
            "loong audit summary --config '/tmp/loong.toml' --limit 25 --since-epoch-s 1700010400 --until-epoch-s 1700010499 --pack-id 'sales-intel' --agent-id 'agent-b'"
        );
    }

    #[test]
    fn audit_discovery_group_correlated_remediation_command_targets_token_summary() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loong.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_400),
            until_epoch_s_filter: Some(1_700_010_499),
            pack_id_filter: Some("sales-intel".to_owned()),
            agent_id_filter: None,
            event_id_filter: Some("evt-2".to_owned()),
            token_id_filter: Some("token-1".to_owned()),
            kind_filter: Some("ToolSearchEvaluated".to_owned()),
            triage_label_filter: Some("tool_search_trust_conflict".to_owned()),
            query_contains_filter: Some("trust:official".to_owned()),
            trust_tier_filter: Some("official".to_owned()),
            result: AuditCommandResult::Recent {
                limit: 1,
                events: Vec::new(),
            },
        };
        let group = AuditDiscoveryGroup {
            group_value: Some("agent-b".to_owned()),
            loaded_events: 2,
            triage_counts: BTreeMap::new(),
            query_requested_tier_counts: BTreeMap::new(),
            structured_requested_tier_counts: BTreeMap::new(),
            effective_tier_counts: BTreeMap::new(),
            filtered_out_tier_counts: BTreeMap::new(),
            trust_filter_applied_events: 0,
            conflicting_requested_tier_events: 0,
            trust_filtered_empty_events: 0,
            first_timestamp_epoch_s: Some(1_700_010_400),
            last_event_id: Some("evt-2".to_owned()),
            last_timestamp_epoch_s: Some(1_700_010_401),
            last_agent_id: Some("agent-b".to_owned()),
            last_pack_id: Some("sales-intel".to_owned()),
            last_query: Some("trust:official search".to_owned()),
            last_returned: Some(0),
            correlated_summary: None,
            correlated_additional_events: 1,
            correlated_non_discovery_event_kind_counts: BTreeMap::from([(
                "AuthorizationDenied".to_owned(),
                1_usize,
            )]),
            correlated_non_discovery_triage_counts: BTreeMap::from([(
                "authorization_denied".to_owned(),
                1_usize,
            )]),
            correlated_attention_hint: Some("adjacent_triage=authorization_denied=1".to_owned()),
            correlated_remediation_hint: Some(
                "grant the required capability or retry with a token scoped for the requested operation"
                    .to_owned(),
            ),
        };

        let command =
            discovery_group_correlated_remediation_command(&execution, 25, Some("agent"), &group)
                .expect("group correlated remediation command should render");

        assert_eq!(
            command,
            "loong audit summary --config '/tmp/loong.toml' --limit 25 --since-epoch-s 1700010400 --until-epoch-s 1700010499 --pack-id 'sales-intel' --agent-id 'agent-b' --triage-label 'authorization_denied' --group-by 'token'"
        );
    }

    #[test]
    fn audit_discovery_text_and_json_render_trust_rollups() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loong.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_400),
            until_epoch_s_filter: Some(1_700_010_499),
            pack_id_filter: Some("sales-intel".to_owned()),
            agent_id_filter: Some("agent-b".to_owned()),
            event_id_filter: Some("evt-2".to_owned()),
            token_id_filter: None,
            kind_filter: Some("ToolSearchEvaluated".to_owned()),
            triage_label_filter: Some("tool_search_trust_conflict".to_owned()),
            query_contains_filter: Some("trust:official".to_owned()),
            trust_tier_filter: Some("official".to_owned()),
            result: AuditCommandResult::Discovery {
                limit: 25,
                loaded_events: 2,
                triage_counts: BTreeMap::from([(
                    "tool_search_trust_conflict".to_owned(),
                    2_usize,
                )]),
                query_requested_tier_counts: BTreeMap::from([("official".to_owned(), 2_usize)]),
                structured_requested_tier_counts: BTreeMap::from([(
                    "verified-community".to_owned(),
                    2_usize,
                )]),
                effective_tier_counts: BTreeMap::new(),
                filtered_out_tier_counts: BTreeMap::from([
                    ("official".to_owned(), 2_usize),
                    ("verified-community".to_owned(), 2_usize),
                ]),
                trust_filter_applied_events: 2,
                conflicting_requested_tier_events: 2,
                trust_filtered_empty_events: 0,
                group_by: None,
                groups: Vec::new(),
                first_timestamp_epoch_s: Some(1_700_010_400),
                last_event_id: Some("evt-2".to_owned()),
                last_timestamp_epoch_s: Some(1_700_010_401),
                last_agent_id: Some("agent-b".to_owned()),
                last_pack_id: Some("sales-intel".to_owned()),
                last_query: Some("trust:official search".to_owned()),
                last_returned: Some(0),
                last_trust_filter_applied: Some(true),
                last_conflicting_requested_tiers: Some(true),
                last_query_requested_tiers: vec!["official".to_owned()],
                last_structured_requested_tiers: vec!["verified-community".to_owned()],
                last_effective_tiers: Vec::new(),
                last_filtered_out_candidates: Some(2),
                last_filtered_out_tier_counts: BTreeMap::from([
                    ("official".to_owned(), 1_usize),
                    ("verified-community".to_owned(), 1_usize),
                ]),
                last_top_provider_ids: Vec::new(),
                last_triage_event_id: Some("evt-2".to_owned()),
                last_triage_label: Some("tool_search_trust_conflict".to_owned()),
                last_triage_timestamp_epoch_s: Some(1_700_010_401),
                last_triage_agent_id: Some("agent-b".to_owned()),
                last_triage_summary: Some(
                    "query=\"trust:official search\" trust_scope=- conflicting_requested_tiers=true filtered_out_candidates=2 top_provider_ids=-"
                        .to_owned(),
                ),
                last_triage_hint: Some(
                    "align query trust prefixes with structured trust_tiers before retrying discovery"
                        .to_owned(),
                ),
            },
        };

        let rendered = render_audit_cli_text(&execution).expect("render audit discovery");
        let payload = audit_cli_json(&execution);

        assert!(rendered.contains("audit discovery"));
        assert!(rendered.contains("since_epoch_s=1700010400"));
        assert!(rendered.contains("until_epoch_s=1700010499"));
        assert!(rendered.contains("pack_id=sales-intel"));
        assert!(rendered.contains("agent_id=agent-b"));
        assert!(rendered.contains(
            "pack_id=sales-intel agent_id=agent-b event_id=evt-2 token_id=- kind=ToolSearchEvaluated"
        ));
        assert!(rendered.contains("kind=ToolSearchEvaluated"));
        assert!(rendered.contains("query_contains=trust:official"));
        assert!(rendered.contains("trust_tier=official"));
        assert!(rendered.contains("query_requested_tier_counts=official=2"));
        assert!(rendered.contains("structured_requested_tier_counts=verified-community=2"));
        assert!(rendered.contains("group_by=- group_count=0"));
        assert!(rendered.contains(
            "last_query=\"trust:official search\" last_returned=0 last_trust_filter_applied=true last_conflicting_requested_tiers=true"
        ));
        assert!(rendered.contains(
            "last_triage_hint=align query trust prefixes with structured trust_tiers before retrying discovery"
        ));

        assert_eq!(payload["command"], "discovery");
        assert_eq!(payload["since_epoch_s_filter"], 1_700_010_400_u64);
        assert_eq!(payload["until_epoch_s_filter"], 1_700_010_499_u64);
        assert_eq!(payload["pack_id_filter"], "sales-intel");
        assert_eq!(payload["agent_id_filter"], "agent-b");
        assert_eq!(payload["event_id_filter"], "evt-2");
        assert_eq!(payload["token_id_filter"], Value::Null);
        assert_eq!(payload["kind_filter"], "ToolSearchEvaluated");
        assert_eq!(payload["triage_label_filter"], "tool_search_trust_conflict");
        assert_eq!(payload["query_contains_filter"], "trust:official");
        assert_eq!(payload["trust_tier_filter"], "official");
        assert_eq!(payload["group_by"], Value::Null);
        assert_eq!(payload["groups"], json!([]));
        assert_eq!(payload["triage_counts"]["tool_search_trust_conflict"], 2);
        assert_eq!(payload["query_requested_tier_counts"]["official"], 2);
        assert_eq!(
            payload["structured_requested_tier_counts"]["verified-community"],
            2
        );
        assert_eq!(payload["last_pack_id"], "sales-intel");
        assert_eq!(payload["last_query"], "trust:official search");
        assert_eq!(payload["last_conflicting_requested_tiers"], true);
        assert_eq!(
            payload["last_triage_hint"],
            "align query trust prefixes with structured trust_tiers before retrying discovery"
        );
    }

    #[test]
    fn audit_recent_reports_missing_journal_with_first_write_hint() {
        let root = unique_temp_dir("loong-audit-cli-missing");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);

        let error = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect_err("missing journal should fail");

        assert!(error.contains("audit journal not found"));
        assert!(error.contains("first audit write"));
    }

    #[test]
    fn audit_recent_reports_in_memory_mode_when_journal_is_missing() {
        let root = unique_temp_dir("loong-audit-cli-in-memory");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config_with_mode(
            &root,
            &journal_path,
            crate::mvp::config::AuditMode::InMemory,
        );

        let error = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect_err("missing in-memory journal should fail");

        assert!(error.contains("audit journal not found"));
        assert!(error.contains("durable audit retention is disabled"));
        assert!(error.contains("[audit].mode = \"in_memory\""));
    }

    #[test]
    fn audit_verify_reports_missing_journal_with_first_write_hint() {
        let root = unique_temp_dir("loong-audit-cli-verify-missing");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);

        let error = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Verify,
        })
        .expect_err("missing journal should fail");

        assert!(error.contains("audit journal not found"));
        assert!(error.contains("first audit write"));
    }

    #[test]
    fn audit_verify_reports_in_memory_mode_when_journal_is_missing() {
        let root = unique_temp_dir("loong-audit-cli-verify-in-memory");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config_with_mode(
            &root,
            &journal_path,
            crate::mvp::config::AuditMode::InMemory,
        );

        let error = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Verify,
        })
        .expect_err("missing in-memory journal should fail");

        assert!(error.contains("audit journal not found"));
        assert!(error.contains("durable audit retention is disabled"));
        assert!(error.contains("[audit].mode = \"in_memory\""));
    }

    #[test]
    fn audit_summary_rolls_up_event_kinds_and_last_seen_fields() {
        let root = unique_temp_dir("loong-audit-cli-summary");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_100,
                    Some("agent-a"),
                    AuditEventKind::TokenIssued {
                        token: CapabilityToken {
                            token_id: "token-0".to_owned(),
                            pack_id: "sales-intel".to_owned(),
                            agent_id: "agent-a".to_owned(),
                            allowed_capabilities: Default::default(),
                            issued_at_epoch_s: 1_700_010_100,
                            expires_at_epoch_s: 1_700_010_200,
                            generation: 0,
                        },
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_101,
                    Some("agent-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_102,
                    Some("agent-c"),
                    AuditEventKind::ProviderFailover {
                        pack_id: "sales-intel".to_owned(),
                        provider_id: "openai".to_owned(),
                        reason: "rate_limited".to_owned(),
                        stage: "response".to_owned(),
                        model: "gpt-5.1".to_owned(),
                        attempt: 1,
                        max_attempts: 3,
                        status_code: Some(429),
                        request_id: Some("req-audit-2".to_owned()),
                        cf_ray: None,
                        auth_error: None,
                        auth_error_code: Some("token_expired".to_owned()),
                        try_next_model: true,
                        auto_model_mode: true,
                        candidate_index: 0,
                        candidate_count: 2,
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_103,
                    Some("agent-d"),
                    AuditEventKind::SecurityScanEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        scanned_plugins: 1,
                        total_findings: 2,
                        high_findings: 1,
                        medium_findings: 1,
                        low_findings: 0,
                        blocked: true,
                        block_reason: Some("unsigned plugin".to_owned()),
                        categories: vec!["signature".to_owned()],
                        finding_ids: vec!["finding-1".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-5",
                    1_700_010_104,
                    Some("agent-e"),
                    AuditEventKind::PluginTrustEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        scanned_plugins: 2,
                        official_plugins: 1,
                        verified_community_plugins: 0,
                        unverified_plugins: 1,
                        high_risk_plugins: 1,
                        high_risk_unverified_plugins: 1,
                        blocked_auto_apply_plugins: 1,
                        review_required_plugin_ids: vec!["stdio-review".to_owned()],
                        review_required_bridges: vec!["process_stdio".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-6",
                    1_700_010_105,
                    Some("agent-f"),
                    AuditEventKind::PlaneInvoked {
                        pack_id: "sales-intel".to_owned(),
                        plane: ExecutionPlane::Runtime,
                        tier: PlaneTier::Core,
                        primary_adapter: "runtime".to_owned(),
                        delegated_core_adapter: None,
                        operation: "turn.complete".to_owned(),
                        required_capabilities: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: None,
            },
        })
        .expect("execute audit summary");

        match execution.result {
            AuditCommandResult::Summary {
                limit,
                loaded_events,
                event_kind_counts,
                triage_counts,
                group_by,
                groups,
                first_timestamp_epoch_s,
                last_event_id,
                last_timestamp_epoch_s,
                last_agent_id,
                last_triage_event_id,
                last_triage_label,
                last_triage_event_kind,
                last_triage_timestamp_epoch_s,
                last_triage_agent_id,
                last_triage_summary,
                last_triage_hint,
            } => {
                assert_eq!(limit, 10);
                assert_eq!(loaded_events, 6);
                assert_eq!(
                    event_kind_counts,
                    BTreeMap::from([
                        ("AuthorizationDenied".to_owned(), 1_usize),
                        ("PlaneInvoked".to_owned(), 1_usize),
                        ("PluginTrustEvaluated".to_owned(), 1_usize),
                        ("ProviderFailover".to_owned(), 1_usize),
                        ("SecurityScanEvaluated".to_owned(), 1_usize),
                        ("TokenIssued".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([
                        ("authorization_denied".to_owned(), 1_usize),
                        ("plugin_trust_blocked".to_owned(), 1_usize),
                        ("provider_failover".to_owned(), 1_usize),
                        ("security_scan_blocked".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(group_by, None);
                assert!(groups.is_empty());
                assert_eq!(first_timestamp_epoch_s, Some(1_700_010_100));
                assert_eq!(last_event_id.as_deref(), Some("evt-6"));
                assert_eq!(last_timestamp_epoch_s, Some(1_700_010_105));
                assert_eq!(last_agent_id.as_deref(), Some("agent-f"));
                assert_eq!(last_triage_event_id.as_deref(), Some("evt-5"));
                assert_eq!(last_triage_label.as_deref(), Some("plugin_trust_blocked"));
                assert_eq!(
                    last_triage_event_kind.as_deref(),
                    Some("PluginTrustEvaluated")
                );
                assert_eq!(last_triage_timestamp_epoch_s, Some(1_700_010_104));
                assert_eq!(last_triage_agent_id.as_deref(), Some("agent-e"));
                assert_eq!(
                    last_triage_summary.as_deref(),
                    Some(
                        "pack_id=sales-intel blocked_auto_apply_plugins=1 review_required_plugins=stdio-review"
                    )
                );
                assert_eq!(
                    last_triage_hint.as_deref(),
                    Some(
                        "review plugin provenance and bootstrap policy before enabling auto-apply for the blocked plugins"
                    )
                );
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_summary_groups_by_token() {
        let root = unique_temp_dir("loong-audit-cli-summary-group-by-token");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_120,
                    Some("agent-issue"),
                    AuditEventKind::TokenIssued {
                        token: CapabilityToken {
                            token_id: "token-a".to_owned(),
                            pack_id: "sales-intel".to_owned(),
                            agent_id: "agent-issue".to_owned(),
                            allowed_capabilities: Default::default(),
                            issued_at_epoch_s: 1_700_010_120,
                            expires_at_epoch_s: 1_700_010_220,
                            generation: 0,
                        },
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_121,
                    Some("agent-deny"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-a".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_122,
                    Some("agent-revoke"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-a".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_123,
                    Some("agent-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "ops-pack".to_owned(),
                        token_id: "token-b".to_owned(),
                        reason: "network egress denied".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-5",
                    1_700_010_124,
                    Some("agent-no-token"),
                    AuditEventKind::PlaneInvoked {
                        pack_id: "sales-intel".to_owned(),
                        plane: ExecutionPlane::Tool,
                        tier: PlaneTier::Core,
                        primary_adapter: "runtime".to_owned(),
                        delegated_core_adapter: None,
                        operation: "tool.call".to_owned(),
                        required_capabilities: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: Some("token".to_owned()),
            },
        })
        .expect("execute audit summary grouped by token");
        let rendered = render_audit_cli_text(&execution).expect("render grouped audit summary");
        let payload = audit_cli_json(&execution);

        match execution.result {
            AuditCommandResult::Summary {
                group_by, groups, ..
            } => {
                assert_eq!(group_by.as_deref(), Some("token"));
                assert_eq!(groups.len(), 3);

                assert_eq!(groups[0].group_value.as_deref(), Some("token-a"));
                assert_eq!(groups[0].loaded_events, 3);
                assert_eq!(
                    groups[0].event_kind_counts,
                    BTreeMap::from([
                        ("AuthorizationDenied".to_owned(), 1_usize),
                        ("TokenIssued".to_owned(), 1_usize),
                        ("TokenRevoked".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    groups[0].triage_counts,
                    BTreeMap::from([("authorization_denied".to_owned(), 1_usize)])
                );
                assert_eq!(groups[0].last_event_id.as_deref(), Some("evt-3"));

                assert_eq!(groups[1].group_value.as_deref(), Some("token-b"));
                assert_eq!(groups[1].loaded_events, 1);
                assert_eq!(groups[1].last_event_id.as_deref(), Some("evt-4"));

                assert_eq!(groups[2].group_value, None);
                assert_eq!(groups[2].loaded_events, 1);
                assert_eq!(groups[2].last_event_id.as_deref(), Some("evt-5"));
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }

        assert!(rendered.contains("group_by=token group_count=3"));
        assert!(rendered.contains(
            "group[token]=token-a loaded_events=3 event_kind_counts=AuthorizationDenied=1,TokenIssued=1,TokenRevoked=1"
        ));
        assert!(rendered.contains("group[token]=(none) loaded_events=1"));

        assert_eq!(payload["group_by"], "token");
        assert_eq!(payload["groups"][0]["group_value"], "token-a");
        assert_eq!(payload["groups"][0]["loaded_events"], 3);
        assert_eq!(payload["groups"][1]["group_value"], "token-b");
        assert_eq!(payload["groups"][2]["group_value"], Value::Null);
    }

    #[test]
    fn audit_summary_ignores_non_blocking_security_scan_for_triage_rollups() {
        let root = unique_temp_dir("loong-audit-cli-summary-non-blocking-scan");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_150,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_151,
                    Some("agent-b"),
                    AuditEventKind::SecurityScanEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        scanned_plugins: 1,
                        total_findings: 1,
                        high_findings: 0,
                        medium_findings: 1,
                        low_findings: 0,
                        blocked: false,
                        block_reason: None,
                        categories: vec!["signature".to_owned()],
                        finding_ids: vec!["finding-1".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_152,
                    Some("agent-c"),
                    AuditEventKind::PlaneInvoked {
                        pack_id: "sales-intel".to_owned(),
                        plane: ExecutionPlane::Runtime,
                        tier: PlaneTier::Core,
                        primary_adapter: "runtime".to_owned(),
                        delegated_core_adapter: None,
                        operation: "turn.complete".to_owned(),
                        required_capabilities: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: None,
            },
        })
        .expect("execute audit summary");

        match execution.result {
            AuditCommandResult::Summary {
                triage_counts,
                last_triage_event_id,
                last_triage_label,
                last_triage_event_kind,
                last_triage_timestamp_epoch_s,
                last_triage_agent_id,
                last_triage_summary,
                last_triage_hint,
                ..
            } => {
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([("authorization_denied".to_owned(), 1_usize)])
                );
                assert_eq!(last_triage_event_id.as_deref(), Some("evt-1"));
                assert_eq!(last_triage_label.as_deref(), Some("authorization_denied"));
                assert_eq!(
                    last_triage_event_kind.as_deref(),
                    Some("AuthorizationDenied")
                );
                assert_eq!(last_triage_timestamp_epoch_s, Some(1_700_010_150));
                assert_eq!(last_triage_agent_id.as_deref(), Some("agent-a"));
                assert_eq!(
                    last_triage_summary.as_deref(),
                    Some("pack_id=sales-intel token_id=token-1 reason=missing capability")
                );
                assert_eq!(
                    last_triage_hint.as_deref(),
                    Some(
                        "grant the required capability or retry with a token scoped for the requested operation"
                    )
                );
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_summary_ignores_non_blocking_plugin_trust_for_triage_rollups() {
        let root = unique_temp_dir("loong-audit-cli-summary-non-blocking-plugin-trust");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_250,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_251,
                    Some("agent-b"),
                    AuditEventKind::PluginTrustEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        scanned_plugins: 2,
                        official_plugins: 1,
                        verified_community_plugins: 1,
                        unverified_plugins: 0,
                        high_risk_plugins: 1,
                        high_risk_unverified_plugins: 0,
                        blocked_auto_apply_plugins: 0,
                        review_required_plugin_ids: vec!["ffi-reviewed".to_owned()],
                        review_required_bridges: vec!["native_ffi".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_252,
                    Some("agent-c"),
                    AuditEventKind::PlaneInvoked {
                        pack_id: "sales-intel".to_owned(),
                        plane: ExecutionPlane::Runtime,
                        tier: PlaneTier::Core,
                        primary_adapter: "runtime".to_owned(),
                        delegated_core_adapter: None,
                        operation: "turn.complete".to_owned(),
                        required_capabilities: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: None,
            },
        })
        .expect("execute audit summary");

        match execution.result {
            AuditCommandResult::Summary {
                triage_counts,
                last_triage_label,
                last_triage_event_kind,
                last_triage_summary,
                last_triage_hint,
                ..
            } => {
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([("authorization_denied".to_owned(), 1_usize)])
                );
                assert_eq!(last_triage_label.as_deref(), Some("authorization_denied"));
                assert_eq!(
                    last_triage_event_kind.as_deref(),
                    Some("AuthorizationDenied")
                );
                assert_eq!(
                    last_triage_summary.as_deref(),
                    Some("pack_id=sales-intel token_id=token-1 reason=missing capability")
                );
                assert_eq!(
                    last_triage_hint.as_deref(),
                    Some(
                        "grant the required capability or retry with a token scoped for the requested operation"
                    )
                );
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_summary_tracks_tool_search_trust_conflict_triage() {
        let root = unique_temp_dir("loong-audit-cli-summary-tool-search-trust");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_260,
                    Some("agent-a"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: Vec::new(),
                        structured_requested_tiers: vec!["official".to_owned()],
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 1,
                        filtered_out_tier_counts: BTreeMap::from([(
                            "verified-community".to_owned(),
                            1_usize,
                        )]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_261,
                    Some("agent-b"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: vec!["verified-community".to_owned()],
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: true,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("official".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: None,
            },
        })
        .expect("execute audit summary");

        match execution.result {
            AuditCommandResult::Summary {
                event_kind_counts,
                triage_counts,
                last_triage_label,
                last_triage_event_kind,
                last_triage_summary,
                last_triage_hint,
                ..
            } => {
                assert_eq!(
                    event_kind_counts,
                    BTreeMap::from([("ToolSearchEvaluated".to_owned(), 2_usize)])
                );
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([
                        ("tool_search_trust_conflict".to_owned(), 1_usize),
                        ("tool_search_trust_empty".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    last_triage_label.as_deref(),
                    Some("tool_search_trust_conflict")
                );
                assert_eq!(
                    last_triage_event_kind.as_deref(),
                    Some("ToolSearchEvaluated")
                );
                assert_eq!(
                    last_triage_summary.as_deref(),
                    Some(
                        "query=\"trust:official search\" trust_scope=- conflicting_requested_tiers=true filtered_out_candidates=2 top_provider_ids=-"
                    )
                );
                assert_eq!(
                    last_triage_hint.as_deref(),
                    Some(
                        "align query trust prefixes with structured trust_tiers before retrying discovery"
                    )
                );
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_summary_rejects_excessive_limit() {
        let mut env = ScopedEnv::new();
        env.set(
            "HOME",
            unique_temp_dir("loong-audit-cli-large-summary-limit-home"),
        );

        let error = execute_audit_command(AuditCommandOptions {
            config: None,
            json: false,
            command: AuditCommands::Summary {
                limit: 10_001,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: None,
            },
        })
        .expect_err("excessive summary limit should fail");

        assert!(error.contains("audit summary limit must be between 1 and 10000"));
    }

    #[test]
    fn audit_summary_text_includes_triage_counts_and_last_seen_fields() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loong.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_100),
            until_epoch_s_filter: Some(1_700_010_199),
            pack_id_filter: None,
            agent_id_filter: None,
            event_id_filter: Some("evt-3".to_owned()),
            token_id_filter: Some("token-2".to_owned()),
            kind_filter: None,
            triage_label_filter: None,
            query_contains_filter: None,
            trust_tier_filter: None,
            result: AuditCommandResult::Summary {
                limit: 50,
                loaded_events: 3,
                event_kind_counts: BTreeMap::from([
                    ("AuthorizationDenied".to_owned(), 2_usize),
                    ("PlaneInvoked".to_owned(), 1_usize),
                ]),
                triage_counts: BTreeMap::from([
                    ("authorization_denied".to_owned(), 2_usize),
                    ("security_scan_blocked".to_owned(), 1_usize),
                ]),
                group_by: None,
                groups: Vec::new(),
                first_timestamp_epoch_s: Some(1_700_010_100),
                last_event_id: Some("evt-3".to_owned()),
                last_timestamp_epoch_s: Some(1_700_010_102),
                last_agent_id: Some("agent-c".to_owned()),
                last_triage_event_id: Some("evt-2".to_owned()),
                last_triage_label: Some("authorization_denied".to_owned()),
                last_triage_event_kind: Some("AuthorizationDenied".to_owned()),
                last_triage_timestamp_epoch_s: Some(1_700_010_101),
                last_triage_agent_id: Some("agent-b".to_owned()),
                last_triage_summary: Some(
                    "pack_id=sales-intel token_id=token-2 reason=missing capability".to_owned(),
                ),
                last_triage_hint: Some(
                    "grant the required capability or retry with a token scoped for the requested operation"
                        .to_owned(),
                ),
            },
        };

        let rendered = render_audit_cli_text(&execution).expect("render audit summary");

        assert!(rendered.contains("audit summary"));
        assert!(rendered.contains("since_epoch_s=1700010100"));
        assert!(rendered.contains("until_epoch_s=1700010199"));
        assert!(rendered.contains(
            "pack_id=- agent_id=- event_id=evt-3 token_id=token-2 kind=- triage_label=-"
        ));
        assert!(rendered.contains("loaded_events=3"));
        assert!(rendered.contains("first_timestamp_epoch_s=1700010100"));
        assert!(rendered.contains("AuthorizationDenied=2"));
        assert!(rendered.contains("PlaneInvoked=1"));
        assert!(rendered.contains("triage_counts=authorization_denied=2,security_scan_blocked=1"));
        assert!(rendered.contains("group_by=- group_count=0"));
        assert!(rendered.contains("last_event_id=evt-3"));
        assert!(rendered.contains("last_agent_id=agent-c"));
        assert!(rendered.contains("last_triage_event_id=evt-2"));
        assert!(rendered.contains("last_triage_label=authorization_denied"));
        assert!(rendered.contains("last_triage_event_kind=AuthorizationDenied"));
        assert!(rendered.contains("last_triage_agent_id=agent-b"));
        assert!(rendered.contains(
            "last_triage_summary=pack_id=sales-intel token_id=token-2 reason=missing capability"
        ));
        assert!(rendered.contains(
            "last_triage_hint=grant the required capability or retry with a token scoped for the requested operation"
        ));
    }

    #[test]
    fn audit_summary_json_includes_triage_fields() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loong.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_200),
            until_epoch_s_filter: Some(1_700_010_299),
            pack_id_filter: None,
            agent_id_filter: None,
            event_id_filter: Some("evt-2".to_owned()),
            token_id_filter: Some("token-1".to_owned()),
            kind_filter: None,
            triage_label_filter: None,
            query_contains_filter: None,
            trust_tier_filter: None,
            result: AuditCommandResult::Summary {
                limit: 25,
                loaded_events: 2,
                event_kind_counts: BTreeMap::from([
                    ("AuthorizationDenied".to_owned(), 1_usize),
                    ("PlaneInvoked".to_owned(), 1_usize),
                ]),
                triage_counts: BTreeMap::from([("authorization_denied".to_owned(), 1_usize)]),
                group_by: None,
                groups: Vec::new(),
                first_timestamp_epoch_s: Some(1_700_010_200),
                last_event_id: Some("evt-2".to_owned()),
                last_timestamp_epoch_s: Some(1_700_010_201),
                last_agent_id: Some("agent-b".to_owned()),
                last_triage_event_id: Some("evt-1".to_owned()),
                last_triage_label: Some("authorization_denied".to_owned()),
                last_triage_event_kind: Some("AuthorizationDenied".to_owned()),
                last_triage_timestamp_epoch_s: Some(1_700_010_200),
                last_triage_agent_id: Some("agent-a".to_owned()),
                last_triage_summary: Some(
                    "pack_id=sales-intel token_id=token-1 reason=missing capability".to_owned(),
                ),
                last_triage_hint: Some(
                    "grant the required capability or retry with a token scoped for the requested operation"
                        .to_owned(),
                ),
            },
        };

        let payload = audit_cli_json(&execution);

        assert_eq!(payload["since_epoch_s_filter"], 1_700_010_200_u64);
        assert_eq!(payload["until_epoch_s_filter"], 1_700_010_299_u64);
        assert_eq!(payload["event_id_filter"], "evt-2");
        assert_eq!(payload["token_id_filter"], "token-1");
        assert_eq!(payload["group_by"], Value::Null);
        assert_eq!(payload["groups"], json!([]));
        assert_eq!(payload["first_timestamp_epoch_s"], 1_700_010_200_u64);
        assert_eq!(payload["triage_counts"]["authorization_denied"], 1);
        assert_eq!(payload["last_triage_event_id"], "evt-1");
        assert_eq!(payload["last_triage_label"], "authorization_denied");
        assert_eq!(payload["last_triage_event_kind"], "AuthorizationDenied");
        assert_eq!(payload["last_triage_timestamp_epoch_s"], 1_700_010_200_u64);
        assert_eq!(payload["last_triage_agent_id"], "agent-a");
        assert_eq!(
            payload["last_triage_summary"],
            "pack_id=sales-intel token_id=token-1 reason=missing capability"
        );
        assert_eq!(
            payload["last_triage_hint"],
            "grant the required capability or retry with a token scoped for the requested operation"
        );
    }

    #[test]
    fn audit_verify_reports_valid_chain_for_fresh_journal() {
        let root = unique_temp_dir("loong-audit-cli-verify");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        let sink =
            crate::kernel::JsonlAuditSink::new(journal_path).expect("jsonl sink should initialize");

        sink.record(sample_audit_event(
            "evt-verify-1",
            1_700_010_300,
            Some("agent-verify"),
            AuditEventKind::TokenRevoked {
                token_id: "token-verify-1".to_owned(),
            },
        ))
        .expect("record first event");

        sink.record(sample_audit_event(
            "evt-verify-2",
            1_700_010_301,
            Some("agent-verify"),
            AuditEventKind::TokenRevoked {
                token_id: "token-verify-2".to_owned(),
            },
        ))
        .expect("record second event");

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Verify,
        })
        .expect("execute audit verify");

        match execution.result {
            AuditCommandResult::Verify {
                loaded_events,
                verified_events,
                valid,
                ..
            } => {
                assert_eq!(loaded_events, 2);
                assert_eq!(verified_events, 2);
                assert!(valid);
            }
            other => panic!("unexpected audit verify result: {other:?}"),
        }
    }

    #[test]
    fn audit_verify_reports_first_invalid_line_for_tampered_chain() {
        let root = unique_temp_dir("loong-audit-cli-verify-tamper");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        let sink = crate::kernel::JsonlAuditSink::new(journal_path.clone())
            .expect("jsonl sink should initialize");

        sink.record(sample_audit_event(
            "evt-tamper-1",
            1_700_010_310,
            Some("agent-tamper"),
            AuditEventKind::TokenRevoked {
                token_id: "token-tamper-1".to_owned(),
            },
        ))
        .expect("record first event");

        sink.record(sample_audit_event(
            "evt-tamper-2",
            1_700_010_311,
            Some("agent-tamper"),
            AuditEventKind::TokenRevoked {
                token_id: "token-tamper-2".to_owned(),
            },
        ))
        .expect("record second event");

        let contents = fs::read_to_string(&journal_path).expect("read audit journal");
        let tampered = contents.replacen("token-tamper-2", "token-tamper-x", 1);
        fs::write(&journal_path, tampered).expect("rewrite tampered journal");

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: true,
            command: AuditCommands::Verify,
        })
        .expect("execute audit verify");

        let payload = audit_cli_json(&execution);

        assert_eq!(payload["command"], "verify");
        assert_eq!(payload["valid"], json!(false));
        assert_eq!(payload["first_invalid_line"], json!(2));
        assert_eq!(payload["reason"], json!("entry_hash mismatch"));
    }

    #[test]
    fn audit_verify_accepts_legacy_prefix_and_verifies_protected_tail() {
        let root = unique_temp_dir("loong-audit-cli-verify-legacy-prefix");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        let legacy_event = sample_audit_event(
            "evt-legacy-1",
            1_700_010_320,
            Some("agent-legacy"),
            AuditEventKind::TokenRevoked {
                token_id: "token-legacy-1".to_owned(),
            },
        );
        write_journal(&journal_path, &[legacy_event]);
        let sink =
            crate::kernel::JsonlAuditSink::new(journal_path).expect("jsonl sink should initialize");

        sink.record(sample_audit_event(
            "evt-verify-legacy-tail",
            1_700_010_321,
            Some("agent-legacy"),
            AuditEventKind::TokenRevoked {
                token_id: "token-legacy-2".to_owned(),
            },
        ))
        .expect("record protected event");

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: true,
            command: AuditCommands::Verify,
        })
        .expect("execute audit verify");

        let payload = audit_cli_json(&execution);

        assert_eq!(payload["command"], "verify");
        assert_eq!(payload["loaded_events"], json!(2));
        assert_eq!(payload["verified_events"], json!(1));
        assert_eq!(payload["valid"], json!(true));
        assert_eq!(payload["first_invalid_line"], Value::Null);
    }

    #[test]
    fn audit_repair_reports_healthy_for_valid_chain() {
        let root = unique_temp_dir("loong-audit-cli-repair-healthy");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        let sink =
            crate::kernel::JsonlAuditSink::new(journal_path).expect("jsonl sink should initialize");

        sink.record(sample_audit_event(
            "evt-repair-healthy-1",
            1_700_010_330,
            Some("agent-repair"),
            AuditEventKind::TokenRevoked {
                token_id: "token-repair-healthy-1".to_owned(),
            },
        ))
        .expect("record first event");

        sink.record(sample_audit_event(
            "evt-repair-healthy-2",
            1_700_010_331,
            Some("agent-repair"),
            AuditEventKind::TokenRevoked {
                token_id: "token-repair-healthy-2".to_owned(),
            },
        ))
        .expect("record second event");

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Repair,
        })
        .expect("execute audit repair");

        match &execution.result {
            AuditCommandResult::Repair {
                total_events,
                repaired_events,
                already_valid_events,
                outcome,
                refused_line,
                refused_reason,
            } => {
                assert_eq!(*total_events, 2);
                assert_eq!(*repaired_events, 0);
                assert_eq!(*already_valid_events, 2);
                assert_eq!(outcome, "healthy");
                assert_eq!(*refused_line, None);
                assert!(refused_reason.is_none());
            }
            other => panic!("unexpected audit repair result: {other:?}"),
        }

        let rendered = render_audit_cli_text(&execution).expect("render audit repair");

        assert!(rendered.contains("audit repair"));
        assert!(rendered.contains("already_valid_events=2"));
        assert!(rendered.contains("outcome=healthy"));
    }

    #[test]
    fn audit_repair_reports_repaired_for_legacy_prefix() {
        let root = unique_temp_dir("loong-audit-cli-repair-legacy-prefix");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        let legacy_event = sample_audit_event(
            "evt-repair-legacy-1",
            1_700_010_340,
            Some("agent-repair"),
            AuditEventKind::TokenRevoked {
                token_id: "token-repair-legacy-1".to_owned(),
            },
        );

        write_journal(&journal_path, &[legacy_event]);

        let sink = crate::kernel::JsonlAuditSink::new(journal_path.clone())
            .expect("jsonl sink should initialize");

        sink.record(sample_audit_event(
            "evt-repair-tail-1",
            1_700_010_341,
            Some("agent-repair"),
            AuditEventKind::TokenRevoked {
                token_id: "token-repair-tail-1".to_owned(),
            },
        ))
        .expect("record protected event");

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: true,
            command: AuditCommands::Repair,
        })
        .expect("execute audit repair");

        match &execution.result {
            AuditCommandResult::Repair {
                total_events,
                repaired_events,
                already_valid_events,
                outcome,
                refused_line,
                refused_reason,
            } => {
                assert_eq!(*total_events, 2);
                assert_eq!(*repaired_events, 2);
                assert_eq!(*already_valid_events, 0);
                assert_eq!(outcome, "repaired");
                assert_eq!(*refused_line, None);
                assert!(refused_reason.is_none());
            }
            other => panic!("unexpected audit repair result: {other:?}"),
        }

        let payload = audit_cli_json(&execution);
        let verify_report = crate::kernel::verify_jsonl_audit_journal(&journal_path)
            .expect("verify repaired journal");

        assert_eq!(payload["command"], "repair");
        assert_eq!(payload["total_events"], json!(2));
        assert_eq!(payload["repaired_events"], json!(2));
        assert_eq!(payload["already_valid_events"], json!(0));
        assert_eq!(payload["outcome"], json!("repaired"));
        assert_eq!(payload["refused_line"], Value::Null);
        assert_eq!(payload["refused_reason"], Value::Null);
        assert!(verify_report.valid);
        assert_eq!(verify_report.verified_events, 2);
    }

    #[test]
    fn audit_repair_reports_refused_for_tampered_journal() {
        let root = unique_temp_dir("loong-audit-cli-repair-refused");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        let sink = crate::kernel::JsonlAuditSink::new(journal_path.clone())
            .expect("jsonl sink should initialize");

        sink.record(sample_audit_event(
            "evt-repair-refused-1",
            1_700_010_350,
            Some("agent-repair"),
            AuditEventKind::TokenRevoked {
                token_id: "token-repair-refused-1".to_owned(),
            },
        ))
        .expect("record first event");

        sink.record(sample_audit_event(
            "evt-repair-refused-2",
            1_700_010_351,
            Some("agent-repair"),
            AuditEventKind::TokenRevoked {
                token_id: "token-repair-refused-2".to_owned(),
            },
        ))
        .expect("record second event");

        let original_contents = fs::read_to_string(&journal_path).expect("read audit journal");
        let tampered_contents =
            original_contents.replacen("token-repair-refused-2", "token-repair-refused-x", 1);

        fs::write(&journal_path, &tampered_contents).expect("rewrite tampered journal");

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: true,
            command: AuditCommands::Repair,
        })
        .expect("execute audit repair");

        match &execution.result {
            AuditCommandResult::Repair {
                total_events,
                repaired_events,
                already_valid_events,
                outcome,
                refused_line,
                refused_reason,
            } => {
                assert_eq!(*total_events, 2);
                assert_eq!(*repaired_events, 0);
                assert_eq!(*already_valid_events, 1);
                assert_eq!(outcome, "refused");
                assert_eq!(*refused_line, Some(2));
                assert_eq!(
                    refused_reason.as_deref(),
                    Some("entry_hash mismatch — event data may be tampered")
                );
            }
            other => panic!("unexpected audit repair result: {other:?}"),
        }

        let payload = audit_cli_json(&execution);
        let rendered = render_audit_cli_text(&execution).expect("render audit repair");

        assert_eq!(payload["command"], "repair");
        assert_eq!(payload["outcome"], json!("refused"));
        assert_eq!(payload["refused_line"], json!(2));
        assert_eq!(
            payload["refused_reason"],
            json!("entry_hash mismatch — event data may be tampered")
        );
        assert!(rendered.contains("outcome=refused"));
        assert!(rendered.contains("refused_line=2"));
        assert!(rendered.contains("refused_reason=entry_hash mismatch"));
    }

    #[test]
    fn audit_summary_json_uses_empty_and_null_triage_fields_when_no_triage_events_exist() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loong.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: None,
            until_epoch_s_filter: None,
            pack_id_filter: None,
            agent_id_filter: None,
            event_id_filter: None,
            token_id_filter: None,
            kind_filter: None,
            triage_label_filter: None,
            query_contains_filter: None,
            trust_tier_filter: None,
            result: AuditCommandResult::Summary {
                limit: 10,
                loaded_events: 1,
                event_kind_counts: BTreeMap::from([("TokenIssued".to_owned(), 1_usize)]),
                triage_counts: BTreeMap::new(),
                group_by: None,
                groups: Vec::new(),
                first_timestamp_epoch_s: Some(1_700_010_300),
                last_event_id: Some("evt-1".to_owned()),
                last_timestamp_epoch_s: Some(1_700_010_300),
                last_agent_id: Some("agent-a".to_owned()),
                last_triage_event_id: None,
                last_triage_label: None,
                last_triage_event_kind: None,
                last_triage_timestamp_epoch_s: None,
                last_triage_agent_id: None,
                last_triage_summary: None,
                last_triage_hint: None,
            },
        };

        let payload = audit_cli_json(&execution);

        assert_eq!(
            payload["triage_counts"].as_object(),
            Some(&serde_json::Map::new())
        );
        assert_eq!(payload["last_triage_event_id"], Value::Null);
        assert_eq!(payload["last_triage_label"], Value::Null);
        assert_eq!(payload["last_triage_event_kind"], Value::Null);
        assert_eq!(payload["last_triage_timestamp_epoch_s"], Value::Null);
        assert_eq!(payload["last_triage_agent_id"], Value::Null);
        assert_eq!(payload["last_triage_summary"], Value::Null);
        assert_eq!(payload["last_triage_hint"], Value::Null);
    }

    #[test]
    fn audit_summary_text_uses_placeholders_when_no_triage_events_exist() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loong.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: None,
            until_epoch_s_filter: None,
            pack_id_filter: None,
            agent_id_filter: None,
            event_id_filter: None,
            token_id_filter: None,
            kind_filter: None,
            triage_label_filter: None,
            query_contains_filter: None,
            trust_tier_filter: None,
            result: AuditCommandResult::Summary {
                limit: 10,
                loaded_events: 1,
                event_kind_counts: BTreeMap::from([("TokenIssued".to_owned(), 1_usize)]),
                triage_counts: BTreeMap::new(),
                group_by: None,
                groups: Vec::new(),
                first_timestamp_epoch_s: Some(1_700_010_300),
                last_event_id: Some("evt-1".to_owned()),
                last_timestamp_epoch_s: Some(1_700_010_300),
                last_agent_id: Some("agent-a".to_owned()),
                last_triage_event_id: None,
                last_triage_label: None,
                last_triage_event_kind: None,
                last_triage_timestamp_epoch_s: None,
                last_triage_agent_id: None,
                last_triage_summary: None,
                last_triage_hint: None,
            },
        };

        let rendered = render_audit_cli_text(&execution).expect("render audit summary");

        assert!(rendered.contains("triage_counts=-"));
        assert!(rendered.contains("last_triage_event_id=-"));
        assert!(rendered.contains("last_triage_label=-"));
        assert!(rendered.contains("last_triage_event_kind=-"));
        assert!(rendered.contains("last_triage_summary=-"));
        assert!(rendered.contains("last_triage_hint=-"));
    }
