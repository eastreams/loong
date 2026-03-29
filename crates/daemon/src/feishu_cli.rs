use std::fs;
use std::path::Path;

use clap::{Args, Subcommand, ValueEnum};
use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use serde_json::{Value, json};

use crate::feishu_support::{
    FeishuAuthCapability, FeishuDaemonContext, build_account_recommendations,
    build_grant_recommendations, build_pkce_pair, feishu_auth_start_command_hint,
    generate_oauth_state, load_feishu_daemon_context, normalized_auth_start_capabilities,
    resolve_scopes, unix_ts_now,
};

const DEFAULT_FEISHU_REDIRECT_URI: &str = "http://127.0.0.1:34819/callback";

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

pub async fn run_feishu_command(command: FeishuCommand) -> CliResult<()> {
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
        "feishu auth start".to_owned(),
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
