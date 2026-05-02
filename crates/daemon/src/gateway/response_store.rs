use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

#[derive(Debug, Clone)]
pub(crate) struct GatewayStoredResponse {
    pub(crate) session_id: String,
    pub(crate) payload: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct GatewayResponseStore {
    sqlite_path: PathBuf,
}

pub(crate) fn gateway_response_store_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("responses.sqlite3")
}

impl GatewayResponseStore {
    pub(crate) fn new(sqlite_path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = sqlite_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!("create gateway response store directory failed: {error}")
            })?;
        }
        let store = Self { sqlite_path };
        let connection = store.open_connection()?;
        Self::initialize_schema(&connection)?;
        Ok(store)
    }

    pub(crate) fn save_response(
        &self,
        response_id: &str,
        session_id: &str,
        payload: &Value,
    ) -> Result<(), String> {
        let payload_json = serde_json::to_string(payload)
            .map_err(|error| format!("serialize gateway response payload failed: {error}"))?;
        let connection = self.open_connection()?;
        connection
            .execute(
                "INSERT INTO gateway_responses(response_id, session_id, payload_json)
                 VALUES(?1, ?2, ?3)
                 ON CONFLICT(response_id) DO UPDATE SET
                   session_id = excluded.session_id,
                   payload_json = excluded.payload_json",
                params![response_id, session_id, payload_json],
            )
            .map_err(|error| format!("persist gateway response failed: {error}"))?;
        Ok(())
    }

    pub(crate) fn load_response(
        &self,
        response_id: &str,
    ) -> Result<Option<GatewayStoredResponse>, String> {
        let connection = self.open_connection()?;
        let row = connection
            .query_row(
                "SELECT response_id, session_id, payload_json
                 FROM gateway_responses
                 WHERE response_id = ?1",
                params![response_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("load gateway response failed: {error}"))?;

        row.map(|(_stored_response_id, session_id, payload_json)| {
            let payload = serde_json::from_str::<Value>(&payload_json)
                .map_err(|error| format!("decode stored gateway response failed: {error}"))?;
            Ok(GatewayStoredResponse {
                session_id,
                payload,
            })
        })
        .transpose()
    }

    pub(crate) fn resolve_session_id(&self, response_id: &str) -> Result<Option<String>, String> {
        self.load_response(response_id)
            .map(|record| record.map(|record| record.session_id))
    }

    pub(crate) fn delete_response(&self, response_id: &str) -> Result<bool, String> {
        let connection = self.open_connection()?;
        let affected = connection
            .execute(
                "DELETE FROM gateway_responses WHERE response_id = ?1",
                params![response_id],
            )
            .map_err(|error| format!("delete gateway response failed: {error}"))?;
        Ok(affected > 0)
    }

    fn open_connection(&self) -> Result<Connection, String> {
        Connection::open(&self.sqlite_path)
            .map_err(|error| format!("open gateway response store failed: {error}"))
    }

    fn initialize_schema(connection: &Connection) -> Result<(), String> {
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS gateway_responses(
                    response_id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
                );
                CREATE INDEX IF NOT EXISTS idx_gateway_responses_session_id
                    ON gateway_responses(session_id);",
            )
            .map_err(|error| format!("initialize gateway response store failed: {error}"))
    }
}
