use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::Html,
    routing::get,
};
use clap::{Args, Subcommand, ValueEnum};
use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use reqwest::Url;
use serde_json::{Value, json};
use tokio::sync::{Mutex, oneshot};

use crate::feishu_support::{
    FeishuAuthCapability, FeishuDaemonContext, build_account_recommendations,
    build_grant_recommendations, build_pkce_pair, feishu_auth_exchange_command_hint,
    feishu_auth_start_command_hint, generate_oauth_state, load_feishu_daemon_context,
    normalized_auth_start_capabilities, resolve_scopes, unix_ts_now,
};

const DEFAULT_FEISHU_REDIRECT_URI: &str = "http://127.0.0.1:34819/callback";
const FEISHU_LOCAL_CALLBACK_WAIT_TIMEOUT_S: u64 = 180;

fn active_cli_command_name() -> &'static str {
    mvp::config::active_cli_command_name()
}

fn normalize_feishu_callback_wait_timeout_s(value: Option<u64>) -> u64 {
    value.unwrap_or(FEISHU_LOCAL_CALLBACK_WAIT_TIMEOUT_S).max(1)
}

fn open_url_in_default_browser(url: &str) -> CliResult<()> {
    let url = url.trim();
    if url.is_empty() {
        return Err("browser launch requires a non-empty URL".to_owned());
    }

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("rundll32");
        command.args(["url.dll,FileProtocolHandler", url]);
        command
    };

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };

    let status = command
        .status()
        .map_err(|error| format!("launch default browser failed: {error}"))?;
    if status.success() {
        return Ok(());
    }
    Err(format!(
        "launch default browser exited with status {status}"
    ))
}

fn print_feishu_serve_preflight(
    resolved: &mvp::config::ResolvedFeishuChannelConfig,
    bind_override: Option<&str>,
    path_override: Option<&str>,
) {
    let bind_override = bind_override
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let path_override = path_override
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match resolved.mode {
        mvp::config::FeishuChannelServeMode::Websocket => {
            #[allow(clippy::print_stdout)]
            {
                println!(
                    "feishu serve preflight: configured_account={} account={} mode=websocket transport=outbound_websocket_client local_listener=disabled",
                    resolved.configured_account_id, resolved.account.label
                );
            }
            if bind_override.is_some() || path_override.is_some() {
                #[allow(clippy::print_stderr)]
                {
                    eprintln!(
                        "warning: effective Feishu serve mode is websocket; local HTTP overrides are ignored. Set `[feishu] mode = \"webhook\"` if you want bind/path to take effect."
                    );
                }
            }
        }
        mvp::config::FeishuChannelServeMode::Webhook => {
            let effective_bind = bind_override.unwrap_or(resolved.webhook_bind.as_str());
            let effective_path = path_override.unwrap_or(resolved.webhook_path.as_str());
            #[allow(clippy::print_stdout)]
            {
                println!(
                    "feishu serve preflight: configured_account={} account={} mode=webhook bind={} path={}",
                    resolved.configured_account_id,
                    resolved.account.label,
                    effective_bind,
                    effective_path
                );
            }
        }
    }
}

fn apply_feishu_mode_override(
    config: &mut mvp::config::LoongClawConfig,
    configured_account_id: &str,
    mode: FeishuServeModeOverride,
) {
    if let Some(account) = config.feishu.accounts.get_mut(configured_account_id) {
        account.mode = Some(mode.to_config_mode());
        return;
    }
    config.feishu.mode = Some(mode.to_config_mode());
}

