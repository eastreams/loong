use super::*;

impl SessionRepository {
    pub fn load_terminal_outcome(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionTerminalOutcomeRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::load_terminal_outcome_with_conn(&conn, &session_id)
    }

    pub fn ensure_approval_request(
        &self,
        record: NewApprovalRequestRecord,
    ) -> Result<ApprovalRequestRecord, String> {
        let approval_request_id =
            normalize_required_text(&record.approval_request_id, "approval_request_id")?;
        let session_id = normalize_required_text(&record.session_id, "session_id")?;
        let turn_id = normalize_required_text(&record.turn_id, "turn_id")?;
        let tool_call_id = normalize_required_text(&record.tool_call_id, "tool_call_id")?;
        let tool_name = normalize_required_text(&record.tool_name, "tool_name")?;
        let approval_key = normalize_required_text(&record.approval_key, "approval_key")?;
        if self.load_session(&session_id)?.is_none() {
            return Err(format!("session `{session_id}` not found"));
        }

        let encoded_request_payload = serde_json::to_string(&record.request_payload_json)
            .map_err(|error| format!("encode approval request payload failed: {error}"))?;
        let encoded_governance_snapshot =
            serde_json::to_string(&record.governance_snapshot_json)
                .map_err(|error| format!("encode approval governance snapshot failed: {error}"))?;
        let requested_at = unix_ts_now();
        let conn = self.open_connection()?;
        match conn.execute(
            "INSERT INTO approval_requests(
                approval_request_id,
                session_id,
                turn_id,
                tool_call_id,
                tool_name,
                approval_key,
                status,
                decision,
                request_payload_json,
                governance_snapshot_json,
                requested_at,
                resolved_at,
                resolved_by_session_id,
                executed_at,
                last_error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, ?10, NULL, NULL, NULL, NULL)",
            params![
                approval_request_id,
                session_id,
                turn_id,
                tool_call_id,
                tool_name,
                approval_key,
                ApprovalRequestStatus::Pending.as_str(),
                encoded_request_payload,
                encoded_governance_snapshot,
                requested_at,
            ],
        ) {
            Ok(_) => {}
            Err(error) if error.to_string().contains("UNIQUE constraint failed") => {
                return self
                    .load_approval_request(&approval_request_id)?
                    .ok_or_else(|| {
                        format!(
                            "approval request `{approval_request_id}` missing after concurrent insert"
                        )
                    });
            }
            Err(error) => return Err(format!("insert approval request row failed: {error}")),
        }

