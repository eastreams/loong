use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::CliResult;

use super::principal::{FeishuGrantScopeSet, FeishuUserPrincipal};

const FEISHU_TOKEN_DB_BUSY_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuGrant {
    pub principal: FeishuUserPrincipal,
    pub access_token: String,
    pub refresh_token: String,
    pub scopes: FeishuGrantScopeSet,
    pub access_expires_at_s: i64,
    pub refresh_expires_at_s: i64,
    pub refreshed_at_s: i64,
}

impl FeishuGrant {
    pub fn is_access_token_expired(&self, now_s: i64) -> bool {
        self.access_expires_at_s <= now_s
    }

    pub fn is_refresh_token_expired(&self, now_s: i64) -> bool {
        self.refresh_expires_at_s <= now_s
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuOauthStateRecord {
    pub state: String,
    pub account_id: String,
    pub principal_hint: String,
    pub scope_csv: String,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub expires_at_s: i64,
    pub created_at_s: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuStoredOauthState {
    pub state: String,
    pub account_id: String,
    pub principal_hint: String,
    pub scope_csv: String,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub expires_at_s: i64,
    pub created_at_s: i64,
}

#[derive(Debug, Clone)]
pub struct FeishuTokenStore {
    path: PathBuf,
}

impl FeishuTokenStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn save_grant(&self, grant: &FeishuGrant) -> CliResult<()> {
        ensure_feishu_schema(&self.path)?;
        let conn = open_connection(&self.path)?;
        let principal_json = serde_json::to_string(&grant.principal)
            .map_err(|error| format!("encode feishu principal failed: {error}"))?;
        conn.execute(
            "INSERT INTO feishu_grants(
                account_id,
                open_id,
                union_id,
                user_id,
                access_token,
                refresh_token,
                scope_csv,
                access_expires_at_s,
                refresh_expires_at_s,
                refreshed_at_s,
                principal_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(account_id, open_id) DO UPDATE SET
                union_id = excluded.union_id,
                user_id = excluded.user_id,
                access_token = excluded.access_token,
                refresh_token = excluded.refresh_token,
                scope_csv = excluded.scope_csv,
                access_expires_at_s = excluded.access_expires_at_s,
                refresh_expires_at_s = excluded.refresh_expires_at_s,
                refreshed_at_s = excluded.refreshed_at_s,
                principal_json = excluded.principal_json",
            params![
                grant.principal.account_id,
                grant.principal.open_id,
                grant.principal.union_id,
                grant.principal.user_id,
                grant.access_token,
                grant.refresh_token,
                grant.scopes.to_scope_csv(),
                grant.access_expires_at_s,
                grant.refresh_expires_at_s,
                grant.refreshed_at_s,
                principal_json,
            ],
        )
        .map_err(|error| format!("save feishu grant failed: {error}"))?;
        Ok(())
    }

    pub fn load_grant(&self, account_id: &str, open_id: &str) -> CliResult<Option<FeishuGrant>> {
        ensure_feishu_schema(&self.path)?;
        let conn = open_connection(&self.path)?;
        conn.query_row(
            "SELECT
                access_token,
                refresh_token,
                scope_csv,
                access_expires_at_s,
                refresh_expires_at_s,
                refreshed_at_s,
                principal_json
             FROM feishu_grants
             WHERE account_id = ?1 AND open_id = ?2",
            params![account_id.trim(), open_id.trim()],
            row_to_grant,
        )
        .optional()
        .map_err(|error| format!("load feishu grant failed: {error}"))
    }

    pub fn list_grants_for_account(&self, account_id: &str) -> CliResult<Vec<FeishuGrant>> {
        ensure_feishu_schema(&self.path)?;
        let conn = open_connection(&self.path)?;
        let mut statement = conn
            .prepare(
                "SELECT
                    access_token,
                    refresh_token,
                    scope_csv,
                    access_expires_at_s,
                    refresh_expires_at_s,
                    refreshed_at_s,
                    principal_json
                 FROM feishu_grants
                 WHERE account_id = ?1
                 ORDER BY refreshed_at_s DESC, open_id ASC",
            )
            .map_err(|error| format!("prepare feishu grant list failed: {error}"))?;
        let rows = statement
            .query_map(params![account_id.trim()], row_to_grant)
            .map_err(|error| format!("query feishu grants failed: {error}"))?;
        let mut grants = Vec::new();
        for row in rows {
            grants.push(row.map_err(|error| format!("read feishu grant row failed: {error}"))?);
        }
        Ok(grants)
    }

    pub fn delete_grant(&self, account_id: &str, open_id: &str) -> CliResult<bool> {
        ensure_feishu_schema(&self.path)?;
        let conn = open_connection(&self.path)?;
        let affected = conn
            .execute(
                "DELETE FROM feishu_grants WHERE account_id = ?1 AND open_id = ?2",
                params![account_id.trim(), open_id.trim()],
            )
            .map_err(|error| format!("delete feishu grant failed: {error}"))?;
        if affected > 0 {
            conn.execute(
                "DELETE FROM feishu_selected_grants WHERE account_id = ?1 AND open_id = ?2",
                params![account_id.trim(), open_id.trim()],
            )
            .map_err(|error| format!("delete feishu selected grant failed: {error}"))?;
        }
        Ok(affected > 0)
    }

    pub fn list_grants(&self, account_id: &str) -> CliResult<Vec<FeishuGrant>> {
        ensure_feishu_schema(&self.path)?;
        let conn = open_connection(&self.path)?;
        let mut statement = conn
            .prepare(
                "SELECT
                    access_token,
                    refresh_token,
                    scope_csv,
                    access_expires_at_s,
                    refresh_expires_at_s,
                    refreshed_at_s,
                    principal_json
                 FROM feishu_grants
                 WHERE account_id = ?1
                 ORDER BY open_id ASC",
            )
            .map_err(|error| format!("prepare feishu grant list failed: {error}"))?;
        let rows = statement
            .query_map(params![account_id.trim()], |row| {
                let principal_json: String = row.get(6)?;
                let principal = serde_json::from_str::<FeishuUserPrincipal>(&principal_json)
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            principal_json.len(),
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                let scope_csv: String = row.get(2)?;
                Ok(FeishuGrant {
                    principal,
                    access_token: row.get(0)?,
                    refresh_token: row.get(1)?,
                    scopes: FeishuGrantScopeSet::from_scopes(scope_csv.split_whitespace()),
                    access_expires_at_s: row.get(3)?,
                    refresh_expires_at_s: row.get(4)?,
                    refreshed_at_s: row.get(5)?,
                })
            })
            .map_err(|error| format!("query feishu grant list failed: {error}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("decode feishu grant list failed: {error}"))
    }

    pub fn set_selected_grant(
        &self,
        account_id: &str,
        open_id: &str,
        updated_at_s: i64,
    ) -> CliResult<()> {
        ensure_feishu_schema(&self.path)?;
        let conn = open_connection(&self.path)?;
        conn.execute(
            "INSERT INTO feishu_selected_grants(
                account_id,
                open_id,
                updated_at_s
            ) VALUES (?1, ?2, ?3)
            ON CONFLICT(account_id) DO UPDATE SET
                open_id = excluded.open_id,
                updated_at_s = excluded.updated_at_s",
            params![account_id.trim(), open_id.trim(), updated_at_s],
        )
        .map_err(|error| format!("save feishu selected grant failed: {error}"))?;
        Ok(())
    }

    pub fn load_selected_grant(&self, account_id: &str) -> CliResult<Option<String>> {
        ensure_feishu_schema(&self.path)?;
        let conn = open_connection(&self.path)?;
        conn.query_row(
            "SELECT open_id
             FROM feishu_selected_grants
             WHERE account_id = ?1",
            params![account_id.trim()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("load feishu selected grant failed: {error}"))
    }

    pub fn clear_selected_grant(&self, account_id: &str) -> CliResult<bool> {
        ensure_feishu_schema(&self.path)?;
        let conn = open_connection(&self.path)?;
        let affected = conn
            .execute(
                "DELETE FROM feishu_selected_grants WHERE account_id = ?1",
                params![account_id.trim()],
            )
            .map_err(|error| format!("clear feishu selected grant failed: {error}"))?;
        Ok(affected > 0)
    }

    pub fn save_oauth_state(
        &self,
        state: &str,
        account_id: &str,
        principal_hint: &str,
        expires_at_s: i64,
    ) -> CliResult<()> {
        let record = FeishuOauthStateRecord {
            state: state.trim().to_owned(),
            account_id: account_id.trim().to_owned(),
            principal_hint: principal_hint.trim().to_owned(),
            scope_csv: String::new(),
            redirect_uri: None,
            code_verifier: None,
            expires_at_s,
            created_at_s: unix_ts_now(),
        };
        self.save_oauth_state_record(&record)
    }

    pub fn save_oauth_state_record(&self, record: &FeishuOauthStateRecord) -> CliResult<()> {
        ensure_feishu_schema(&self.path)?;
        let conn = open_connection(&self.path)?;
        conn.execute(
            "INSERT INTO feishu_oauth_states(
                state,
                account_id,
                principal_hint,
                scope_csv,
                redirect_uri,
                code_verifier,
                expires_at_s,
                created_at_s
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(state) DO UPDATE SET
                account_id = excluded.account_id,
                principal_hint = excluded.principal_hint,
                scope_csv = excluded.scope_csv,
                redirect_uri = excluded.redirect_uri,
                code_verifier = excluded.code_verifier,
                expires_at_s = excluded.expires_at_s,
                created_at_s = excluded.created_at_s",
            params![
                record.state.trim(),
                record.account_id.trim(),
                record.principal_hint.trim(),
                record.scope_csv.trim(),
                record.redirect_uri,
                record.code_verifier,
                record.expires_at_s,
                record.created_at_s,
            ],
        )
        .map_err(|error| format!("save feishu oauth state failed: {error}"))?;
        Ok(())
    }

    pub fn consume_oauth_state(
        &self,
        state: &str,
        now_s: i64,
    ) -> CliResult<FeishuStoredOauthState> {
        ensure_feishu_schema(&self.path)?;
        let mut conn = open_connection(&self.path)?;
        let state_key = state.trim();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("begin feishu oauth state transaction failed: {error}"))?;
        let record = tx
            .query_row(
                "SELECT
                    account_id,
                    principal_hint,
                    scope_csv,
                    redirect_uri,
                    code_verifier,
                    expires_at_s,
                    created_at_s
                 FROM feishu_oauth_states
                 WHERE state = ?1",
                params![state_key],
                |row| {
                    Ok(FeishuStoredOauthState {
                        state: state_key.to_owned(),
                        account_id: row.get(0)?,
                        principal_hint: row.get(1)?,
                        scope_csv: row.get(2)?,
                        redirect_uri: row.get(3)?,
                        code_verifier: row.get(4)?,
                        expires_at_s: row.get(5)?,
                        created_at_s: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load feishu oauth state failed: {error}"))?
            .ok_or_else(|| "feishu oauth state was not found".to_owned())?;

        tx.execute(
            "DELETE FROM feishu_oauth_states WHERE state = ?1",
            params![state_key],
        )
        .map_err(|error| format!("delete feishu oauth state failed: {error}"))?;
        tx.commit()
            .map_err(|error| format!("commit feishu oauth state transaction failed: {error}"))?;

        if record.expires_at_s <= now_s {
            return Err("feishu oauth state expired".to_owned());
        }

        Ok(record)
    }
}

fn open_connection(path: &Path) -> CliResult<Connection> {
    let conn =
        Connection::open(path).map_err(|error| format!("open feishu token db failed: {error}"))?;
    conn.busy_timeout(std::time::Duration::from_millis(
        FEISHU_TOKEN_DB_BUSY_TIMEOUT_MS,
    ))
    .map_err(|error| format!("configure feishu token db busy timeout failed: {error}"))?;
    Ok(conn)
}

fn ensure_feishu_schema(path: &Path) -> CliResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        let created_parent = !parent.exists();
        fs::create_dir_all(parent)
            .map_err(|error| format!("create feishu token db directory failed: {error}"))?;
        if created_parent {
            harden_feishu_token_store_parent_dir(parent)?;
        }
    }

    let conn = open_connection(path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS feishu_oauth_states (
            state TEXT PRIMARY KEY NOT NULL,
            account_id TEXT NOT NULL,
            principal_hint TEXT NOT NULL DEFAULT '',
            scope_csv TEXT NOT NULL DEFAULT '',
            redirect_uri TEXT,
            code_verifier TEXT,
            expires_at_s INTEGER NOT NULL,
            created_at_s INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS feishu_grants (
            account_id TEXT NOT NULL,
            open_id TEXT NOT NULL,
            union_id TEXT,
            user_id TEXT,
            access_token TEXT NOT NULL,
            refresh_token TEXT NOT NULL,
            scope_csv TEXT NOT NULL DEFAULT '',
            access_expires_at_s INTEGER NOT NULL,
            refresh_expires_at_s INTEGER NOT NULL,
            refreshed_at_s INTEGER NOT NULL,
            principal_json TEXT NOT NULL,
            PRIMARY KEY (account_id, open_id)
        );
        CREATE TABLE IF NOT EXISTS feishu_selected_grants (
            account_id TEXT PRIMARY KEY NOT NULL,
            open_id TEXT NOT NULL,
            updated_at_s INTEGER NOT NULL
        );",
    )
    .map_err(|error| format!("initialize feishu token schema failed: {error}"))?;
    harden_feishu_token_store_file_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn harden_feishu_token_store_parent_dir(path: &Path) -> CliResult<()> {
    use std::os::unix::fs::PermissionsExt;

    if !path.exists() {
        return Ok(());
    }
    let mut permissions = fs::metadata(path)
        .map_err(|error| {
            format!(
                "read feishu token db directory metadata `{}` failed: {error}",
                path.display()
            )
        })?
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).map_err(|error| {
        format!(
            "set feishu token db directory permissions `{}` failed: {error}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn harden_feishu_token_store_parent_dir(_path: &Path) -> CliResult<()> {
    Ok(())
}

#[cfg(unix)]
fn harden_feishu_token_store_file_permissions(path: &Path) -> CliResult<()> {
    use std::os::unix::fs::PermissionsExt;

    if path.as_os_str().is_empty() || path == Path::new(":memory:") || !path.exists() {
        return Ok(());
    }
    let mut permissions = fs::metadata(path)
        .map_err(|error| {
            format!(
                "read feishu token db metadata `{}` failed: {error}",
                path.display()
            )
        })?
        .permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions).map_err(|error| {
        format!(
            "set feishu token db permissions `{}` failed: {error}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn harden_feishu_token_store_file_permissions(_path: &Path) -> CliResult<()> {
    Ok(())
}

fn row_to_grant(row: &rusqlite::Row<'_>) -> rusqlite::Result<FeishuGrant> {
    let principal_json: String = row.get(6)?;
    let principal =
        serde_json::from_str::<FeishuUserPrincipal>(&principal_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                principal_json.len(),
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    let scope_csv: String = row.get(2)?;
    Ok(FeishuGrant {
        principal,
        access_token: row.get(0)?,
        refresh_token: row.get(1)?,
        scopes: FeishuGrantScopeSet::from_scopes(scope_csv.split_whitespace()),
        access_expires_at_s: row.get(3)?,
        refresh_expires_at_s: row.get(4)?,
        refreshed_at_s: row.get(5)?,
    })
}

fn unix_ts_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn unique_temp_db(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}.sqlite3"))
    }

    fn sample_principal() -> FeishuUserPrincipal {
        FeishuUserPrincipal {
            account_id: "feishu_main".to_owned(),
            open_id: "ou_123".to_owned(),
            union_id: Some("on_456".to_owned()),
            user_id: Some("u_789".to_owned()),
            name: Some("Alice".to_owned()),
            tenant_key: Some("tenant_x".to_owned()),
            avatar_url: None,
            email: Some("alice@example.com".to_owned()),
            enterprise_email: None,
        }
    }

    fn sample_grant(principal: &FeishuUserPrincipal) -> FeishuGrant {
        FeishuGrant {
            principal: principal.clone(),
            access_token: "u-token".to_owned(),
            refresh_token: "r-token".to_owned(),
            scopes: FeishuGrantScopeSet::from_scopes(["offline_access", "docx:document:readonly"]),
            access_expires_at_s: 1_700_007_200,
            refresh_expires_at_s: 1_702_592_000,
            refreshed_at_s: 1_700_000_000,
        }
    }

    #[test]
    fn token_store_round_trips_grant_for_principal() {
        let path = unique_temp_db("grant-round-trip");
        let store = FeishuTokenStore::new(path);
        let principal = sample_principal();
        let grant = sample_grant(&principal);

        store.save_grant(&grant).expect("save grant");
        let loaded = store
            .load_grant("feishu_main", "ou_123")
            .expect("load grant");

        assert_eq!(
            loaded.as_ref().map(|value| value.access_token.as_str()),
            Some("u-token")
        );
        assert_eq!(
            loaded.as_ref().map(|value| value.refresh_token.as_str()),
            Some("r-token")
        );
        assert_eq!(
            loaded
                .as_ref()
                .map(|value| value.principal.storage_key())
                .as_deref(),
            Some("feishu_main:ou_123")
        );
    }

    #[test]
    fn token_store_rejects_expired_oauth_state() {
        let path = unique_temp_db("oauth-state-expiry");
        let store = FeishuTokenStore::new(path);
        store
            .save_oauth_state("state-1", "feishu_main", "ou_123", 10)
            .expect("save state");

        let result = store.consume_oauth_state("state-1", 11);

        assert!(matches!(result, Err(error) if error.contains("expired")));
    }

    #[test]
    fn token_store_oauth_state_is_single_use() {
        let path = unique_temp_db("oauth-state-single-use");
        let store = FeishuTokenStore::new(path);
        store
            .save_oauth_state("state-1", "feishu_main", "ou_123", 1_700_000_100)
            .expect("save oauth state");

        let consumed = store
            .consume_oauth_state("state-1", 1_700_000_000)
            .expect("consume oauth state");
        assert_eq!(consumed.state, "state-1");
        assert_eq!(consumed.account_id, "feishu_main");

        let second = store.consume_oauth_state("state-1", 1_700_000_000);
        assert!(matches!(second, Err(error) if error.contains("not found")));
    }

    #[cfg(unix)]
    #[test]
    fn token_store_hardens_secret_db_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir().join(format!(
            "feishu-token-store-perms-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time before epoch")
                .as_nanos()
        ));
        let path = temp_dir.join("private").join("feishu.sqlite3");
        let store = FeishuTokenStore::new(path.clone());

        store
            .save_oauth_state("state-1", "feishu_main", "ou_123", 1_700_000_100)
            .expect("save oauth state");

        let parent_mode = fs::metadata(path.parent().expect("sqlite parent"))
            .expect("read sqlite parent metadata")
            .permissions()
            .mode()
            & 0o777;
        let file_mode = fs::metadata(&path)
            .expect("read sqlite file metadata")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(parent_mode, 0o700);
        assert_eq!(file_mode, 0o600);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn token_store_lists_grants_for_resolved_account_identity() {
        let path = unique_temp_db("grant-list");
        let store = FeishuTokenStore::new(path);
        let principal = sample_principal();
        let mut secondary_principal = sample_principal();
        secondary_principal.open_id = "ou_456".to_owned();
        let mut foreign_principal = sample_principal();
        foreign_principal.account_id = "feishu_backup".to_owned();
        foreign_principal.open_id = "ou_999".to_owned();

        store
            .save_grant(&sample_grant(&principal))
            .expect("save primary grant");
        store
            .save_grant(&sample_grant(&secondary_principal))
            .expect("save secondary grant");
        store
            .save_grant(&sample_grant(&foreign_principal))
            .expect("save foreign grant");

        let listed = store
            .list_grants("feishu_main")
            .expect("list grants for account");

        assert_eq!(listed.len(), 2);
        assert_eq!(
            listed
                .iter()
                .map(|value| value.principal.open_id.as_str())
                .collect::<Vec<_>>(),
            vec!["ou_123", "ou_456"]
        );
    }

    #[test]
    fn token_store_persists_selected_grant_for_account() {
        let path = unique_temp_db("grant-selection");
        let store = FeishuTokenStore::new(path);

        store
            .set_selected_grant("feishu_main", "ou_456", 1_700_000_100)
            .expect("save selected grant");

        let selected = store
            .load_selected_grant("feishu_main")
            .expect("load selected grant");

        assert_eq!(selected.as_deref(), Some("ou_456"));

        store
            .set_selected_grant("feishu_main", "ou_123", 1_700_000_200)
            .expect("update selected grant");

        let updated = store
            .load_selected_grant("feishu_main")
            .expect("reload selected grant");

        assert_eq!(updated.as_deref(), Some("ou_123"));
    }

    #[test]
    fn deleting_selected_grant_clears_selected_mapping() {
        let path = unique_temp_db("grant-selection-delete");
        let store = FeishuTokenStore::new(path);
        let principal = sample_principal();
        let grant = sample_grant(&principal);

        store.save_grant(&grant).expect("save grant");
        store
            .set_selected_grant("feishu_main", "ou_123", 1_700_000_100)
            .expect("set selected grant");
        store
            .delete_grant("feishu_main", "ou_123")
            .expect("delete selected grant");

        let selected = store
            .load_selected_grant("feishu_main")
            .expect("reload selected mapping");

        assert_eq!(selected, None);
    }
}