fn reject_feishu_websocket_listener_overrides(
    bind_override: Option<&str>,
    path_override: Option<&str>,
) -> CliResult<()> {
    let bind_override = bind_override
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let path_override = path_override
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if bind_override.is_none() && path_override.is_none() {
        return Ok(());
    }
    Err(
        "feishu serve --mode websocket does not open a local HTTP listener; remove --bind/--path or switch to --mode webhook"
            .to_owned(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FeishuLoopbackRedirectSpec {
    redirect_uri: String,
    bind_target: String,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FeishuCapturedLocalCallback {
    state: String,
    code: String,
    callback_url: String,
}

#[derive(Clone)]
struct FeishuLocalCallbackState {
    redirect_uri: String,
    expected_state: String,
    callback_sender: Arc<Mutex<Option<oneshot::Sender<FeishuCapturedLocalCallback>>>>,
}

#[derive(Debug)]
struct FeishuLocalCallbackServer {
    callback_receiver: oneshot::Receiver<FeishuCapturedLocalCallback>,
    shutdown_sender: Option<oneshot::Sender<()>>,
    join_handle: tokio::task::JoinHandle<CliResult<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FeishuServeModeOverride {
    Websocket,
    Webhook,
}

impl FeishuServeModeOverride {
    fn to_config_mode(self) -> mvp::config::FeishuChannelServeMode {
        match self {
            Self::Websocket => mvp::config::FeishuChannelServeMode::Websocket,
            Self::Webhook => mvp::config::FeishuChannelServeMode::Webhook,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum FeishuCommand {
    /// Start or inspect user OAuth grants and state
    Auth {
        #[command(subcommand)]
        command: FeishuAuthCommand,
    },
    /// Resolve the selected user grant and print Feishu profile details
    Whoami(FeishuGrantArgs),
    /// Create or append Feishu docx documents
    Doc {
        #[command(subcommand)]
        command: FeishuDocCommand,
    },
    /// Read Feishu resources
    Read {
        #[command(subcommand)]
        command: FeishuReadCommand,
    },
    /// Inspect Feishu message history and message details
    Messages {
        #[command(subcommand)]
        command: FeishuMessagesCommand,
    },
    /// Search Feishu resources
    Search {
        #[command(subcommand)]
        command: FeishuSearchCommand,
    },
    /// Inspect Feishu calendar resources
    Calendar {
        #[command(subcommand)]
        command: FeishuCalendarCommand,
    },
    /// Inspect and write Feishu Bitable resources
    Bitable {
        #[command(subcommand)]
        command: FeishuBitableCommand,
    },
    /// Send one Feishu text, post, image, file, or card message
    Send(FeishuSendArgs),
    /// Reply to one Feishu message or thread with text, post, image, file, or card content
    Reply(FeishuReplyArgs),
    /// Run Feishu serve mode (webhook or websocket)
    Serve {
        #[command(subcommand)]
        command: Option<FeishuServeCommand>,
        #[command(flatten)]
        args: FeishuServeArgs,
    },
}

#[derive(Subcommand, Debug)]
pub enum FeishuAuthCommand {
    /// Start an interactive login or generate an OAuth authorize URL and persist short-lived state locally
    #[command(visible_alias = "login")]
    Start(FeishuAuthStartArgs),
    /// Exchange an OAuth authorization code for a stored user grant
    Exchange(FeishuAuthExchangeArgs),
    /// List stored user grants for the resolved Feishu account
    List(FeishuAuthListArgs),
    /// Select the default stored user grant for the resolved Feishu account
    Select(FeishuAuthSelectArgs),
    /// Inspect stored grant freshness and required scope coverage
    Status(FeishuGrantArgs),
    /// Delete a stored user grant
    #[command(visible_alias = "logout")]
    Revoke(FeishuGrantArgs),
}

#[derive(Subcommand, Debug)]
pub enum FeishuServeCommand {
    /// Run Feishu in websocket mode (default, no local HTTP listener)
    Websocket(FeishuServeWebsocketArgs),
    /// Run Feishu in webhook mode (local HTTP listener)
    Webhook(FeishuServeWebhookArgs),
}

#[derive(Subcommand, Debug)]
pub enum FeishuDocCommand {
    /// Create a Feishu docx document and optionally insert initial markdown or html content
    Create(FeishuDocCreateArgs),
    /// Append markdown or html content to an existing Feishu docx document
    Append(FeishuDocAppendArgs),
}

#[derive(Subcommand, Debug)]
pub enum FeishuReadCommand {
    /// Read a Feishu docx document
    Doc(FeishuReadDocArgs),
}

#[derive(Subcommand, Debug)]
pub enum FeishuMessagesCommand {
    /// Read message history for a container such as a chat
    History(FeishuMessagesHistoryArgs),
    /// Fetch one message detail record
    Get(FeishuMessagesGetArgs),
    /// Download one explicit image or file resource from a Feishu message; audio and media messages use file resources
    Resource(FeishuMessagesResourceArgs),
}

#[derive(Subcommand, Debug)]
pub enum FeishuSearchCommand {
    /// Search Feishu messages
    Messages(FeishuSearchMessagesArgs),
}

#[derive(Subcommand, Debug)]
pub enum FeishuCalendarCommand {
    /// List calendars or fetch primary calendars
    List(FeishuCalendarListArgs),
    /// Fetch free/busy data for a user or room
    Freebusy(FeishuCalendarFreebusyArgs),
}

#[derive(Subcommand, Debug)]
pub enum FeishuBitableCommand {
    /// Create a Bitable app
    AppCreate(FeishuBitableAppCreateArgs),
    /// Fetch Bitable app metadata
    AppGet(FeishuBitableAppGetArgs),
    /// List Bitable apps through the Drive API
    AppList(FeishuBitableAppListArgs),
    /// Update Bitable app metadata
    AppPatch(FeishuBitableAppPatchArgs),
    /// Copy a Bitable app
    AppCopy(FeishuBitableAppCopyArgs),
    /// List data tables in a Bitable app
    ListTables(FeishuBitableListTablesArgs),
    /// Create a data table in a Bitable app
    CreateTable(FeishuBitableCreateTableArgs),
    /// Rename a data table in a Bitable app
    PatchTable(FeishuBitablePatchTableArgs),
    /// Batch create data tables in a Bitable app
    BatchCreateTables(FeishuBitableBatchCreateTablesArgs),
    /// Create a record in a Bitable table
    CreateRecord(FeishuBitableCreateRecordArgs),
    /// Update a record in a Bitable table
    UpdateRecord(FeishuBitableUpdateRecordArgs),
    /// Delete a record in a Bitable table
    DeleteRecord(FeishuBitableDeleteRecordArgs),
    /// Batch create records in a Bitable table
    BatchCreateRecords(FeishuBitableBatchCreateRecordsArgs),
    /// Batch update records in a Bitable table
    BatchUpdateRecords(FeishuBitableBatchUpdateRecordsArgs),
    /// Batch delete records in a Bitable table
    BatchDeleteRecords(FeishuBitableBatchDeleteRecordsArgs),
    /// Create a field in a Bitable table
    CreateField(FeishuBitableCreateFieldArgs),
    /// List fields in a Bitable table
    ListFields(FeishuBitableListFieldsArgs),
    /// Update a field in a Bitable table
    UpdateField(FeishuBitableUpdateFieldArgs),
    /// Delete a field in a Bitable table
    DeleteField(FeishuBitableDeleteFieldArgs),
    /// Create a view in a Bitable table
    CreateView(FeishuBitableCreateViewArgs),
    /// Get a view in a Bitable table
    GetView(FeishuBitableGetViewArgs),
    /// List views in a Bitable table
    ListViews(FeishuBitableListViewsArgs),
    /// Patch a view in a Bitable table
    PatchView(FeishuBitablePatchViewArgs),
    /// Search records in a Bitable table
    SearchRecords(FeishuBitableSearchRecordsArgs),
}

#[derive(Args, Debug, Clone)]
pub struct FeishuCommonArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub account: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuGrantArgs {
    #[command(flatten)]
    pub common: FeishuCommonArgs,
    #[arg(long)]
    pub open_id: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuAuthStartArgs {
    #[command(flatten)]
    pub common: FeishuCommonArgs,
    #[arg(long, default_value = DEFAULT_FEISHU_REDIRECT_URI)]
    pub redirect_uri: String,
    #[arg(long, default_value_t = false)]
    pub no_launch_browser: bool,
    #[arg(long)]
    pub wait_timeout_s: Option<u64>,
    #[arg(long)]
    pub principal_hint: Option<String>,
    #[arg(long = "scope")]
    pub scopes: Vec<String>,
    #[arg(long = "capability", value_enum)]
    pub capabilities: Vec<FeishuAuthCapability>,
    #[arg(long, default_value_t = false)]
    pub include_message_write: bool,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuAuthExchangeArgs {
    #[command(flatten)]
    pub common: FeishuCommonArgs,
    #[arg(long)]
    pub state: Option<String>,
    #[arg(long)]
    pub code: Option<String>,
    #[arg(long)]
    pub callback_url: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuAuthListArgs {
    #[command(flatten)]
    pub common: FeishuCommonArgs,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuAuthSelectArgs {
    #[command(flatten)]
    pub common: FeishuCommonArgs,
    #[arg(long)]
    pub open_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuReadDocArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub url: String,
    #[arg(long)]
    pub lang: Option<u8>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuDocCreateArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub folder_token: Option<String>,
    #[arg(long, conflicts_with = "content_path")]
    pub content: Option<String>,
    #[arg(long, conflicts_with = "content")]
    pub content_path: Option<String>,
    #[arg(long)]
    pub content_type: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuDocAppendArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub url: String,
    #[arg(
        long,
        required_unless_present = "content_path",
        conflicts_with = "content_path"
    )]
    pub content: Option<String>,
    #[arg(long, required_unless_present = "content", conflicts_with = "content")]
    pub content_path: Option<String>,
    #[arg(long)]
    pub content_type: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuMessagesHistoryArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long, default_value = "chat")]
    pub container_id_type: String,
    #[arg(long)]
    pub container_id: String,
    #[arg(long)]
    pub start_time: Option<String>,
    #[arg(long)]
    pub end_time: Option<String>,
    #[arg(long)]
    pub sort_type: Option<String>,
    #[arg(long)]
    pub page_size: Option<usize>,
    #[arg(long)]
    pub page_token: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuMessagesGetArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub message_id: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FeishuMessageResourceCliType {
    Image,
    #[value(alias = "audio", alias = "media")]
    File,
}

impl FeishuMessageResourceCliType {
    fn as_resource_type(self) -> mvp::channel::feishu::api::FeishuMessageResourceType {
        match self {
            Self::Image => mvp::channel::feishu::api::FeishuMessageResourceType::Image,
            Self::File => mvp::channel::feishu::api::FeishuMessageResourceType::File,
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct FeishuMessagesResourceArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub message_id: String,
    #[arg(long)]
    pub file_key: String,
    #[arg(long = "type", value_enum)]
    pub resource_type: FeishuMessageResourceCliType,
    #[arg(long)]
    pub output: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuSearchMessagesArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub query: String,
    #[arg(long)]
    pub user_id_type: Option<String>,
    #[arg(long, value_delimiter = ',')]
    pub from_ids: Vec<String>,
    #[arg(long, value_delimiter = ',')]
    pub chat_ids: Vec<String>,
    #[arg(long, value_delimiter = ',')]
    pub at_chatter_ids: Vec<String>,
    #[arg(long)]
    pub message_type: Option<String>,
    #[arg(long)]
    pub from_type: Option<String>,
    #[arg(long)]
    pub chat_type: Option<String>,
    #[arg(long)]
    pub start_time: Option<String>,
    #[arg(long)]
    pub end_time: Option<String>,
    #[arg(long)]
    pub page_size: Option<usize>,
    #[arg(long)]
    pub page_token: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuCalendarListArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long, default_value_t = false)]
    pub primary: bool,
    #[arg(long)]
    pub user_id_type: Option<String>,
    #[arg(long)]
    pub page_size: Option<usize>,
    #[arg(long)]
    pub page_token: Option<String>,
    #[arg(long)]
    pub sync_token: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuCalendarFreebusyArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub user_id_type: Option<String>,
    #[arg(long)]
    pub time_min: String,
    #[arg(long)]
    pub time_max: String,
    #[arg(long)]
    pub user_id: Option<String>,
    #[arg(long)]
    pub room_id: Option<String>,
    #[arg(long)]
    pub include_external_calendar: Option<bool>,
    #[arg(long)]
    pub only_busy: Option<bool>,
    #[arg(long)]
    pub need_rsvp_status: Option<bool>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableListTablesArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub page_size: Option<usize>,
    #[arg(long)]
    pub page_token: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableAppCreateArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub folder_token: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableAppGetArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableAppListArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub folder_token: Option<String>,
    #[arg(long)]
    pub page_size: Option<usize>,
    #[arg(long)]
    pub page_token: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableAppPatchArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub is_advanced: Option<bool>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableAppCopyArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub folder_token: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableCreateRecordArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub fields: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableCreateTableArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub default_view_name: Option<String>,
    #[arg(long)]
    pub fields: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitablePatchTableArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub name: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableBatchCreateTablesArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub tables: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableSearchRecordsArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub view_id: Option<String>,
    #[arg(long = "field-name")]
    pub field_names: Vec<String>,
    #[arg(long)]
    pub filter: Option<String>,
    #[arg(long)]
    pub sort: Option<String>,
    #[arg(long)]
    pub automatic_fields: bool,
    #[arg(long)]
    pub page_size: Option<usize>,
    #[arg(long)]
    pub page_token: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableUpdateRecordArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub record_id: String,
    #[arg(long)]
    pub fields: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableDeleteRecordArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub record_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableBatchCreateRecordsArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub records: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableBatchUpdateRecordsArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub records: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableBatchDeleteRecordsArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub records: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableCreateFieldArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub field_name: String,
    #[arg(long = "type")]
    pub field_type: i64,
    #[arg(long)]
    pub property: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableListFieldsArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub view_id: Option<String>,
    #[arg(long)]
    pub page_size: Option<usize>,
    #[arg(long)]
    pub page_token: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableUpdateFieldArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub field_id: String,
    #[arg(long)]
    pub field_name: Option<String>,
    #[arg(long = "type")]
    pub field_type: Option<i64>,
    #[arg(long)]
    pub property: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableDeleteFieldArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub field_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableCreateViewArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub view_name: String,
    #[arg(long)]
    pub view_type: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableGetViewArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub view_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitableListViewsArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub page_size: Option<usize>,
    #[arg(long)]
    pub page_token: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuBitablePatchViewArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub app_token: String,
    #[arg(long)]
    pub table_id: String,
    #[arg(long)]
    pub view_id: String,
    #[arg(long)]
    pub view_name: String,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuSendArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub receive_id_type: Option<String>,
    #[arg(long)]
    pub receive_id: String,
    #[arg(long)]
    pub text: Option<String>,
    #[arg(long = "post-json")]
    pub post_json: Option<String>,
    #[arg(long)]
    pub image_key: Option<String>,
    #[arg(long)]
    pub file_key: Option<String>,
    #[arg(long)]
    pub image_path: Option<String>,
    #[arg(long)]
    pub file_path: Option<String>,
    #[arg(long)]
    pub file_type: Option<String>,
    #[arg(long, default_value_t = false)]
    pub card: bool,
    #[arg(long)]
    pub uuid: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuReplyArgs {
    #[command(flatten)]
    pub grant: FeishuGrantArgs,
    #[arg(long)]
    pub message_id: String,
    #[arg(long)]
    pub text: Option<String>,
    #[arg(long = "post-json")]
    pub post_json: Option<String>,
    #[arg(long)]
    pub image_key: Option<String>,
    #[arg(long)]
    pub file_key: Option<String>,
    #[arg(long)]
    pub image_path: Option<String>,
    #[arg(long)]
    pub file_path: Option<String>,
    #[arg(long)]
    pub file_type: Option<String>,
    #[arg(long, default_value_t = false)]
    pub card: bool,
    #[arg(long, default_value_t = false)]
    pub reply_in_thread: bool,
    #[arg(long)]
    pub uuid: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuServeArgs {
    #[command(flatten)]
    pub common: FeishuCommonArgs,
    #[arg(long, value_enum)]
    pub mode: Option<FeishuServeModeOverride>,
    #[arg(long)]
    pub bind: Option<String>,
    #[arg(long)]
    pub path: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuServeWebsocketArgs {
    #[command(flatten)]
    pub common: FeishuCommonArgs,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuServeWebhookArgs {
    #[command(flatten)]
    pub common: FeishuCommonArgs,
    #[arg(long)]
    pub bind: Option<String>,
    #[arg(long)]
    pub path: Option<String>,
}

fn feishu_serve_args_from_subcommand(command: FeishuServeCommand) -> FeishuServeArgs {
    match command {
        FeishuServeCommand::Websocket(command) => FeishuServeArgs {
            common: command.common,
            mode: Some(FeishuServeModeOverride::Websocket),
            bind: None,
            path: None,
        },
        FeishuServeCommand::Webhook(command) => FeishuServeArgs {
            common: command.common,
            mode: Some(FeishuServeModeOverride::Webhook),
            bind: command.bind,
            path: command.path,
        },
    }
}

pub async fn run_feishu_serve_command(args: &FeishuServeArgs) -> CliResult<()> {
    let (resolved_path, mut config) = mvp::config::load(args.common.config.as_deref())?;
    let account_resolution_hint =
        "rerun with `--account <configured_account_id>` using one of those configured accounts";
    let initial_resolved = mvp::channel::feishu::api::resolve_requested_feishu_account(
        &config.feishu,
        args.common.account.as_deref(),
        account_resolution_hint,
    )?;
    if let Some(mode) = args.mode {
        if mode == FeishuServeModeOverride::Websocket {
            reject_feishu_websocket_listener_overrides(args.bind.as_deref(), args.path.as_deref())?;
        }
        apply_feishu_mode_override(&mut config, &initial_resolved.configured_account_id, mode);
    }
    let resolved = mvp::channel::feishu::api::resolve_requested_feishu_account(
        &config.feishu,
        args.common.account.as_deref(),
        account_resolution_hint,
    )?;
    print_feishu_serve_preflight(&resolved, args.bind.as_deref(), args.path.as_deref());
    crate::with_graceful_shutdown(mvp::channel::run_feishu_channel_with_stop(
        resolved_path,
        config,
        args.common.account.as_deref(),
        args.bind.as_deref(),
        args.path.as_deref(),
        mvp::channel::ChannelServeStopHandle::new(),
        true,
    ))
    .await
}

async fn run_feishu_auth_command(command: FeishuAuthCommand) -> CliResult<()> {
    match command {
        FeishuAuthCommand::Start(args) => {
            if args.common.json {
                let payload = execute_feishu_auth_start(&args).await?;
                print_feishu_payload(&payload, args.common.json, render_auth_start_text)?;
            } else if is_loopback_callback_redirect_uri(args.redirect_uri.as_str()) {
                run_feishu_auth_start_loopback_flow(&args).await?;
            } else {
                let payload = execute_feishu_auth_start(&args).await?;
                print_feishu_payload(&payload, args.common.json, render_auth_start_text)?;
                if args.no_launch_browser {
                    println!("browser: disabled (--no-launch-browser)");
                } else {
                    let authorize_url = required_json_string(&payload, "authorize_url")?;
                    match open_url_in_default_browser(authorize_url.as_str()) {
                        Ok(()) => println!("browser: launched authorize_url in default browser"),
                        Err(error) => {
                            eprintln!("warning: {error}");
                            eprintln!("manual step: open this URL in a browser: {authorize_url}");
                        }
                    }
                }
            }
        }
        FeishuAuthCommand::Exchange(args) => {
            let payload = execute_feishu_auth_exchange(&args).await?;
            print_feishu_payload(&payload, args.common.json, render_auth_exchange_text)?;
        }
        FeishuAuthCommand::List(args) => {
            let payload = execute_feishu_auth_list(&args).await?;
            print_feishu_payload(&payload, args.common.json, render_auth_list_text)?;
        }
        FeishuAuthCommand::Select(args) => {
            let payload = execute_feishu_auth_select(&args).await?;
            print_feishu_payload(&payload, args.common.json, render_auth_select_text)?;
        }
        FeishuAuthCommand::Status(args) => {
            let payload = execute_feishu_auth_status(&args).await?;
            print_feishu_payload(&payload, args.common.json, render_auth_status_text)?;
        }
        FeishuAuthCommand::Revoke(args) => {
            let payload = execute_feishu_auth_revoke(&args).await?;
            print_feishu_payload(&payload, args.common.json, render_auth_revoke_text)?;
        }
    }
    Ok(())
}

pub async fn run_feishu_command(command: FeishuCommand) -> CliResult<()> {
    match command {
        FeishuCommand::Auth { command } => run_feishu_auth_command(command).await?,
        FeishuCommand::Whoami(args) => {
            let payload = execute_feishu_whoami(&args).await?;
            print_feishu_payload(&payload, args.common.json, render_whoami_text)?;
        }
        FeishuCommand::Doc { command } => match command {
            FeishuDocCommand::Create(args) => {
                let payload = execute_feishu_doc_create(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_doc_create_text)?;
            }
            FeishuDocCommand::Append(args) => {
                let payload = execute_feishu_doc_append(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_doc_append_text)?;
            }
        },
        FeishuCommand::Read { command } => match command {
            FeishuReadCommand::Doc(args) => {
                let payload = execute_feishu_read_doc(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_read_doc_text)?;
            }
        },
        FeishuCommand::Messages { command } => match command {
            FeishuMessagesCommand::History(args) => {
                let payload = execute_feishu_messages_history(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_messages_history_text,
                )?;
            }
            FeishuMessagesCommand::Get(args) => {
                let payload = execute_feishu_messages_get(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_messages_get_text)?;
            }
            FeishuMessagesCommand::Resource(args) => {
                let payload = execute_feishu_messages_resource(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_messages_resource_text,
                )?;
            }
        },
        FeishuCommand::Search { command } => match command {
            FeishuSearchCommand::Messages(args) => {
                let payload = execute_feishu_search_messages(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_search_messages_text,
                )?;
            }
        },
        FeishuCommand::Calendar { command } => match command {
            FeishuCalendarCommand::List(args) => {
                let payload = execute_feishu_calendar_list(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_calendar_list_text)?;
            }
            FeishuCalendarCommand::Freebusy(args) => {
                let payload = execute_feishu_calendar_freebusy(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_calendar_freebusy_text,
                )?;
            }
        },
        FeishuCommand::Bitable { command } => match command {
            FeishuBitableCommand::AppCreate(args) => {
                let payload = execute_feishu_bitable_app_create(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_bitable_app_text)?;
            }
            FeishuBitableCommand::AppGet(args) => {
                let payload = execute_feishu_bitable_app_get(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_bitable_app_text)?;
            }
            FeishuBitableCommand::AppList(args) => {
                let payload = execute_feishu_bitable_app_list(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_bitable_app_list_text,
                )?;
            }
            FeishuBitableCommand::AppPatch(args) => {
                let payload = execute_feishu_bitable_app_patch(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_bitable_app_text)?;
            }
            FeishuBitableCommand::AppCopy(args) => {
                let payload = execute_feishu_bitable_app_copy(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_bitable_app_text)?;
            }
            FeishuBitableCommand::ListTables(args) => {
                let payload = execute_feishu_bitable_list_tables(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_bitable_list_tables_text,
                )?;
            }
            FeishuBitableCommand::CreateTable(args) => {
                let payload = execute_feishu_bitable_create_table(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_bitable_table_text)?;
            }
            FeishuBitableCommand::PatchTable(args) => {
                let payload = execute_feishu_bitable_patch_table(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_bitable_table_text)?;
            }
            FeishuBitableCommand::BatchCreateTables(args) => {
                let payload = execute_feishu_bitable_batch_create_tables(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_bitable_table_batch_create_text,
                )?;
            }
            FeishuBitableCommand::CreateRecord(args) => {
                let payload = execute_feishu_bitable_create_record(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_bitable_create_record_text,
                )?;
            }
            FeishuBitableCommand::UpdateRecord(args) => {
                let payload = execute_feishu_bitable_update_record(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_bitable_create_record_text,
                )?;
            }
            FeishuBitableCommand::DeleteRecord(args) => {
                let payload = execute_feishu_bitable_delete_record(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_bitable_delete_record_text,
                )?;
            }
            FeishuBitableCommand::BatchCreateRecords(args) => {
                let payload = execute_feishu_bitable_batch_create_records(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_bitable_batch_records_text,
                )?;
            }
            FeishuBitableCommand::BatchUpdateRecords(args) => {
                let payload = execute_feishu_bitable_batch_update_records(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_bitable_batch_records_text,
                )?;
            }
            FeishuBitableCommand::BatchDeleteRecords(args) => {
                let payload = execute_feishu_bitable_batch_delete_records(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_bitable_batch_records_text,
                )?;
            }
            FeishuBitableCommand::CreateField(args) => {
                let payload = execute_feishu_bitable_create_field(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_bitable_field_text)?;
            }
            FeishuBitableCommand::ListFields(args) => {
                let payload = execute_feishu_bitable_list_fields(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_bitable_field_list_text,
                )?;
            }
            FeishuBitableCommand::UpdateField(args) => {
                let payload = execute_feishu_bitable_update_field(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_bitable_field_text)?;
            }
            FeishuBitableCommand::DeleteField(args) => {
                let payload = execute_feishu_bitable_delete_field(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_bitable_delete_field_text,
                )?;
            }
            FeishuBitableCommand::CreateView(args) => {
                let payload = execute_feishu_bitable_create_view(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_bitable_view_text)?;
            }
            FeishuBitableCommand::GetView(args) => {
                let payload = execute_feishu_bitable_get_view(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_bitable_view_text)?;
            }
            FeishuBitableCommand::ListViews(args) => {
                let payload = execute_feishu_bitable_list_views(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_bitable_view_list_text,
                )?;
            }
            FeishuBitableCommand::PatchView(args) => {
                let payload = execute_feishu_bitable_patch_view(&args).await?;
                print_feishu_payload(&payload, args.grant.common.json, render_bitable_view_text)?;
            }
            FeishuBitableCommand::SearchRecords(args) => {
                let payload = execute_feishu_bitable_search_records(&args).await?;
                print_feishu_payload(
                    &payload,
                    args.grant.common.json,
                    render_bitable_search_records_text,
                )?;
            }
        },
        FeishuCommand::Send(args) => {
            let payload = execute_feishu_send(&args).await?;
            print_feishu_payload(&payload, args.grant.common.json, render_send_text)?;
        }
        FeishuCommand::Reply(args) => {
            let payload = execute_feishu_reply(&args).await?;
            print_feishu_payload(&payload, args.grant.common.json, render_reply_text)?;
        }
        FeishuCommand::Serve { command, args } => {
            let args = command
                .map(feishu_serve_args_from_subcommand)
                .unwrap_or(args);
            run_feishu_serve_command(&args).await?;
        }
    }
    Ok(())
}

fn resolve_feishu_auth_exchange_input(
    args: &FeishuAuthExchangeArgs,
) -> CliResult<(String, String, Option<String>)> {
    if let Some(callback_url) = args
        .callback_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if args
            .state
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            || args
                .code
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
        {
            return Err(
                "feishu auth exchange accepts either --callback-url or --state/--code, not both"
                    .to_owned(),
            );
        }

        let url = Url::parse(callback_url)
            .map_err(|error| format!("parse callback url `{callback_url}` failed: {error}"))?;
        let mut state = None;
        let mut code = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "state" if state.is_none() => state = Some(value.into_owned()),
                "code" if code.is_none() => code = Some(value.into_owned()),
                _ => {}
            }
        }
        let state = state
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!("callback url `{callback_url}` is missing query parameter `state`")
            })?;
        let code = code
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!("callback url `{callback_url}` is missing query parameter `code`")
            })?;
        return Ok((state, code, Some(callback_url.to_owned())));
    }

    let state = args
        .state
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "feishu auth exchange requires --callback-url or both --state and --code".to_owned()
        })?
        .to_owned();
    let code = args
        .code
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "feishu auth exchange requires --callback-url or both --state and --code".to_owned()
        })?
        .to_owned();
    Ok((state, code, None))
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

fn is_loopback_callback_redirect_uri(redirect_uri: &str) -> bool {
    let redirect_uri = redirect_uri.trim();
    if redirect_uri.is_empty() {
        return false;
    }
    let Ok(url) = Url::parse(redirect_uri) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https") && url.host_str().is_some_and(is_loopback_host)
}

fn parse_feishu_loopback_redirect_spec(
    redirect_uri: &str,
) -> CliResult<FeishuLoopbackRedirectSpec> {
    let redirect_uri = redirect_uri.trim();
    let url = Url::parse(redirect_uri)
        .map_err(|error| format!("parse Feishu redirect uri `{redirect_uri}` failed: {error}"))?;
    if url.scheme() != "http" {
        return Err(format!(
            "loopback callback listener only supports http redirect URIs, found `{}`",
            url.scheme()
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| format!("redirect uri `{redirect_uri}` is missing host"))?;
    if !is_loopback_host(host) {
        return Err(format!(
            "redirect uri `{redirect_uri}` is not loopback; automatic callback listener is disabled"
        ));
    }
    let port = url.port_or_known_default().ok_or_else(|| {
        format!("redirect uri `{redirect_uri}` is missing an explicit or known port")
    })?;
    let bind_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let mut redirect_base = url.clone();
    redirect_base.set_query(None);
    redirect_base.set_fragment(None);
    let path = match url.path().trim() {
        "" => "/".to_owned(),
        path => path.to_owned(),
    };
    Ok(FeishuLoopbackRedirectSpec {
        redirect_uri: redirect_base.to_string(),
        bind_target: format!("{bind_host}:{port}"),
        path,
    })
}

async fn feishu_local_callback_handler(
    State(state): State<FeishuLocalCallbackState>,
    Query(query): Query<std::collections::BTreeMap<String, String>>,
) -> (StatusCode, Html<String>) {
    let state_value = query
        .get("state")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let code_value = query
        .get("code")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let Some(state_value) = state_value else {
        return (
            StatusCode::BAD_REQUEST,
            Html(
                "<html><body><h1>Feishu authorization callback missing state</h1><p>Return to the terminal and retry the authorization flow.</p></body></html>"
                    .to_owned(),
            ),
        );
    };
    if state_value != state.expected_state {
        return (
            StatusCode::BAD_REQUEST,
            Html(
                "<html><body><h1>Feishu authorization callback state mismatch</h1><p>Return to the terminal and retry the authorization flow.</p></body></html>"
                    .to_owned(),
            ),
        );
    }
    let Some(code_value) = code_value else {
        return (
            StatusCode::BAD_REQUEST,
            Html(
                "<html><body><h1>Feishu authorization callback missing code</h1><p>Return to the terminal and retry the authorization flow.</p></body></html>"
                    .to_owned(),
            ),
        );
    };

    let callback_url = format!(
        "{}?code={}&state={}",
        state.redirect_uri, code_value, state_value
    );
    let callback = FeishuCapturedLocalCallback {
        state: state_value.to_owned(),
        code: code_value.to_owned(),
        callback_url,
    };
    let mut sender_guard = state.callback_sender.lock().await;
    if let Some(sender) = sender_guard.take() {
        let _ = sender.send(callback);
        return (
            StatusCode::OK,
            Html(
                "<html><body><h1>Feishu authorization received</h1><p>You can return to the terminal while LoongClaw finishes the token exchange.</p></body></html>"
                    .to_owned(),
            ),
        );
    }

    (
        StatusCode::OK,
        Html(
            "<html><body><h1>Feishu authorization already received</h1><p>Return to the terminal if the first browser tab is still waiting.</p></body></html>"
                .to_owned(),
        ),
    )
}

async fn start_feishu_local_callback_server(
    redirect_uri: &str,
    expected_state: &str,
) -> CliResult<FeishuLocalCallbackServer> {
    let spec = parse_feishu_loopback_redirect_spec(redirect_uri)?;
    let listener = tokio::net::TcpListener::bind(spec.bind_target.as_str())
        .await
        .map_err(|error| {
            format!(
                "start local Feishu callback listener on {} failed: {error}",
                spec.bind_target
            )
        })?;
    let (callback_sender, callback_receiver) = oneshot::channel();
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let state = FeishuLocalCallbackState {
        redirect_uri: spec.redirect_uri,
        expected_state: expected_state.trim().to_owned(),
        callback_sender: Arc::new(Mutex::new(Some(callback_sender))),
    };
    let app = Router::new()
        .route(spec.path.as_str(), get(feishu_local_callback_handler))
        .with_state(state);
    let join_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_receiver.await;
            })
            .await
            .map_err(|error| format!("local Feishu callback listener stopped: {error}"))
    });
    Ok(FeishuLocalCallbackServer {
        callback_receiver,
        shutdown_sender: Some(shutdown_sender),
        join_handle,
    })
}

async fn wait_for_feishu_local_callback(
    mut server: FeishuLocalCallbackServer,
    timeout: Duration,
) -> CliResult<Option<FeishuCapturedLocalCallback>> {
    let callback_result = tokio::time::timeout(timeout, &mut server.callback_receiver).await;
    if let Some(shutdown_sender) = server.shutdown_sender.take() {
        let _ = shutdown_sender.send(());
    }
    let join_result = server
        .join_handle
        .await
        .map_err(|error| format!("join local Feishu callback listener task failed: {error}"))?;
    match callback_result {
        Ok(Ok(callback)) => {
            join_result?;
            Ok(Some(callback))
        }
        Ok(Err(_)) => {
            Err("local Feishu callback listener closed before receiving a callback".to_owned())
        }
        Err(_) => {
            join_result?;
            Ok(None)
        }
    }
}

fn update_auth_start_payload_for_loopback_listener(
    payload: &mut Value,
    redirect_uri: &str,
    exchange_command: &str,
    wait_timeout_s: u64,
) {
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    object.insert(
        "flow".to_owned(),
        Value::String("local_callback_listener".to_owned()),
    );
    object.insert("listener_started".to_owned(), Value::Bool(true));
    object.insert(
        "manual_note".to_owned(),
        Value::String(format!(
            "LoongClaw is waiting for a loopback callback on {redirect_uri}. Open the authorize URL, finish the browser grant, and this command will exchange the code automatically. If the listener stops or times out after {wait_timeout_s}s, fallback to `{exchange_command}`."
        )),
    );
}

async fn run_feishu_auth_start_loopback_flow(args: &FeishuAuthStartArgs) -> CliResult<()> {
    let payload = execute_feishu_auth_start(args).await?;
    let redirect_uri = required_json_string(&payload, "redirect_uri")?.to_owned();
    let expected_state = required_json_string(&payload, "state")?.to_owned();
    let exchange_command = payload
        .get("exchange_command")
        .and_then(Value::as_str)
        .unwrap_or("loong feishu auth exchange --callback-url '<full_callback_url>'")
        .to_owned();
    let authorize_url = required_json_string(&payload, "authorize_url")?.to_owned();
    let wait_timeout_s = normalize_feishu_callback_wait_timeout_s(args.wait_timeout_s);

    let server =
        match start_feishu_local_callback_server(redirect_uri.as_str(), expected_state.as_str())
            .await
        {
            Ok(server) => server,
            Err(error) => {
                print_feishu_payload(&payload, false, render_auth_start_text)?;
                eprintln!("{error}");
                eprintln!(
                    "manual fallback: finish browser authorization, then run `{exchange_command}`"
                );
                return Ok(());
            }
        };

    let mut display_payload = payload.clone();
    update_auth_start_payload_for_loopback_listener(
        &mut display_payload,
        redirect_uri.as_str(),
        exchange_command.as_str(),
        wait_timeout_s,
    );
    print_feishu_payload(&display_payload, false, render_auth_start_text)?;
    if args.no_launch_browser {
        println!("browser: disabled (--no-launch-browser)");
    } else {
        match open_url_in_default_browser(authorize_url.as_str()) {
            Ok(()) => println!("browser: launched authorize_url in default browser"),
            Err(error) => {
                eprintln!("warning: {error}");
                eprintln!("manual fallback: open this URL in a browser: {authorize_url}");
            }
        }
    }
    println!(
        "listener: waiting for Feishu callback on {} (timeout={}s)",
        redirect_uri, wait_timeout_s
    );

    let callback =
        wait_for_feishu_local_callback(server, Duration::from_secs(wait_timeout_s)).await?;
    let Some(callback) = callback else {
        eprintln!("warning: timed out waiting for local Feishu callback on {redirect_uri}");
        eprintln!(
            "manual fallback: copy the full browser callback URL and run `{exchange_command}`"
        );
        return Ok(());
    };

    let exchange_payload = execute_feishu_auth_exchange(&FeishuAuthExchangeArgs {
        common: args.common.clone(),
        state: None,
        code: None,
        callback_url: Some(callback.callback_url),
    })
    .await?;
    print_feishu_payload(&exchange_payload, false, render_auth_exchange_text)
}

pub async fn execute_feishu_auth_start(args: &FeishuAuthStartArgs) -> CliResult<Value> {
    let context = load_feishu_daemon_context(
        args.common.config.as_deref(),
        args.common.account.as_deref(),
    )?;
    let client = context.build_client()?;
    let capabilities =
        normalized_auth_start_capabilities(&args.capabilities, args.include_message_write);
    let scopes = resolve_scopes(
        &context.default_scopes(),
        &args.scopes,
        &capabilities,
        args.include_message_write,
    );
    let state = generate_oauth_state();
    let (code_verifier, code_challenge) = build_pkce_pair();
    let now_s = unix_ts_now();
    let record = mvp::channel::feishu::api::FeishuOauthStateRecord {
        state: state.clone(),
        account_id: context.account_id().to_owned(),
        principal_hint: args.principal_hint.clone().unwrap_or_default(),
        scope_csv: scopes.join(" "),
        redirect_uri: Some(args.redirect_uri.trim().to_owned()),
        code_verifier: Some(code_verifier),
        expires_at_s: now_s + context.config.feishu_integration.oauth_state_ttl_s as i64,
        created_at_s: now_s,
    };
    context.store.save_oauth_state_record(&record)?;
    let authorize_url = mvp::channel::feishu::api::build_authorize_url(
        &mvp::channel::feishu::api::FeishuAuthStartSpec {
            app_id: client.app_id().to_owned(),
            redirect_uri: args.redirect_uri.trim().to_owned(),
            scopes: scopes.clone(),
            state: state.clone(),
            code_challenge: Some(code_challenge),
            code_challenge_method: Some("S256".to_owned()),
        },
    )?;
    let exchange_command =
        feishu_auth_exchange_command_hint(context.resolved.configured_account_id.as_str());
    let loopback_redirect = is_loopback_callback_redirect_uri(args.redirect_uri.as_str());

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "config": context.config_path.display().to_string(),
        "redirect_uri": args.redirect_uri.trim(),
        "state": state,
        "authorize_url": authorize_url,
        "sqlite_path": context.store.path().display().to_string(),
        "expires_at_s": record.expires_at_s,
        "capabilities": capabilities
            .iter()
            .map(|capability| capability.as_cli_value())
            .collect::<Vec<_>>(),
        "scopes": scopes,
        "flow": "manual_exchange",
        "listener_started": false,
        "exchange_command": exchange_command,
        "loopback_redirect_uri": loopback_redirect,
        "manual_note": if loopback_redirect {
            "LoongClaw could not use the automatic loopback listener for `feishu auth login`; if the browser shows connection refused after authorization, copy the full callback URL from the address bar and run the exchange command."
        } else {
            "LoongClaw stores OAuth state locally and expects a follow-up `feishu auth exchange` command to finish the grant."
        },
    }))
}

