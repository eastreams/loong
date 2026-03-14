#[cfg(feature = "channel-telegram")]
use std::time::Duration;
use std::{fmt, str::FromStr};
#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use async_trait::async_trait;
use serde::Serialize;
#[cfg(feature = "channel-telegram")]
use tokio::time::sleep;

use crate::CliResult;
#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use crate::KernelContext;
#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use crate::acp::{AcpConversationTurnOptions, AcpTurnProvenance};
#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_kernel_context};

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use super::config::{
    ChannelResolvedAccountRoute, LoongClawConfig, ResolvedFeishuChannelConfig,
    ResolvedTelegramChannelConfig,
};
#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use super::conversation::{
    ConversationSessionAddress, ConversationTurnCoordinator, ProviderErrorMode,
};

#[cfg(feature = "channel-feishu")]
mod feishu;
mod registry;
mod runtime_state;
#[cfg(feature = "channel-telegram")]
mod telegram;

pub use registry::{
    CHANNEL_OPERATION_SEND_ID, CHANNEL_OPERATION_SERVE_ID, ChannelCapability,
    ChannelCatalogCommandFamilyDescriptor, ChannelCatalogEntry, ChannelCatalogImplementationStatus,
    ChannelCatalogOperation, ChannelCatalogOperationAvailability,
    ChannelCatalogOperationRequirement, ChannelCommandFamilyDescriptor, ChannelDoctorCheckSpec,
    ChannelDoctorCheckTrigger, ChannelDoctorOperationSpec, ChannelInventory,
    ChannelOnboardingDescriptor, ChannelOnboardingStrategy, ChannelOperationDescriptor,
    ChannelOperationHealth, ChannelOperationStatus, ChannelRuntimeCommandDescriptor,
    ChannelStatusSnapshot, ChannelSurface, FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    FEISHU_COMMAND_FAMILY_DESCRIPTOR, FEISHU_RUNTIME_COMMAND_DESCRIPTOR,
    TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR, TELEGRAM_COMMAND_FAMILY_DESCRIPTOR,
    TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR, catalog_only_channel_entries, channel_inventory,
    channel_status_snapshots, list_channel_catalog, normalize_channel_catalog_id,
    normalize_channel_platform, resolve_channel_catalog_command_family_descriptor,
    resolve_channel_catalog_entry, resolve_channel_catalog_operation,
    resolve_channel_command_family_descriptor, resolve_channel_doctor_operation_spec,
    resolve_channel_onboarding_descriptor, resolve_channel_operation_descriptor,
    resolve_channel_runtime_command_descriptor,
};
pub use runtime_state::ChannelOperationRuntime;
use runtime_state::ChannelOperationRuntimeTracker;

#[derive(Debug, Clone, Default)]
pub struct ChannelDelivery {
    #[allow(dead_code)]
    pub ack_cursor: Option<String>,
    #[allow(dead_code)]
    pub source_message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelSendReceipt {
    pub channel: &'static str,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelPlatform {
    Telegram,
    Feishu,
}

impl ChannelPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::Feishu => "feishu",
        }
    }
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSession {
    pub platform: ChannelPlatform,
    pub account_id: Option<String>,
    pub conversation_id: String,
    pub thread_id: Option<String>,
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
impl ChannelSession {
    pub fn new(platform: ChannelPlatform, conversation_id: impl Into<String>) -> Self {
        Self {
            platform,
            account_id: None,
            conversation_id: conversation_id.into(),
            thread_id: None,
        }
    }

    pub fn with_account(
        platform: ChannelPlatform,
        account_id: impl Into<String>,
        conversation_id: impl Into<String>,
    ) -> Self {
        Self {
            platform,
            account_id: Some(account_id.into()),
            conversation_id: conversation_id.into(),
            thread_id: None,
        }
    }

    pub fn with_thread(
        platform: ChannelPlatform,
        conversation_id: impl Into<String>,
        thread_id: impl Into<String>,
    ) -> Self {
        Self {
            platform,
            account_id: None,
            conversation_id: conversation_id.into(),
            thread_id: Some(thread_id.into()),
        }
    }

    pub fn with_account_and_thread(
        platform: ChannelPlatform,
        account_id: impl Into<String>,
        conversation_id: impl Into<String>,
        thread_id: impl Into<String>,
    ) -> Self {
        Self {
            platform,
            account_id: Some(account_id.into()),
            conversation_id: conversation_id.into(),
            thread_id: Some(thread_id.into()),
        }
    }

    pub fn session_key(&self) -> String {
        let account_id = self
            .account_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let conversation_id = self.conversation_id.trim();
        let thread_id = self
            .thread_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        match (account_id, thread_id) {
            (Some(account_id), Some(thread_id)) => format!(
                "{}:{account_id}:{conversation_id}:{thread_id}",
                self.platform.as_str()
            ),
            (Some(account_id), None) => {
                format!("{}:{account_id}:{conversation_id}", self.platform.as_str())
            }
            (None, Some(thread_id)) => {
                format!("{}:{conversation_id}:{thread_id}", self.platform.as_str())
            }
            (None, None) => format!("{}:{conversation_id}", self.platform.as_str()),
        }
    }

    pub fn conversation_address(&self) -> ConversationSessionAddress {
        let mut address = ConversationSessionAddress::from_session_id(self.session_key())
            .with_channel_scope(self.platform.as_str(), self.conversation_id.clone());
        if let Some(account_id) = self.account_id.as_deref() {
            address = address.with_account_id(account_id);
        }
        if let Some(thread_id) = self.thread_id.as_deref() {
            address = address.with_thread_id(thread_id);
        }
        address
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelOutboundTargetKind {
    Conversation,
    MessageReply,
    ReceiveId,
}

impl ChannelOutboundTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::MessageReply => "message_reply",
            Self::ReceiveId => "receive_id",
        }
    }
}

impl fmt::Display for ChannelOutboundTargetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ChannelOutboundTargetKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
        match normalized.as_str() {
            "conversation" => Ok(Self::Conversation),
            "message_reply" => Ok(Self::MessageReply),
            "receive_id" => Ok(Self::ReceiveId),
            _ => Err(format!(
                "unsupported channel target kind `{value}`; expected conversation, message_reply, or receive_id"
            )),
        }
    }
}

