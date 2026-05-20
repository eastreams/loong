    #[cfg(feature = "memory-sqlite")]
    use std::fs;

    #[cfg(feature = "memory-sqlite")]
    use crate::session::repository::{
        ApprovalRequestStatus, NewApprovalRequestRecord, NewSessionEvent, NewSessionRecord,
        SessionKind, SessionRepository, SessionState,
    };
    #[cfg(feature = "memory-sqlite")]
    use crate::session::store::SessionStoreConfig;
    #[cfg(feature = "memory-sqlite")]
    use crate::{
        acp::{
            AcpRoutingOrigin, AcpSessionBindingScope, AcpSessionMetadata, AcpSessionMode,
            AcpSessionState, AcpSessionStore, AcpSqliteSessionStore,
        },
        config::LoongConfig,
        test_support::ScopedEnv,
    };

    use super::*;

    #[test]
    fn initial_snapshot_is_empty_and_not_ready() {
        let manager = ControlPlaneManager::new();
        let snapshot = manager.snapshot();
        assert_eq!(snapshot.presence_count, 0);
        assert_eq!(snapshot.session_count, 0);
        assert_eq!(snapshot.pending_approval_count, 0);
        assert_eq!(snapshot.acp_session_count, 0);
        assert!(!snapshot.runtime_ready);
        assert_eq!(snapshot.state_version, ControlPlaneStateVersion::default());
    }

    #[test]
    fn presence_change_bumps_presence_version_and_global_seq() {
        let manager = ControlPlaneManager::new();
        let event = manager.record_presence_changed(3, serde_json::json!({ "presence_count": 3 }));
        assert_eq!(event.kind, ControlPlaneEventKind::PresenceChanged);
        assert_eq!(event.event_name, "presence.changed");
        assert_eq!(event.seq, 1);
        assert_eq!(event.state_version.presence, 1);
        assert_eq!(event.state_version.health, 0);
        assert_eq!(manager.snapshot().presence_count, 3);
    }

    #[test]
    fn health_change_updates_runtime_ready_without_mutating_other_counts() {
        let manager = ControlPlaneManager::new();
        manager.set_presence_count(2);
        let event = manager.record_health_changed(true, serde_json::json!({ "healthy": true }));
        assert_eq!(event.seq, 1);
        assert_eq!(event.state_version.health, 1);
        assert_eq!(event.state_version.presence, 0);
        let snapshot = manager.snapshot();
        assert!(snapshot.runtime_ready);
        assert_eq!(snapshot.presence_count, 2);
    }

    #[test]
    fn approval_events_update_pending_count_and_keep_global_sequence() {
        let manager = ControlPlaneManager::new();
        let requested =
            manager.record_approval_requested(2, serde_json::json!({ "request_id": "apr-1" }));
        let resolved =
            manager.record_approval_resolved(1, serde_json::json!({ "request_id": "apr-1" }), true);
        assert_eq!(requested.seq, 1);
        assert_eq!(resolved.seq, 2);
        assert_eq!(resolved.state_version.approvals, 2);
        assert!(resolved.targeted);
        assert_eq!(manager.snapshot().pending_approval_count, 1);
    }

    #[test]
    fn session_and_acp_versions_advance_independently() {
        let manager = ControlPlaneManager::new();
        let session_event =
            manager.record_sessions_changed(4, serde_json::json!({ "session_count": 4 }));
        let acp_event =
            manager.record_acp_session_changed(2, serde_json::json!({ "acp_session_count": 2 }));
        assert_eq!(session_event.seq, 1);
        assert_eq!(acp_event.seq, 2);
        assert_eq!(session_event.state_version.sessions, 1);
        assert_eq!(session_event.state_version.acp, 0);
        assert_eq!(acp_event.state_version.sessions, 1);
        assert_eq!(acp_event.state_version.acp, 1);
        let snapshot = manager.snapshot();
        assert_eq!(snapshot.session_count, 4);
        assert_eq!(snapshot.acp_session_count, 2);
    }

    #[test]
    fn session_message_event_is_targetable_without_changing_counts() {
        let manager = ControlPlaneManager::new();
        manager.set_session_count(5);
        let event =
            manager.record_session_message(serde_json::json!({ "session_id": "s-1" }), true);
        assert_eq!(event.kind, ControlPlaneEventKind::SessionMessage);
        assert!(event.targeted);
        assert_eq!(event.state_version.sessions, 1);
        assert_eq!(manager.snapshot().session_count, 5);
    }

    #[test]
    fn turn_registry_prunes_oldest_terminal_turns() {
        let registry = ControlPlaneTurnRegistry::new();
        let first_turn = registry.issue_turn("session-0");
        let first_output = "output-0".to_owned();
        registry
            .complete_success(
                first_turn.turn_id.as_str(),
                first_output.as_str(),
                Some("completed"),
                None,
            )
            .expect("complete first turn");
        let mut newest_turn_id = first_turn.turn_id.clone();
        for index in 1..=CONTROL_PLANE_TURN_TERMINAL_RETENTION_LIMIT {
            let session_id = format!("session-{index}");
            let output_text = format!("output-{index}");
            let turn = registry.issue_turn(session_id.as_str());
            registry
                .complete_success(
                    turn.turn_id.as_str(),
                    output_text.as_str(),
                    Some("completed"),
                    None,
                )
                .expect("complete retained turn");
            newest_turn_id = turn.turn_id;
        }
        let removed_turn = registry
            .read_turn(first_turn.turn_id.as_str())
            .expect("read pruned turn");
        let retained_turn = registry
            .read_turn(newest_turn_id.as_str())
            .expect("read retained turn");
        let retained_terminal_count = {
            let turns = registry
                .turns
                .read()
                .unwrap_or_else(|error| error.into_inner());
            turns
                .values()
                .filter(|record| record.snapshot.status.is_terminal())
                .count()
        };
        assert!(removed_turn.is_none());
        assert!(retained_turn.is_some());
        assert_eq!(
            retained_terminal_count,
            CONTROL_PLANE_TURN_TERMINAL_RETENTION_LIMIT
        );
    }

    #[test]
    fn turn_registry_rejects_mutation_after_terminal_completion() {
        let registry = ControlPlaneTurnRegistry::new();
        let turn = registry.issue_turn("session-1");
        registry
            .complete_success(turn.turn_id.as_str(), "done", Some("completed"), None)
            .expect("complete turn");
        let runtime_event_error = registry
            .record_runtime_event(turn.turn_id.as_str(), json!({ "type": "late" }))
            .expect_err("late runtime event should be rejected");
        let completion_error = registry
            .complete_failure(turn.turn_id.as_str(), "late failure")
            .expect_err("late completion should be rejected");
        assert!(runtime_event_error.contains("control_plane_turn_already_terminal"));
        assert!(completion_error.contains("control_plane_turn_already_terminal"));
    }

    #[test]
    fn recent_events_retains_chronological_tail_with_bounded_capacity() {
        let manager = ControlPlaneManager::new();
        for idx in 0..300 {
            let _ = manager.record_session_message(serde_json::json!({ "idx": idx }), false);
        }

        let events = manager.recent_events(256, true);
        assert_eq!(events.len(), 256);
        assert_eq!(events.first().expect("first").payload["idx"], 44);
        assert_eq!(events.last().expect("last").payload["idx"], 299);
        assert_eq!(events.first().expect("first").seq, 45);
        assert_eq!(events.last().expect("last").seq, 300);
    }

    #[test]
    fn recent_events_can_exclude_targeted_records() {
        let manager = ControlPlaneManager::new();
        let _ = manager.record_session_message(serde_json::json!({ "kind": "broadcast" }), false);
        let _ = manager.record_session_message(serde_json::json!({ "kind": "targeted" }), true);

        let broadcast_only = manager.recent_events(10, false);
        assert_eq!(broadcast_only.len(), 1);
        assert_eq!(broadcast_only[0].payload["kind"], "broadcast");

        let all = manager.recent_events(10, true);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn recent_events_limit_returns_latest_subset_in_order() {
        let manager = ControlPlaneManager::new();
        let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
        let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));
        let _ = manager.record_sessions_changed(3, serde_json::json!({ "idx": 3 }));

        let events = manager.recent_events(2, true);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].payload["idx"], 2);
        assert_eq!(events[1].payload["idx"], 3);
    }

    #[test]
    fn recent_events_after_returns_earliest_unseen_page() {
        let manager = ControlPlaneManager::new();
        for idx in 1..=5 {
            let payload = serde_json::json!({ "idx": idx });
            let _ = manager.record_session_message(payload, false);
        }

        let events = manager.recent_events_after(1, 2, true);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].payload["idx"], 2);
        assert_eq!(events[1].payload["idx"], 3);
        assert_eq!(events[0].seq, 2);
        assert_eq!(events[1].seq, 3);
    }

    #[tokio::test]
    async fn wait_for_recent_events_returns_immediately_when_seq_is_available() {
        let manager = ControlPlaneManager::new();
        let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
        let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));

        let events = manager.wait_for_recent_events(1, 10, true, 1000).await;

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload["idx"], 2);
    }

    #[tokio::test]
    async fn wait_for_recent_events_blocks_until_new_event_arrives() {
        let manager = std::sync::Arc::new(ControlPlaneManager::new());
        let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
        let waiter = {
            let manager = manager.clone();
            tokio::spawn(async move { manager.wait_for_recent_events(1, 10, true, 1_000).await })
        };

        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));

        let events = waiter.await.expect("waiter join");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload["idx"], 2);
    }

    #[tokio::test]
    async fn subscribe_receives_new_event_broadcast() {
        let manager = ControlPlaneManager::new();
        let mut receiver = manager.subscribe();

        let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));

        let received = receiver.recv().await.expect("receive broadcast");
        assert_eq!(received.seq, 1);
        assert_eq!(received.payload["idx"], 1);
    }

    #[test]
    fn connection_registry_issues_and_resolves_ephemeral_token() {
        let registry = ControlPlaneConnectionRegistry::new();
        let lease = registry.issue(ControlPlaneConnectionPrincipal {
            connection_id: "cp-1".to_owned(),
            client_id: "cli".to_owned(),
            role: "operator".to_owned(),
            scopes: BTreeSet::from(["operator.read".to_owned()]),
            device_id: Some("device-1".to_owned()),
        });
        assert!(lease.token.starts_with("cpt-"));
        assert!(lease.expires_at_ms >= lease.issued_at_ms);

        let resolved = registry
            .resolve(&lease.token)
            .expect("resolve lease")
            .expect("lease should exist");
        assert_eq!(resolved.principal.client_id, "cli");
        assert_eq!(resolved.principal.role, "operator");
        assert!(resolved.principal.scopes.contains("operator.read"));
        assert_eq!(resolved.principal.device_id.as_deref(), Some("device-1"));
    }

    #[test]
    fn connection_registry_expires_and_revokes_tokens() {
        let registry = ControlPlaneConnectionRegistry::new();
        let expired = registry.issue_with_ttl_ms(
            ControlPlaneConnectionPrincipal {
                connection_id: "cp-2".to_owned(),
                client_id: "cli".to_owned(),
                role: "operator".to_owned(),
                scopes: BTreeSet::new(),
                device_id: None,
            },
            0,
        );
        registry
            .connections
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .get_mut(&expired.token)
            .expect("expired lease should exist")
            .expires_at_ms = current_time_ms().saturating_sub(1);
        assert!(
            registry
                .resolve(&expired.token)
                .expect("resolve expired")
                .is_none()
        );

        let active = registry.issue(ControlPlaneConnectionPrincipal {
            connection_id: "cp-3".to_owned(),
            client_id: "cli".to_owned(),
            role: "operator".to_owned(),
            scopes: BTreeSet::new(),
            device_id: None,
        });
        assert!(registry.revoke(&active.token));
        assert!(
            registry
                .resolve(&active.token)
                .expect("resolve revoked")
                .is_none()
        );
    }

    #[test]
    fn connection_registry_tracks_monotonic_acknowledged_seq() {
        let registry = ControlPlaneConnectionRegistry::new();
        let lease = registry.issue(ControlPlaneConnectionPrincipal {
            connection_id: "cp-ack".to_owned(),
            client_id: "cli".to_owned(),
            role: "operator".to_owned(),
            scopes: BTreeSet::from(["operator.read".to_owned()]),
            device_id: Some("device-1".to_owned()),
        });

        let updated = registry
            .acknowledge_seq(&lease.token, 7)
            .expect("acknowledge seq")
            .expect("lease should exist");
        assert_eq!(updated.acknowledged_seq, Some(7));

        let downgraded = registry
            .acknowledge_seq(&lease.token, 3)
            .expect("acknowledge smaller seq")
            .expect("lease should still exist");
        assert_eq!(downgraded.acknowledged_seq, Some(7));

        let resolved = registry
            .resolve(&lease.token)
            .expect("resolve lease")
            .expect("lease should exist");
        assert_eq!(resolved.acknowledged_seq, Some(7));
    }

    #[test]
    fn connection_registry_restores_non_expired_leases_from_snapshot() {
        let registry = ControlPlaneConnectionRegistry::new();
        let future_expiry = current_time_ms().saturating_add(30_000);
        let leases = vec![
            ControlPlaneConnectionLease {
                token: "cpt-restore-active".to_owned(),
                principal: ControlPlaneConnectionPrincipal {
                    connection_id: "cp-restore".to_owned(),
                    client_id: "cli".to_owned(),
                    role: "operator".to_owned(),
                    scopes: BTreeSet::from(["operator.read".to_owned()]),
                    device_id: Some("device-1".to_owned()),
                },
                issued_at_ms: current_time_ms(),
                expires_at_ms: future_expiry,
                acknowledged_seq: Some(9),
            },
            ControlPlaneConnectionLease {
                token: "cpt-restore-expired".to_owned(),
                principal: ControlPlaneConnectionPrincipal {
                    connection_id: "cp-expired".to_owned(),
                    client_id: "cli".to_owned(),
                    role: "operator".to_owned(),
                    scopes: BTreeSet::new(),
                    device_id: None,
                },
                issued_at_ms: current_time_ms().saturating_sub(60_000),
                expires_at_ms: current_time_ms().saturating_sub(1),
                acknowledged_seq: None,
            },
        ];

        let restored = registry.restore_leases(&leases).expect("restore leases");
        assert_eq!(restored, 1);

        let resolved = registry
            .resolve("cpt-restore-active")
            .expect("resolve restored lease")
            .expect("active lease should exist");
        assert_eq!(resolved.acknowledged_seq, Some(9));
        assert_eq!(resolved.principal.device_id.as_deref(), Some("device-1"));
        assert!(
            registry
                .resolve("cpt-restore-expired")
                .expect("resolve expired restored lease")
                .is_none()
        );
    }

    #[test]
    fn challenge_registry_issues_and_consumes_nonce_once() {
        let registry = ControlPlaneChallengeRegistry::new();
        let challenge = registry.issue();
        assert!(challenge.nonce.starts_with("cpc-"));
        assert!(challenge.expires_at_ms >= challenge.issued_at_ms);

        let consumed = registry
            .consume(&challenge.nonce)
            .expect("consume challenge")
            .expect("challenge should exist");
        assert_eq!(consumed, challenge);
        assert!(
            registry
                .consume(&challenge.nonce)
                .expect("consume challenge again")
                .is_none()
        );
    }

    #[test]
    fn challenge_registry_drops_expired_nonce() {
        let registry = ControlPlaneChallengeRegistry::new();
        let challenge = registry.issue_with_ttl_ms(0);
        registry
            .challenges
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .get_mut(&challenge.nonce)
            .expect("challenge should exist")
            .expires_at_ms = current_time_ms().saturating_sub(1);
        assert!(
            registry
                .consume(&challenge.nonce)
                .expect("consume expired challenge")
                .is_none()
        );
    }

    #[test]
    fn pairing_registry_creates_and_deduplicates_pending_request() {
        let registry = ControlPlanePairingRegistry::new();
        let scopes = BTreeSet::from(["operator.read".to_owned()]);
        let first = registry
            .evaluate_connect("device-1", "cli", "pk-1", "operator", &scopes, None)
            .expect("evaluate connect");
        let second = registry
            .evaluate_connect("device-1", "cli", "pk-1", "operator", &scopes, None)
            .expect("evaluate connect");

        let ControlPlanePairingConnectDecision::PairingRequired {
            request: first_request,
            created: first_created,
        } = first
        else {
            panic!("expected first pairing request");
        };
        let ControlPlanePairingConnectDecision::PairingRequired {
            request: second_request,
            created: second_created,
        } = second
        else {
            panic!("expected second pairing request");
        };
        assert!(first_created);
        assert!(!second_created);
        assert_eq!(
            first_request.pairing_request_id,
            second_request.pairing_request_id
        );
        assert_eq!(registry.list_requests(None, 10).len(), 1);
    }

    #[test]
    fn pairing_registry_approves_and_requires_device_token() {
        let registry = ControlPlanePairingRegistry::new();
        let scopes = BTreeSet::from(["operator.read".to_owned()]);
        let pending = registry
            .evaluate_connect("device-1", "cli", "pk-1", "operator", &scopes, None)
            .expect("evaluate connect");
        let request_id = match pending {
            ControlPlanePairingConnectDecision::PairingRequired { request, .. } => {
                request.pairing_request_id
            }
            other @ ControlPlanePairingConnectDecision::Authorized
            | other @ ControlPlanePairingConnectDecision::DeviceTokenRequired
            | other @ ControlPlanePairingConnectDecision::DeviceTokenInvalid => {
                panic!("expected pairing request, got {other:?}")
            }
        };
        let approved = registry
            .resolve_request(&request_id, true)
            .expect("resolve request")
            .expect("request should exist");
        assert_eq!(approved.status, ControlPlanePairingStatus::Approved);
        let token = approved.device_token.expect("device token");

        let missing_token = registry
            .evaluate_connect("device-1", "cli", "pk-1", "operator", &scopes, None)
            .expect("evaluate connect");
        assert_eq!(
            missing_token,
            ControlPlanePairingConnectDecision::DeviceTokenRequired
        );

        let invalid_token = registry
            .evaluate_connect(
                "device-1",
                "cli",
                "pk-1",
                "operator",
                &scopes,
                Some("wrong"),
            )
            .expect("evaluate connect");
        assert_eq!(
            invalid_token,
            ControlPlanePairingConnectDecision::DeviceTokenInvalid
        );

        let authorized = registry
            .evaluate_connect("device-1", "cli", "pk-1", "operator", &scopes, Some(&token))
            .expect("evaluate connect");
        assert_eq!(authorized, ControlPlanePairingConnectDecision::Authorized);
    }

    #[test]
    fn pairing_registry_rejects_request_without_issuing_device_token() {
        let registry = ControlPlanePairingRegistry::new();
        let scopes = BTreeSet::from(["operator.read".to_owned()]);
        let pending = registry
            .evaluate_connect("device-1", "cli", "pk-1", "operator", &scopes, None)
            .expect("evaluate connect");
        let request_id = match pending {
            ControlPlanePairingConnectDecision::PairingRequired { request, .. } => {
                request.pairing_request_id
            }
            other @ ControlPlanePairingConnectDecision::Authorized
            | other @ ControlPlanePairingConnectDecision::DeviceTokenRequired
            | other @ ControlPlanePairingConnectDecision::DeviceTokenInvalid => {
                panic!("expected pairing request, got {other:?}")
            }
        };
        let rejected = registry
            .resolve_request(&request_id, false)
            .expect("resolve request")
            .expect("request should exist");
        assert_eq!(rejected.status, ControlPlanePairingStatus::Rejected);
        assert!(rejected.device_token.is_none());
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn pairing_registry_with_memory_config_rehydrates_pending_and_approved_state() {
        let memory_config = isolated_memory_config("pairing-registry-persistence");
        let registry = ControlPlanePairingRegistry::with_memory_config(memory_config.clone())
            .expect("persistent pairing registry");
        let scopes = BTreeSet::from(["operator.read".to_owned()]);

        let pending = registry
            .evaluate_connect(
                "device-pending",
                "cli",
                "pk-pending",
                "operator",
                &scopes,
                None,
            )
            .expect("evaluate connect");
        let pending_request_id = match pending {
            ControlPlanePairingConnectDecision::PairingRequired { request, .. } => {
                request.pairing_request_id.clone()
            }
            other @ ControlPlanePairingConnectDecision::Authorized
            | other @ ControlPlanePairingConnectDecision::DeviceTokenRequired
            | other @ ControlPlanePairingConnectDecision::DeviceTokenInvalid => {
                panic!("expected pending pairing request, got {other:?}")
            }
        };

        let approved_pending = registry
            .evaluate_connect(
                "device-approved",
                "cli",
                "pk-approved",
                "operator",
                &scopes,
                None,
            )
            .expect("evaluate connect");
        let approved_request_id = match approved_pending {
            ControlPlanePairingConnectDecision::PairingRequired { request, .. } => {
                request.pairing_request_id.clone()
            }
            other @ ControlPlanePairingConnectDecision::Authorized
            | other @ ControlPlanePairingConnectDecision::DeviceTokenRequired
            | other @ ControlPlanePairingConnectDecision::DeviceTokenInvalid => {
                panic!("expected pairing request, got {other:?}")
            }
        };

        let approved = registry
            .resolve_request(&approved_request_id, true)
            .expect("resolve request")
            .expect("approved request");
        let device_token = approved.device_token.expect("device token");

        let restored = ControlPlanePairingRegistry::with_memory_config(memory_config)
            .expect("restored pairing registry");
        let requests = restored.list_requests(None, 10);
        assert!(
            requests
                .iter()
                .any(|request| request.pairing_request_id == pending_request_id
                    && request.status == ControlPlanePairingStatus::Pending)
        );
        let authorized = restored
            .evaluate_connect(
                "device-approved",
                "cli",
                "pk-approved",
                "operator",
                &scopes,
                Some(&device_token),
            )
            .expect("evaluate connect");
        assert_eq!(authorized, ControlPlanePairingConnectDecision::Authorized);
    }

    #[test]
    fn pairing_registry_requires_repairing_for_scope_upgrade() {
        let registry = ControlPlanePairingRegistry::new();
        let initial_scopes = BTreeSet::from(["operator.read".to_owned()]);
        let pending = registry
            .evaluate_connect("device-1", "cli", "pk-1", "operator", &initial_scopes, None)
            .expect("evaluate connect");
        let request_id = match pending {
            ControlPlanePairingConnectDecision::PairingRequired { request, .. } => {
                request.pairing_request_id.clone()
            }
            other @ ControlPlanePairingConnectDecision::Authorized
            | other @ ControlPlanePairingConnectDecision::DeviceTokenRequired
            | other @ ControlPlanePairingConnectDecision::DeviceTokenInvalid => {
                panic!("expected pairing request, got {other:?}")
            }
        };
        let approved = registry
            .resolve_request(&request_id, true)
            .expect("resolve request")
            .expect("approved request");
        let device_token = approved.device_token.expect("device token");

        let upgraded_scopes =
            BTreeSet::from(["operator.read".to_owned(), "operator.acp".to_owned()]);
        let upgraded = registry
            .evaluate_connect(
                "device-1",
                "cli",
                "pk-1",
                "operator",
                &upgraded_scopes,
                Some(&device_token),
            )
            .expect("evaluate connect");
        let upgraded_request = match upgraded {
            ControlPlanePairingConnectDecision::PairingRequired { request, .. } => request,
            other @ ControlPlanePairingConnectDecision::Authorized
            | other @ ControlPlanePairingConnectDecision::DeviceTokenRequired
            | other @ ControlPlanePairingConnectDecision::DeviceTokenInvalid => {
                panic!("expected upgraded pairing request, got {other:?}")
            }
        };
        assert_eq!(upgraded_request.role, "operator");
        assert_eq!(upgraded_request.requested_scopes, upgraded_scopes);
    }

    #[test]
    fn pairing_registry_requires_repairing_for_role_change() {
        let registry = ControlPlanePairingRegistry::new();
        let scopes = BTreeSet::from(["operator.read".to_owned()]);
        let pending = registry
            .evaluate_connect("device-1", "cli", "pk-1", "operator", &scopes, None)
            .expect("evaluate connect");
        let request_id = match pending {
            ControlPlanePairingConnectDecision::PairingRequired { request, .. } => {
                request.pairing_request_id.clone()
            }
            other @ ControlPlanePairingConnectDecision::Authorized
            | other @ ControlPlanePairingConnectDecision::DeviceTokenRequired
            | other @ ControlPlanePairingConnectDecision::DeviceTokenInvalid => {
                panic!("expected pairing request, got {other:?}")
            }
        };
        let approved = registry
            .resolve_request(&request_id, true)
            .expect("resolve request")
            .expect("approved request");
        let device_token = approved.device_token.expect("device token");

        let reparing = registry
            .evaluate_connect(
                "device-1",
                "cli",
                "pk-1",
                "node",
                &scopes,
                Some(&device_token),
            )
            .expect("evaluate connect");
        let reparing_request = match reparing {
            ControlPlanePairingConnectDecision::PairingRequired { request, .. } => request,
            other @ ControlPlanePairingConnectDecision::Authorized
            | other @ ControlPlanePairingConnectDecision::DeviceTokenRequired
            | other @ ControlPlanePairingConnectDecision::DeviceTokenInvalid => {
                panic!("expected role-change pairing request, got {other:?}")
            }
        };

        assert_eq!(reparing_request.role, "node");
        assert_eq!(reparing_request.requested_scopes, scopes);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn pairing_registry_does_not_leave_pending_request_when_persistence_fails() {
        let registry = broken_pairing_registry("pending-persist-failure");
        let scopes = BTreeSet::from(["operator.read".to_owned()]);

        let error = registry
            .evaluate_connect("device-1", "cli", "pk-1", "operator", &scopes, None)
            .expect_err("evaluate_connect should surface persistence failure");

        assert!(!error.trim().is_empty());
        assert!(registry.list_requests(None, 10).is_empty());
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn pairing_registry_does_not_mutate_memory_when_approval_persistence_fails() {
        let request = ControlPlanePairingRequestRecord {
            pairing_request_id: "pair-1".to_owned(),
            device_id: "device-1".to_owned(),
            client_id: "cli".to_owned(),
            public_key: "pk-1".to_owned(),
            role: "operator".to_owned(),
            requested_scopes: BTreeSet::from(["operator.read".to_owned()]),
            status: ControlPlanePairingStatus::Pending,
            requested_at_ms: 1,
            resolved_at_ms: None,
            issued_token_id: None,
            device_token: None,
        };
        let registry =
            broken_pairing_registry_with_request("approve-persist-failure", request.clone());

        let error = registry
            .resolve_request("pair-1", true)
            .expect_err("resolve_request should surface persistence failure");

        assert!(!error.trim().is_empty());

        let requests = registry.list_requests(None, 10);
        assert_eq!(requests, vec![request]);
        assert!(
            registry
                .approved_devices
                .read()
                .unwrap_or_else(|lock_error| lock_error.into_inner())
                .is_empty()
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn pairing_registry_does_not_mutate_memory_when_rejection_persistence_fails() {
        let request = ControlPlanePairingRequestRecord {
            pairing_request_id: "pair-1".to_owned(),
            device_id: "device-1".to_owned(),
            client_id: "cli".to_owned(),
            public_key: "pk-1".to_owned(),
            role: "operator".to_owned(),
            requested_scopes: BTreeSet::from(["operator.read".to_owned()]),
            status: ControlPlanePairingStatus::Pending,
            requested_at_ms: 1,
            resolved_at_ms: None,
            issued_token_id: None,
            device_token: None,
        };
        let registry =
            broken_pairing_registry_with_request("reject-persist-failure", request.clone());

        let error = registry
            .resolve_request("pair-1", false)
            .expect_err("resolve_request should surface persistence failure");

        assert!(!error.trim().is_empty());

        let requests = registry.list_requests(None, 10);
        assert_eq!(requests, vec![request]);
    }

    #[cfg(feature = "memory-sqlite")]
    fn isolated_memory_config(test_name: &str) -> SessionStoreConfig {
        let base = std::env::temp_dir().join(format!(
            "loong-control-plane-view-{test_name}-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&base);
        let db_path = base.join("memory.sqlite3");
        let _ = fs::remove_file(&db_path);
        SessionStoreConfig {
            sqlite_path: Some(db_path),
            runtime_config: None,
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn broken_memory_config(test_name: &str) -> SessionStoreConfig {
        let base = std::env::temp_dir().join(format!(
            "loong-control-plane-broken-{test_name}-{}",
            std::process::id()
        ));
        let sqlite_path = base.join("sqlite-dir");
        let _ = fs::create_dir_all(&sqlite_path);
        SessionStoreConfig {
            sqlite_path: Some(sqlite_path),
            runtime_config: None,
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn broken_pairing_registry(test_name: &str) -> ControlPlanePairingRegistry {
        ControlPlanePairingRegistry {
            nonce: AtomicU64::new(0),
            requests: RwLock::new(BTreeMap::new()),
            approved_devices: RwLock::new(BTreeMap::new()),
            memory_config: Some(broken_memory_config(test_name)),
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn broken_pairing_registry_with_request(
        test_name: &str,
        request: ControlPlanePairingRequestRecord,
    ) -> ControlPlanePairingRegistry {
        let mut requests = BTreeMap::new();
        requests.insert(request.pairing_request_id.clone(), request);
        ControlPlanePairingRegistry {
            nonce: AtomicU64::new(0),
            requests: RwLock::new(requests),
            approved_devices: RwLock::new(BTreeMap::new()),
            memory_config: Some(broken_memory_config(test_name)),
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn seeded_repository_view(test_name: &str) -> ControlPlaneRepositoryView {
        let config = isolated_memory_config(test_name);
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root session");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create visible child session");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: serde_json::json!({
                "task": "research control plane parity",
                "label": "Child",
                "execution": {
                    "mode": "async",
                    "depth": 1,
                    "max_depth": 3,
                    "active_children": 0,
                    "max_active_children": 2,
                    "timeout_seconds": 90,
                    "allow_shell_in_child": false,
                    "child_tool_allowlist": ["read"],
                    "workspace_root": "/tmp/loong/control-plane/child-session",
                    "kernel_bound": false,
                    "runtime_narrowing": {}
                }
            }),
        })
        .expect("append child session event");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("child-session".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-child".to_owned(),
                    owner_kind: "delegate_async".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Waiting,
                    intent_summary: Some("research control plane parity".to_owned()),
                    verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 100,
                },
            ),
        })
        .expect("append child task progress event");
        repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: "apr-visible".to_owned(),
            session_id: "child-session".to_owned(),
            turn_id: "turn-visible".to_owned(),
            tool_call_id: "call-visible".to_owned(),
            tool_name: "delegate".to_owned(),
            approval_key: "tool:delegate".to_owned(),
            request_payload_json: serde_json::json!({
                "tool": "delegate",
            }),
            governance_snapshot_json: serde_json::json!({
                "reason": "governed_tool_requires_approval",
                "rule_id": "approval-visible",
            }),
        })
        .expect("create visible approval request");
        repo.upsert_session_tool_policy(crate::session::repository::NewSessionToolPolicyRecord {
            session_id: "child-session".to_owned(),
            requested_tool_ids: vec!["read".to_owned()],
            runtime_narrowing: crate::tools::runtime_config::ToolRuntimeNarrowing::default(),
        })
        .expect("create visible tool policy");

        repo.create_session(NewSessionRecord {
            session_id: "hidden-root".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Hidden".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create hidden root session");
        repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: "apr-hidden".to_owned(),
            session_id: "hidden-root".to_owned(),
            turn_id: "turn-hidden".to_owned(),
            tool_call_id: "call-hidden".to_owned(),
            tool_name: "delegate_async".to_owned(),
            approval_key: "tool:delegate_async".to_owned(),
            request_payload_json: serde_json::json!({
                "tool": "delegate_async",
            }),
            governance_snapshot_json: serde_json::json!({
                "reason": "governed_tool_requires_approval",
                "rule_id": "approval-hidden",
            }),
        })
        .expect("create hidden approval request");

        ControlPlaneRepositoryView::new(config, ToolConfig::default(), "root-session")
    }

    #[cfg(feature = "memory-sqlite")]
    fn seeded_acp_view(test_name: &str) -> ControlPlaneAcpView {
        let memory_config = isolated_memory_config(test_name);
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root session");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create visible child session");
        repo.create_session(NewSessionRecord {
            session_id: "hidden-root".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Hidden".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create hidden root session");

        let mut config = LoongConfig::default();
        let sqlite_path = memory_config
            .sqlite_path
            .as_ref()
            .expect("sqlite path")
            .display()
            .to_string();
        config.memory.sqlite_path = sqlite_path;
        config.acp.enabled = true;

        let store = AcpSqliteSessionStore::new(Some(config.memory.resolved_sqlite_path()));
        store
            .upsert(AcpSessionMetadata {
                session_key: "agent:codex:child-session".to_owned(),
                conversation_id: Some("conversation-visible".to_owned()),
                binding: Some(AcpSessionBindingScope {
                    route_session_id: "child-session".to_owned(),
                    channel_id: Some("feishu".to_owned()),
                    account_id: Some("lark-prod".to_owned()),
                    conversation_id: Some("oc_visible".to_owned()),
                    participant_id: None,
                    thread_id: Some("thread-visible".to_owned()),
                }),
                activation_origin: Some(AcpRoutingOrigin::ExplicitRequest),
                backend_id: "acpx".to_owned(),
                runtime_session_name: "runtime-visible".to_owned(),
                working_directory: None,
                backend_session_id: Some("backend-visible".to_owned()),
                agent_session_id: Some("agent-visible".to_owned()),
                mode: Some(AcpSessionMode::Interactive),
                state: AcpSessionState::Ready,
                last_activity_ms: 100,
                last_error: None,
            })
            .expect("seed visible ACP session");
        store
            .upsert(AcpSessionMetadata {
                session_key: "agent:codex:hidden-root".to_owned(),
                conversation_id: Some("conversation-hidden".to_owned()),
                binding: Some(AcpSessionBindingScope {
                    route_session_id: "hidden-root".to_owned(),
                    channel_id: Some("telegram".to_owned()),
                    account_id: None,
                    conversation_id: Some("hidden".to_owned()),
                    participant_id: None,
                    thread_id: None,
                }),
                activation_origin: Some(AcpRoutingOrigin::AutomaticDispatch),
                backend_id: "acpx".to_owned(),
                runtime_session_name: "runtime-hidden".to_owned(),
                working_directory: None,
                backend_session_id: Some("backend-hidden".to_owned()),
                agent_session_id: Some("agent-hidden".to_owned()),
                mode: Some(AcpSessionMode::Review),
                state: AcpSessionState::Busy,
                last_activity_ms: 200,
                last_error: Some("hidden".to_owned()),
            })
            .expect("seed hidden ACP session");

        ControlPlaneAcpView::new(config, "root-session")
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn repository_view_lists_visible_sessions_and_snapshot_counts() {
        let view = seeded_repository_view("session-list");
        let snapshot = view.snapshot_summary().expect("snapshot summary");
        assert_eq!(snapshot.current_session_id, "root-session");
        assert_eq!(snapshot.session_count, 2);
        assert_eq!(snapshot.pending_approval_count, 1);
        assert_eq!(snapshot.acp_session_count, 0);

        let sessions = view.list_sessions(false, 50).expect("visible session list");
        assert_eq!(sessions.current_session_id, "root-session");
        assert_eq!(sessions.matched_count, 2);
        assert_eq!(sessions.returned_count, 2);
        assert!(
            sessions
                .sessions
                .iter()
                .any(|session| session.session.session_id == "root-session")
        );
        let child = sessions
            .sessions
            .iter()
            .find(|session| session.session.session_id == "child-session")
            .expect("child session view");
        assert_eq!(child.workflow.workflow_id, "root-session");
        assert_eq!(
            child.workflow.task.as_deref(),
            Some("research control plane parity")
        );
        assert_eq!(child.workflow.phase.as_deref(), Some("execute"));
        assert_eq!(
            child
                .workflow
                .binding
                .as_ref()
                .expect("workflow binding")
                .mode,
            "advisory_only"
        );
        assert!(
            !sessions
                .sessions
                .iter()
                .any(|session| session.session.session_id == "hidden-root")
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn repository_view_reads_visible_session_and_filters_hidden_approvals() {
        let view = seeded_repository_view("session-read");
        let observation = view
            .read_session("child-session", 20, None, 50)
            .expect("visible session read")
            .expect("visible session observation");
        assert_eq!(observation.session.session.session_id, "child-session");
        assert_eq!(observation.session.workflow.workflow_id, "root-session");
        assert_eq!(
            observation.session.workflow.task.as_deref(),
            Some("research control plane parity")
        );
        assert_eq!(
            observation.session.workflow.phase.as_deref(),
            Some("execute")
        );
        assert_eq!(
            observation
                .session
                .workflow
                .binding
                .as_ref()
                .expect("workflow binding")
                .execution_surface,
            "delegate.async"
        );
        assert_eq!(observation.recent_events.len(), 2);
        let recent_event_kinds = observation
            .recent_events
            .iter()
            .map(|event| event.event_kind.as_str())
            .collect::<Vec<_>>();
        assert!(recent_event_kinds.contains(&"delegate_started"));
        assert!(recent_event_kinds.contains(&crate::task_progress::TASK_PROGRESS_EVENT_KIND));

        let approvals = view
            .list_approvals(None, Some(ApprovalRequestStatus::Pending), 50)
            .expect("approval list");
        assert_eq!(approvals.current_session_id, "root-session");
        assert_eq!(approvals.matched_count, 1);
        assert_eq!(approvals.returned_count, 1);
        assert_eq!(approvals.approvals[0].approval_request_id, "apr-visible");

        let error = view
            .read_session("hidden-root", 20, None, 50)
            .expect_err("hidden session should be rejected");
        assert!(error.contains("visibility_denied"));
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn repository_view_lists_visible_background_tasks_with_workflow_metadata() {
        let view = seeded_repository_view("task-list");
        let tasks = view
            .list_background_tasks(false, 50)
            .expect("background task list");

        assert_eq!(tasks.current_session_id, "root-session");
        assert_eq!(tasks.matched_count, 1);
        assert_eq!(tasks.returned_count, 1);

        let task = tasks.tasks.first().expect("first background task");
        assert_eq!(task.task_id, "task-child");
        assert_eq!(task.task_session_id, "child-session");
        assert_eq!(task.owner_session_id, "child-session");
        assert_eq!(task.workflow.workflow_id, "root-session");
        assert_eq!(
            task.workflow.task.as_deref(),
            Some("research control plane parity")
        );
        assert_eq!(task.workflow.phase.as_deref(), Some("execute"));
        let binding = task.workflow.binding.as_ref().expect("workflow binding");
        assert_eq!(binding.session_id, "child-session");
        assert_eq!(binding.task_id, "task-child");
        assert_eq!(binding.task_session_id, "child-session");
        assert_eq!(binding.mode, "advisory_only");
        assert_eq!(task.delegate_mode.as_deref(), Some("async"));
        assert_eq!(task.delegate_phase.as_deref(), Some("running"));
        assert_eq!(task.approval_request_count, 1);
        assert_eq!(task.approval_attention_count, 1);
        assert_eq!(task.requested_tool_ids, vec!["read".to_owned()]);
        assert_eq!(task.visible_requested_tool_ids, vec!["read".to_owned()]);
        assert_eq!(task.effective_tool_ids, vec!["read".to_owned()]);
        assert_eq!(task.visible_effective_tool_ids, vec!["read".to_owned()]);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn repository_view_reads_visible_background_task_detail() {
        let view = seeded_repository_view("task-read");
        let task = view
            .read_background_task("task-child")
            .expect("background task read")
            .expect("background task detail");

        assert_eq!(task.task_id, "task-child");
        assert_eq!(task.task_session_id, "child-session");
        assert_eq!(task.owner_session_id, "child-session");
        assert_eq!(task.workflow.workflow_id, "root-session");
        assert_eq!(
            task.workflow
                .binding
                .as_ref()
                .expect("workflow binding")
                .task_session_id,
            "child-session"
        );
        assert_eq!(
            task.workflow
                .binding
                .as_ref()
                .expect("workflow binding")
                .worktree
                .as_ref()
                .expect("worktree binding")
                .workspace_root,
            "/tmp/loong/control-plane/child-session"
        );
        assert_eq!(task.delegate_mode.as_deref(), Some("async"));
        assert_eq!(task.session_state, "running");
        assert_eq!(task.approval_request_count, 1);

        let legacy_alias = view
            .read_background_task("child-session")
            .expect("background task legacy alias")
            .expect("background task legacy detail");
        assert_eq!(legacy_alias.task_id, "task-child");
        assert_eq!(legacy_alias.task_session_id, "child-session");
        assert_eq!(legacy_alias.owner_session_id, "child-session");
        assert_eq!(legacy_alias.session_id, "child-session");

        let hidden_error = view
            .read_background_task("hidden-root")
            .expect_err("hidden session should be rejected");
        assert!(hidden_error.contains("visibility_denied"));
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn repository_view_deduplicates_background_tasks_by_canonical_task_id() {
        let config = isolated_memory_config("task-deduplicate-control-plane");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");

        for session_id in ["child-old", "child-new"] {
            repo.create_session(NewSessionRecord {
                session_id: session_id.to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some(session_id.to_owned()),
                state: SessionState::Running,
            })
            .expect("create child");
            repo.append_event(NewSessionEvent {
                session_id: session_id.to_owned(),
                event_kind: "delegate_started".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                payload_json: serde_json::json!({
                    "task": "repair durable task identity",
                    "label": session_id,
                    "execution": {
                        "mode": "async",
                        "depth": 1,
                        "max_depth": 3,
                        "active_children": 0,
                        "max_active_children": 2,
                        "timeout_seconds": 90,
                        "allow_shell_in_child": false,
                        "child_tool_allowlist": ["read"],
                        "workspace_root": format!("/tmp/loong/control-plane/{session_id}"),
                        "kernel_bound": false,
                        "runtime_narrowing": {}
                    }
                }),
            })
            .expect("append delegate event");
        }

        for (session_id, updated_at) in [("child-old", 10), ("child-new", 20)] {
            repo.append_event(NewSessionEvent {
                session_id: session_id.to_owned(),
                event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
                actor_session_id: Some(session_id.to_owned()),
                payload_json: crate::task_progress::task_progress_event_payload(
                    "unit_test",
                    &crate::task_progress::TaskProgressRecord {
                        task_id: "task-shared".to_owned(),
                        owner_kind: "delegate_async".to_owned(),
                        status: crate::task_progress::TaskProgressStatus::Waiting,
                        intent_summary: Some(format!("owner {session_id}")),
                        verification_state: Some(
                            crate::task_progress::TaskVerificationState::Pending,
                        ),
                        active_handles: Vec::new(),
                        resume_recipe: None,
                        updated_at,
                    },
                ),
            })
            .expect("append task progress");
        }

        let view = ControlPlaneRepositoryView::new(config, ToolConfig::default(), "root-session");
        let tasks = view
            .list_background_tasks(false, 50)
            .expect("background task list");
        assert_eq!(tasks.matched_count, 1);
        assert_eq!(tasks.tasks[0].task_id, "task-shared");
        assert_eq!(tasks.tasks[0].session_id, "child-new");

        let task = view
            .read_background_task("task-shared")
            .expect("background task read")
            .expect("background task detail");
        assert_eq!(task.task_id, "task-shared");
        assert_eq!(task.session_id, "child-new");
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn acp_view_lists_visible_sessions_and_counts_snapshot() {
        let view = seeded_acp_view("acp-list");
        assert_eq!(view.current_session_id(), "root-session");
        let count = view
            .visible_session_count()
            .await
            .expect("visible ACP session count");
        assert_eq!(count, 1);

        let sessions = view.list_sessions(50).expect("visible ACP session list");
        assert_eq!(sessions.current_session_id, "root-session");
        assert_eq!(sessions.matched_count, 1);
        assert_eq!(sessions.returned_count, 1);
        assert_eq!(
            sessions.sessions[0].session_key,
            "agent:codex:child-session"
        );
        assert_eq!(
            sessions.sessions[0]
                .binding
                .as_ref()
                .expect("binding")
                .route_session_id,
            "child-session"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn acp_view_visibility_ignores_memory_env_overrides() {
        let mut env = ScopedEnv::new();
        env.set("LOONG_SQLITE_PATH", "/tmp/env-visibility-memory.sqlite3");
        env.set("LOONG_MEMORY_PROFILE", "profile_plus_window");

        let view = seeded_acp_view("acp-list-ignore-env");
        let count = view
            .visible_session_count()
            .await
            .expect("visible ACP session count");
        assert_eq!(count, 1);

        let sessions = view.list_sessions(50).expect("visible ACP session list");
        assert_eq!(sessions.matched_count, 1);
        assert_eq!(
            sessions.sessions[0].session_key,
            "agent:codex:child-session"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn acp_view_reads_visible_session_status_and_filters_hidden_sessions() {
        let view = seeded_acp_view("acp-read");
        let visible = view
            .read_session("agent:codex:child-session")
            .await
            .expect("ACP session read")
            .expect("visible ACP session");
        assert_eq!(visible.current_session_id, "root-session");
        assert_eq!(visible.metadata.session_key, "agent:codex:child-session");
        assert_eq!(visible.status.session_key, "agent:codex:child-session");
        assert_eq!(visible.status.state, AcpSessionState::Ready);
        assert_eq!(visible.status.mode, Some(AcpSessionMode::Interactive));
        assert!(
            visible
                .status
                .last_error
                .as_deref()
                .is_some_and(|error| error.starts_with("status_unavailable:")),
            "expected ACP read fallback to surface status_unavailable"
        );

        let error = view
            .read_session("agent:codex:hidden-root")
            .await
            .expect_err("hidden ACP session should be rejected");
        assert!(error.contains("visibility_denied"));
    }