pub async fn execute_feishu_auth_exchange(args: &FeishuAuthExchangeArgs) -> CliResult<Value> {
    let context = load_feishu_daemon_context(
        args.common.config.as_deref(),
        args.common.account.as_deref(),
    )?;
    let (state, code, callback_url) = resolve_feishu_auth_exchange_input(args)?;
    let now_s = unix_ts_now();
    let stored_state = context.store.consume_oauth_state(&state, now_s)?;
    if stored_state.account_id.trim() != context.account_id() {
        return Err(format!(
            "oauth state belongs to account `{}` but current command resolved `{}`",
            stored_state.account_id,
            context.account_id()
        ));
    }
    let client = context.build_client()?;
    let scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes(
        stored_state.scope_csv.split_whitespace(),
    );
    let payload = client
        .exchange_authorization_code(
            &code,
            stored_state.redirect_uri.as_deref(),
            &scopes,
            stored_state.code_verifier.as_deref(),
        )
        .await?;
    let user_access_token = required_json_string(&payload, "access_token")?;
    let user_info = client.get_user_info(&user_access_token).await?;
    let principal =
        mvp::channel::feishu::api::map_user_info_to_principal(context.account_id(), &user_info)?;
    let grant = mvp::channel::feishu::api::parse_token_exchange_response(
        &payload,
        now_s,
        principal.clone(),
    )?;
    context.store.save_grant(&grant)?;
    context
        .store
        .set_selected_grant(context.account_id(), &principal.open_id, now_s)?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "config": context.config_path.display().to_string(),
        "principal": principal,
        "granted_scopes": grant.scopes.as_slice(),
        "access_expires_at_s": grant.access_expires_at_s,
        "refresh_expires_at_s": grant.refresh_expires_at_s,
        "selected_open_id": grant.principal.open_id,
        "effective_open_id": grant.principal.open_id,
        "callback_url": callback_url,
    }))
}