pub use self::ChannelOutboundTargetKind as ChannelCatalogTargetKind;

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelOutboundTarget {
    pub platform: ChannelPlatform,
    pub kind: ChannelOutboundTargetKind,
    pub id: String,
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
impl ChannelOutboundTarget {
    pub fn new(
        platform: ChannelPlatform,
        kind: ChannelOutboundTargetKind,
        id: impl Into<String>,
    ) -> Self {
        Self {
            platform,
            kind,
            id: id.into(),
        }
    }

    pub fn telegram_chat(chat_id: i64) -> Self {
        Self::new(
            ChannelPlatform::Telegram,
            ChannelOutboundTargetKind::Conversation,
            chat_id.to_string(),
        )
    }

    pub fn telegram_chat_thread(chat_id: i64, thread_id: i64) -> Self {
        Self::new(
            ChannelPlatform::Telegram,
            ChannelOutboundTargetKind::Conversation,
            format!("{chat_id}:topic:{thread_id}"),
        )
    }

    pub fn feishu_message_reply(message_id: impl Into<String>) -> Self {
        Self::new(
            ChannelPlatform::Feishu,
            ChannelOutboundTargetKind::MessageReply,
            message_id,
        )
    }

    pub fn feishu_receive_id(receive_id: impl Into<String>) -> Self {
        Self::new(
            ChannelPlatform::Feishu,
            ChannelOutboundTargetKind::ReceiveId,
            receive_id,
        )
    }

    pub fn trimmed_id(&self) -> CliResult<&str> {
        let id = self.id.trim();
        if id.is_empty() {
            return Err(format!(
                "channel target id is empty for {} {}",
                self.platform.as_str(),
                self.kind.as_str()
            ));
        }
        Ok(id)
    }
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[derive(Debug, Clone)]
pub struct ChannelInboundMessage {
    pub session: ChannelSession,
    pub reply_target: ChannelOutboundTarget,
    pub text: String,
    pub delivery: ChannelDelivery,
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ChannelResolvedAcpTurnHints {
    bootstrap_mcp_servers: Vec<String>,
    working_directory: Option<PathBuf>,
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[allow(dead_code)]
#[async_trait]
pub trait ChannelAdapter {
    fn name(&self) -> &str;
    async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>>;
    async fn send_text(&self, target: &ChannelOutboundTarget, text: &str) -> CliResult<()>;
    async fn start_typing(&self, _target: &ChannelOutboundTarget) -> CliResult<()> {
        Ok(())
    }
    async fn stop_typing(&self, _target: &ChannelOutboundTarget) -> CliResult<()> {
        Ok(())
    }
    async fn ack_inbound(&mut self, _message: &ChannelInboundMessage) -> CliResult<()> {
        Ok(())
    }
    async fn complete_batch(&mut self) -> CliResult<()> {
        Ok(())
    }
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
type ChannelProcessFuture = Pin<Box<dyn Future<Output = CliResult<String>> + Send>>;

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
type ChannelCommandFuture<'a> = Pin<Box<dyn Future<Output = CliResult<()>> + Send + 'a>>;

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum KnownChannelSessionSendTarget {
    Telegram {
        account_id: Option<String>,
        chat_id: String,
        thread_id: Option<String>,
    },
    Feishu {
        account_id: Option<String>,
        conversation_id: String,
        reply_message_id: Option<String>,
    },
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
fn parse_known_channel_session_send_target(
    config: &LoongClawConfig,
    session_id: &str,
) -> CliResult<KnownChannelSessionSendTarget> {
    let mut parts = session_id.split(':');
    let channel = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("sessions_send_channel_unsupported: `{session_id}`"))?;
    let scope = parts.map(str::trim).collect::<Vec<_>>();

    match channel {
        "telegram" => parse_telegram_session_send_target(config, session_id, scope.as_slice()),
        "feishu" | "lark" => parse_feishu_session_send_target(config, session_id, scope.as_slice()),
        _ => Err(format!("sessions_send_channel_unsupported: `{session_id}`")),
    }
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
fn parse_telegram_session_send_target(
    config: &LoongClawConfig,
    session_id: &str,
    scope: &[&str],
) -> CliResult<KnownChannelSessionSendTarget> {
    let account_known = |account_id: &str| {
        config
            .telegram
            .resolve_account_for_session_account_id(Some(account_id))
            .is_ok()
    };

    match scope {
        [chat_id] if !chat_id.is_empty() => Ok(KnownChannelSessionSendTarget::Telegram {
            account_id: None,
            chat_id: (*chat_id).to_owned(),
            thread_id: None,
        }),
        [first_segment, second_segment]
            if !first_segment.is_empty() && !second_segment.is_empty() =>
        {
            if account_known(first_segment) {
                Ok(KnownChannelSessionSendTarget::Telegram {
                    account_id: Some((*first_segment).to_owned()),
                    chat_id: (*second_segment).to_owned(),
                    thread_id: None,
                })
            } else {
                Ok(KnownChannelSessionSendTarget::Telegram {
                    account_id: None,
                    chat_id: (*first_segment).to_owned(),
                    thread_id: Some((*second_segment).to_owned()),
                })
            }
        }
        [account_id, chat_id, thread_id]
            if !account_id.is_empty() && !chat_id.is_empty() && !thread_id.is_empty() =>
        {
            if !account_known(account_id) {
                return Err(format!("sessions_send_channel_unsupported: `{session_id}`"));
            }
            Ok(KnownChannelSessionSendTarget::Telegram {
                account_id: Some((*account_id).to_owned()),
                chat_id: (*chat_id).to_owned(),
                thread_id: Some((*thread_id).to_owned()),
            })
        }
        _ => Err(format!("sessions_send_channel_unsupported: `{session_id}`")),
    }
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
fn parse_feishu_session_send_target(
    config: &LoongClawConfig,
    session_id: &str,
    scope: &[&str],
) -> CliResult<KnownChannelSessionSendTarget> {
    let account_known = |account_id: &str| {
        config
            .feishu
            .resolve_account_for_session_account_id(Some(account_id))
            .is_ok()
    };

    match scope {
        [conversation_id] if !conversation_id.is_empty() => {
            Ok(KnownChannelSessionSendTarget::Feishu {
                account_id: None,
                conversation_id: (*conversation_id).to_owned(),
                reply_message_id: None,
            })
        }
        [first_segment, second_segment]
            if !first_segment.is_empty() && !second_segment.is_empty() =>
        {
            if account_known(first_segment) {
                Ok(KnownChannelSessionSendTarget::Feishu {
                    account_id: Some((*first_segment).to_owned()),
                    conversation_id: (*second_segment).to_owned(),
                    reply_message_id: None,
                })
            } else {
                Ok(KnownChannelSessionSendTarget::Feishu {
                    account_id: None,
                    conversation_id: (*first_segment).to_owned(),
                    reply_message_id: Some((*second_segment).to_owned()),
                })
            }
        }
        [account_id, conversation_id, reply_message_id]
            if !account_id.is_empty()
                && !conversation_id.is_empty()
                && !reply_message_id.is_empty() =>
        {
            if !account_known(account_id) {
                return Err(format!("sessions_send_channel_unsupported: `{session_id}`"));
            }
            Ok(KnownChannelSessionSendTarget::Feishu {
                account_id: Some((*account_id).to_owned()),
                conversation_id: (*conversation_id).to_owned(),
                reply_message_id: Some((*reply_message_id).to_owned()),
            })
        }
        _ => Err(format!("sessions_send_channel_unsupported: `{session_id}`")),
    }
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
async fn process_channel_batch<A, F>(
    adapter: &mut A,
    batch: Vec<ChannelInboundMessage>,
    runtime: Option<&ChannelOperationRuntimeTracker>,
    mut process: F,
) -> CliResult<bool>
where
    A: ChannelAdapter + Send + Sync + ?Sized,
    F: FnMut(ChannelInboundMessage) -> ChannelProcessFuture,
{
    if batch.is_empty() {
        adapter.complete_batch().await?;
        return Ok(false);
    }

    for message in &batch {
        if let Some(runtime) = runtime {
            runtime.mark_run_start().await?;
        }

        let _ = adapter.start_typing(&message.reply_target).await;

        let result = async {
            let reply = process(message.clone()).await?;
            adapter.send_text(&message.reply_target, &reply).await?;
            adapter.ack_inbound(message).await?;
            Ok::<(), String>(())
        }
        .await;

        let _ = adapter.stop_typing(&message.reply_target).await;

        if let Some(runtime) = runtime {
            runtime.mark_run_end().await?;
        }

        result?;
    }

    adapter.complete_batch().await?;
    Ok(true)
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[derive(Debug, Clone)]
struct ChannelCommandContext<R> {
    resolved_path: PathBuf,
    config: LoongClawConfig,
    resolved: R,
    route: ChannelResolvedAccountRoute,
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
impl<R> ChannelCommandContext<R> {
    fn emit_route_notice(&self, platform: ChannelPlatform) {
        if let Some(notice) = render_channel_route_notice(platform, &self.route) {
            #[allow(clippy::print_stderr)]
            {
                eprintln!("warning: {notice}");
            }
        }
    }
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
trait ChannelResolvedRuntimeAccount {
    fn runtime_account_id(&self) -> &str;
    fn runtime_account_label(&self) -> &str;
}

#[cfg(feature = "channel-telegram")]
impl ChannelResolvedRuntimeAccount for ResolvedTelegramChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

#[cfg(feature = "channel-feishu")]
impl ChannelResolvedRuntimeAccount for ResolvedFeishuChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
async fn run_channel_send_command<R, F, G>(
    context: ChannelCommandContext<R>,
    spec: ChannelSendCommandSpec,
    send: F,
    render_success: G,
) -> CliResult<()>
where
    F: for<'a> FnOnce(&'a ChannelCommandContext<R>) -> ChannelCommandFuture<'a>,
    G: FnOnce(&ChannelCommandContext<R>) -> String,
{
    crate::runtime_env::initialize_runtime_environment(
        &context.config,
        Some(context.resolved_path.as_path()),
    );
    context.emit_route_notice(spec.family.runtime.platform);
    send(&context).await?;

    #[allow(clippy::print_stdout)]
    {
        println!("{}", render_success(&context));
    }
    Ok(())
}

#[cfg(feature = "channel-telegram")]
fn load_telegram_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTelegramChannelConfig>> {
    let (resolved_path, config) = super::config::load(config_path)?;
    build_telegram_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-telegram")]
fn build_telegram_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTelegramChannelConfig>> {
    let resolved = config.telegram.resolve_account(account_id)?;
    let route = config
        .telegram
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "telegram account `{}` is disabled by configuration",
            resolved.configured_account_id
        ));
    }
    Ok(ChannelCommandContext {
        resolved_path,
        config,
        resolved,
        route,
    })
}

#[cfg(feature = "channel-feishu")]
fn load_feishu_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedFeishuChannelConfig>> {
    let (resolved_path, config) = super::config::load(config_path)?;
    build_feishu_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-feishu")]
fn build_feishu_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedFeishuChannelConfig>> {
    let resolved = config.feishu.resolve_account(account_id)?;
    let route = config
        .feishu
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "feishu account `{}` is disabled by configuration",
            resolved.configured_account_id
        ));
    }
    Ok(ChannelCommandContext {
        resolved_path,
        config,
        resolved,
        route,
    })
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[derive(Debug, Clone, Copy)]
struct ChannelSendCommandSpec {
    family: ChannelCommandFamilyDescriptor,
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[derive(Debug, Clone, Copy)]
struct ChannelServeRuntimeSpec<'a> {
    platform: ChannelPlatform,
    operation_id: &'static str,
    account_id: &'a str,
    account_label: &'a str,
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[derive(Debug, Clone, Copy)]
struct ChannelServeCommandSpec {
    family: ChannelCommandFamilyDescriptor,
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
fn channel_runtime_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
fn ensure_channel_operation_runtime_slot_available_in_dir(
    runtime_dir: &std::path::Path,
    spec: ChannelServeRuntimeSpec<'_>,
) -> CliResult<()> {
    let Some(runtime) = runtime_state::load_channel_operation_runtime_for_account_from_dir(
        runtime_dir,
        spec.platform,
        spec.operation_id,
        spec.account_id,
        channel_runtime_now_ms(),
    ) else {
        return Ok(());
    };
    if runtime.running_instances == 0 {
        return Ok(());
    }

    let pid = runtime
        .pid
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    Err(format!(
        "{} account `{}` already has an active {} runtime (pid={}, running_instances={}); stop the existing instance or wait for it to become stale before restarting",
        spec.platform.as_str(),
        spec.account_id,
        spec.operation_id,
        pid,
        runtime.running_instances
    ))
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
async fn with_channel_serve_runtime<T, F, Fut>(
    spec: ChannelServeRuntimeSpec<'_>,
    run: F,
) -> CliResult<T>
where
    F: FnOnce(Arc<ChannelOperationRuntimeTracker>) -> Fut,
    Fut: Future<Output = CliResult<T>>,
{
    ensure_channel_operation_runtime_slot_available_in_dir(
        runtime_state::default_channel_runtime_state_dir().as_path(),
        spec,
    )?;
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            spec.platform,
            spec.operation_id,
            spec.account_id,
            spec.account_label,
        )
        .await?,
    );
    let result = run(runtime.clone()).await;
    let shutdown_result = runtime.shutdown().await;
    match result {
        Err(error) => Err(error),
        Ok(value) => {
            shutdown_result?;
            Ok(value)
        }
    }
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
async fn run_channel_serve_command<R, V, F>(
    context: ChannelCommandContext<R>,
    spec: ChannelServeCommandSpec,
    validate: V,
    run: F,
) -> CliResult<()>
where
    R: ChannelResolvedRuntimeAccount,
    V: FnOnce(&R) -> CliResult<()>,
    F: for<'a> FnOnce(
        &'a ChannelCommandContext<R>,
        KernelContext,
        Arc<ChannelOperationRuntimeTracker>,
    ) -> ChannelCommandFuture<'a>,
{
    validate(&context.resolved)?;
    crate::runtime_env::initialize_runtime_environment(
        &context.config,
        Some(context.resolved_path.as_path()),
    );
    let kernel_ctx = bootstrap_kernel_context(
        spec.family.runtime.serve_bootstrap_agent_id,
        DEFAULT_TOKEN_TTL_S,
    )?;
    let runtime_account_id = context.resolved.runtime_account_id().to_owned();
    let runtime_account_label = context.resolved.runtime_account_label().to_owned();

    with_channel_serve_runtime(
        ChannelServeRuntimeSpec {
            platform: spec.family.runtime.platform,
            operation_id: spec.family.serve().id,
            account_id: runtime_account_id.as_str(),
            account_label: runtime_account_label.as_str(),
        },
        move |runtime| async move {
            context.emit_route_notice(spec.family.runtime.platform);
            run(&context, kernel_ctx, runtime).await
        },
    )
    .await
}

#[cfg(test)]
async fn with_channel_serve_runtime_in_dir<T, F, Fut>(
    runtime_dir: &std::path::Path,
    process_id: u32,
    spec: ChannelServeRuntimeSpec<'_>,
    run: F,
) -> CliResult<T>
where
    F: FnOnce(Arc<ChannelOperationRuntimeTracker>) -> Fut,
    Fut: Future<Output = CliResult<T>>,
{
    ensure_channel_operation_runtime_slot_available_in_dir(runtime_dir, spec)?;
    let runtime = Arc::new(
        runtime_state::start_channel_operation_runtime_tracker_for_test(
            runtime_dir,
            spec.platform,
            spec.operation_id,
            spec.account_id,
            spec.account_label,
            process_id,
        )
        .await?,
    );
    let result = run(runtime.clone()).await;
    let shutdown_result = runtime.shutdown().await;
    match result {
        Err(error) => Err(error),
        Ok(value) => {
            shutdown_result?;
            Ok(value)
        }
    }
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub async fn run_telegram_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-telegram") {
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(not(feature = "channel-telegram"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(feature = "channel-telegram")]
    {
        let context = load_telegram_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                family: TELEGRAM_COMMAND_FAMILY_DESCRIPTOR,
            },
            |context| {
                Box::pin(async move {
                    let token = context.resolved.bot_token().ok_or_else(|| {
                        "telegram bot token missing (set telegram.bot_token or env)".to_owned()
                    })?;
                    telegram::run_telegram_send(
                        &context.resolved,
                        token,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "telegram message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub async fn run_telegram_channel(
    config_path: Option<&str>,
    once: bool,
    account_id: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-telegram") {
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(not(feature = "channel-telegram"))]
    {
        let _ = (config_path, once, account_id);
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(feature = "channel-telegram")]
    {
        let context = load_telegram_command_context(config_path, account_id)?;
        let token = context.resolved.bot_token().ok_or_else(|| {
            "telegram bot token missing (set telegram.bot_token or env)".to_owned()
        })?;
        run_channel_serve_command(
            context,
            ChannelServeCommandSpec {
                family: TELEGRAM_COMMAND_FAMILY_DESCRIPTOR,
            },
            validate_telegram_security_config,
            move |context, kernel_ctx, runtime| {
                Box::pin(async move {
                    let route = context.route.clone();
                    let resolved_path = context.resolved_path.clone();
                    let resolved = context.resolved.clone();
                    let batch_config = context.config.clone();
                    let batch_kernel_ctx = Arc::new(crate::KernelContext {
                        kernel: kernel_ctx.kernel.clone(),
                        token: kernel_ctx.token.clone(),
                    });
                    let mut adapter = telegram::TelegramAdapter::new(&resolved, token);

                    println!(
                        "{} channel started (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, timeout={}s)",
                        adapter.name(),
                        resolved_path.display(),
                        resolved.configured_account_id,
                        resolved.account.label,
                        route.selected_by_default(),
                        route.default_account_source.as_str(),
                        resolved.polling_timeout_s
                    );

                    loop {
                        let batch = adapter.receive_batch().await?;
                        let config = batch_config.clone();
                        let kernel_ctx = batch_kernel_ctx.clone();
                        let had_messages =
                            process_channel_batch(&mut adapter, batch, Some(runtime.as_ref()), |message| {
                                let config = config.clone();
                                let kernel_ctx = kernel_ctx.clone();
                                Box::pin(async move {
                                    process_inbound_with_provider(
                                        &config,
                                        &message,
                                        Some(kernel_ctx.as_ref()),
                                    )
                                    .await
                                })
                            })
                            .await?;
                        if !had_messages && once {
                            break;
                        }
                        if once {
                            break;
                        }
                        sleep(Duration::from_millis(250)).await;
                    }
                    Ok(())
                })
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_feishu_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
    as_card: bool,
) -> CliResult<()> {
    if !cfg!(feature = "channel-feishu") {
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(not(feature = "channel-feishu"))]
    {
        let _ = (config_path, account_id, target, target_kind, text, as_card);
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(feature = "channel-feishu")]
    {
        let context = load_feishu_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                family: FEISHU_COMMAND_FAMILY_DESCRIPTOR,
            },
            |context| {
                Box::pin(async move {
                    feishu::run_feishu_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                        as_card,
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "feishu message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={}, receive_id_type={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind,
                    context.resolved.receive_id_type
                )
            },
        )
        .await
    }
}

pub async fn run_feishu_channel(
    config_path: Option<&str>,
    account_id: Option<&str>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-feishu") {
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(not(feature = "channel-feishu"))]
    {
        let _ = (config_path, account_id, bind_override, path_override);
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(feature = "channel-feishu")]
    {
        let context = load_feishu_command_context(config_path, account_id)?;
        let bind_override = bind_override.map(str::to_owned);
        let path_override = path_override.map(str::to_owned);
        run_channel_serve_command(
            context,
            ChannelServeCommandSpec {
                family: FEISHU_COMMAND_FAMILY_DESCRIPTOR,
            },
            validate_feishu_security_config,
            move |context, kernel_ctx, runtime| {
                Box::pin(async move {
                    let route = context.route.clone();
                    let resolved_path = context.resolved_path.clone();
                    let resolved = context.resolved.clone();
                    let config = context.config.clone();
                    feishu::run_feishu_channel(
                        &config,
                        &resolved,
                        &resolved_path,
                        route.selected_by_default(),
                        route.default_account_source,
                        bind_override.as_deref(),
                        path_override.as_deref(),
                        kernel_ctx,
                        runtime,
                    )
                    .await
                })
            },
        )
        .await
    }
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
pub(crate) async fn send_text_to_known_session(
    config: &LoongClawConfig,
    session_id: &str,
    text: &str,
) -> CliResult<ChannelSendReceipt> {
    match parse_known_channel_session_send_target(config, session_id)? {
        KnownChannelSessionSendTarget::Telegram {
            account_id,
            chat_id,
            thread_id,
        } => {
            #[cfg(not(feature = "channel-telegram"))]
            {
                let _ = (config, account_id, chat_id, thread_id, text);
                Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned())
            }

            #[cfg(feature = "channel-telegram")]
            {
                let resolved = config
                    .telegram
                    .resolve_account_for_session_account_id(account_id.as_deref())?;
                if !resolved.enabled {
                    return Err(
                        "sessions_send_channel_disabled: telegram channel is disabled by config"
                            .to_owned(),
                    );
                }
                let allowed_chat_id = chat_id.parse::<i64>().map_err(|error| {
                    format!("sessions_send_invalid_telegram_target: `{chat_id}`: {error}")
                })?;
                if !resolved.allowed_chat_ids.contains(&allowed_chat_id) {
                    return Err(format!(
                        "sessions_send_target_not_allowed: telegram target `{allowed_chat_id}` is not present in telegram.allowed_chat_ids"
                    ));
                }
                let token = resolved.bot_token().ok_or_else(|| {
                    "telegram bot token missing (set telegram.bot_token or env)".to_owned()
                })?;
                let target = match thread_id {
                    Some(thread_id) => format!("{chat_id}:topic:{thread_id}"),
                    None => chat_id,
                };
                telegram::run_telegram_send(
                    &resolved,
                    token,
                    ChannelOutboundTargetKind::Conversation,
                    target.as_str(),
                    text,
                )
                .await?;
                Ok(ChannelSendReceipt {
                    channel: "telegram",
                    target,
                })
            }
        }
        KnownChannelSessionSendTarget::Feishu {
            account_id,
            conversation_id,
            reply_message_id,
        } => {
            #[cfg(not(feature = "channel-feishu"))]
            {
                let _ = (config, account_id, conversation_id, reply_message_id, text);
                Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned())
            }

            #[cfg(feature = "channel-feishu")]
            {
                let resolved = config
                    .feishu
                    .resolve_account_for_session_account_id(account_id.as_deref())?;
                if !resolved.enabled {
                    return Err(
                        "sessions_send_channel_disabled: feishu channel is disabled by config"
                            .to_owned(),
                    );
                }
                if !resolved
                    .allowed_chat_ids
                    .iter()
                    .any(|allowed| allowed.trim() == conversation_id)
                {
                    return Err(format!(
                        "sessions_send_target_not_allowed: feishu target `{conversation_id}` is not present in feishu.allowed_chat_ids"
                    ));
                }
                let (target_kind, target) = match reply_message_id {
                    Some(message_id) => (ChannelOutboundTargetKind::MessageReply, message_id),
                    None => (ChannelOutboundTargetKind::ReceiveId, conversation_id),
                };
                feishu::run_feishu_send(&resolved, target_kind, target.as_str(), text, false)
                    .await?;
                Ok(ChannelSendReceipt {
                    channel: "feishu",
                    target,
                })
            }
        }
    }
}

#[cfg(not(any(feature = "channel-telegram", feature = "channel-feishu")))]
pub(crate) async fn send_text_to_known_session(
    _config: &LoongClawConfig,
    session_id: &str,
    _text: &str,
) -> CliResult<ChannelSendReceipt> {
    Err(format!("sessions_send_channel_unsupported: `{session_id}`"))
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
pub(super) async fn process_inbound_with_provider(
    config: &LoongClawConfig,
    message: &ChannelInboundMessage,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<String> {
    let address = message.session.conversation_address();
    let acp_turn_hints = resolve_channel_acp_turn_hints(config, &message.session)?;
    let acp_options = AcpConversationTurnOptions::automatic()
        .with_additional_bootstrap_mcp_servers(&acp_turn_hints.bootstrap_mcp_servers)
        .with_working_directory(acp_turn_hints.working_directory.as_deref())
        .with_provenance(channel_message_acp_turn_provenance(message));
    ConversationTurnCoordinator::new()
        .handle_turn_with_address_and_acp_options(
            config,
            &address,
            &message.text,
            ProviderErrorMode::Propagate,
            &acp_options,
            kernel_ctx,
        )
        .await
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
fn resolve_channel_acp_turn_hints(
    config: &LoongClawConfig,
    session: &ChannelSession,
) -> CliResult<ChannelResolvedAcpTurnHints> {
    match session.platform {
        ChannelPlatform::Telegram => {
            let resolved = config
                .telegram
                .resolve_account_for_session_account_id(session.account_id.as_deref())?;
            let acp = resolved.acp;
            let working_directory = acp.resolved_working_directory();
            Ok(ChannelResolvedAcpTurnHints {
                bootstrap_mcp_servers: acp.bootstrap_mcp_servers,
                working_directory,
            })
        }
        ChannelPlatform::Feishu => {
            let resolved = config
                .feishu
                .resolve_account_for_session_account_id(session.account_id.as_deref())?;
            let acp = resolved.acp;
            let working_directory = acp.resolved_working_directory();
            Ok(ChannelResolvedAcpTurnHints {
                bootstrap_mcp_servers: acp.bootstrap_mcp_servers,
                working_directory,
            })
        }
    }
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
fn channel_message_acp_turn_provenance(message: &ChannelInboundMessage) -> AcpTurnProvenance<'_> {
    AcpTurnProvenance {
        trace_id: None,
        source_message_id: message.delivery.source_message_id.as_deref(),
        ack_cursor: message.delivery.ack_cursor.as_deref(),
    }
}

fn render_channel_route_notice(
    platform: ChannelPlatform,
    route: &ChannelResolvedAccountRoute,
) -> Option<String> {
    if !route.uses_implicit_fallback_default() {
        return None;
    }
    Some(format!(
        "{} omitted --account and routed to configured account `{}` via fallback default selection; set {}.default_account or pass --account to avoid routing surprises",
        platform.as_str(),
        route.selected_configured_account_id,
        platform.as_str()
    ))
}

#[cfg(feature = "channel-telegram")]
fn validate_telegram_security_config(config: &ResolvedTelegramChannelConfig) -> CliResult<()> {
    if config.allowed_chat_ids.is_empty() {
        return Err(
            "telegram.allowed_chat_ids is empty; configure at least one trusted chat id".to_owned(),
        );
    }
    Ok(())
}

#[cfg(feature = "channel-feishu")]
fn validate_feishu_security_config(config: &ResolvedFeishuChannelConfig) -> CliResult<()> {
    let has_allowlist = config
        .allowed_chat_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowlist {
        return Err(
            "feishu.allowed_chat_ids is empty; configure at least one trusted chat id".to_owned(),
        );
    }

    let has_verification_token = config
        .verification_token()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_verification_token {
        return Err(
            "feishu.verification_token is missing; configure token or verification_token_env"
                .to_owned(),
        );
    }

    let has_encrypt_key = config
        .encrypt_key()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_encrypt_key {
        return Err("feishu.encrypt_key is missing; configure key or encrypt_key_env".to_owned());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_runtime_dir(suffix: &str) -> PathBuf {
        let unique = format!(
            "loongclaw-channel-mod-{suffix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    fn now_ms_for_test() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_millis() as u64
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[derive(Default)]
    struct RecordingAdapter {
        sent: Arc<Mutex<Vec<(ChannelOutboundTarget, String)>>>,
        acked: Arc<Mutex<Vec<Option<String>>>>,
        completed_batches: Arc<Mutex<usize>>,
        typing_events: Arc<Mutex<Vec<String>>>,
        fail_send: Arc<Mutex<bool>>,
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[async_trait]
    impl ChannelAdapter for RecordingAdapter {
        fn name(&self) -> &str {
            "recording"
        }

        async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>> {
            Ok(Vec::new())
        }

        async fn send_text(&self, target: &ChannelOutboundTarget, text: &str) -> CliResult<()> {
            if *self.fail_send.lock().expect("fail send flag") {
                return Err("send failed".to_owned());
            }
            self.sent
                .lock()
                .expect("sent log")
                .push((target.clone(), text.to_owned()));
            Ok(())
        }

        async fn start_typing(&self, target: &ChannelOutboundTarget) -> CliResult<()> {
            self.typing_events
                .lock()
                .expect("typing log")
                .push(format!("start:{}", target.id));
            Ok(())
        }

        async fn stop_typing(&self, target: &ChannelOutboundTarget) -> CliResult<()> {
            self.typing_events
                .lock()
                .expect("typing log")
                .push(format!("stop:{}", target.id));
            Ok(())
        }

        async fn ack_inbound(&mut self, message: &ChannelInboundMessage) -> CliResult<()> {
            self.acked
                .lock()
                .expect("ack log")
                .push(message.delivery.ack_cursor.clone());
            Ok(())
        }

        async fn complete_batch(&mut self) -> CliResult<()> {
            *self
                .completed_batches
                .lock()
                .expect("completed batch count") += 1;
            Ok(())
        }
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[tokio::test]
    async fn channel_adapter_default_ack_hooks_are_noop() {
        #[derive(Default)]
        struct NoopAdapter;

        #[async_trait]
        impl ChannelAdapter for NoopAdapter {
            fn name(&self) -> &str {
                "noop"
            }

            async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>> {
                Ok(Vec::new())
            }

            async fn send_text(
                &self,
                _target: &ChannelOutboundTarget,
                _text: &str,
            ) -> CliResult<()> {
                Ok(())
            }
        }

        let mut adapter = NoopAdapter;
        let message = ChannelInboundMessage {
            session: ChannelSession::new(ChannelPlatform::Telegram, "1"),
            reply_target: ChannelOutboundTarget::telegram_chat(1),
            text: "hello".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: Some("2".to_owned()),
                source_message_id: Some("42".to_owned()),
            },
        };

        adapter
            .ack_inbound(&message)
            .await
            .expect("default ack hook should succeed");
        adapter
            .start_typing(&message.reply_target)
            .await
            .expect("default typing start hook should succeed");
        adapter
            .stop_typing(&message.reply_target)
            .await
            .expect("default typing stop hook should succeed");
        adapter
            .complete_batch()
            .await
            .expect("default batch completion hook should succeed");
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[tokio::test]
    async fn process_channel_batch_acknowledges_after_successful_delivery() {
        let mut adapter = RecordingAdapter::default();
        let batch = vec![ChannelInboundMessage {
            session: ChannelSession::new(ChannelPlatform::Telegram, "1"),
            reply_target: ChannelOutboundTarget::telegram_chat(1),
            text: "hello".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: Some("101".to_owned()),
                source_message_id: Some("55".to_owned()),
            },
        }];

        let had_messages = process_channel_batch(
            &mut adapter,
            batch,
            None,
            |message: ChannelInboundMessage| {
                Box::pin(async move { Ok(format!("reply: {}", message.text)) })
            },
        )
        .await
        .expect("batch should process");

        assert!(had_messages);
        assert_eq!(
            adapter.sent.lock().expect("sent log").as_slice(),
            &[(
                ChannelOutboundTarget::telegram_chat(1),
                "reply: hello".to_owned(),
            )]
        );
        assert_eq!(
            adapter.acked.lock().expect("ack log").as_slice(),
            &[Some("101".to_owned())]
        );
        assert_eq!(
            adapter.typing_events.lock().expect("typing log").as_slice(),
            &["start:1".to_owned(), "stop:1".to_owned()]
        );
        assert_eq!(*adapter.completed_batches.lock().expect("completed"), 1);
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[tokio::test]
    async fn process_channel_batch_stops_typing_when_processing_fails() {
        let mut adapter = RecordingAdapter::default();
        let batch = vec![ChannelInboundMessage {
            session: ChannelSession::new(ChannelPlatform::Telegram, "1"),
            reply_target: ChannelOutboundTarget::telegram_chat(1),
            text: "hello".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: Some("101".to_owned()),
                source_message_id: Some("55".to_owned()),
            },
        }];

        let error = process_channel_batch(&mut adapter, batch, None, |_message| {
            Box::pin(async move { Err("process failed".to_owned()) })
        })
        .await
        .expect_err("batch should fail");

        assert_eq!(error, "process failed");
        assert!(adapter.sent.lock().expect("sent log").is_empty());
        assert!(adapter.acked.lock().expect("ack log").is_empty());
        assert_eq!(
            adapter.typing_events.lock().expect("typing log").as_slice(),
            &["start:1".to_owned(), "stop:1".to_owned()]
        );
        assert_eq!(*adapter.completed_batches.lock().expect("completed"), 0);
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[tokio::test]
    async fn process_channel_batch_stops_typing_when_send_fails() {
        let mut adapter = RecordingAdapter::default();
        *adapter.fail_send.lock().expect("fail send flag") = true;
        let batch = vec![ChannelInboundMessage {
            session: ChannelSession::new(ChannelPlatform::Telegram, "1"),
            reply_target: ChannelOutboundTarget::telegram_chat(1),
            text: "hello".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: Some("101".to_owned()),
                source_message_id: Some("55".to_owned()),
            },
        }];

        let error = process_channel_batch(
            &mut adapter,
            batch,
            None,
            |message: ChannelInboundMessage| {
                Box::pin(async move { Ok(format!("reply: {}", message.text)) })
            },
        )
        .await
        .expect_err("batch should fail");

        assert_eq!(error, "send failed");
        assert!(adapter.acked.lock().expect("ack log").is_empty());
        assert_eq!(
            adapter.typing_events.lock().expect("typing log").as_slice(),
            &["start:1".to_owned(), "stop:1".to_owned()]
        );
        assert_eq!(*adapter.completed_batches.lock().expect("completed"), 0);
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn channel_session_key_is_stable() {
        let session = ChannelSession::new(ChannelPlatform::Telegram, "123");
        assert_eq!(session.session_key(), "telegram:123");
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn channel_session_key_includes_thread_id_when_present() {
        let session = ChannelSession::with_thread(ChannelPlatform::Feishu, "oc_123", "om_thread_1");
        assert_eq!(session.session_key(), "feishu:oc_123:om_thread_1");
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn channel_session_key_includes_account_identity_when_present() {
        let session = ChannelSession::with_account(ChannelPlatform::Telegram, "bot_123456", "123");
        assert_eq!(session.session_key(), "telegram:bot_123456:123");
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn channel_session_key_includes_account_identity_and_thread_when_present() {
        let session = ChannelSession::with_account_and_thread(
            ChannelPlatform::Feishu,
            "lark_cli_a1b2c3",
            "oc_123",
            "om_thread_1",
        );
        assert_eq!(
            session.session_key(),
            "feishu:lark_cli_a1b2c3:oc_123:om_thread_1"
        );
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn channel_session_exposes_structured_conversation_address() {
        let session = ChannelSession::with_account_and_thread(
            ChannelPlatform::Feishu,
            "lark_cli_a1b2c3",
            "oc_123",
            "om_thread_1",
        );

        let address = session.conversation_address();

        assert_eq!(
            address.session_id,
            "feishu:lark_cli_a1b2c3:oc_123:om_thread_1"
        );
        assert_eq!(address.channel_id.as_deref(), Some("feishu"));
        assert_eq!(address.account_id.as_deref(), Some("lark_cli_a1b2c3"));
        assert_eq!(address.conversation_id.as_deref(), Some("oc_123"));
        assert_eq!(address.thread_id.as_deref(), Some("om_thread_1"));
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn resolve_channel_acp_turn_hints_uses_telegram_runtime_account_identity() {
        let config: crate::config::LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "default_account": "Work Bot",
                "accounts": {
                    "Work Bot": {
                        "account_id": "Ops-Bot",
                        "bot_token_env": "WORK_TELEGRAM_TOKEN",
                        "allowed_chat_ids": [1001],
                        "acp": {
                            "bootstrap_mcp_servers": ["search"],
                            "working_directory": " /workspace/ops "
                        }
                    }
                }
            }
        }))
        .expect("deserialize config");
        let session = ChannelSession::with_account(ChannelPlatform::Telegram, "ops-bot", "1001");

        let hints = resolve_channel_acp_turn_hints(&config, &session)
            .expect("resolve telegram ACP turn hints");
        assert_eq!(hints.bootstrap_mcp_servers, vec!["search".to_owned()]);
        assert_eq!(
            hints.working_directory,
            Some(PathBuf::from("/workspace/ops"))
        );
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn resolve_channel_acp_turn_hints_uses_feishu_runtime_account_identity() {
        let config: crate::config::LoongClawConfig = serde_json::from_value(serde_json::json!({
            "feishu": {
                "default_account": "Lark Prod",
                "accounts": {
                    "Lark Prod": {
                        "domain": "lark",
                        "app_id": "cli_lark_123",
                        "app_secret": "secret",
                        "allowed_chat_ids": ["oc_123"],
                        "acp": {
                            "bootstrap_mcp_servers": ["search"],
                            "working_directory": "/workspace/lark"
                        }
                    }
                }
            }
        }))
        .expect("deserialize config");
        let session =
            ChannelSession::with_account(ChannelPlatform::Feishu, "lark_cli_lark_123", "oc_123");

        let hints = resolve_channel_acp_turn_hints(&config, &session)
            .expect("resolve feishu ACP turn hints");
        assert_eq!(hints.bootstrap_mcp_servers, vec!["search".to_owned()]);
        assert_eq!(
            hints.working_directory,
            Some(PathBuf::from("/workspace/lark"))
        );
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn channel_message_acp_turn_provenance_uses_delivery_identifiers() {
        let message = ChannelInboundMessage {
            session: ChannelSession::with_account(ChannelPlatform::Telegram, "ops-bot", "1001"),
            reply_target: ChannelOutboundTarget::telegram_chat(1001),
            text: "hello".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: Some("cursor-55".to_owned()),
                source_message_id: Some("message-42".to_owned()),
            },
        };

        let provenance = channel_message_acp_turn_provenance(&message);

        assert_eq!(provenance.trace_id, None);
        assert_eq!(provenance.source_message_id, Some("message-42"));
        assert_eq!(provenance.ack_cursor, Some("cursor-55"));
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn channel_outbound_target_preserves_platform_kind_and_id() {
        let target = ChannelOutboundTarget::new(
            ChannelPlatform::Feishu,
            ChannelOutboundTargetKind::MessageReply,
            "om_123",
        );
        assert_eq!(target.platform, ChannelPlatform::Feishu);
        assert_eq!(target.kind, ChannelOutboundTargetKind::MessageReply);
        assert_eq!(target.id, "om_123");
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn channel_outbound_target_encodes_telegram_forum_topics() {
        let target = ChannelOutboundTarget::telegram_chat_thread(123, 7);

        assert_eq!(target.platform, ChannelPlatform::Telegram);
        assert_eq!(target.kind, ChannelOutboundTargetKind::Conversation);
        assert_eq!(target.id, "123:topic:7");
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn render_channel_route_notice_warns_on_implicit_multi_account_fallback() {
        let route = crate::config::ChannelResolvedAccountRoute {
            requested_account_id: None,
            configured_account_count: 2,
            selected_configured_account_id: "alerts".to_owned(),
            default_account_source: crate::config::ChannelDefaultAccountSelectionSource::Fallback,
        };

        let rendered = render_channel_route_notice(ChannelPlatform::Telegram, &route)
            .expect("fallback route should warn");

        assert!(rendered.contains("telegram"));
        assert!(rendered.contains("alerts"));
        assert!(rendered.contains("--account"));
        assert!(rendered.contains("telegram.default_account"));
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn render_channel_route_notice_is_silent_for_explicit_account_selection() {
        let route = crate::config::ChannelResolvedAccountRoute {
            requested_account_id: Some("work".to_owned()),
            configured_account_count: 2,
            selected_configured_account_id: "work".to_owned(),
            default_account_source: crate::config::ChannelDefaultAccountSelectionSource::Fallback,
        };

        assert!(render_channel_route_notice(ChannelPlatform::Telegram, &route).is_none());
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn telegram_command_context_preserves_route_metadata() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "accounts": {
                    "Work": {
                        "bot_token": "123456:work-token",
                        "allowed_chat_ids": [1001]
                    },
                    "Alerts": {
                        "bot_token": "654321:alerts-token",
                        "allowed_chat_ids": [2002]
                    }
                }
            }
        }))
        .expect("deserialize telegram context config");

        let context =
            build_telegram_command_context(PathBuf::from("/tmp/loongclaw.toml"), config, None)
                .expect("build telegram command context");

        assert_eq!(context.resolved_path, PathBuf::from("/tmp/loongclaw.toml"));
        assert_eq!(context.resolved.configured_account_id, "alerts");
        assert!(context.route.selected_by_default());
        assert!(context.route.uses_implicit_fallback_default());
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_command_context_rejects_disabled_resolved_account() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "feishu": {
                "enabled": true,
                "accounts": {
                    "Primary": {
                        "enabled": false,
                        "app_id": "cli_primary",
                        "app_secret": "secret"
                    }
                }
            }
        }))
        .expect("deserialize feishu context config");

        let error = build_feishu_command_context(
            PathBuf::from("/tmp/loongclaw.toml"),
            config,
            Some("Primary"),
        )
        .expect_err("disabled feishu account should fail");

        assert!(error.contains("disabled"));
        assert!(error.contains("primary"));
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[tokio::test]
    async fn with_channel_serve_runtime_tracks_running_state_and_shutdown() {
        let runtime_dir = temp_runtime_dir("serve-runtime-wrapper");
        let runtime_dir_for_body = runtime_dir.clone();
        let operation = ChannelServeRuntimeSpec {
            platform: ChannelPlatform::Telegram,
            operation_id: CHANNEL_OPERATION_SERVE_ID,
            account_id: "bot_123456",
            account_label: "bot:123456",
        };

        let result = with_channel_serve_runtime_in_dir(
            runtime_dir.as_path(),
            9191,
            operation,
            |runtime| async move {
                let live = runtime_state::load_channel_operation_runtime_for_account_from_dir(
                    runtime_dir_for_body.as_path(),
                    ChannelPlatform::Telegram,
                    CHANNEL_OPERATION_SERVE_ID,
                    "bot_123456",
                    0,
                )
                .expect("runtime should exist while serve body is running");
                assert!(live.running);
                assert_eq!(live.pid, Some(9191));
                assert_eq!(live.account_id.as_deref(), Some("bot_123456"));
                assert_eq!(live.account_label.as_deref(), Some("bot:123456"));
                assert!(
                    Arc::strong_count(&runtime) >= 1,
                    "runtime handle should stay alive in serve body"
                );
                Ok::<_, String>("ok".to_owned())
            },
        )
        .await
        .expect("serve runtime wrapper should succeed");

        assert_eq!(result, "ok");

        let finished = runtime_state::load_channel_operation_runtime_for_account_from_dir(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            "bot_123456",
            0,
        )
        .expect("runtime should remain readable after shutdown");
        assert!(!finished.running);
        assert_eq!(finished.pid, Some(9191));
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[tokio::test]
    async fn with_channel_serve_runtime_rejects_duplicate_running_instance() {
        let runtime_dir = temp_runtime_dir("serve-runtime-duplicate");
        let now = now_ms_for_test();
        runtime_state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            "bot_123456",
            7001,
            true,
            true,
            1,
            Some(now),
            Some(now),
            Some(7001),
        )
        .expect("seed running runtime state");

        let error = with_channel_serve_runtime_in_dir(
            runtime_dir.as_path(),
            9191,
            ChannelServeRuntimeSpec {
                platform: ChannelPlatform::Telegram,
                operation_id: CHANNEL_OPERATION_SERVE_ID,
                account_id: "bot_123456",
                account_label: "bot:123456",
            },
            |_runtime| async move { Ok::<_, String>("ok".to_owned()) },
        )
        .await
        .expect_err("duplicate running instance should be rejected");

        assert!(error.contains("already has an active serve runtime"));
        assert!(error.contains("bot_123456"));
        assert!(error.contains("7001"));
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[tokio::test]
    async fn with_channel_serve_runtime_allows_takeover_when_previous_instance_is_stale() {
        let runtime_dir = temp_runtime_dir("serve-runtime-stale-takeover");
        let now = now_ms_for_test();
        runtime_state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            "bot_123456",
            7001,
            true,
            true,
            1,
            Some(now.saturating_sub(60_000)),
            Some(now.saturating_sub(60_000)),
            Some(7001),
        )
        .expect("seed stale runtime state");

        let result = with_channel_serve_runtime_in_dir(
            runtime_dir.as_path(),
            9191,
            ChannelServeRuntimeSpec {
                platform: ChannelPlatform::Telegram,
                operation_id: CHANNEL_OPERATION_SERVE_ID,
                account_id: "bot_123456",
                account_label: "bot:123456",
            },
            |_runtime| async move { Ok::<_, String>("ok".to_owned()) },
        )
        .await
        .expect("stale runtime should not block startup");

        assert_eq!(result, "ok");

        let runtime = runtime_state::load_channel_operation_runtime_for_account_from_dir(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            "bot_123456",
            now_ms_for_test(),
        )
        .expect("runtime should remain readable after takeover");
        assert_eq!(runtime.pid, Some(9191));
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn telegram_security_validation_requires_allowlist() {
        let config = LoongClawConfig::default();
        let resolved = config
            .telegram
            .resolve_account(None)
            .expect("resolve telegram account");
        let error = validate_telegram_security_config(&resolved)
            .expect_err("empty allowlist must be rejected");
        assert!(error.contains("allowed_chat_ids"));
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn telegram_security_validation_accepts_configured_allowlist() {
        let mut config = LoongClawConfig::default();
        config.telegram.allowed_chat_ids = vec![123_i64];
        let resolved = config
            .telegram
            .resolve_account(None)
            .expect("resolve telegram account");
        assert!(validate_telegram_security_config(&resolved).is_ok());
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_security_validation_requires_secrets_and_allowlist() {
        let config = LoongClawConfig::default();
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let error =
            validate_feishu_security_config(&resolved).expect_err("empty config must be rejected");
        assert!(error.contains("allowed_chat_ids"));
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_security_validation_accepts_complete_configuration() {
        let mut config = LoongClawConfig::default();
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.verification_token = Some("token-123".to_owned());
        config.feishu.verification_token_env = None;
        config.feishu.encrypt_key = Some("encrypt-key-123".to_owned());
        config.feishu.encrypt_key_env = None;

        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        assert!(validate_feishu_security_config(&resolved).is_ok());
    }
}
