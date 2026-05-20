use std::fs;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use loong_app as mvp;
use loong_spec::CliResult;
use serde_json::{Value, json};

use crate::feishu_onboarding::{
    FeishuOnboardApplyOptions, FeishuOnboardCredentialSource, FeishuOnboardCredentials,
    apply_manual_feishu_onboarding, onboard_via_qr_registration,
};
use crate::feishu_support::{
    FeishuAuthCapability, FeishuConfiguredCapability, FeishuDaemonContext,
    build_account_recommendations, build_grant_recommendations, build_pkce_pair,
    configured_capabilities_from_config, feishu_auth_start_command_hint, generate_oauth_state,
    load_feishu_daemon_context, normalized_auth_start_capabilities,
    summarize_required_doc_write_scope_status, summarize_required_message_write_scope_status,
    unix_ts_now,
};

const DEFAULT_FEISHU_REDIRECT_URI: &str = "http://127.0.0.1:34819/callback";

#[path = "feishu_cli/bitable.rs"]
mod bitable;
#[path = "feishu_cli/render.rs"]
mod render;
#[cfg(test)]
#[path = "feishu_cli/render_tests.rs"]
mod render_tests;

pub use self::bitable::*;
use self::render::*;

fn active_cli_command_name() -> &'static str {
    mvp::config::active_cli_command_name()
}

#[derive(Subcommand, Debug)]
pub enum FeishuCommand {
    /// Start or inspect user OAuth grants and state
    Auth {
        #[command(subcommand)]
        command: FeishuAuthCommand,
    },
    /// Create or update Feishu/Lark bot channel credentials in loong.toml
    Onboard(FeishuOnboardArgs),
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
    Serve(FeishuServeArgs),
}