pub async fn execute_feishu_auth_list(args: &FeishuAuthListArgs) -> CliResult<Value> {
    let context = load_feishu_daemon_context(
        args.common.config.as_deref(),
        args.common.account.as_deref(),
    )?;
    let required_scopes = context.default_scopes();
    let now_s = unix_ts_now();
    let inventory = mvp::channel::feishu::api::inspect_grants_for_account(
        &context.store,
        context.account_id(),
    )?;
    let effective_open_id = inventory.effective_open_id.clone();
    let recommendations =
        build_account_recommendations(context.resolved.configured_account_id.as_str(), &inventory);
    let grants = inventory
        .grants
        .iter()
        .map(|grant| {
            serialize_grant_summary(
                grant,
                context.resolved.configured_account_id.as_str(),
                now_s,
                &required_scopes,
                inventory.selected_open_id.as_deref(),
                effective_open_id.as_deref(),
            )
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "config": context.config_path.display().to_string(),
        "grant_count": grants.len(),
        "selected_open_id": inventory.selected_open_id,
        "effective_open_id": effective_open_id,
        "recommendations": recommendations,
        "required_scopes": required_scopes,
        "grants": grants,
    }))
}

pub async fn execute_feishu_auth_select(args: &FeishuAuthSelectArgs) -> CliResult<Value> {
    let context = load_feishu_daemon_context(
        args.common.config.as_deref(),
        args.common.account.as_deref(),
    )?;
    let open_id = args.open_id.trim();
    let grant = context
        .store
        .load_grant(context.account_id(), open_id)?
        .ok_or_else(|| {
            let cli = active_cli_command_name();
            format!(
                "no stored Feishu grant for account `{}` and open_id `{}`; run `{cli} feishu auth list --account {}` first",
                context.resolved.configured_account_id,
                open_id,
                context.resolved.configured_account_id
            )
        })?;
    let now_s = unix_ts_now();
    context
        .store
        .set_selected_grant(context.account_id(), open_id, now_s)?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "config": context.config_path.display().to_string(),
        "selected_open_id": open_id,
        "effective_open_id": open_id,
        "grant": serialize_grant_summary(
            &grant,
            context.resolved.configured_account_id.as_str(),
            now_s,
            &context.default_scopes(),
            Some(open_id),
            Some(open_id),
        ),
    }))
}

pub async fn execute_feishu_auth_status(args: &FeishuGrantArgs) -> CliResult<Value> {
    let context = load_feishu_daemon_context(
        args.common.config.as_deref(),
        args.common.account.as_deref(),
    )?;
    let required_scopes = context.default_scopes();
    let now_s = unix_ts_now();
    let inventory = mvp::channel::feishu::api::inspect_grants_for_account(
        &context.store,
        context.account_id(),
    )?;
    let explicit_open_id = args
        .open_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let effective_open_id =
        mvp::channel::feishu::api::effective_selected_open_id(&inventory, explicit_open_id)
            .map(str::to_owned);

    if explicit_open_id.is_none() && inventory.selection_required() {
        let recommendations = build_account_recommendations(
            context.resolved.configured_account_id.as_str(),
            &inventory,
        );
        let grants = inventory
            .grants
            .iter()
            .map(|grant| {
                serialize_grant_summary(
                    grant,
                    context.resolved.configured_account_id.as_str(),
                    now_s,
                    &required_scopes,
                    inventory.selected_open_id.as_deref(),
                    effective_open_id.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        return Ok(json!({
            "account_id": context.account_id(),
            "configured_account": context.resolved.configured_account_label,
            "config": context.config_path.display().to_string(),
            "status_scope": "account",
            "grant_count": grants.len(),
            "selected_open_id": inventory.selected_open_id,
            "effective_open_id": effective_open_id,
            "recommendations": recommendations,
            "required_scopes": required_scopes,
            "grants": grants,
        }));
    }
    let resolution = mvp::channel::feishu::api::resolve_grant_selection(
        &context.store,
        context.account_id(),
        explicit_open_id,
    )?;
    let requested_open_id = resolution.requested_open_id.clone();
    let available_open_ids = resolution
        .missing_explicit_open_id()
        .map(|_| {
            resolution
                .available_open_ids()
                .into_iter()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let effective_open_id = resolution.effective_open_id().map(str::to_owned);
    let grant = resolution.selected_grant().cloned();
    if requested_open_id.is_some() && grant.is_none() && !available_open_ids.is_empty() {
        return Ok(json!({
            "account_id": context.account_id(),
            "configured_account": context.resolved.configured_account_label,
            "config": context.config_path.display().to_string(),
            "status_scope": "grant",
            "requested_open_id": requested_open_id,
            "available_open_ids": available_open_ids,
            "status": mvp::channel::feishu::api::auth::summarize_grant_status(None, now_s, &required_scopes),
            "doc_write_status": mvp::channel::feishu::api::summarize_doc_write_scope_status(None),
            "message_write_status": mvp::channel::feishu::api::summarize_message_write_scope_status(None),
            "recommendations": crate::feishu_support::FeishuGrantRecommendations {
                auth_start_command: None,
                select_command: Some(crate::feishu_support::feishu_auth_select_command_hint(
                    context.resolved.configured_account_id.as_str(),
                )),
                missing_required_scopes: Vec::new(),
                missing_doc_write_scope: false,
                missing_message_write_scope: false,
                requested_open_id_missing: true,
                refresh_token_expired: false,
            },
            "selected_open_id": inventory.selected_open_id,
            "effective_open_id": effective_open_id,
            "grant": Value::Null,
            "required_scopes": required_scopes,
        }));
    }
    let status = mvp::channel::feishu::api::auth::summarize_grant_status(
        grant.as_ref(),
        now_s,
        &required_scopes,
    );

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "config": context.config_path.display().to_string(),
        "status_scope": "grant",
        "requested_open_id": requested_open_id,
        "available_open_ids": available_open_ids,
        "status": status,
        "doc_write_status": mvp::channel::feishu::api::summarize_doc_write_scope_status(grant.as_ref()),
        "message_write_status": mvp::channel::feishu::api::summarize_message_write_scope_status(grant.as_ref()),
        "recommendations": build_grant_recommendations(
            context.resolved.configured_account_id.as_str(),
            grant.as_ref(),
            now_s,
            &required_scopes,
        ),
        "selected_open_id": inventory.selected_open_id,
        "effective_open_id": effective_open_id,
        "grant": grant
            .as_ref()
            .map(|value| {
                serialize_grant_summary(
                    value,
                    context.resolved.configured_account_id.as_str(),
                    now_s,
                    &required_scopes,
                    inventory.selected_open_id.as_deref(),
                    effective_open_id.as_deref(),
                )
            }),
        "required_scopes": required_scopes,
    }))
}

pub async fn execute_feishu_auth_revoke(args: &FeishuGrantArgs) -> CliResult<Value> {
    let context = load_feishu_daemon_context(
        args.common.config.as_deref(),
        args.common.account.as_deref(),
    )?;
    let resolution = mvp::channel::feishu::api::resolve_grant_selection(
        &context.store,
        context.account_id(),
        args.open_id.as_deref(),
    )?;
    if resolution.selected_grant().is_none()
        && (resolution.selection_required() || resolution.missing_explicit_open_id().is_some())
    {
        return Err(describe_grant_selection_error(&context, &resolution));
    }
    let (deleted, deleted_open_id) = if let Some(grant) = resolution.selected_grant() {
        let deleted = context
            .store
            .delete_grant(context.account_id(), grant.principal.open_id.as_str())?;
        (deleted, Some(grant.principal.open_id.clone()))
    } else {
        (false, args.open_id.clone())
    };
    let inventory = mvp::channel::feishu::api::inspect_grants_for_account(
        &context.store,
        context.account_id(),
    )?;
    let recommendations =
        build_account_recommendations(context.resolved.configured_account_id.as_str(), &inventory);

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "config": context.config_path.display().to_string(),
        "deleted": deleted,
        "open_id": deleted_open_id,
        "grant_count": inventory.grants.len(),
        "selected_open_id": inventory.selected_open_id,
        "effective_open_id": inventory.effective_open_id,
        "recommendations": recommendations,
    }))
}

pub async fn execute_feishu_whoami(args: &FeishuGrantArgs) -> CliResult<Value> {
    let context = load_feishu_daemon_context(
        args.common.config.as_deref(),
        args.common.account.as_deref(),
    )?;
    let grant = require_selected_grant(&context, args.open_id.as_deref())?;
    let client = context.build_client()?;
    let grant =
        mvp::channel::feishu::api::ensure_fresh_user_grant(&client, &context.store, &grant).await?;
    let user_info = client.get_user_info(&grant.access_token).await?;
    let principal =
        mvp::channel::feishu::api::map_user_info_to_principal(context.account_id(), &user_info)?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "config": context.config_path.display().to_string(),
        "principal": principal,
        "user_info": user_info,
        "grant_scopes": grant.scopes.as_slice(),
    }))
}

pub async fn execute_feishu_read_doc(args: &FeishuReadDocArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    let client = context.build_client()?;
    let document = mvp::channel::feishu::api::resources::docs::fetch_document_content(
        &client,
        &grant.access_token,
        &args.url,
        args.lang,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "document": document,
    }))
}

