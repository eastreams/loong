use super::*;

mod render_tests {
    use super::*;

    #[allow(dead_code)]
    fn sample_grant_summary(selected: bool, effective_selected: bool) -> Value {
        json!({
            "selected": selected,
            "effective_selected": effective_selected,
            "principal": {
                "open_id": "ou_123",
                "name": "Alice"
            },
            "status": {
                "access_token_expired": false,
                "refresh_token_expired": false,
                "missing_scopes": ["docx:document:readonly"]
            },
            "message_write_status": {
                "ready": false,
                "matched_scopes": []
            },
            "recommendations": {
                "auth_start_command": "loong feishu auth start --account feishu_main --capability message-write"
            }
        })
    }

    #[test]
    fn render_auth_list_text_includes_stale_selection_and_select_hint() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "selected_open_id": Value::Null,
            "grants": [sample_grant_summary(false, false)],
            "recommendations": {
                "select_command": "loong feishu auth select --account feishu_main --open-id <open_id>",
                "stale_selected_open_id": "ou_missing"
            }
        });

        let rendered = render_auth_list_text(&payload).expect("render auth list");

        assert!(rendered.contains("configured_account: work"));
        assert!(rendered.contains(
            "select_hint: loong feishu auth select --account feishu_main --open-id <open_id>"
        ));
        assert!(rendered.contains("stale_selected_open_id: ou_missing"));
        assert!(rendered.contains("missing_scopes: docx:document:readonly"));
    }

    #[test]
    fn render_auth_select_text_includes_selected_grant_summary() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "selected_open_id": "ou_123",
            "grant": sample_grant_summary(true, true)
        });

        let rendered = render_auth_select_text(&payload).expect("render auth select");

        assert!(rendered.contains("configured_account: work"));
        assert!(rendered.contains("selected_open_id: ou_123"));
        assert!(
            rendered.contains(
                "open_id: ou_123 | selected: true | effective_selected: true | name: Alice"
            )
        );
    }

    #[test]
    fn render_auth_list_text_distinguishes_persisted_and_effective_selection() {
        let payload = json!({
            "account_id": "feishu_main",
            "selected_open_id": Value::Null,
            "effective_open_id": "ou_123",
            "grants": [sample_grant_summary(false, true)],
            "recommendations": {}
        });

        let rendered = render_auth_list_text(&payload).expect("render auth list");

        assert!(rendered.contains(
            "open_id: ou_123 | selected: false | effective_selected: true | name: Alice"
        ));
    }

    #[test]
    fn render_auth_start_text_includes_configured_account_when_present() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "state": "state_123",
            "redirect_uri": "http://127.0.0.1:34819/callback",
            "authorize_url": "https://open.feishu.cn/open-apis/authen/v1/authorize",
            "sqlite_path": "/tmp/feishu.sqlite3",
            "capabilities": ["read-only"],
            "scopes": ["offline_access", "docx:document:readonly"],
        });

        let rendered = render_auth_start_text(&payload).expect("render auth start");

        assert!(rendered.contains("configured_account: work"));
    }

    #[test]
    fn render_onboard_text_includes_qr_registration_summary() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "config": "/tmp/loong.toml",
            "credential_source": "qr_registration",
            "domain": "lark",
            "mode": "websocket",
            "owner_open_id": "ou_owner_1",
            "bot_name": "Loong Bot",
            "bot_open_id": "ou_bot_1",
            "qr_url": "https://scan.example/activate",
            "owner_direct_chat_bootstrap_applied": true,
            "serve_command": "loong feishu serve --account work",
            "status_command": "loong doctor",
            "notes": ["defaulted inbound bootstrap access to `allowed_chat_ids = [\"*\"]` and `allowed_sender_ids = [\"ou_owner_1\"]` so the onboarding user can start a direct Feishu/Lark chat immediately"],
        });

        let rendered = render_onboard_text(&payload).expect("render onboard");

        assert!(rendered.contains("feishu onboard"));
        assert!(rendered.contains("configured_account: work"));
        assert!(rendered.contains("credential_source: qr_registration"));
        assert!(rendered.contains("bot_name: Loong Bot"));
        assert!(rendered.contains("allowed_sender_ids = [\"ou_owner_1\"]"));
        assert!(rendered.contains("serve_command: loong feishu serve --account work"));
    }

    #[test]
    fn render_auth_exchange_text_includes_selected_and_effective_open_ids() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "principal": {
                "open_id": "ou_123",
                "name": "Alice"
            },
            "granted_scopes": ["offline_access", "docx:document:readonly"],
            "selected_open_id": "ou_123",
            "effective_open_id": "ou_123",
        });

        let rendered = render_auth_exchange_text(&payload).expect("render auth exchange");

        assert!(rendered.contains("configured_account: work"));
        assert!(rendered.contains("selected_open_id: ou_123"));
        assert!(rendered.contains("effective_open_id: ou_123"));
    }

    #[test]
    fn render_auth_revoke_text_includes_remaining_grant_state_and_hints() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "open_id": "ou_789",
            "deleted": true,
            "grant_count": 2,
            "selected_open_id": Value::Null,
            "effective_open_id": Value::Null,
            "recommendations": {
                "select_command": "loong feishu auth select --account feishu_main --open-id <open_id>"
            }
        });

        let rendered = render_auth_revoke_text(&payload).expect("render auth revoke");

        assert!(rendered.contains("configured_account: work"));
        assert!(rendered.contains("grant_count: 2"));
        assert!(rendered.contains("selected_open_id: -"));
        assert!(rendered.contains("effective_open_id: -"));
        assert!(rendered.contains(
            "select_hint: loong feishu auth select --account feishu_main --open-id <open_id>"
        ));
    }

    #[test]
    fn render_auth_status_text_account_scope_includes_missing_scopes_and_select_hint() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "status_scope": "account",
            "grant_count": 2,
            "selected_open_id": Value::Null,
            "grants": [sample_grant_summary(false, false)],
            "recommendations": {
                "select_command": "loong feishu auth select --account feishu_main --open-id <open_id>",
                "stale_selected_open_id": "ou_missing"
            }
        });

        let rendered = render_auth_status_text(&payload).expect("render auth status");

        assert!(rendered.contains("configured_account: work"));
        assert!(rendered.contains("status_scope: account"));
        assert!(rendered.contains(
            "select_hint: loong feishu auth select --account feishu_main --open-id <open_id>"
        ));
        assert!(rendered.contains("stale_selected_open_id: ou_missing"));
        assert!(rendered.contains("missing_scopes: docx:document:readonly"));
    }

    #[test]
    fn render_auth_status_text_grant_scope_includes_requested_open_id_and_available_options() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "status_scope": "grant",
            "status": {
                "has_grant": false,
                "access_token_expired": false,
                "refresh_token_expired": false,
                "missing_scopes": []
            },
            "message_write_status": {
                "ready": false,
                "matched_scopes": []
            },
            "recommendations": {
                "auth_start_command": Value::Null,
                "select_command": "loong feishu auth select --account feishu_main --open-id <open_id>"
            },
            "selected_open_id": Value::Null,
            "effective_open_id": Value::Null,
            "requested_open_id": "ou_missing",
            "available_open_ids": ["ou_456", "ou_123"]
        });

        let rendered = render_auth_status_text(&payload).expect("render auth status");

        assert!(rendered.contains("configured_account: work"));
        assert!(rendered.contains("requested_open_id: ou_missing"));
        assert!(rendered.contains(
            "select_hint: loong feishu auth select --account feishu_main --open-id <open_id>"
        ));
        assert!(rendered.contains("available_open_ids: ou_456, ou_123"));
    }

    #[test]
    fn render_whoami_text_includes_configured_account_when_present() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "principal": {
                "open_id": "ou_123",
                "name": "Alice",
                "email": "alice@example.com",
                "tenant_key": "tenant_x"
            }
        });

        let rendered = render_whoami_text(&payload).expect("render whoami");

        assert!(rendered.contains("configured_account: work"));
    }

    #[test]
    fn render_read_doc_text_includes_configured_account_when_present() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "document": {
                "document_id": "doxcnDemo",
                "content": "hello"
            }
        });

        let rendered = render_read_doc_text(&payload).expect("render read doc");

        assert!(rendered.contains("configured_account: work"));
    }

    #[test]
    fn render_doc_create_text_includes_configured_account_when_present() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "document": {
                "document_id": "doxcnCreated",
                "url": "https://open.feishu.cn/docx/doxcnCreated"
            },
            "content_inserted": true,
            "inserted_block_count": 1,
            "insert_batch_count": 1,
            "content_type": "markdown"
        });

        let rendered = render_doc_create_text(&payload).expect("render doc create");

        assert!(rendered.contains("configured_account: work"));
        assert!(rendered.contains("content_inserted: true"));
        assert!(rendered.contains("insert_batch_count: 1"));
    }

    #[test]
    fn render_doc_append_text_includes_configured_account_when_present() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "document": {
                "document_id": "doxcnExisting",
                "url": "https://open.feishu.cn/docx/doxcnExisting"
            },
            "inserted_block_count": 1,
            "insert_batch_count": 1,
            "content_type": "markdown"
        });

        let rendered = render_doc_append_text(&payload).expect("render doc append");

        assert!(rendered.contains("configured_account: work"));
        assert!(rendered.contains("inserted_block_count: 1"));
        assert!(rendered.contains("insert_batch_count: 1"));
    }

    #[test]
    fn render_messages_history_text_includes_configured_account_when_present() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "page": {
                "items": [{"message_id": "om_1"}],
                "has_more": true,
                "page_token": "next-page"
            }
        });

        let rendered = render_messages_history_text(&payload).expect("render messages history");

        assert!(rendered.contains("configured_account: work"));
    }

    #[test]
    fn render_messages_get_text_includes_configured_account_when_present() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "message": {
                "message_id": "om_1"
            }
        });

        let rendered = render_messages_get_text(&payload).expect("render messages get");

        assert!(rendered.contains("configured_account: work"));
    }

    #[test]
    fn render_messages_resource_text_includes_configured_account_when_present() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "message_id": "om_resource_1",
            "file_key": "file_resource_1",
            "resource_type": "file",
            "path": "/tmp/spec-sheet.pdf",
            "bytes_written": 18,
            "content_type": "application/pdf",
            "file_name": "spec-sheet.pdf"
        });

        let rendered = render_messages_resource_text(&payload).expect("render messages resource");

        assert!(rendered.contains("configured_account: work"));
    }

    #[test]
    fn render_send_text_includes_configured_account_when_present() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "delivery": {
                "message_id": "om_1",
                "receive_id_type": "chat_id",
                "receive_id": "oc_1",
                "uuid": "uuid-1",
                "msg_type": "text"
            }
        });

        let rendered = render_send_text(&payload).expect("render send");

        assert!(rendered.contains("configured_account: work"));
    }

    #[test]
    fn render_reply_text_includes_configured_account_when_present() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "delivery": {
                "message_id": "om_2",
                "reply_to_message_id": "om_1",
                "reply_in_thread": true,
                "uuid": "uuid-2",
                "msg_type": "interactive"
            }
        });

        let rendered = render_reply_text(&payload).expect("render reply");

        assert!(rendered.contains("configured_account: work"));
    }

    #[test]
    fn render_search_messages_text_includes_configured_account_when_present() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "page": {
                "items": [{"message_id": "om_1"}],
                "has_more": false,
                "page_token": "page-1"
            }
        });

        let rendered = render_search_messages_text(&payload).expect("render search messages");

        assert!(rendered.contains("configured_account: work"));
    }

    #[test]
    fn render_calendar_list_text_includes_configured_account_when_present() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "primary": true,
            "calendars": {
                "calendars": [{
                    "calendar": {
                        "calendar_id": "cal_1"
                    }
                }]
            }
        });

        let rendered = render_calendar_list_text(&payload).expect("render calendar list");

        assert!(rendered.contains("configured_account: work"));
    }

    #[test]
    fn render_calendar_freebusy_text_includes_configured_account_when_present() {
        let payload = json!({
            "account_id": "feishu_main",
            "configured_account": "work",
            "result": {
                "freebusy_list": [{
                    "start_time": "2026-03-12T09:00:00+08:00",
                    "end_time": "2026-03-12T10:00:00+08:00"
                }]
            }
        });

        let rendered = render_calendar_freebusy_text(&payload).expect("render calendar freebusy");

        assert!(rendered.contains("configured_account: work"));
    }
}