        self.load_approval_request(&approval_request_id)?
            .ok_or_else(|| {
                format!("approval request `{approval_request_id}` disappeared after insert")
            })
    }

    pub fn load_approval_request(
        &self,
        approval_request_id: &str,
    ) -> Result<Option<ApprovalRequestRecord>, String> {
        let approval_request_id =
            normalize_required_text(approval_request_id, "approval_request_id")?;
        let conn = self.open_connection()?;
        let raw = conn
            .query_row(
                "SELECT
                    approval_request_id,
                    session_id,
                    turn_id,
                    tool_call_id,
                    tool_name,
                    approval_key,
                    status,
                    decision,
                    request_payload_json,
                    governance_snapshot_json,
                    requested_at,
                    resolved_at,
                    resolved_by_session_id,
                    executed_at,
                    last_error
                 FROM approval_requests
                 WHERE approval_request_id = ?1",
                params![approval_request_id],
                |row| {
                    Ok(RawApprovalRequestRecord {
                        approval_request_id: row.get(0)?,
                        session_id: row.get(1)?,
                        turn_id: row.get(2)?,
                        tool_call_id: row.get(3)?,
                        tool_name: row.get(4)?,
                        approval_key: row.get(5)?,
                        status: row.get(6)?,
                        decision: row.get(7)?,
                        request_payload_json: row.get(8)?,
                        governance_snapshot_json: row.get(9)?,
                        requested_at: row.get(10)?,
                        resolved_at: row.get(11)?,
                        resolved_by_session_id: row.get(12)?,
                        executed_at: row.get(13)?,
                        last_error: row.get(14)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load approval request row failed: {error}"))?;
        raw.map(ApprovalRequestRecord::try_from_raw).transpose()
    }

    pub fn list_approval_requests_for_session(
        &self,
        session_id: &str,
        status: Option<ApprovalRequestStatus>,
    ) -> Result<Vec<ApprovalRequestRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::list_approval_requests_for_session_with_conn(&conn, &session_id, status)
    }

    pub(crate) fn list_approval_requests_for_session_with_conn(
        conn: &Connection,
        session_id: &str,
        status: Option<ApprovalRequestStatus>,
    ) -> Result<Vec<ApprovalRequestRecord>, String> {
        let mut requests = Vec::new();
        match status {
            Some(status) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT
                            approval_request_id,
                            session_id,
                            turn_id,
                            tool_call_id,
                            tool_name,
                            approval_key,
                            status,
                            decision,
                            request_payload_json,
                            governance_snapshot_json,
                            requested_at,
                            resolved_at,
                            resolved_by_session_id,
                            executed_at,
                            last_error
                         FROM approval_requests
                         WHERE session_id = ?1 AND status = ?2
                         ORDER BY requested_at DESC, approval_request_id ASC",
                    )
                    .map_err(|error| {
                        format!("prepare approval request list query failed: {error}")
                    })?;
                let rows = stmt
                    .query_map(params![session_id, status.as_str()], |row| {
                        Ok(RawApprovalRequestRecord {
                            approval_request_id: row.get(0)?,
                            session_id: row.get(1)?,
                            turn_id: row.get(2)?,
                            tool_call_id: row.get(3)?,
                            tool_name: row.get(4)?,
                            approval_key: row.get(5)?,
                            status: row.get(6)?,
                            decision: row.get(7)?,
                            request_payload_json: row.get(8)?,
                            governance_snapshot_json: row.get(9)?,
                            requested_at: row.get(10)?,
                            resolved_at: row.get(11)?,
                            resolved_by_session_id: row.get(12)?,
                            executed_at: row.get(13)?,
                            last_error: row.get(14)?,
                        })
                    })
                    .map_err(|error| format!("query approval request list failed: {error}"))?;
                for row in rows {
                    let raw = row
                        .map_err(|error| format!("decode approval request row failed: {error}"))?;
                    requests.push(ApprovalRequestRecord::try_from_raw(raw)?);
                }
            }
            None => {
                let mut stmt = conn
                    .prepare(
                        "SELECT
                            approval_request_id,
                            session_id,
                            turn_id,
                            tool_call_id,
                            tool_name,
                            approval_key,
                            status,
                            decision,
                            request_payload_json,
                            governance_snapshot_json,
                            requested_at,
                            resolved_at,
                            resolved_by_session_id,
                            executed_at,
                            last_error
                         FROM approval_requests
                         WHERE session_id = ?1
                         ORDER BY requested_at DESC, approval_request_id ASC",
                    )
                    .map_err(|error| {
                        format!("prepare approval request list query failed: {error}")
                    })?;
                let rows = stmt
                    .query_map(params![session_id], |row| {
                        Ok(RawApprovalRequestRecord {
                            approval_request_id: row.get(0)?,
                            session_id: row.get(1)?,
                            turn_id: row.get(2)?,
                            tool_call_id: row.get(3)?,
                            tool_name: row.get(4)?,
                            approval_key: row.get(5)?,
                            status: row.get(6)?,
                            decision: row.get(7)?,
                            request_payload_json: row.get(8)?,
                            governance_snapshot_json: row.get(9)?,
                            requested_at: row.get(10)?,
                            resolved_at: row.get(11)?,
                            resolved_by_session_id: row.get(12)?,
                            executed_at: row.get(13)?,
                            last_error: row.get(14)?,
                        })
                    })
                    .map_err(|error| format!("query approval request list failed: {error}"))?;
                for row in rows {
                    let raw = row
                        .map_err(|error| format!("decode approval request row failed: {error}"))?;
                    requests.push(ApprovalRequestRecord::try_from_raw(raw)?);
                }
            }
        }
        Ok(requests)
    }

    pub fn transition_approval_request_if_current(
        &self,
        approval_request_id: &str,
        request: TransitionApprovalRequestIfCurrentRequest,
    ) -> Result<Option<ApprovalRequestRecord>, String> {
        let approval_request_id =
            normalize_required_text(approval_request_id, "approval_request_id")?;
        let resolved_by_session_id = normalize_optional_text(request.resolved_by_session_id);
        let last_error = normalize_optional_text(request.last_error);
        let decision = request.decision.map(ApprovalDecision::as_str);
        let resolution_ts = matches!(
            request.next_status,
            ApprovalRequestStatus::Approved | ApprovalRequestStatus::Denied
        )
        .then(unix_ts_now);
        let conn = self.open_connection()?;
        let affected = conn
            .execute(
                "UPDATE approval_requests
                 SET status = ?3,
                     decision = CASE WHEN ?4 IS NULL THEN decision ELSE ?4 END,
                     resolved_at = CASE WHEN ?5 IS NULL THEN resolved_at ELSE ?5 END,
                     resolved_by_session_id = CASE WHEN ?6 IS NULL THEN resolved_by_session_id ELSE ?6 END,
                     executed_at = CASE WHEN ?7 IS NULL THEN executed_at ELSE ?7 END,
                     last_error = ?8
                 WHERE approval_request_id = ?1 AND status = ?2",
                params![
                    approval_request_id,
                    request.expected_status.as_str(),
                    request.next_status.as_str(),
                    decision,
                    resolution_ts,
                    resolved_by_session_id,
                    request.executed_at,
                    last_error,
                ],
            )
            .map_err(|error| format!("conditionally update approval request failed: {error}"))?;
        if affected == 0 {
            return Ok(None);
        }

        self.load_approval_request(&approval_request_id)?
            .map(Some)
            .ok_or_else(|| {
                format!("approval request `{approval_request_id}` missing after conditional update")
            })
    }

    pub fn upsert_approval_grant(
        &self,
        record: NewApprovalGrantRecord,
    ) -> Result<ApprovalGrantRecord, String> {
        let scope_session_id =
            normalize_required_text(&record.scope_session_id, "scope_session_id")?;
        let approval_key = normalize_required_text(&record.approval_key, "approval_key")?;
        let created_by_session_id = normalize_optional_text(record.created_by_session_id);
        let ts = unix_ts_now();
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT INTO approval_grants(
                scope_session_id,
                approval_key,
                created_by_session_id,
                created_at,
                updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(scope_session_id, approval_key) DO UPDATE SET
                created_by_session_id = COALESCE(excluded.created_by_session_id, approval_grants.created_by_session_id),
                updated_at = excluded.updated_at",
            params![scope_session_id, approval_key, created_by_session_id, ts, ts],
        )
        .map_err(|error| format!("upsert approval grant failed: {error}"))?;

        self.load_approval_grant(&scope_session_id, &approval_key)?
            .ok_or_else(|| {
                format!(
                    "approval grant `{}:{}` disappeared after upsert",
                    scope_session_id, approval_key
                )
            })
    }

    pub fn load_approval_grant(
        &self,
        scope_session_id: &str,
        approval_key: &str,
    ) -> Result<Option<ApprovalGrantRecord>, String> {
        let scope_session_id = normalize_required_text(scope_session_id, "scope_session_id")?;
        let approval_key = normalize_required_text(approval_key, "approval_key")?;
        let conn = self.open_connection()?;
        let raw = conn
            .query_row(
                "SELECT scope_session_id, approval_key, created_by_session_id, created_at, updated_at
                 FROM approval_grants
                 WHERE scope_session_id = ?1 AND approval_key = ?2",
                params![scope_session_id, approval_key],
                |row| {
                    Ok(RawApprovalGrantRecord {
                        scope_session_id: row.get(0)?,
                        approval_key: row.get(1)?,
                        created_by_session_id: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load approval grant failed: {error}"))?;
        raw.map(ApprovalGrantRecord::try_from_raw).transpose()
    }

    pub fn upsert_session_tool_consent(
        &self,
        record: NewSessionToolConsentRecord,
    ) -> Result<SessionToolConsentRecord, String> {
        let requested_scope_session_id =
            normalize_required_text(&record.scope_session_id, "scope_session_id")?;
        let scope_session_id = self
            .lineage_root_session_id(&requested_scope_session_id)?
            .ok_or_else(|| format!("session `{requested_scope_session_id}` not found"))?;
        let session_exists = self
            .load_session_summary_with_legacy_fallback(&scope_session_id)?
            .is_some();
        if !session_exists {
            return Err(format!("session `{scope_session_id}` not found"));
        }
        let updated_by_session_id = normalize_optional_text(record.updated_by_session_id);
        let ts = unix_ts_now();
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT INTO session_tool_consent(
                scope_session_id,
                mode,
                updated_by_session_id,
                created_at,
                updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(scope_session_id) DO UPDATE SET
                mode = excluded.mode,
                updated_by_session_id = excluded.updated_by_session_id,
                updated_at = excluded.updated_at",
            params![
                scope_session_id,
                record.mode.as_str(),
                updated_by_session_id,
                ts,
                ts
            ],
        )
        .map_err(|error| format!("upsert session tool consent failed: {error}"))?;

        self.load_session_tool_consent(&scope_session_id)?
            .ok_or_else(|| {
                format!("session tool consent `{scope_session_id}` disappeared after upsert")
            })
    }

    pub fn load_session_tool_consent(
        &self,
        scope_session_id: &str,
    ) -> Result<Option<SessionToolConsentRecord>, String> {
        let requested_scope_session_id =
            normalize_required_text(scope_session_id, "scope_session_id")?;
        let scope_session_id = match self.lineage_root_session_id(&requested_scope_session_id)? {
            Some(root_scope_session_id) => root_scope_session_id,
            None => return Ok(None),
        };
        let conn = self.open_connection()?;
        let raw = conn
            .query_row(
                "SELECT scope_session_id, mode, updated_by_session_id, created_at, updated_at
                 FROM session_tool_consent
                 WHERE scope_session_id = ?1",
                params![scope_session_id],
                |row| {
                    Ok(RawSessionToolConsentRecord {
                        scope_session_id: row.get(0)?,
                        mode: row.get(1)?,
                        updated_by_session_id: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load session tool consent failed: {error}"))?;
        raw.map(SessionToolConsentRecord::try_from_raw).transpose()
    }

    pub fn upsert_session_tool_policy(
        &self,
        record: NewSessionToolPolicyRecord,
    ) -> Result<SessionToolPolicyRecord, String> {
        let session_id = normalize_required_text(&record.session_id, "session_id")?;
        let session_exists = self
            .load_session_summary_with_legacy_fallback(&session_id)?
            .is_some();
        if !session_exists {
            return Err(format!("session `{session_id}` not found"));
        }

        let requested_tool_ids = normalize_tool_id_list(record.requested_tool_ids);
        let encoded_requested_tool_ids = serde_json::to_string(&requested_tool_ids)
            .map_err(|error| format!("encode session tool policy tool ids failed: {error}"))?;
        let encoded_runtime_narrowing = serde_json::to_string(&record.runtime_narrowing)
            .map_err(|error| format!("encode session tool policy narrowing failed: {error}"))?;
        let updated_at = unix_ts_now();
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT INTO session_tool_policies(
                session_id,
                requested_tool_ids_json,
                runtime_narrowing_json,
                updated_at
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET
                requested_tool_ids_json = excluded.requested_tool_ids_json,
                runtime_narrowing_json = excluded.runtime_narrowing_json,
                updated_at = excluded.updated_at",
            params![
                session_id,
                encoded_requested_tool_ids,
                encoded_runtime_narrowing,
                updated_at,
            ],
        )
        .map_err(|error| format!("upsert session tool policy failed: {error}"))?;

        self.load_session_tool_policy(&record.session_id)?
            .ok_or_else(|| {
                format!(
                    "session tool policy `{}` disappeared after upsert",
                    record.session_id
                )
            })
    }

    pub fn load_session_tool_policy(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionToolPolicyRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        let raw = conn
            .query_row(
                "SELECT
                    session_id,
                    requested_tool_ids_json,
                    runtime_narrowing_json,
                    updated_at
                 FROM session_tool_policies
                 WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok(RawSessionToolPolicyRecord {
                        session_id: row.get(0)?,
                        requested_tool_ids_json: row.get(1)?,
                        runtime_narrowing_json: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load session tool policy failed: {error}"))?;
        raw.map(SessionToolPolicyRecord::try_from_raw).transpose()
    }

    pub fn delete_session_tool_policy(&self, session_id: &str) -> Result<bool, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        let affected = conn
            .execute(
                "DELETE FROM session_tool_policies
                 WHERE session_id = ?1",
                params![session_id],
            )
            .map_err(|error| format!("delete session tool policy failed: {error}"))?;
        Ok(affected > 0)
    }

    pub fn ensure_control_plane_pairing_request(
        &self,
        record: NewControlPlanePairingRequestRecord,
    ) -> Result<ControlPlanePairingRequestRecord, String> {
        let pairing_request_id =
            normalize_required_text(&record.pairing_request_id, "pairing_request_id")?;
        let device_id = normalize_required_text(&record.device_id, "device_id")?;
        let client_id = normalize_required_text(&record.client_id, "client_id")?;
        let public_key = normalize_required_text(&record.public_key, "public_key")?;
        let role = normalize_required_text(&record.role, "role")?;
        let requested_scopes_json = encode_string_set_json(&record.requested_scopes)?;
        let requested_at_ms = unix_time_ms_now();
        let conn = self.open_connection()?;
        match conn.execute(
            "INSERT INTO control_plane_pairing_requests(
                pairing_request_id,
                device_id,
                client_id,
                public_key,
                role,
                requested_scopes_json,
                status,
                requested_at_ms,
                resolved_at_ms,
                issued_token_id,
                last_error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, NULL)",
            params![
                &pairing_request_id,
                device_id,
                client_id,
                public_key,
                role,
                requested_scopes_json,
                ControlPlanePairingRequestStatus::Pending.as_str(),
                requested_at_ms,
            ],
        ) {
            Ok(_) => {}
            Err(error) if error.to_string().contains("UNIQUE constraint failed") => {
                return self
                    .load_control_plane_pairing_request(&pairing_request_id)?
                    .ok_or_else(|| {
                        format!(
                            "control-plane pairing request `{pairing_request_id}` missing after concurrent insert"
                        )
                    });
            }
            Err(error) => {
                return Err(format!(
                    "insert control-plane pairing request row failed: {error}"
                ));
            }
        }

        self.load_control_plane_pairing_request(&pairing_request_id)?
            .ok_or_else(|| {
                format!(
                    "control-plane pairing request `{pairing_request_id}` disappeared after insert"
                )
            })
    }

    pub fn load_control_plane_pairing_request(
        &self,
        pairing_request_id: &str,
    ) -> Result<Option<ControlPlanePairingRequestRecord>, String> {
        let pairing_request_id = normalize_required_text(pairing_request_id, "pairing_request_id")?;
        let conn = self.open_connection()?;
        let raw = conn
            .query_row(
                "SELECT
                    pairing_request_id,
                    device_id,
                    client_id,
                    public_key,
                    role,
                    requested_scopes_json,
                    status,
                    requested_at_ms,
                    resolved_at_ms,
                    issued_token_id,
                    last_error
                 FROM control_plane_pairing_requests
                 WHERE pairing_request_id = ?1",
                params![pairing_request_id],
                |row| {
                    Ok(RawControlPlanePairingRequestRecord {
                        pairing_request_id: row.get(0)?,
                        device_id: row.get(1)?,
                        client_id: row.get(2)?,
                        public_key: row.get(3)?,
                        role: row.get(4)?,
                        requested_scopes_json: row.get(5)?,
                        status: row.get(6)?,
                        requested_at_ms: row.get(7)?,
                        resolved_at_ms: row.get(8)?,
                        issued_token_id: row.get(9)?,
                        last_error: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load control-plane pairing request row failed: {error}"))?;
        raw.map(ControlPlanePairingRequestRecord::try_from_raw)
            .transpose()
    }

    pub fn list_control_plane_pairing_requests(
        &self,
        status: Option<ControlPlanePairingRequestStatus>,
    ) -> Result<Vec<ControlPlanePairingRequestRecord>, String> {
        let conn = self.open_connection()?;
        let mut requests = Vec::new();
        match status {
            Some(status) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT
                            pairing_request_id,
                            device_id,
                            client_id,
                            public_key,
                            role,
                            requested_scopes_json,
                            status,
                            requested_at_ms,
                            resolved_at_ms,
                            issued_token_id,
                            last_error
                         FROM control_plane_pairing_requests
                         WHERE status = ?1
                         ORDER BY requested_at_ms DESC, pairing_request_id ASC",
                    )
                    .map_err(|error| {
                        format!("prepare control-plane pairing request list query failed: {error}")
                    })?;
                let rows = stmt
                    .query_map(params![status.as_str()], |row| {
                        Ok(RawControlPlanePairingRequestRecord {
                            pairing_request_id: row.get(0)?,
                            device_id: row.get(1)?,
                            client_id: row.get(2)?,
                            public_key: row.get(3)?,
                            role: row.get(4)?,
                            requested_scopes_json: row.get(5)?,
                            status: row.get(6)?,
                            requested_at_ms: row.get(7)?,
                            resolved_at_ms: row.get(8)?,
                            issued_token_id: row.get(9)?,
                            last_error: row.get(10)?,
                        })
                    })
                    .map_err(|error| {
                        format!("query control-plane pairing request list failed: {error}")
                    })?;
                for row in rows {
                    let raw = row.map_err(|error| {
                        format!("decode control-plane pairing request row failed: {error}")
                    })?;
                    let request = ControlPlanePairingRequestRecord::try_from_raw(raw)?;
                    requests.push(request);
                }
            }
            None => {
                let mut stmt = conn
                    .prepare(
                        "SELECT
                            pairing_request_id,
                            device_id,
                            client_id,
                            public_key,
                            role,
                            requested_scopes_json,
                            status,
                            requested_at_ms,
                            resolved_at_ms,
                            issued_token_id,
                            last_error
                         FROM control_plane_pairing_requests
                         ORDER BY requested_at_ms DESC, pairing_request_id ASC",
                    )
                    .map_err(|error| {
                        format!("prepare control-plane pairing request list query failed: {error}")
                    })?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok(RawControlPlanePairingRequestRecord {
                            pairing_request_id: row.get(0)?,
                            device_id: row.get(1)?,
                            client_id: row.get(2)?,
                            public_key: row.get(3)?,
                            role: row.get(4)?,
                            requested_scopes_json: row.get(5)?,
                            status: row.get(6)?,
                            requested_at_ms: row.get(7)?,
                            resolved_at_ms: row.get(8)?,
                            issued_token_id: row.get(9)?,
                            last_error: row.get(10)?,
                        })
                    })
                    .map_err(|error| {
                        format!("query control-plane pairing request list failed: {error}")
                    })?;
                for row in rows {
                    let raw = row.map_err(|error| {
                        format!("decode control-plane pairing request row failed: {error}")
                    })?;
                    let request = ControlPlanePairingRequestRecord::try_from_raw(raw)?;
                    requests.push(request);
                }
            }
        }
        Ok(requests)
    }

    pub fn transition_control_plane_pairing_request_if_current(
        &self,
        pairing_request_id: &str,
        request: TransitionControlPlanePairingRequestIfCurrentRequest,
    ) -> Result<Option<ControlPlanePairingRequestRecord>, String> {
        let pairing_request_id = normalize_required_text(pairing_request_id, "pairing_request_id")?;
        let last_error = normalize_optional_text(request.last_error);
        let resolution_ts = matches!(
            request.next_status,
            ControlPlanePairingRequestStatus::Approved | ControlPlanePairingRequestStatus::Rejected
        )
        .then(unix_time_ms_now);
        let conn = self.open_connection()?;
        let affected = conn
            .execute(
                "UPDATE control_plane_pairing_requests
                 SET status = ?3,
                     resolved_at_ms = CASE WHEN ?4 IS NULL THEN resolved_at_ms ELSE ?4 END,
                     issued_token_id = CASE WHEN ?5 IS NULL THEN issued_token_id ELSE ?5 END,
                     last_error = ?6
                 WHERE pairing_request_id = ?1 AND status = ?2",
                params![
                    &pairing_request_id,
                    request.expected_status.as_str(),
                    request.next_status.as_str(),
                    resolution_ts,
                    request.issued_token_id,
                    last_error,
                ],
            )
            .map_err(|error| {
                format!("conditionally update control-plane pairing request failed: {error}")
            })?;
        if affected == 0 {
            return Ok(None);
        }

        self.load_control_plane_pairing_request(&pairing_request_id)?
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "control-plane pairing request `{pairing_request_id}` missing after conditional update"
                )
            })
    }

    pub fn approve_control_plane_pairing_request(
        &self,
        request: &ControlPlanePairingRequestRecord,
        token: NewControlPlaneDeviceTokenRecord,
    ) -> Result<Option<ControlPlanePairingRequestRecord>, String> {
        if request.status != ControlPlanePairingRequestStatus::Approved {
            return Err(
                "control-plane pairing approval persistence requires approved status".to_owned(),
            );
        }

        let pairing_request_id =
            normalize_required_text(&request.pairing_request_id, "pairing_request_id")?;
        let resolved_at_ms = request.resolved_at_ms.ok_or_else(|| {
            "approved control-plane pairing request requires resolved_at_ms".to_owned()
        })?;
        let issued_token_id = request.issued_token_id.clone().ok_or_else(|| {
            "approved control-plane pairing request requires issued_token_id".to_owned()
        })?;
        let token_id = normalize_required_text(&token.token_id, "token_id")?;
        let device_id = normalize_required_text(&token.device_id, "device_id")?;
        let public_key = normalize_required_text(&token.public_key, "public_key")?;
        let role = normalize_required_text(&token.role, "role")?;
        let token_hash = normalize_required_text(&token.token_hash, "token_hash")?;
        let approved_scopes_json = encode_string_set_json(&token.approved_scopes)?;
        let last_used_at_ms = token.last_used_at_ms;
        let expires_at_ms = token.expires_at_ms;
        let revoked_at_ms = token.revoked_at_ms;
        let pairing_request_binding = token.pairing_request_id;
        let mut conn = self.open_connection()?;
        let tx = conn.transaction().map_err(|error| {
            format!("open control-plane pairing approval transaction failed: {error}")
        })?;
        let affected = tx
            .execute(
                "UPDATE control_plane_pairing_requests
                 SET status = ?3,
                     resolved_at_ms = ?4,
                     issued_token_id = ?5,
                     last_error = NULL
                 WHERE pairing_request_id = ?1 AND status = ?2",
                params![
                    &pairing_request_id,
                    ControlPlanePairingRequestStatus::Pending.as_str(),
                    ControlPlanePairingRequestStatus::Approved.as_str(),
                    resolved_at_ms,
                    issued_token_id,
                ],
            )
            .map_err(|error| {
                format!("approve control-plane pairing request transaction update failed: {error}")
            })?;
        if affected == 0 {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO control_plane_device_tokens(
                token_id,
                device_id,
                public_key,
                role,
                approved_scopes_json,
                token_hash,
                issued_at_ms,
                expires_at_ms,
                revoked_at_ms,
                last_used_at_ms,
                pairing_request_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(device_id) DO UPDATE SET
                token_id = excluded.token_id,
                public_key = excluded.public_key,
                role = excluded.role,
                approved_scopes_json = excluded.approved_scopes_json,
                token_hash = excluded.token_hash,
                issued_at_ms = excluded.issued_at_ms,
                expires_at_ms = excluded.expires_at_ms,
                revoked_at_ms = excluded.revoked_at_ms,
                last_used_at_ms = excluded.last_used_at_ms,
                pairing_request_id = excluded.pairing_request_id",
            params![
                token_id,
                device_id,
                public_key,
                role,
                approved_scopes_json,
                token_hash,
                resolved_at_ms,
                expires_at_ms,
                revoked_at_ms,
                last_used_at_ms,
                pairing_request_binding,
            ],
        )
        .map_err(|error| {
            format!("approve control-plane pairing request token upsert failed: {error}")
        })?;
        tx.commit().map_err(|error| {
            format!("commit control-plane pairing approval transaction failed: {error}")
        })?;

        self.load_control_plane_pairing_request(&pairing_request_id)?
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "control-plane pairing request `{pairing_request_id}` missing after approval commit"
                )
            })
    }

    pub fn upsert_control_plane_device_token(
        &self,
        record: NewControlPlaneDeviceTokenRecord,
    ) -> Result<ControlPlaneDeviceTokenRecord, String> {
        let token_id = normalize_required_text(&record.token_id, "token_id")?;
        let device_id = normalize_required_text(&record.device_id, "device_id")?;
        let public_key = normalize_required_text(&record.public_key, "public_key")?;
        let role = normalize_required_text(&record.role, "role")?;
        let token_hash = normalize_required_text(&record.token_hash, "token_hash")?;
        let approved_scopes_json = encode_string_set_json(&record.approved_scopes)?;
        let issued_at_ms = unix_time_ms_now();
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT INTO control_plane_device_tokens(
                token_id,
                device_id,
                public_key,
                role,
                approved_scopes_json,
                token_hash,
                issued_at_ms,
                expires_at_ms,
                revoked_at_ms,
                last_used_at_ms,
                pairing_request_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(device_id) DO UPDATE SET
                token_id = excluded.token_id,
                public_key = excluded.public_key,
                role = excluded.role,
                approved_scopes_json = excluded.approved_scopes_json,
                token_hash = excluded.token_hash,
                issued_at_ms = excluded.issued_at_ms,
                expires_at_ms = excluded.expires_at_ms,
                revoked_at_ms = excluded.revoked_at_ms,
                last_used_at_ms = excluded.last_used_at_ms,
                pairing_request_id = excluded.pairing_request_id",
            params![
                token_id,
                device_id,
                public_key,
                role,
                approved_scopes_json,
                token_hash,
                issued_at_ms,
                record.expires_at_ms,
                record.revoked_at_ms,
                record.last_used_at_ms,
                record.pairing_request_id,
            ],
        )
        .map_err(|error| format!("upsert control-plane device token failed: {error}"))?;

        self.load_control_plane_device_token_by_device_id(&device_id)?
            .ok_or_else(|| {
                format!("control-plane device token for `{device_id}` disappeared after upsert")
            })
    }

    pub fn load_control_plane_device_token_by_device_id(
        &self,
        device_id: &str,
    ) -> Result<Option<ControlPlaneDeviceTokenRecord>, String> {
        let device_id = normalize_required_text(device_id, "device_id")?;
        let conn = self.open_connection()?;
        let raw = conn
            .query_row(
                "SELECT
                    token_id,
                    device_id,
                    public_key,
                    role,
                    approved_scopes_json,
                    token_hash,
                    issued_at_ms,
                    expires_at_ms,
                    revoked_at_ms,
                    last_used_at_ms,
                    pairing_request_id
                 FROM control_plane_device_tokens
                 WHERE device_id = ?1",
                params![device_id],
                |row| {
                    Ok(RawControlPlaneDeviceTokenRecord {
                        token_id: row.get(0)?,
                        device_id: row.get(1)?,
                        public_key: row.get(2)?,
                        role: row.get(3)?,
                        approved_scopes_json: row.get(4)?,
                        token_hash: row.get(5)?,
                        issued_at_ms: row.get(6)?,
                        expires_at_ms: row.get(7)?,
                        revoked_at_ms: row.get(8)?,
                        last_used_at_ms: row.get(9)?,
                        pairing_request_id: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load control-plane device token failed: {error}"))?;
        raw.map(ControlPlaneDeviceTokenRecord::try_from_raw)
            .transpose()
    }

    pub fn list_control_plane_device_tokens(
        &self,
    ) -> Result<Vec<ControlPlaneDeviceTokenRecord>, String> {
        let conn = self.open_connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT
                    token_id,
                    device_id,
                    public_key,
                    role,
                    approved_scopes_json,
                    token_hash,
                    issued_at_ms,
                    expires_at_ms,
                    revoked_at_ms,
                    last_used_at_ms,
                    pairing_request_id
                 FROM control_plane_device_tokens
                 ORDER BY issued_at_ms DESC, token_id ASC",
            )
            .map_err(|error| {
                format!("prepare control-plane device token list query failed: {error}")
            })?;
        let rows = stmt
            .query_map([], |row| {
                Ok(RawControlPlaneDeviceTokenRecord {
                    token_id: row.get(0)?,
                    device_id: row.get(1)?,
                    public_key: row.get(2)?,
                    role: row.get(3)?,
                    approved_scopes_json: row.get(4)?,
                    token_hash: row.get(5)?,
                    issued_at_ms: row.get(6)?,
                    expires_at_ms: row.get(7)?,
                    revoked_at_ms: row.get(8)?,
                    last_used_at_ms: row.get(9)?,
                    pairing_request_id: row.get(10)?,
                })
            })
            .map_err(|error| format!("query control-plane device token list failed: {error}"))?;
        let mut tokens = Vec::new();
        for row in rows {
            let raw = row.map_err(|error| {
                format!("decode control-plane device token row failed: {error}")
            })?;
            let token = ControlPlaneDeviceTokenRecord::try_from_raw(raw)?;
            tokens.push(token);
        }
        Ok(tokens)
    }

    pub fn upsert_terminal_outcome(
        &self,
        session_id: &str,
        status: &str,
        payload_json: Value,
    ) -> Result<SessionTerminalOutcomeRecord, String> {
        self.upsert_terminal_outcome_with_frozen_result(session_id, status, payload_json, None)
    }

    pub fn upsert_terminal_outcome_with_frozen_result(
        &self,
        session_id: &str,
        status: &str,
        payload_json: Value,
        frozen_result: Option<FrozenResult>,
    ) -> Result<SessionTerminalOutcomeRecord, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let status = normalize_required_text(status, "status")?;
        if self.load_session(&session_id)?.is_none() {
            return Err(format!("session `{session_id}` not found"));
        }

        let encoded_payload = serde_json::to_string(&payload_json)
            .map_err(|error| format!("encode session terminal outcome payload failed: {error}"))?;
        let encoded_frozen_result = encode_optional_frozen_result(&frozen_result)?;
        let recorded_at = unix_ts_now();
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT INTO session_terminal_outcomes(
                session_id,
                status,
                payload_json,
                frozen_result_json,
                recorded_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_id) DO UPDATE SET
                status = excluded.status,
                payload_json = excluded.payload_json,
                frozen_result_json = COALESCE(
                    excluded.frozen_result_json,
                    session_terminal_outcomes.frozen_result_json
                ),
                recorded_at = excluded.recorded_at",
            params![
                session_id,
                status,
                encoded_payload,
                encoded_frozen_result,
                recorded_at
            ],
        )
        .map_err(|error| format!("upsert session terminal outcome failed: {error}"))?;

        self.load_terminal_outcome(&session_id)?
            .ok_or_else(|| format!("session `{session_id}` missing after terminal outcome upsert"))
    }

    pub fn finalize_session_terminal(
        &self,
        session_id: &str,
        request: FinalizeSessionTerminalRequest,
    ) -> Result<FinalizeSessionTerminalResult, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let event_kind = normalize_required_text(&request.event_kind, "event_kind")?;
        let outcome_status = normalize_required_text(&request.outcome_status, "outcome_status")?;
        let actor_session_id = normalize_optional_text(request.actor_session_id);
        let last_error = normalize_optional_text(request.last_error);
        let event_payload_json = request.event_payload_json;
        let outcome_payload_json = request.outcome_payload_json;
        let frozen_result = request.frozen_result;
        let encoded_event_payload = serde_json::to_string(&event_payload_json)
            .map_err(|error| format!("encode session terminal event payload failed: {error}"))?;
        let event_search_text =
            session_event_search_text(event_kind.as_str(), encoded_event_payload.as_str());
        let encoded_outcome_payload = serde_json::to_string(&outcome_payload_json)
            .map_err(|error| format!("encode session terminal outcome payload failed: {error}"))?;
        let encoded_frozen_result = encode_optional_frozen_result(&frozen_result)?;
        let ts = unix_ts_now();

        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("open session terminal transaction failed: {error}"))?;
        let affected = tx
            .execute(
                "UPDATE sessions
                 SET state = ?2, updated_at = ?3, last_error = ?4
                 WHERE session_id = ?1",
                params![
                    session_id,
                    request.state.as_str(),
                    ts,
                    last_error.as_deref(),
                ],
            )
            .map_err(|error| {
                format!("update session state in terminal finalize failed: {error}")
            })?;
        if affected == 0 {
            return Err(format!("session `{session_id}` not found"));
        }
        tx.execute(
            "INSERT INTO session_events(
                session_id, event_kind, actor_session_id, payload_json, search_text, ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                event_kind,
                actor_session_id.as_deref(),
                encoded_event_payload,
                event_search_text,
                ts
            ],
        )
        .map_err(|error| format!("insert session terminal event failed: {error}"))?;
        let event_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO session_terminal_outcomes(
                session_id,
                status,
                payload_json,
                frozen_result_json,
                recorded_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_id) DO UPDATE SET
                status = excluded.status,
                payload_json = excluded.payload_json,
                frozen_result_json = COALESCE(
                    excluded.frozen_result_json,
                    session_terminal_outcomes.frozen_result_json
                ),
                recorded_at = excluded.recorded_at",
            params![
                session_id,
                outcome_status,
                encoded_outcome_payload,
                encoded_frozen_result,
                ts
            ],
        )
        .map_err(|error| format!("upsert session terminal outcome in finalize failed: {error}"))?;
        tx.commit()
            .map_err(|error| format!("commit session terminal finalize failed: {error}"))?;

        let session = self
            .load_session(&session_id)?
            .ok_or_else(|| format!("session `{session_id}` missing after terminal finalize"))?;
        let terminal_outcome = self.load_terminal_outcome(&session_id)?.ok_or_else(|| {
            format!("session `{session_id}` missing terminal outcome after terminal finalize")
        })?;

        Ok(FinalizeSessionTerminalResult {
            session,
            event: SessionEventRecord {
                id: event_id,
                session_id,
                event_kind,
                actor_session_id,
                payload_json: event_payload_json,
                ts,
            },
            terminal_outcome,
        })
    }

    pub fn finalize_session_terminal_if_current(
        &self,
        session_id: &str,
        expected_state: SessionState,
        request: FinalizeSessionTerminalRequest,
    ) -> Result<Option<FinalizeSessionTerminalResult>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let event_kind = normalize_required_text(&request.event_kind, "event_kind")?;
        let outcome_status = normalize_required_text(&request.outcome_status, "outcome_status")?;
        let actor_session_id = normalize_optional_text(request.actor_session_id);
        let last_error = normalize_optional_text(request.last_error);
        let event_payload_json = request.event_payload_json;
        let outcome_payload_json = request.outcome_payload_json;
        let frozen_result = request.frozen_result;
        let encoded_event_payload = serde_json::to_string(&event_payload_json)
            .map_err(|error| format!("encode session terminal event payload failed: {error}"))?;
        let event_search_text =
            session_event_search_text(event_kind.as_str(), encoded_event_payload.as_str());
        let encoded_outcome_payload = serde_json::to_string(&outcome_payload_json)
            .map_err(|error| format!("encode session terminal outcome payload failed: {error}"))?;
        let encoded_frozen_result = encode_optional_frozen_result(&frozen_result)?;
        let ts = unix_ts_now();

        let mut conn = self.open_connection()?;
        let tx = conn.transaction().map_err(|error| {
            format!("open conditional session terminal transaction failed: {error}")
        })?;
        let affected = tx
            .execute(
                "UPDATE sessions
                 SET state = ?3, updated_at = ?4, last_error = ?5
                 WHERE session_id = ?1 AND state = ?2",
                params![
                    session_id,
                    expected_state.as_str(),
                    request.state.as_str(),
                    ts,
                    last_error.as_deref(),
                ],
            )
            .map_err(|error| {
                format!("conditionally update session state in terminal finalize failed: {error}")
            })?;
        if affected == 0 {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO session_events(
                session_id, event_kind, actor_session_id, payload_json, search_text, ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                event_kind,
                actor_session_id.as_deref(),
                encoded_event_payload,
                event_search_text,
                ts
            ],
        )
        .map_err(|error| format!("insert conditional session terminal event failed: {error}"))?;
        let event_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO session_terminal_outcomes(
                session_id,
                status,
                payload_json,
                frozen_result_json,
                recorded_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_id) DO UPDATE SET
                status = excluded.status,
                payload_json = excluded.payload_json,
                frozen_result_json = COALESCE(
                    excluded.frozen_result_json,
                    session_terminal_outcomes.frozen_result_json
                ),
                recorded_at = excluded.recorded_at",
            params![
                session_id,
                outcome_status,
                encoded_outcome_payload,
                encoded_frozen_result,
                ts
            ],
        )
        .map_err(|error| {
            format!("upsert session terminal outcome in conditional finalize failed: {error}")
        })?;
        tx.commit().map_err(|error| {
            format!("commit conditional session terminal finalize failed: {error}")
        })?;

        let session = self.load_session(&session_id)?.ok_or_else(|| {
            format!("session `{session_id}` missing after conditional terminal finalize")
        })?;
        let terminal_outcome = self.load_terminal_outcome(&session_id)?.ok_or_else(|| {
            format!(
                "session `{session_id}` missing terminal outcome after conditional terminal finalize"
            )
        })?;

        Ok(Some(FinalizeSessionTerminalResult {
            session,
            event: SessionEventRecord {
                id: event_id,
                session_id,
                event_kind,
                actor_session_id,
                payload_json: event_payload_json,
                ts,
            },
            terminal_outcome,
        }))
    }

    pub fn append_event(&self, event: NewSessionEvent) -> Result<SessionEventRecord, String> {
        let session_id = normalize_required_text(&event.session_id, "session_id")?;
        let event_kind = normalize_required_text(&event.event_kind, "event_kind")?;

        let ts = unix_ts_now();
        let payload_json = serde_json::to_string(&event.payload_json)
            .map_err(|error| format!("encode session event payload failed: {error}"))?;
        let event_search_text =
            session_event_search_text(event_kind.as_str(), payload_json.as_str());
        let actor_session_id = normalize_optional_text(event.actor_session_id);

        let conn = self.open_connection()?;
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
                params![session_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("check session exists failed: {error}"))?;
        if !exists {
            return Err(format!("session `{session_id}` not found"));
        }

        conn.execute(
            "INSERT INTO session_events(
                session_id, event_kind, actor_session_id, payload_json, search_text, ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                event_kind,
                actor_session_id,
                payload_json,
                event_search_text,
                ts,
            ],
        )
        .map_err(|error| format!("insert session event failed: {error}"))?;

        Ok(SessionEventRecord {
            id: conn.last_insert_rowid(),
            session_id,
            event_kind,
            actor_session_id,
            payload_json: event.payload_json,
            ts,
        })
    }

    pub fn append_event_if_session_active(
        &self,
        event: NewSessionEvent,
    ) -> Result<Option<SessionEventRecord>, String> {
        let session_id = normalize_required_text(&event.session_id, "session_id")?;
        let event_kind = normalize_required_text(&event.event_kind, "event_kind")?;
        let actor_session_id = normalize_optional_text(event.actor_session_id);
        let ts = unix_ts_now();
        let payload_value = event.payload_json;
        let payload_json = serde_json::to_string(&payload_value)
            .map_err(|error| format!("encode session event payload failed: {error}"))?;
        let event_search_text =
            session_event_search_text(event_kind.as_str(), payload_json.as_str());

        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("open session append transaction failed: {error}"))?;
        let raw_state = tx
            .query_row(
                "SELECT state FROM sessions WHERE session_id = ?1",
                params![session_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load session state for append failed: {error}"))?;
        let Some(raw_state) = raw_state else {
            return Ok(None);
        };
        let session_state = SessionState::from_db(raw_state.as_str())?;
        let session_is_terminal = matches!(
            session_state,
            SessionState::Completed | SessionState::Failed | SessionState::TimedOut
        );
        if session_is_terminal {
            return Ok(None);
        }

        tx.execute(
            "INSERT INTO session_events(
                session_id, event_kind, actor_session_id, payload_json, search_text, ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id.as_str(),
                event_kind.as_str(),
                actor_session_id.as_deref(),
                payload_json.as_str(),
                event_search_text.as_str(),
                ts,
            ],
        )
        .map_err(|error| format!("insert session event failed: {error}"))?;
        let event_id = tx.last_insert_rowid();
        tx.commit()
            .map_err(|error| format!("commit session append transaction failed: {error}"))?;

        Ok(Some(SessionEventRecord {
            id: event_id,
            session_id,
            event_kind,
            actor_session_id,
            payload_json: payload_value,
            ts,
        }))
    }

    pub(super) fn open_connection(&self) -> Result<Connection, String> {
        Connection::open(&self.db_path)
            .map_err(|error| format!("open session repository sqlite db failed: {error}"))
    }

    pub(super) fn create_session_with_event_in_tx(
        tx: &Transaction<'_>,
        request: CreateSessionWithEventRequest,
    ) -> Result<SessionEventRecord, String> {
        let session_id = normalize_required_text(&request.session.session_id, "session_id")?;
        let parent_session_id = normalize_optional_text(request.session.parent_session_id);
        let label = normalize_optional_text(request.session.label);
        let event_kind = normalize_required_text(&request.event_kind, "event_kind")?;
        let actor_session_id = normalize_optional_text(request.actor_session_id);
        let event_payload_json = request.event_payload_json;
        let encoded_event_payload = serde_json::to_string(&event_payload_json)
            .map_err(|error| format!("encode session event payload failed: {error}"))?;
        let event_search_text =
            session_event_search_text(event_kind.as_str(), encoded_event_payload.as_str());
        let ts = unix_ts_now();

        tx.execute(
            "INSERT INTO sessions(
                session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![
                session_id,
                request.session.kind.as_str(),
                parent_session_id,
                label,
                request.session.state.as_str(),
                ts,
                ts,
            ],
        )
        .map_err(|error| format!("insert session row failed: {error}"))?;
        seed_session_tree_for_new_session(tx, &session_id, ts)?;
        tx.execute(
            "INSERT INTO session_events(
                session_id, event_kind, actor_session_id, payload_json, search_text, ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                event_kind,
                actor_session_id.as_deref(),
                encoded_event_payload,
                event_search_text,
                ts
            ],
        )
        .map_err(|error| format!("insert session event failed: {error}"))?;

        Ok(SessionEventRecord {
            id: tx.last_insert_rowid(),
            session_id,
            event_kind,
            actor_session_id,
            payload_json: event_payload_json,
            ts,
        })
    }

    pub(super) fn count_active_direct_children_with_conn(
        conn: &Connection,
        parent_session_id: &str,
    ) -> Result<usize, String> {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM sessions
                 WHERE parent_session_id = ?1
                   AND state IN ('ready', 'running')",
                params![parent_session_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("count active direct child sessions failed: {error}"))?;
        usize::try_from(count)
            .map_err(|error| format!("active direct child count overflowed usize: {error}"))
    }
}