pub async fn execute_feishu_doc_create(args: &FeishuDocCreateArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    let action = format!("{} feishu doc create", active_cli_command_name());
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        mvp::channel::feishu::api::FEISHU_DOC_WRITE_ACCEPTED_SCOPES,
        action.as_str(),
    )?;
    let client = context.build_client()?;
    let initial_content = prepare_feishu_doc_cli_content(
        action.as_str(),
        args.content.as_deref(),
        args.content_path.as_deref(),
        args.content_type.as_deref(),
        false,
    )?;
    let document = mvp::channel::feishu::api::resources::docs::create_document(
        &client,
        &grant.access_token,
        args.title.as_deref(),
        args.folder_token.as_deref(),
    )
    .await?;

    let mut content_inserted = false;
    let mut inserted_block_count = 0_usize;
    let mut insert_batch_count = 0_usize;
    if let Some(initial_content) = initial_content.as_ref() {
        let converted = mvp::channel::feishu::api::resources::docs::convert_content_to_blocks(
            &client,
            &grant.access_token,
            initial_content.content_type,
            initial_content.content.as_str(),
        )
        .await?;
        let insert_summary = mvp::channel::feishu::api::resources::docs::create_nested_blocks(
            &client,
            &grant.access_token,
            document.document_id.as_str(),
            &converted,
        )
        .await?;
        inserted_block_count = insert_summary.inserted_block_count;
        insert_batch_count = insert_summary.batch_count;
        content_inserted = true;
    }

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "document": document,
        "content_inserted": content_inserted,
        "inserted_block_count": inserted_block_count,
        "insert_batch_count": insert_batch_count,
        "content_type": initial_content.as_ref().map(|content| content.content_type),
    }))
}

pub async fn execute_feishu_doc_append(args: &FeishuDocAppendArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    let action = format!("{} feishu doc append", active_cli_command_name());
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        mvp::channel::feishu::api::FEISHU_DOC_WRITE_ACCEPTED_SCOPES,
        action.as_str(),
    )?;
    let client = context.build_client()?;
    let url = args.url.trim();
    if url.is_empty() {
        return Err(format!("{action} requires --url"));
    }
    let content = prepare_feishu_doc_cli_content(
        action.as_str(),
        args.content.as_deref(),
        args.content_path.as_deref(),
        args.content_type.as_deref(),
        true,
    )?
    .ok_or_else(|| format!("{action} requires --content or --content-path"))?;
    let document_id = mvp::channel::feishu::api::resources::docs::extract_document_id(url)
        .ok_or_else(|| "failed to resolve Feishu document id".to_owned())?;
    let converted = mvp::channel::feishu::api::resources::docs::convert_content_to_blocks(
        &client,
        &grant.access_token,
        content.content_type,
        content.content.as_str(),
    )
    .await?;
    let insert_summary = mvp::channel::feishu::api::resources::docs::create_nested_blocks(
        &client,
        &grant.access_token,
        document_id.as_str(),
        &converted,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "document": {
            "document_id": document_id.clone(),
            "url": format!("https://open.feishu.cn/docx/{document_id}")
        },
        "inserted_block_count": insert_summary.inserted_block_count,
        "insert_batch_count": insert_summary.batch_count,
        "content_type": content.content_type,
    }))
}

pub async fn execute_feishu_messages_history(args: &FeishuMessagesHistoryArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    let client = context.build_client()?;
    let tenant_access_token = client.get_tenant_access_token().await?;
    let page = mvp::channel::feishu::api::resources::messages::fetch_message_history(
        &client,
        &tenant_access_token,
        &mvp::channel::feishu::api::resources::messages::FeishuMessageHistoryQuery {
            container_id_type: args.container_id_type.clone(),
            container_id: args.container_id.clone(),
            start_time: args.start_time.clone(),
            end_time: args.end_time.clone(),
            sort_type: args.sort_type.clone(),
            page_size: args.page_size,
            page_token: args.page_token.clone(),
        },
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "page": page,
    }))
}

pub async fn execute_feishu_messages_get(args: &FeishuMessagesGetArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    let client = context.build_client()?;
    let tenant_access_token = client.get_tenant_access_token().await?;
    let message = mvp::channel::feishu::api::resources::messages::fetch_message_detail(
        &client,
        &tenant_access_token,
        &args.message_id,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "message": message,
    }))
}

pub async fn execute_feishu_messages_resource(
    args: &FeishuMessagesResourceArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    let client = context.build_client()?;
    let tenant_access_token = client.get_tenant_access_token().await?;
    let resource = mvp::channel::feishu::api::resources::media::download_message_resource(
        &client,
        &tenant_access_token,
        &args.message_id,
        &args.file_key,
        args.resource_type.as_resource_type(),
        mvp::channel::feishu::api::resources::media::FEISHU_MESSAGE_RESOURCE_DOWNLOAD_MAX_BYTES,
    )
    .await?;
    let output = args.output.trim();
    if output.is_empty() {
        return Err("feishu messages resource requires --output".to_owned());
    }
    let output_path = std::path::Path::new(output);
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create parent directory {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(output_path, &resource.bytes).map_err(|error| {
        format!(
            "failed to write Feishu resource file {}: {error}",
            output_path.display()
        )
    })?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "message_id": resource.message_id,
        "file_key": resource.file_key,
        "resource_type": resource.resource_type.as_api_value(),
        "content_type": resource.content_type,
        "file_name": resource.file_name,
        "path": output_path.display().to_string(),
        "bytes_written": resource.bytes.len(),
    }))
}

pub async fn execute_feishu_search_messages(args: &FeishuSearchMessagesArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    let client = context.build_client()?;
    let page = mvp::channel::feishu::api::resources::messages::search_messages(
        &client,
        &grant.access_token,
        &mvp::channel::feishu::api::resources::messages::FeishuSearchMessagesQuery {
            user_id_type: args.user_id_type.clone(),
            page_size: args.page_size,
            page_token: args.page_token.clone(),
            query: args.query.clone(),
            from_ids: args.from_ids.clone(),
            chat_ids: args.chat_ids.clone(),
            message_type: args.message_type.clone(),
            at_chatter_ids: args.at_chatter_ids.clone(),
            from_type: args.from_type.clone(),
            chat_type: args.chat_type.clone(),
            start_time: args.start_time.clone(),
            end_time: args.end_time.clone(),
        },
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "page": page,
    }))
}

pub async fn execute_feishu_calendar_list(args: &FeishuCalendarListArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    let client = context.build_client()?;
    if args.primary {
        let calendars = mvp::channel::feishu::api::resources::calendar::get_primary_calendars(
            &client,
            &grant.access_token,
            args.user_id_type.as_deref().or(Some("open_id")),
        )
        .await?;
        return Ok(json!({
            "account_id": context.account_id(),
            "configured_account": context.resolved.configured_account_label,
            "principal": grant.principal,
            "primary": true,
            "calendars": calendars,
        }));
    }

    let page = mvp::channel::feishu::api::resources::calendar::list_calendars(
        &client,
        &grant.access_token,
        &mvp::channel::feishu::api::resources::calendar::FeishuCalendarListQuery {
            page_size: args.page_size,
            page_token: args.page_token.clone(),
            sync_token: args.sync_token.clone(),
        },
    )
    .await?;
    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "primary": false,
        "page": page,
    }))
}

pub async fn execute_feishu_calendar_freebusy(
    args: &FeishuCalendarFreebusyArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    let client = context.build_client()?;
    let effective_user_id = args.user_id.clone().or_else(|| {
        if args.room_id.as_deref().is_some() {
            None
        } else {
            Some(grant.principal.open_id.clone())
        }
    });
    let page = mvp::channel::feishu::api::resources::calendar::get_freebusy(
        &client,
        &grant.access_token,
        &mvp::channel::feishu::api::resources::calendar::FeishuCalendarFreebusyQuery {
            user_id_type: args.user_id_type.clone().or_else(|| {
                effective_user_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|_| "open_id".to_owned())
            }),
            time_min: args.time_min.clone(),
            time_max: args.time_max.clone(),
            user_id: effective_user_id,
            room_id: args.room_id.clone(),
            include_external_calendar: args.include_external_calendar,
            only_busy: args.only_busy,
            need_rsvp_status: args.need_rsvp_status,
        },
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": page,
    }))
}

pub async fn execute_feishu_bitable_list_tables(
    args: &FeishuBitableListTablesArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:table:read"],
        "loongclaw feishu bitable list-tables",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::list_bitable_tables(
        &client,
        &grant.access_token,
        &args.app_token,
        args.page_token.as_deref(),
        args.page_size,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "tables": result.items,
        "has_more": result.has_more,
        "page_token": result.page_token,
    }))
}

pub async fn execute_feishu_bitable_app_create(
    args: &FeishuBitableAppCreateArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable app-create",
    )?;
    let client = context.build_client()?;
    let app = mvp::channel::feishu::api::resources::bitable::create_bitable_app(
        &client,
        &grant.access_token,
        &args.name,
        args.folder_token.as_deref(),
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "app": app,
    }))
}

pub async fn execute_feishu_bitable_app_get(args: &FeishuBitableAppGetArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable app-get",
    )?;
    let client = context.build_client()?;
    let app = mvp::channel::feishu::api::resources::bitable::get_bitable_app(
        &client,
        &grant.access_token,
        &args.app_token,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "app": app,
    }))
}

pub async fn execute_feishu_bitable_app_list(args: &FeishuBitableAppListArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["drive:drive:readonly"],
        "loongclaw feishu bitable app-list",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::list_bitable_apps(
        &client,
        &grant.access_token,
        &mvp::channel::feishu::api::resources::bitable::BitableAppListQuery {
            folder_token: args.folder_token.clone(),
            page_size: args.page_size,
            page_token: args.page_token.clone(),
        },
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "apps": result.apps,
        "page_token": result.page_token,
        "has_more": result.has_more,
    }))
}

pub async fn execute_feishu_bitable_app_patch(
    args: &FeishuBitableAppPatchArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable app-patch",
    )?;
    let client = context.build_client()?;
    let app = mvp::channel::feishu::api::resources::bitable::patch_bitable_app(
        &client,
        &grant.access_token,
        &args.app_token,
        args.name.as_deref(),
        args.is_advanced,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "app": app,
    }))
}

pub async fn execute_feishu_bitable_app_copy(args: &FeishuBitableAppCopyArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable app-copy",
    )?;
    let client = context.build_client()?;
    let app = mvp::channel::feishu::api::resources::bitable::copy_bitable_app(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.name,
        args.folder_token.as_deref(),
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "app": app,
    }))
}

pub async fn execute_feishu_bitable_create_record(
    args: &FeishuBitableCreateRecordArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:create"],
        "loongclaw feishu bitable create-record",
    )?;
    let client = context.build_client()?;
    let fields = serde_json::from_str::<Value>(&args.fields)
        .map_err(|error| format!("invalid --fields JSON: {error}"))?;
    if !fields.is_object() {
        return Err("--fields must be a JSON object (e.g. '{\"Name\": \"value\"}')".to_owned());
    }
    let record = mvp::channel::feishu::api::resources::bitable::create_bitable_record(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        fields,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "record": record,
    }))
}

pub async fn execute_feishu_bitable_create_table(
    args: &FeishuBitableCreateTableArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable create-table",
    )?;
    let client = context.build_client()?;
    let fields = args
        .fields
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| format!("invalid --fields JSON: {error}"))?;
    let fields = match fields {
        Some(Value::Array(items)) => Some(items),
        Some(_) => return Err("--fields must be a JSON array".to_owned()),
        None => None,
    };
    let result = mvp::channel::feishu::api::resources::bitable::create_bitable_table(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.name,
        args.default_view_name.as_deref(),
        fields,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_patch_table(
    args: &FeishuBitablePatchTableArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable patch-table",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::patch_bitable_table(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.name,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_batch_create_tables(
    args: &FeishuBitableBatchCreateTablesArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable batch-create-tables",
    )?;
    let client = context.build_client()?;
    let tables = serde_json::from_str::<Value>(&args.tables)
        .map_err(|error| format!("invalid --tables JSON: {error}"))?;
    let tables = match tables {
        Value::Array(items) => items,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_) => {
            return Err("--tables must be a JSON array".to_owned());
        }
    };
    let result = mvp::channel::feishu::api::resources::bitable::batch_create_bitable_tables(
        &client,
        &grant.access_token,
        &args.app_token,
        tables,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_search_records(
    args: &FeishuBitableSearchRecordsArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:retrieve"],
        "loongclaw feishu bitable search-records",
    )?;
    let client = context.build_client()?;
    let filter = args
        .filter
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| format!("invalid --filter JSON: {error}"))?;
    let sort = args
        .sort
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| format!("invalid --sort JSON: {error}"))?;
    let result = mvp::channel::feishu::api::resources::bitable::search_bitable_records(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &mvp::channel::feishu::api::resources::bitable::BitableRecordSearchQuery {
            page_token: args.page_token.clone(),
            page_size: args.page_size,
            view_id: args.view_id.clone(),
            filter,
            sort,
            field_names: (!args.field_names.is_empty()).then(|| args.field_names.clone()),
            automatic_fields: args.automatic_fields.then_some(true),
        },
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_update_record(
    args: &FeishuBitableUpdateRecordArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:write"],
        "loongclaw feishu bitable update-record",
    )?;
    let client = context.build_client()?;
    let fields = serde_json::from_str::<Value>(&args.fields)
        .map_err(|error| format!("invalid --fields JSON: {error}"))?;
    if !fields.is_object() {
        return Err("--fields must be a JSON object (e.g. '{\"Name\": \"value\"}')".to_owned());
    }
    let record = mvp::channel::feishu::api::resources::bitable::update_bitable_record(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.record_id,
        fields,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "record": record,
    }))
}

pub async fn execute_feishu_bitable_delete_record(
    args: &FeishuBitableDeleteRecordArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:write"],
        "loongclaw feishu bitable delete-record",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::delete_bitable_record(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.record_id,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "deleted": result.deleted,
        "record_id": result.record_id,
    }))
}