#[derive(Subcommand, Debug)]
pub enum FeishuAuthCommand {
    /// Generate an OAuth authorize URL and persist short-lived state locally
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
    Revoke(FeishuGrantArgs),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FeishuOnboardDomainArg {
    Feishu,
    Lark,
}

impl FeishuOnboardDomainArg {
    fn as_config_domain(self) -> mvp::config::FeishuDomain {
        match self {
            Self::Feishu => mvp::config::FeishuDomain::Feishu,
            Self::Lark => mvp::config::FeishuDomain::Lark,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FeishuOnboardModeArg {
    Websocket,
    Webhook,
}

impl FeishuOnboardModeArg {
    fn as_config_mode(self) -> mvp::config::FeishuChannelServeMode {
        match self {
            Self::Websocket => mvp::config::FeishuChannelServeMode::Websocket,
            Self::Webhook => mvp::config::FeishuChannelServeMode::Webhook,
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct FeishuOnboardArgs {
    #[command(flatten)]
    pub common: FeishuCommonArgs,
    #[arg(long, default_value_t = FeishuOnboardDomainArg::Feishu, value_enum)]
    pub domain: FeishuOnboardDomainArg,
    #[arg(long, value_enum)]
    pub mode: Option<FeishuOnboardModeArg>,
    #[arg(long)]
    pub timeout_s: Option<u64>,
    #[arg(long, default_value_t = false)]
    pub manual: bool,
    #[arg(long)]
    pub app_id: Option<String>,
    #[arg(long)]
    pub app_secret: Option<String>,
    #[arg(long)]
    pub verification_token: Option<String>,
    #[arg(long)]
    pub encrypt_key: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FeishuAuthStartArgs {
    #[command(flatten)]
    pub common: FeishuCommonArgs,
    #[arg(long, default_value = DEFAULT_FEISHU_REDIRECT_URI)]
    pub redirect_uri: String,
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
    pub state: String,
    #[arg(long)]
    pub code: String,
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
    #[arg(long)]
    pub bind: Option<String>,
    #[arg(long)]
    pub path: Option<String>,
}

pub fn run_feishu_command(
    command: FeishuCommand,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = CliResult<()>> + Send>> {
    Box::pin(async move {
        match command {
            FeishuCommand::Auth { command } => match command {
                FeishuAuthCommand::Start(args) => {
                    let payload = execute_feishu_auth_start(&args).await?;
                    print_feishu_payload(&payload, args.common.json, render_auth_start_text)?;
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
            },
            FeishuCommand::Onboard(args) => {
                let payload = execute_feishu_onboard(&args).await?;
                print_feishu_payload(&payload, args.common.json, render_onboard_text)?;
            }
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
                    print_feishu_payload(
                        &payload,
                        args.grant.common.json,
                        render_messages_get_text,
                    )?;
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
                    print_feishu_payload(
                        &payload,
                        args.grant.common.json,
                        render_calendar_list_text,
                    )?;
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
                    print_feishu_payload(
                        &payload,
                        args.grant.common.json,
                        render_bitable_app_text,
                    )?;
                }
                FeishuBitableCommand::AppGet(args) => {
                    let payload = execute_feishu_bitable_app_get(&args).await?;
                    print_feishu_payload(
                        &payload,
                        args.grant.common.json,
                        render_bitable_app_text,
                    )?;
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
                    print_feishu_payload(
                        &payload,
                        args.grant.common.json,
                        render_bitable_app_text,
                    )?;
                }
                FeishuBitableCommand::AppCopy(args) => {
                    let payload = execute_feishu_bitable_app_copy(&args).await?;
                    print_feishu_payload(
                        &payload,
                        args.grant.common.json,
                        render_bitable_app_text,
                    )?;
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
                    print_feishu_payload(
                        &payload,
                        args.grant.common.json,
                        render_bitable_table_text,
                    )?;
                }
                FeishuBitableCommand::PatchTable(args) => {
                    let payload = execute_feishu_bitable_patch_table(&args).await?;
                    print_feishu_payload(
                        &payload,
                        args.grant.common.json,
                        render_bitable_table_text,
                    )?;
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
                    print_feishu_payload(
                        &payload,
                        args.grant.common.json,
                        render_bitable_field_text,
                    )?;
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
                    print_feishu_payload(
                        &payload,
                        args.grant.common.json,
                        render_bitable_field_text,
                    )?;
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
                    print_feishu_payload(
                        &payload,
                        args.grant.common.json,
                        render_bitable_view_text,
                    )?;
                }
                FeishuBitableCommand::GetView(args) => {
                    let payload = execute_feishu_bitable_get_view(&args).await?;
                    print_feishu_payload(
                        &payload,
                        args.grant.common.json,
                        render_bitable_view_text,
                    )?;
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
                    print_feishu_payload(
                        &payload,
                        args.grant.common.json,
                        render_bitable_view_text,
                    )?;
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
            FeishuCommand::Serve(args) => {
                mvp::channel::run_feishu_channel(
                    args.common.config.as_deref(),
                    args.common.account.as_deref(),
                    args.bind.as_deref(),
                    args.path.as_deref(),
                )
                .await?;
            }
        }
        Ok(())
    })
}

pub async fn execute_feishu_onboard(args: &FeishuOnboardArgs) -> CliResult<Value> {
    ensure_feishu_onboard_config_exists(args.common.config.as_deref())?;

    let mode = args
        .mode
        .unwrap_or(FeishuOnboardModeArg::Websocket)
        .as_config_mode();
    let manual = args.manual
        || args.app_id.is_some()
        || args.app_secret.is_some()
        || args.verification_token.is_some()
        || args.encrypt_key.is_some();

    if mode != mvp::config::FeishuChannelServeMode::Webhook
        && (args.verification_token.is_some() || args.encrypt_key.is_some())
    {
        return Err("webhook verification_token/encrypt_key require `--mode webhook`".to_owned());
    }

    let result = if manual {
        let app_id = trimmed_opt(args.app_id.as_deref())
            .ok_or_else(|| "manual Feishu onboarding requires `--app-id`".to_owned())?;
        let app_secret = trimmed_opt(args.app_secret.as_deref())
            .ok_or_else(|| "manual Feishu onboarding requires `--app-secret`".to_owned())?;
        let verification_token = trimmed_opt(args.verification_token.as_deref()).map(str::to_owned);
        let encrypt_key = trimmed_opt(args.encrypt_key.as_deref()).map(str::to_owned);
        if mode == mvp::config::FeishuChannelServeMode::Webhook
            && (verification_token.is_none() || encrypt_key.is_none())
        {
            return Err(
                "manual Feishu webhook onboarding requires both `--verification-token` and `--encrypt-key`"
                    .to_owned(),
            );
        }

        apply_manual_feishu_onboarding(
            args.common.config.as_deref(),
            args.common.account.as_deref(),
            &FeishuOnboardCredentials {
                app_id: app_id.to_owned(),
                app_secret: app_secret.to_owned(),
                verification_token,
                encrypt_key,
            },
            FeishuOnboardApplyOptions {
                domain: args.domain.as_config_domain(),
                mode,
            },
        )?
    } else {
        if mode != mvp::config::FeishuChannelServeMode::Websocket {
            return Err(
                "QR-based Feishu/Lark onboarding currently supports `--mode websocket` only; use `--manual` for webhook credentials"
                    .to_owned(),
            );
        }
        onboard_via_qr_registration(
            args.common.config.as_deref(),
            args.common.account.as_deref(),
            args.domain.as_config_domain(),
            args.timeout_s,
            Some(mode),
        )
        .await?
    };

    let serve_command = if result.configured_account_id == "feishu_cli_default" {
        "loong feishu serve".to_owned()
    } else {
        format!(
            "loong feishu serve --account {}",
            result.configured_account_id
        )
    };
    let mut notes = vec!["run `loong doctor` to verify the saved channel contract".to_owned()];
    if result.owner_direct_chat_bootstrap_applied {
        if let Some(owner_open_id) = result.owner_open_id.as_deref() {
            notes.push(format!(
                "defaulted inbound bootstrap access to `allowed_chat_ids = [\"*\"]` and `allowed_sender_ids = [\"{owner_open_id}\"]` so the onboarding user can start a direct Feishu/Lark chat immediately"
            ));
            notes.push(
                "tighten `allowed_chat_ids` after first-run validation if you want the bot limited to specific chats"
                    .to_owned(),
            );
        }
    } else {
        notes.push(
            "set `feishu.allowed_chat_ids` and, when needed, `feishu.allowed_sender_ids` before running the long-lived reply loop in production"
                .to_owned(),
        );
    }
    if result.credential_source == FeishuOnboardCredentialSource::QrRegistration {
        notes.push(
            "QR registration writes the generated bot app_id/app_secret directly into loong.toml and defaults the channel to websocket mode"
                .to_owned(),
        );
    }
    if result.mode == mvp::config::FeishuChannelServeMode::Webhook {
        notes.push(
            "webhook mode expects Feishu event delivery to target the bind/path you pass to `loong feishu serve`"
                .to_owned(),
        );
    }

    Ok(json!({
        "account_id": result.runtime_account_id,
        "configured_account": result.configured_account_label,
        "configured_account_id": result.configured_account_id,
        "config": result.config_path,
        "credential_source": result.credential_source.as_str(),
        "domain": result.domain.as_str(),
        "mode": result.mode.as_str(),
        "owner_open_id": result.owner_open_id,
        "bot_name": result.bot_name,
        "bot_open_id": result.bot_open_id,
        "qr_url": result.qr_url,
        "qr_rendered": result.qr_rendered,
        "owner_direct_chat_bootstrap_applied": result.owner_direct_chat_bootstrap_applied,
        "serve_command": serve_command,
        "status_command": "loong doctor",
        "notes": notes,
    }))
}

pub async fn execute_feishu_auth_start(args: &FeishuAuthStartArgs) -> CliResult<Value> {
    let context = load_feishu_daemon_context(
        args.common.config.as_deref(),
        args.common.account.as_deref(),
    )?;
    let client = context.build_client()?;
    let capabilities =
        normalized_auth_start_capabilities(&args.capabilities, args.include_message_write);
    let scopes = context.required_scopes(&args.scopes, &capabilities, args.include_message_write);
    let reported_capabilities = if capabilities.is_empty()
        && context
            .config
            .feishu_integration
            .has_explicit_capability_config()
    {
        configured_capabilities_from_config(&context.config.feishu_integration)
            .into_iter()
            .map(FeishuConfiguredCapability::as_config_key)
            .collect::<Vec<_>>()
    } else {
        capabilities
            .iter()
            .map(|capability| capability.as_cli_value())
            .collect::<Vec<_>>()
    };
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

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "config": context.config_path.display().to_string(),
        "redirect_uri": args.redirect_uri.trim(),
        "state": state,
        "authorize_url": authorize_url,
        "sqlite_path": context.store.path().display().to_string(),
        "expires_at_s": record.expires_at_s,
        "capabilities": reported_capabilities,
        "scopes": scopes,
    }))
}

pub async fn execute_feishu_auth_exchange(args: &FeishuAuthExchangeArgs) -> CliResult<Value> {
    let context = load_feishu_daemon_context(
        args.common.config.as_deref(),
        args.common.account.as_deref(),
    )?;
    let now_s = unix_ts_now();
    let stored_state = context.store.consume_oauth_state(&args.state, now_s)?;
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
            &args.code,
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
    }))
}

pub async fn execute_feishu_auth_list(args: &FeishuAuthListArgs) -> CliResult<Value> {
    let context = load_feishu_daemon_context(
        args.common.config.as_deref(),
        args.common.account.as_deref(),
    )?;
    let required_scopes = context.required_scopes(&[], &[], false);
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
            &context.required_scopes(&[], &[], false),
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
    let required_scopes = context.required_scopes(&[], &[], false);
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
            "doc_write_status": summarize_required_doc_write_scope_status(None, &required_scopes),
            "message_write_status": summarize_required_message_write_scope_status(None, &required_scopes),
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
        "doc_write_status": summarize_required_doc_write_scope_status(grant.as_ref(), &required_scopes),
        "message_write_status": summarize_required_message_write_scope_status(grant.as_ref(), &required_scopes),
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
            &mvp::channel::feishu::api::resources::calendar::FeishuPrimaryCalendarQuery {
                user_id_type: Some(
                    args.user_id_type
                        .clone()
                        .unwrap_or_else(|| "open_id".to_owned()),
                ),
            },
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

fn ensure_feishu_onboard_config_exists(raw: Option<&str>) -> CliResult<PathBuf> {
    let path = raw
        .map(mvp::config::expand_path)
        .unwrap_or_else(mvp::config::default_config_path);
    verify_feishu_onboard_config_exists(&path)
}

fn verify_feishu_onboard_config_exists(path: &Path) -> CliResult<PathBuf> {
    if path.exists() {
        return Ok(path.to_path_buf());
    }
    let cli = active_cli_command_name();
    Err(format!(
        "config file {} not found; run `{cli} onboard` to complete initial configuration before running `{cli} feishu onboard`",
        path.display(),
    ))
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
        "doc_write_status": summarize_required_doc_write_scope_status(Some(grant), required_scopes),
        "message_write_status": summarize_required_message_write_scope_status(Some(grant), required_scopes),
        "recommendations": build_grant_recommendations(
            configured_account_id,
            Some(grant),
            now_s,
            required_scopes,
        ),
    })
}

#[cfg(test)]
mod onboard_config_precheck_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn verify_returns_path_when_config_file_exists() {
        let dir = TempDir::new().expect("create tempdir");
        let config_path = dir.path().join("config.toml");
        fs::write(&config_path, "").expect("write stub config");

        let resolved = verify_feishu_onboard_config_exists(&config_path)
            .expect("precheck should pass when the config file exists");

        assert_eq!(resolved, config_path);
    }

    #[test]
    fn verify_returns_error_hinting_onboard_when_config_file_missing() {
        let dir = TempDir::new().expect("create tempdir");
        let missing = dir.path().join("config.toml");

        let err = verify_feishu_onboard_config_exists(&missing)
            .expect_err("precheck should fail when the config file is missing");

        assert!(
            err.contains("config file"),
            "error should mention config file: {err}"
        );
        assert!(
            err.contains("not found"),
            "error should mention not found: {err}"
        );
        assert!(
            err.contains(&format!("`{} onboard`", active_cli_command_name())),
            "error should reference `{} onboard` as the remediation: {err}",
            active_cli_command_name(),
        );
        assert!(
            err.contains(&missing.display().to_string()),
            "error should surface the missing config path: {err}",
        );
    }

    #[test]
    fn ensure_honors_explicit_config_override_when_present() {
        let dir = TempDir::new().expect("create tempdir");
        let config_path = dir.path().join("config.toml");
        fs::write(&config_path, "").expect("write stub config");

        let display = config_path.display().to_string();
        let resolved = ensure_feishu_onboard_config_exists(Some(display.as_str()))
            .expect("precheck should pass for an explicit existing path");

        assert_eq!(resolved, config_path);
    }

    #[test]
    fn ensure_errors_when_explicit_config_override_missing() {
        let dir = TempDir::new().expect("create tempdir");
        let missing = dir.path().join("does-not-exist.toml");
        let display = missing.display().to_string();

        let err = ensure_feishu_onboard_config_exists(Some(display.as_str()))
            .expect_err("precheck should fail when the explicit path is missing");

        assert!(
            err.contains(&display),
            "error should surface the explicit missing path: {err}",
        );
        assert!(
            err.contains("onboard"),
            "error should point the user at the onboard command: {err}",
        );
    }
}