pub async fn execute_feishu_bitable_batch_create_records(
    args: &FeishuBitableBatchCreateRecordsArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:write"],
        "loongclaw feishu bitable batch-create-records",
    )?;
    let client = context.build_client()?;
    let records = serde_json::from_str::<Value>(&args.records)
        .map_err(|error| format!("invalid --records JSON: {error}"))?;
    let records = match records {
        Value::Array(items) => items,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_) => {
            return Err("--records must be a JSON array".to_owned());
        }
    };
    if records.len() > 500 {
        return Err(format!(
            "feishu.bitable.record.batch_create: batch size must be <= 500, got {}",
            records.len()
        ));
    }
    let result = mvp::channel::feishu::api::resources::bitable::batch_create_bitable_records(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        records,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_batch_update_records(
    args: &FeishuBitableBatchUpdateRecordsArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:write"],
        "loongclaw feishu bitable batch-update-records",
    )?;
    let client = context.build_client()?;
    let records = serde_json::from_str::<Value>(&args.records)
        .map_err(|error| format!("invalid --records JSON: {error}"))?;
    let records = match records {
        Value::Array(items) => items,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_) => {
            return Err("--records must be a JSON array".to_owned());
        }
    };
    if records.len() > 500 {
        return Err(format!(
            "feishu.bitable.record.batch_update: batch size must be <= 500, got {}",
            records.len()
        ));
    }
    let result = mvp::channel::feishu::api::resources::bitable::batch_update_bitable_records(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        records,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_batch_delete_records(
    args: &FeishuBitableBatchDeleteRecordsArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:write"],
        "loongclaw feishu bitable batch-delete-records",
    )?;
    let client = context.build_client()?;
    let records = serde_json::from_str::<Value>(&args.records)
        .map_err(|error| format!("invalid --records JSON: {error}"))?;
    let records = match records {
        Value::Array(items) => items
            .into_iter()
            .map(|item| {
                item.as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| "--records must be a JSON array of strings".to_owned())
            })
            .collect::<CliResult<Vec<_>>>()?,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_) => {
            return Err("--records must be a JSON array".to_owned());
        }
    };
    if records.len() > 500 {
        return Err(format!(
            "feishu.bitable.record.batch_delete: batch size must be <= 500, got {}",
            records.len()
        ));
    }
    let result = mvp::channel::feishu::api::resources::bitable::batch_delete_bitable_records(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        records,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_create_field(
    args: &FeishuBitableCreateFieldArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable create-field",
    )?;
    let client = context.build_client()?;
    let property = args
        .property
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| format!("invalid --property JSON: {error}"))?;
    let field = mvp::channel::feishu::api::resources::bitable::create_bitable_field(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.field_name,
        args.field_type,
        property,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "field": field,
    }))
}

pub async fn execute_feishu_bitable_list_fields(
    args: &FeishuBitableListFieldsArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable list-fields",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::list_bitable_fields(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &mvp::channel::feishu::api::resources::bitable::BitableFieldListQuery {
            view_id: args.view_id.clone(),
            page_size: args.page_size,
            page_token: args.page_token.clone(),
        },
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "fields": result.items,
        "page_token": result.page_token,
        "has_more": result.has_more,
        "total": result.total,
    }))
}

pub async fn execute_feishu_bitable_update_field(
    args: &FeishuBitableUpdateFieldArgs,
) -> CliResult<Value> {
    let field_name = args
        .field_name
        .as_deref()
        .ok_or_else(|| "--field-name and --type are required for field update".to_owned())?;
    let field_type = args
        .field_type
        .ok_or_else(|| "--field-name and --type are required for field update".to_owned())?;

    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable update-field",
    )?;
    let client = context.build_client()?;
    let property = args
        .property
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| format!("invalid --property JSON: {error}"))?;
    let field = mvp::channel::feishu::api::resources::bitable::update_bitable_field(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.field_id,
        field_name,
        field_type,
        property,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "field": field,
    }))
}

pub async fn execute_feishu_bitable_delete_field(
    args: &FeishuBitableDeleteFieldArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable delete-field",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::delete_bitable_field(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.field_id,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "deleted": result.deleted,
        "field_id": result.field_id,
    }))
}

pub async fn execute_feishu_bitable_create_view(
    args: &FeishuBitableCreateViewArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable create-view",
    )?;
    let client = context.build_client()?;
    let view = mvp::channel::feishu::api::resources::bitable::create_bitable_view(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.view_name,
        args.view_type.as_deref(),
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "view": view,
    }))
}

pub async fn execute_feishu_bitable_get_view(args: &FeishuBitableGetViewArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable get-view",
    )?;
    let client = context.build_client()?;
    let view = mvp::channel::feishu::api::resources::bitable::get_bitable_view(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.view_id,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "view": view,
    }))
}

pub async fn execute_feishu_bitable_list_views(
    args: &FeishuBitableListViewsArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable list-views",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::list_bitable_views(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &mvp::channel::feishu::api::resources::bitable::BitableViewListQuery {
            page_size: args.page_size,
            page_token: args.page_token.clone(),
        },
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "views": result.items,
        "page_token": result.page_token,
        "has_more": result.has_more,
        "total": result.total,
    }))
}

pub async fn execute_feishu_bitable_patch_view(
    args: &FeishuBitablePatchViewArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loongclaw feishu bitable patch-view",
    )?;
    let client = context.build_client()?;
    let view = mvp::channel::feishu::api::resources::bitable::patch_bitable_view(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.view_id,
        &args.view_name,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "view": view,
    }))
}

pub async fn execute_feishu_send(args: &FeishuSendArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    let action = format!("{} feishu send", active_cli_command_name());
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        mvp::channel::feishu::api::FEISHU_MESSAGE_WRITE_ACCEPTED_SCOPES,
        action.as_str(),
    )?;
    let client = context.build_client()?;
    let tenant_access_token = client.get_tenant_access_token().await?;
    let receive_id_type = trimmed_opt(args.receive_id_type.as_deref())
        .unwrap_or(context.resolved.receive_id_type.as_str())
        .to_owned();
    let uuid = trimmed_opt(args.uuid.as_deref()).map(ToOwned::to_owned);
    let body = mvp::channel::feishu::api::resolve_operator_outbound_message_body(
        action.as_str(),
        &client,
        &tenant_access_token,
        &mvp::channel::feishu::api::FeishuOperatorOutboundMessageInput {
            text: args.text.clone(),
            card: args.card,
            post_json: args.post_json.clone(),
            image_key: args.image_key.clone(),
            image_path: args.image_path.clone(),
            file_key: args.file_key.clone(),
            file_path: args.file_path.clone(),
            file_type: args.file_type.clone(),
        },
    )
    .await?;
    let msg_type = body.msg_type();
    let delivery = mvp::channel::feishu::api::resources::messages::send_outbound_message(
        &client,
        &tenant_access_token,
        &receive_id_type,
        &args.receive_id,
        &body,
        uuid.as_deref(),
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "delivery": {
            "mode": "send",
            "receive_id_type": receive_id_type,
            "receive_id": args.receive_id,
            "uuid": uuid,
            "msg_type": msg_type,
            "message_id": delivery.message_id,
            "root_id": delivery.root_id,
            "parent_id": delivery.parent_id,
        },
    }))
}

pub async fn execute_feishu_reply(args: &FeishuReplyArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    let action = format!("{} feishu reply", active_cli_command_name());
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        mvp::channel::feishu::api::FEISHU_MESSAGE_WRITE_ACCEPTED_SCOPES,
        action.as_str(),
    )?;
    let client = context.build_client()?;
    let tenant_access_token = client.get_tenant_access_token().await?;
    let body = mvp::channel::feishu::api::resolve_operator_outbound_message_body(
        action.as_str(),
        &client,
        &tenant_access_token,
        &mvp::channel::feishu::api::FeishuOperatorOutboundMessageInput {
            text: args.text.clone(),
            card: args.card,
            post_json: args.post_json.clone(),
            image_key: args.image_key.clone(),
            image_path: args.image_path.clone(),
            file_key: args.file_key.clone(),
            file_path: args.file_path.clone(),
            file_type: args.file_type.clone(),
        },
    )
    .await?;
    let msg_type = body.msg_type();
    let uuid = trimmed_opt(args.uuid.as_deref()).map(ToOwned::to_owned);
    let delivery = mvp::channel::feishu::api::resources::messages::reply_outbound_message(
        &client,
        &tenant_access_token,
        &args.message_id,
        &body,
        args.reply_in_thread,
        uuid.as_deref(),
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "delivery": {
            "mode": "reply",
            "message_id": delivery.message_id,
            "reply_to_message_id": args.message_id,
            "reply_in_thread": args.reply_in_thread,
            "uuid": uuid,
            "msg_type": msg_type,
            "root_id": delivery.root_id,
            "parent_id": delivery.parent_id,
        },
    }))
}

async fn load_context_and_fresh_grant(
    args: &FeishuGrantArgs,
) -> CliResult<(FeishuDaemonContext, mvp::channel::feishu::api::FeishuGrant)> {
    let context = load_feishu_daemon_context(
        args.common.config.as_deref(),
        args.common.account.as_deref(),
    )?;
    let grant = require_selected_grant(&context, args.open_id.as_deref())?;
    let client = context.build_client()?;
    let grant =
        mvp::channel::feishu::api::ensure_fresh_user_grant(&client, &context.store, &grant).await?;
    Ok((context, grant))
}

fn require_selected_grant(
    context: &FeishuDaemonContext,
    open_id: Option<&str>,
) -> CliResult<mvp::channel::feishu::api::FeishuGrant> {
    let resolution = mvp::channel::feishu::api::resolve_grant_selection(
        &context.store,
        context.account_id(),
        open_id,
    )?;
    if let Some(grant) = resolution.selected_grant().cloned() {
        return Ok(grant);
    }
    Err(describe_grant_selection_error(context, &resolution))
}

fn describe_grant_selection_error(
    context: &FeishuDaemonContext,
    resolution: &mvp::channel::feishu::api::FeishuGrantResolution,
) -> String {
    let display_account_id = context.resolved.configured_account_id.as_str();
    let cli = active_cli_command_name();
    if let Some(requested_open_id) = resolution.missing_explicit_open_id() {
        if resolution.inventory.grants.is_empty() {
            return format!(
                "no stored Feishu grant for account `{display_account_id}` and open_id `{requested_open_id}`; run `{}` first",
                feishu_auth_start_command_hint(display_account_id, false, false),
            );
        }
        let available_open_ids = resolution.available_open_ids().join(", ");
        return format!(
            "no stored Feishu grant for account `{display_account_id}` and open_id `{requested_open_id}`; available open_ids: {available_open_ids}; run `{}` or `{cli} feishu auth list --account {display_account_id}`",
            crate::feishu_support::feishu_auth_select_command_hint(display_account_id),
        );
    }
    if resolution.selection_required() {
        let open_ids = resolution.available_open_ids().join(", ");
        let stale_selected_hint = resolution
            .inventory
            .stale_selected_open_id
            .as_deref()
            .map(|open_id| format!("stale selected open_id `{open_id}` was cleared; "))
            .unwrap_or_default();
        return format!(
            "{stale_selected_hint}multiple stored Feishu grants exist for account `{display_account_id}` ({open_ids}); run `{cli} feishu auth list --account {display_account_id}`, then `{}` or pass `--open-id`",
            crate::feishu_support::feishu_auth_select_command_hint(display_account_id),
        );
    }
    format!(
        "no stored Feishu grant for account `{display_account_id}`; run `{}` first",
        feishu_auth_start_command_hint(display_account_id, false, false),
    )
}

fn required_json_string(payload: &Value, field: &str) -> CliResult<String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("feishu oauth payload missing {field}"))
}

fn trimmed_opt(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedFeishuDocContent {
    content: String,
    content_type: &'static str,
}

fn resolve_feishu_doc_content_type(
    action: &str,
    has_content: bool,
    raw: Option<&str>,
) -> CliResult<Option<&'static str>> {
    match trimmed_opt(raw) {
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "markdown" => Ok(Some("markdown")),
            "html" => Ok(Some("html")),
            other => Err(format!(
                "unsupported Feishu document content_type `{other}`; expected `markdown` or `html`"
            )),
        },
        None if !has_content && raw.is_some() => {
            Err(format!("{action} requires --content or --content-path"))
        }
        None => Ok(None),
    }
}

fn prepare_feishu_doc_cli_content(
    action: &str,
    content: Option<&str>,
    content_path: Option<&str>,
    raw_content_type: Option<&str>,
    required: bool,
) -> CliResult<Option<PreparedFeishuDocContent>> {
    let inline_content = trimmed_opt(content).map(ToOwned::to_owned);
    let file_path = trimmed_opt(content_path);
    if inline_content.is_some() && file_path.is_some() {
        return Err(format!(
            "{action} accepts either --content or --content-path, not both"
        ));
    }

    let has_content = inline_content.is_some() || file_path.is_some();
    let explicit_content_type =
        resolve_feishu_doc_content_type(action, has_content, raw_content_type)?;

    match (inline_content, file_path) {
        (Some(content), None) => Ok(Some(PreparedFeishuDocContent {
            content,
            content_type: explicit_content_type.unwrap_or("markdown"),
        })),
        (None, Some(path)) => {
            let content = read_feishu_doc_text_file(action, "--content-path", path)?;
            Ok(Some(PreparedFeishuDocContent {
                content,
                content_type: explicit_content_type
                    .unwrap_or_else(|| infer_feishu_doc_content_type_from_path(Path::new(path))),
            }))
        }
        (None, None) if required => Err(format!("{action} requires --content or --content-path")),
        (None, None) => Ok(None),
        (Some(_), Some(_)) => Err(format!(
            "{action} accepts either --content or --content-path, not both"
        )),
    }
}

fn infer_feishu_doc_content_type_from_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("html") | Some("htm") => "html",
        Some("md") | Some("markdown") => "markdown",
        _ => "markdown",
    }
}

fn read_feishu_doc_text_file(action: &str, field: &str, raw_path: &str) -> CliResult<String> {
    let path = Path::new(raw_path.trim());
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "{action} failed to read {field} `{}`: {error}",
            path.display()
        )
    })?;
    if bytes.is_empty() {
        return Err(format!(
            "{action} requires {field} `{}` to be non-empty UTF-8 text",
            path.display()
        ));
    }
    let content = String::from_utf8(bytes).map_err(|error| {
        format!(
            "{action} requires {field} `{}` to contain valid UTF-8 text: {error}",
            path.display()
        )
    })?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "{action} requires {field} `{}` to be non-empty UTF-8 text",
            path.display()
        ));
    }
    Ok(trimmed.to_owned())
}

fn ensure_grant_has_any_scope(
    grant: &mvp::channel::feishu::api::FeishuGrant,
    configured_account_id: &str,
    accepted: &[&str],
    action: &str,
) -> CliResult<()> {
    let include_doc_write = accepted
        .iter()
        .copied()
        .any(|scope| mvp::channel::feishu::api::FEISHU_DOC_WRITE_ACCEPTED_SCOPES.contains(&scope));
    let include_message_write = accepted.iter().copied().any(|scope| {
        mvp::channel::feishu::api::FEISHU_MESSAGE_WRITE_ACCEPTED_SCOPES.contains(&scope)
    });
    if accepted
        .iter()
        .copied()
        .any(|scope| grant.scopes.contains(scope))
    {
        return Ok(());
    }

    Err(format!(
        "{action} requires at least one Feishu scope [{}] for `{}`; rerun `{}` or pass the required scopes manually",
        accepted.join(", "),
        grant.principal.storage_key(),
        feishu_auth_start_command_hint(
            configured_account_id,
            include_message_write,
            include_doc_write,
        ),
    ))
}

fn serialize_grant_summary(
    grant: &mvp::channel::feishu::api::FeishuGrant,
    configured_account_id: &str,
    now_s: i64,
    required_scopes: &[String],
    selected_open_id: Option<&str>,
    effective_open_id: Option<&str>,
) -> Value {
    json!({
        "selected": selected_open_id
            .map(str::trim)
            .is_some_and(|open_id| open_id == grant.principal.open_id),
        "effective_selected": effective_open_id
            .map(str::trim)
            .is_some_and(|open_id| open_id == grant.principal.open_id),
        "principal": grant.principal,
        "scopes": grant.scopes.as_slice(),
        "access_expires_at_s": grant.access_expires_at_s,
        "refresh_expires_at_s": grant.refresh_expires_at_s,
        "refreshed_at_s": grant.refreshed_at_s,
        "status": mvp::channel::feishu::api::auth::summarize_grant_status(Some(grant), now_s, required_scopes),
        "doc_write_status": mvp::channel::feishu::api::summarize_doc_write_scope_status(Some(grant)),
        "message_write_status": mvp::channel::feishu::api::summarize_message_write_scope_status(Some(grant)),
        "recommendations": build_grant_recommendations(
            configured_account_id,
            Some(grant),
            now_s,
            required_scopes,
        ),
    })
}

#[allow(clippy::print_stdout)]
fn print_feishu_payload(
    payload: &Value,
    as_json: bool,
    render_text: fn(&Value) -> CliResult<String>,
) -> CliResult<()> {
    if as_json {
        let encoded = serde_json::to_string_pretty(payload)
            .map_err(|error| format!("serialize feishu command output failed: {error}"))?;
        println!("{encoded}");
        return Ok(());
    }
    println!("{}", render_text(payload)?);
    Ok(())
}

fn render_auth_start_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu auth login".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!("state: {}", required_json_string(payload, "state")?),
        format!(
            "redirect_uri: {}",
            required_json_string(payload, "redirect_uri")?
        ),
        format!(
            "authorize_url: {}",
            required_json_string(payload, "authorize_url")?
        ),
        format!(
            "capabilities: {}",
            payload
                .get("capabilities")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "-".to_owned())
        ),
        format!(
            "scopes: {}",
            payload
                .get("scopes")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        ),
        format!(
            "sqlite_path: {}",
            required_json_string(payload, "sqlite_path")?
        ),
    ]);
    if let Some(flow) = payload.get("flow").and_then(Value::as_str) {
        lines.push(format!("flow: {flow}"));
    }
    if let Some(exchange_command) = payload.get("exchange_command").and_then(Value::as_str) {
        lines.push(format!("next_step: {exchange_command}"));
    }
    if let Some(manual_note) = payload.get("manual_note").and_then(Value::as_str) {
        lines.push(format!("note: {manual_note}"));
    }
    if let Some(status) = payload.get("status").and_then(Value::as_str) {
        lines.push(format!("status: {status}"));
    }
    Ok(lines.join("\n"))
}

fn render_auth_exchange_text(payload: &Value) -> CliResult<String> {
    let principal = payload
        .get("principal")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut lines = vec![
        "feishu auth exchange".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "open_id: {}",
            principal
                .get("open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "name: {}",
            principal.get("name").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "scopes: {}",
            payload
                .get("granted_scopes")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        ),
        format!(
            "selected_open_id: {}",
            payload
                .get("selected_open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "effective_open_id: {}",
            payload
                .get("effective_open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_auth_list_text(payload: &Value) -> CliResult<String> {
    let grants = payload
        .get("grants")
        .and_then(Value::as_array)
        .ok_or_else(|| "feishu auth list payload missing grants".to_owned())?;
    let mut lines = vec![
        "feishu auth list".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
        format!("grant_count: {}", grants.len()),
        format!(
            "selected_open_id: {}",
            payload
                .get("selected_open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.insert(2, format!("configured_account: {configured_account}"));
    }
    if let Some(effective_open_id) = payload.get("effective_open_id").and_then(Value::as_str) {
        lines.push(format!("effective_open_id: {effective_open_id}"));
    }
    if let Some(select_hint) = payload
        .get("recommendations")
        .and_then(|value| value.get("select_command"))
        .and_then(Value::as_str)
    {
        lines.push(format!("select_hint: {select_hint}"));
    }
    if let Some(stale_selected_open_id) = payload
        .get("recommendations")
        .and_then(|value| value.get("stale_selected_open_id"))
        .and_then(Value::as_str)
    {
        lines.push(format!("stale_selected_open_id: {stale_selected_open_id}"));
    }
    if let Some(auth_start_hint) = payload
        .get("recommendations")
        .and_then(|value| value.get("auth_start_command"))
        .and_then(Value::as_str)
    {
        lines.push(format!("auth_start_hint: {auth_start_hint}"));
    }
    for grant in grants {
        lines.push(render_auth_grant_summary_line(grant));
    }
    Ok(lines.join("\n"))
}

fn render_auth_select_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu auth select".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(format!(
        "selected_open_id: {}",
        required_json_string(payload, "selected_open_id")?
    ));
    if let Some(grant) = payload.get("grant") {
        lines.push(render_auth_grant_summary_line(grant));
    }
    Ok(lines.join("\n"))
}

fn render_auth_status_text(payload: &Value) -> CliResult<String> {
    if payload
        .get("status_scope")
        .and_then(Value::as_str)
        .is_some_and(|scope| scope == "account")
    {
        let grants = payload
            .get("grants")
            .and_then(Value::as_array)
            .ok_or_else(|| "feishu auth status payload missing grants".to_owned())?;
        let mut lines = vec![
            "feishu auth status".to_owned(),
            format!("account: {}", required_json_string(payload, "account_id")?),
            format!("status_scope: account"),
            format!("grant_count: {}", grants.len()),
            format!(
                "selected_open_id: {}",
                payload
                    .get("selected_open_id")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            ),
        ];
        if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str)
        {
            lines.insert(2, format!("configured_account: {configured_account}"));
        }
        if let Some(effective_open_id) = payload.get("effective_open_id").and_then(Value::as_str) {
            lines.push(format!("effective_open_id: {effective_open_id}"));
        }
        if let Some(select_hint) = payload
            .get("recommendations")
            .and_then(|value| value.get("select_command"))
            .and_then(Value::as_str)
        {
            lines.push(format!("select_hint: {select_hint}"));
        }
        if let Some(stale_selected_open_id) = payload
            .get("recommendations")
            .and_then(|value| value.get("stale_selected_open_id"))
            .and_then(Value::as_str)
        {
            lines.push(format!("stale_selected_open_id: {stale_selected_open_id}"));
        }
        for grant in grants {
            lines.push(render_auth_grant_summary_line(grant));
        }
        return Ok(lines.join("\n"));
    }

    let status = payload
        .get("status")
        .ok_or_else(|| "feishu auth status payload missing status".to_owned())?;
    let doc_write_status = payload.get("doc_write_status").unwrap_or(&Value::Null);
    let message_write_status = payload.get("message_write_status").unwrap_or(&Value::Null);
    let auth_start_hint = payload
        .get("recommendations")
        .and_then(|value| value.get("auth_start_command"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let select_hint = payload
        .get("recommendations")
        .and_then(|value| value.get("select_command"))
        .and_then(Value::as_str);
    let selected_open_id = payload
        .get("selected_open_id")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let effective_open_id = payload
        .get("effective_open_id")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let mut lines = vec![
        "feishu auth status".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
        format!(
            "has_grant: {}",
            status
                .get("has_grant")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "access_token_expired: {}",
            status
                .get("access_token_expired")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        ),
        format!(
            "refresh_token_expired: {}",
            status
                .get("refresh_token_expired")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        ),
        format!(
            "missing_scopes: {}",
            status
                .get("missing_scopes")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        ),
        format!(
            "doc_write_ready: {}",
            doc_write_status
                .get("ready")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "matched_doc_write_scopes: {}",
            doc_write_status
                .get("matched_scopes")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "-".to_owned())
        ),
        format!(
            "message_write_ready: {}",
            message_write_status
                .get("ready")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "matched_write_scopes: {}",
            message_write_status
                .get("matched_scopes")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "-".to_owned())
        ),
        format!("auth_start_hint: {auth_start_hint}"),
        format!("selected_open_id: {selected_open_id}"),
        format!("effective_open_id: {effective_open_id}"),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.insert(2, format!("configured_account: {configured_account}"));
    }
    if let Some(requested_open_id) = payload.get("requested_open_id").and_then(Value::as_str) {
        lines.push(format!("requested_open_id: {requested_open_id}"));
    }
    if let Some(select_hint) = select_hint {
        lines.push(format!("select_hint: {select_hint}"));
    }
    if let Some(available_open_ids) = payload
        .get("available_open_ids")
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
    {
        lines.push(format!(
            "available_open_ids: {}",
            available_open_ids
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(lines.join("\n"))
}

fn render_auth_grant_summary_line(grant: &Value) -> String {
    let principal = grant
        .get("principal")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let status = grant
        .get("status")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let doc_write_status = grant
        .get("doc_write_status")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let message_write_status = grant
        .get("message_write_status")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let missing_scopes = status
        .get("missing_scopes")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "-".to_owned());
    let matched_doc_write_scopes = doc_write_status
        .get("matched_scopes")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "-".to_owned());
    let matched_write_scopes = message_write_status
        .get("matched_scopes")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "-".to_owned());
    let auth_start_hint = grant
        .get("recommendations")
        .and_then(|value| value.get("auth_start_command"))
        .and_then(Value::as_str)
        .unwrap_or("-");

    format!(
        "open_id: {} | selected: {} | effective_selected: {} | name: {} | access_expired: {} | refresh_expired: {} | missing_scopes: {} | doc_write_ready: {} | matched_doc_write_scopes: {} | message_write_ready: {} | matched_write_scopes: {} | auth_start_hint: {}",
        principal
            .get("open_id")
            .and_then(Value::as_str)
            .unwrap_or("-"),
        grant
            .get("selected")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        grant
            .get("effective_selected")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        principal.get("name").and_then(Value::as_str).unwrap_or("-"),
        status
            .get("access_token_expired")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        status
            .get("refresh_token_expired")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        missing_scopes,
        doc_write_status
            .get("ready")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        matched_doc_write_scopes,
        message_write_status
            .get("ready")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        matched_write_scopes,
        auth_start_hint,
    )
}

fn render_auth_revoke_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu auth revoke".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "open_id: {}",
            payload
                .get("open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "deleted: {}",
            payload
                .get("deleted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
    ]);
    if let Some(grant_count) = payload.get("grant_count").and_then(Value::as_u64) {
        lines.push(format!("grant_count: {grant_count}"));
    }
    if payload.get("selected_open_id").is_some() {
        lines.push(format!(
            "selected_open_id: {}",
            payload
                .get("selected_open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ));
    }
    if payload.get("effective_open_id").is_some() {
        lines.push(format!(
            "effective_open_id: {}",
            payload
                .get("effective_open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ));
    }
    if let Some(select_hint) = payload
        .get("recommendations")
        .and_then(|value| value.get("select_command"))
        .and_then(Value::as_str)
    {
        lines.push(format!("select_hint: {select_hint}"));
    }
    if let Some(auth_start_hint) = payload
        .get("recommendations")
        .and_then(|value| value.get("auth_start_command"))
        .and_then(Value::as_str)
    {
        lines.push(format!("auth_start_hint: {auth_start_hint}"));
    }
    Ok(lines.join("\n"))
}

fn render_whoami_text(payload: &Value) -> CliResult<String> {
    let principal = payload
        .get("principal")
        .ok_or_else(|| "feishu whoami payload missing principal".to_owned())?;
    let mut lines = vec![
        "feishu whoami".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "open_id: {}",
            principal
                .get("open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "name: {}",
            principal.get("name").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "email: {}",
            principal
                .get("email")
                .and_then(Value::as_str)
                .or_else(|| principal.get("enterprise_email").and_then(Value::as_str))
                .unwrap_or("-")
        ),
        format!(
            "tenant_key: {}",
            principal
                .get("tenant_key")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_read_doc_text(payload: &Value) -> CliResult<String> {
    let document = payload
        .get("document")
        .ok_or_else(|| "feishu read doc payload missing document".to_owned())?;
    let mut lines = vec![
        "feishu read doc".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(format!(
        "document_id: {}",
        document
            .get("document_id")
            .and_then(Value::as_str)
            .unwrap_or("-")
    ));
    lines.push(String::new());
    lines.push(
        document
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    );
    Ok(lines.join("\n"))
}

fn render_doc_create_text(payload: &Value) -> CliResult<String> {
    let document = payload
        .get("document")
        .ok_or_else(|| "feishu doc create payload missing document".to_owned())?;
    let mut lines = vec![
        "feishu doc create".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "document_id: {}",
            document
                .get("document_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "url: {}",
            document.get("url").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "content_inserted: {}",
            payload
                .get("content_inserted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "inserted_block_count: {}",
            payload
                .get("inserted_block_count")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ),
        format!(
            "insert_batch_count: {}",
            payload
                .get("insert_batch_count")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ),
        format!(
            "content_type: {}",
            payload
                .get("content_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_doc_append_text(payload: &Value) -> CliResult<String> {
    let document = payload
        .get("document")
        .ok_or_else(|| "feishu doc append payload missing document".to_owned())?;
    let mut lines = vec![
        "feishu doc append".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "document_id: {}",
            document
                .get("document_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "url: {}",
            document.get("url").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "inserted_block_count: {}",
            payload
                .get("inserted_block_count")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ),
        format!(
            "insert_batch_count: {}",
            payload
                .get("insert_batch_count")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ),
        format!(
            "content_type: {}",
            payload
                .get("content_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_messages_history_text(payload: &Value) -> CliResult<String> {
    let page = payload
        .get("page")
        .ok_or_else(|| "feishu message history payload missing page".to_owned())?;
    let mut lines = vec![
        "feishu messages history".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "items: {}",
            page.get("items")
                .and_then(Value::as_array)
                .map_or(0, std::vec::Vec::len)
        ),
        format!(
            "has_more: {}",
            page.get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "page_token: {}",
            page.get("page_token")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_messages_get_text(payload: &Value) -> CliResult<String> {
    let message = payload
        .get("message")
        .ok_or_else(|| "feishu message get payload missing message".to_owned())?;
    let encoded = serde_json::to_string_pretty(message)
        .map_err(|error| format!("serialize feishu message detail failed: {error}"))?;
    let mut lines = vec![
        "feishu messages get".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(encoded);
    Ok(lines.join("\n"))
}

fn render_messages_resource_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu messages resource".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "message_id: {}",
            payload
                .get("message_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "file_key: {}",
            payload
                .get("file_key")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "type: {}",
            payload
                .get("resource_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "path: {}",
            payload.get("path").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "bytes_written: {}",
            payload
                .get("bytes_written")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ),
        format!(
            "content_type: {}",
            payload
                .get("content_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "file_name: {}",
            payload
                .get("file_name")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_send_text(payload: &Value) -> CliResult<String> {
    let delivery = payload
        .get("delivery")
        .ok_or_else(|| "feishu send payload missing delivery".to_owned())?;
    let mut lines = vec![
        "feishu send".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "message_id: {}",
            delivery
                .get("message_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "receive_id_type: {}",
            delivery
                .get("receive_id_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "receive_id: {}",
            delivery
                .get("receive_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "uuid: {}",
            delivery.get("uuid").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "msg_type: {}",
            delivery
                .get("msg_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_reply_text(payload: &Value) -> CliResult<String> {
    let delivery = payload
        .get("delivery")
        .ok_or_else(|| "feishu reply payload missing delivery".to_owned())?;
    let mut lines = vec![
        "feishu reply".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "message_id: {}",
            delivery
                .get("message_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "reply_to_message_id: {}",
            delivery
                .get("reply_to_message_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "reply_in_thread: {}",
            delivery
                .get("reply_in_thread")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "uuid: {}",
            delivery.get("uuid").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "msg_type: {}",
            delivery
                .get("msg_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_search_messages_text(payload: &Value) -> CliResult<String> {
    let page = payload
        .get("page")
        .ok_or_else(|| "feishu message search payload missing page".to_owned())?;
    let mut lines = vec![
        "feishu search messages".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "items: {}",
            page.get("items")
                .and_then(Value::as_array)
                .map_or(0, std::vec::Vec::len)
        ),
        format!(
            "has_more: {}",
            page.get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "page_token: {}",
            page.get("page_token")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_calendar_list_text(payload: &Value) -> CliResult<String> {
    if payload
        .get("primary")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let calendars = payload
            .get("calendars")
            .and_then(|value| value.get("calendars"))
            .and_then(Value::as_array)
            .map_or(0, std::vec::Vec::len);
        let mut lines = vec![
            "feishu calendar list".to_owned(),
            format!("account: {}", required_json_string(payload, "account_id")?),
        ];
        if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str)
        {
            lines.push(format!("configured_account: {configured_account}"));
        }
        lines.extend([
            "primary: true".to_owned(),
            format!("calendars: {calendars}"),
        ]);
        return Ok(lines.join("\n"));
    }

    let page = payload
        .get("page")
        .ok_or_else(|| "feishu calendar list payload missing page".to_owned())?;
    let mut lines = vec![
        "feishu calendar list".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        "primary: false".to_owned(),
        format!(
            "calendars: {}",
            page.get("calendar_list")
                .and_then(Value::as_array)
                .map_or(0, std::vec::Vec::len)
        ),
        format!(
            "sync_token: {}",
            page.get("sync_token")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_calendar_freebusy_text(payload: &Value) -> CliResult<String> {
    let result = payload
        .get("result")
        .ok_or_else(|| "feishu calendar freebusy payload missing result".to_owned())?;
    let mut lines = vec![
        "feishu calendar freebusy".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(format!(
        "slots: {}",
        result
            .get("freebusy_list")
            .and_then(Value::as_array)
            .map_or(0, std::vec::Vec::len)
    ));
    Ok(lines.join("\n"))
}

fn render_bitable_list_tables_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu bitable list-tables".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "tables: {}",
            payload
                .get("tables")
                .and_then(Value::as_array)
                .map_or(0, std::vec::Vec::len)
        ),
        format!(
            "has_more: {}",
            payload
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "page_token: {}",
            payload
                .get("page_token")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_bitable_app_text(payload: &Value) -> CliResult<String> {
    let app = payload
        .get("app")
        .ok_or_else(|| "feishu bitable app payload missing app".to_owned())?;
    let mut lines = vec![
        "feishu bitable app".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "app_token: {}",
            app.get("app_token").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "name: {}",
            app.get("name").and_then(Value::as_str).unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_bitable_app_list_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu bitable app-list".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "apps: {}",
            payload
                .get("apps")
                .and_then(Value::as_array)
                .map_or(0, std::vec::Vec::len)
        ),
        format!(
            "has_more: {}",
            payload
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "page_token: {}",
            payload
                .get("page_token")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_bitable_create_record_text(payload: &Value) -> CliResult<String> {
    let record = payload
        .get("record")
        .ok_or_else(|| "feishu bitable create payload missing record".to_owned())?;
    let mut lines = vec![
        "feishu bitable create-record".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "record_id: {}",
            record
                .get("record_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "fields: {}",
            record
                .get("fields")
                .and_then(Value::as_object)
                .map_or(0, serde_json::Map::len)
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_bitable_table_text(payload: &Value) -> CliResult<String> {
    let result = payload
        .get("result")
        .ok_or_else(|| "feishu bitable table payload missing result".to_owned())?;
    let mut lines = vec![
        "feishu bitable table".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "table_id: {}",
            result
                .get("table_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "name: {}",
            result.get("name").and_then(Value::as_str).unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_bitable_table_batch_create_text(payload: &Value) -> CliResult<String> {
    let result = payload
        .get("result")
        .ok_or_else(|| "feishu bitable batch create tables payload missing result".to_owned())?;
    let mut lines = vec![
        "feishu bitable batch-create-tables".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(format!(
        "table_ids: {}",
        result
            .get("table_ids")
            .and_then(Value::as_array)
            .map_or(0, std::vec::Vec::len)
    ));
    Ok(lines.join("\n"))
}

fn render_bitable_search_records_text(payload: &Value) -> CliResult<String> {
    let result = payload
        .get("result")
        .ok_or_else(|| "feishu bitable search payload missing result".to_owned())?;
    let mut lines = vec![
        "feishu bitable search-records".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "records: {}",
            result
                .get("items")
                .and_then(Value::as_array)
                .map_or(0, std::vec::Vec::len)
        ),
        format!(
            "has_more: {}",
            result
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "page_token: {}",
            result
                .get("page_token")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_bitable_delete_record_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu bitable delete-record".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "record_id: {}",
            payload
                .get("record_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "deleted: {}",
            payload
                .get("deleted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_bitable_batch_records_text(payload: &Value) -> CliResult<String> {
    let result = payload
        .get("result")
        .ok_or_else(|| "feishu bitable batch payload missing result".to_owned())?;
    let mut lines = vec![
        "feishu bitable batch-records".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    if let Some(records) = result.get("records").and_then(Value::as_array) {
        lines.push(format!("records: {}", records.len()));
    }
    if let Some(success) = result.get("success").and_then(Value::as_bool) {
        lines.push(format!("success: {success}"));
    }
    Ok(lines.join("\n"))
}

fn render_bitable_field_text(payload: &Value) -> CliResult<String> {
    let field = payload
        .get("field")
        .ok_or_else(|| "feishu bitable field payload missing field".to_owned())?;
    let mut lines = vec![
        "feishu bitable field".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "field_id: {}",
            field.get("field_id").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "field_name: {}",
            field
                .get("field_name")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_bitable_field_list_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu bitable list-fields".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(format!(
        "fields: {}",
        payload
            .get("fields")
            .and_then(Value::as_array)
            .map_or(0, std::vec::Vec::len)
    ));
    Ok(lines.join("\n"))
}

fn render_bitable_delete_field_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu bitable delete-field".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "field_id: {}",
            payload
                .get("field_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "deleted: {}",
            payload
                .get("deleted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_bitable_view_text(payload: &Value) -> CliResult<String> {
    let view = payload
        .get("view")
        .ok_or_else(|| "feishu bitable view payload missing view".to_owned())?;
    let mut lines = vec![
        "feishu bitable view".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "view_id: {}",
            view.get("view_id").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "view_name: {}",
            view.get("view_name").and_then(Value::as_str).unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

fn render_bitable_view_list_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu bitable list-views".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(format!(
        "views: {}",
        payload
            .get("views")
            .and_then(Value::as_array)
            .map_or(0, std::vec::Vec::len)
    ));
    Ok(lines.join("\n"))
}

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
                "auth_start_command": "loong feishu auth login --account feishu_main --capability message-write"
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
            "flow": "manual_exchange",
            "exchange_command": "loong feishu auth exchange --account work --callback-url '<full_callback_url>'",
            "manual_note": "Copy the full callback URL into `feishu auth exchange`.",
        });

        let rendered = render_auth_start_text(&payload).expect("render auth start");

        assert!(rendered.contains("configured_account: work"));
        assert!(rendered.contains("flow: manual_exchange"));
        assert!(rendered.contains("next_step: loong feishu auth exchange --account work --callback-url '<full_callback_url>'"));
        assert!(rendered.contains("note: Copy the full callback URL into `feishu auth exchange`."));
    }

    #[test]
    fn resolve_feishu_auth_exchange_input_accepts_callback_url() {
        let args = FeishuAuthExchangeArgs {
            common: FeishuCommonArgs {
                config: None,
                account: Some("work".to_owned()),
                json: false,
            },
            state: None,
            code: None,
            callback_url: Some(
                "http://127.0.0.1:34819/callback?code=code-123&state=state-123".to_owned(),
            ),
        };

        let (state, code, callback_url) =
            resolve_feishu_auth_exchange_input(&args).expect("parse callback url");

        assert_eq!(state, "state-123");
        assert_eq!(code, "code-123");
        assert_eq!(
            callback_url.as_deref(),
            Some("http://127.0.0.1:34819/callback?code=code-123&state=state-123")
        );
    }

    #[test]
    fn parse_feishu_loopback_redirect_spec_accepts_loopback_http_uri() {
        let spec = parse_feishu_loopback_redirect_spec("http://127.0.0.1:34819/callback")
            .expect("parse loopback redirect spec");

        assert_eq!(spec.redirect_uri, "http://127.0.0.1:34819/callback");
        assert_eq!(spec.bind_target, "127.0.0.1:34819");
        assert_eq!(spec.path, "/callback");
    }

    #[test]
    fn reject_feishu_websocket_listener_overrides_requires_no_bind_or_path() {
        assert!(reject_feishu_websocket_listener_overrides(None, None).is_ok());
        let error = reject_feishu_websocket_listener_overrides(Some("127.0.0.1:8080"), None)
            .expect_err("websocket listener overrides should be rejected");
        assert!(error.contains("--mode websocket"));
    }

    #[test]
    fn apply_feishu_mode_override_prefers_selected_account_override() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.feishu.accounts.insert(
            "work".to_owned(),
            mvp::config::FeishuAccountConfig::default(),
        );

        apply_feishu_mode_override(&mut config, "work", FeishuServeModeOverride::Webhook);

        assert_eq!(
            config
                .feishu
                .accounts
                .get("work")
                .and_then(|account| account.mode),
            Some(mvp::config::FeishuChannelServeMode::Webhook)
        );
    }

    #[tokio::test]
    async fn start_feishu_local_callback_server_rejects_unavailable_port() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind occupied listener");
        let port = listener
            .local_addr()
            .expect("occupied listener addr")
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");
        let error = start_feishu_local_callback_server(redirect_uri.as_str(), "state-123")
            .await
            .expect_err("occupied port should reject local callback listener");

        assert!(error.contains("start local Feishu callback listener"));
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
