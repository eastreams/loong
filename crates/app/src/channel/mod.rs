#[cfg(feature = "channel-telegram")]
use std::time::Duration;
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
#[cfg(feature = "channel-telegram")]
use tokio::time::sleep;

use crate::CliResult;
#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use crate::KernelContext;
#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_kernel_context};

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use super::config::{
    ChannelResolvedAccountRoute, LoongClawConfig, ResolvedFeishuChannelConfig,
    ResolvedTelegramChannelConfig,
};
#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use super::conversation::{ConversationTurnCoordinator, ProviderErrorMode};

#[cfg(feature = "channel-feishu")]
mod feishu;
mod registry;
mod runtime_state;
#[cfg(feature = "channel-telegram")]
mod telegram;

pub use registry::{
    ChannelCatalogEntry, ChannelCatalogOperation, ChannelOperationHealth, ChannelOperationStatus,
    ChannelStatusSnapshot, channel_status_snapshots, list_channel_catalog,
    normalize_channel_platform,
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
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelOutboundTargetKind {
    Conversation,
    MessageReply,
    ReceiveId,
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
impl ChannelOutboundTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::MessageReply => "message_reply",
            Self::ReceiveId => "receive_id",
        }
    }
}

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
#[allow(dead_code)]
#[async_trait]
pub trait ChannelAdapter {
    fn name(&self) -> &str;
    async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>>;
    async fn send_text(&self, target: &ChannelOutboundTarget, text: &str) -> CliResult<()>;
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
async fn process_channel_batch<A, F>(
    adapter: &mut A,
    batch: Vec<ChannelInboundMessage>,
    runtime: Option<&ChannelOperationRuntimeTracker>,
    mut process: F,
) -> CliResult<bool>
where
    A: ChannelAdapter + Send + ?Sized,
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

        let result = async {
            let reply = process(message.clone()).await?;
            adapter.send_text(&message.reply_target, &reply).await?;
            adapter.ack_inbound(message).await?;
            Ok::<(), String>(())
        }
        .await;

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
struct ChannelServeRuntimeSpec<'a> {
    platform: ChannelPlatform,
    operation_id: &'static str,
    account_id: &'a str,
    account_label: &'a str,
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
        validate_telegram_security_config(&context.resolved)?;
        apply_runtime_env(&context.config);
        let kernel_ctx = bootstrap_kernel_context("channel-telegram", DEFAULT_TOKEN_TTL_S)?;
        let token = context.resolved.bot_token().ok_or_else(|| {
            "telegram bot token missing (set telegram.bot_token or env)".to_owned()
        })?;
        let route = context.route.clone();
        let resolved_path = context.resolved_path.clone();
        let resolved = context.resolved.clone();
        let batch_config = context.config.clone();
        let batch_kernel_ctx = Arc::new(crate::KernelContext {
            kernel: kernel_ctx.kernel.clone(),
            token: kernel_ctx.token.clone(),
        });
        let runtime_account_id = resolved.account.id.clone();
        let runtime_account_label = resolved.account.label.clone();

        with_channel_serve_runtime(
            ChannelServeRuntimeSpec {
                platform: ChannelPlatform::Telegram,
                operation_id: "serve",
                account_id: runtime_account_id.as_str(),
                account_label: runtime_account_label.as_str(),
            },
            move |runtime| async move {
                let mut adapter = telegram::TelegramAdapter::new(&resolved, token);
                context.emit_route_notice(ChannelPlatform::Telegram);

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
            }
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_feishu_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    receive_id: &str,
    text: &str,
    as_card: bool,
) -> CliResult<()> {
    if !cfg!(feature = "channel-feishu") {
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(not(feature = "channel-feishu"))]
    {
        let _ = (config_path, account_id, receive_id, text, as_card);
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(feature = "channel-feishu")]
    {
        let context = load_feishu_command_context(config_path, account_id)?;
        apply_runtime_env(&context.config);
        context.emit_route_notice(ChannelPlatform::Feishu);
        feishu::run_feishu_send(&context.resolved, receive_id, text, as_card).await?;

        println!(
            "feishu message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, receive_id_type={})",
            context.resolved_path.display(),
            context.resolved.configured_account_id,
            context.resolved.account.label,
            context.route.selected_by_default(),
            context.route.default_account_source.as_str(),
            context.resolved.receive_id_type
        );
        Ok(())
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
        validate_feishu_security_config(&context.resolved)?;
        apply_runtime_env(&context.config);
        let kernel_ctx = bootstrap_kernel_context("channel-feishu", DEFAULT_TOKEN_TTL_S)?;
        let route = context.route.clone();
        let resolved_path = context.resolved_path.clone();
        let resolved = context.resolved.clone();
        let config = context.config.clone();
        let runtime_account_id = resolved.account.id.clone();
        let runtime_account_label = resolved.account.label.clone();

        with_channel_serve_runtime(
            ChannelServeRuntimeSpec {
                platform: ChannelPlatform::Feishu,
                operation_id: "serve",
                account_id: runtime_account_id.as_str(),
                account_label: runtime_account_label.as_str(),
            },
            move |runtime| async move {
                context.emit_route_notice(ChannelPlatform::Feishu);
                feishu::run_feishu_channel(
                    &config,
                    &resolved,
                    &resolved_path,
                    route.selected_by_default(),
                    route.default_account_source,
                    bind_override,
                    path_override,
                    kernel_ctx,
                    runtime.clone(),
                )
                .await
            },
        )
        .await
    }
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
pub(super) async fn process_inbound_with_provider(
    config: &LoongClawConfig,
    message: &ChannelInboundMessage,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<String> {
    let session_key = message.session.session_key();
    ConversationTurnCoordinator::new()
        .handle_turn(
            config,
            &session_key,
            &message.text,
            ProviderErrorMode::Propagate,
            kernel_ctx,
        )
        .await
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
fn apply_runtime_env(config: &LoongClawConfig) {
    crate::memory::runtime_config::apply_memory_runtime_env(&config.memory);
    crate::process_env::set_var(
        "LOONGCLAW_FILE_ROOT",
        config.tools.resolved_file_root().display().to_string(),
    );
    crate::process_env::set_var(
        "LOONGCLAW_EXTERNAL_SKILLS_ENABLED",
        config.external_skills.enabled.to_string(),
    );
    crate::process_env::set_var(
        "LOONGCLAW_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL",
        config.external_skills.require_download_approval.to_string(),
    );
    crate::process_env::set_var(
        "LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS",
        config
            .external_skills
            .normalized_allowed_domains()
            .join(","),
    );
    crate::process_env::set_var(
        "LOONGCLAW_EXTERNAL_SKILLS_BLOCKED_DOMAINS",
        config
            .external_skills
            .normalized_blocked_domains()
            .join(","),
    );
    // Populate the typed tool runtime config so executors never hit env vars
    // on the hot path.  Ignore the error if already initialised.
    let tool_rt = crate::tools::runtime_config::ToolRuntimeConfig {
        file_root: Some(config.tools.resolved_file_root()),
        shell_allow: config
            .tools
            .shell_allow
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .collect(),
        shell_deny: config
            .tools
            .shell_deny
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .collect(),
        shell_approval_required: config
            .tools
            .shell_approval_required
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .collect(),
        shell_default_mode: crate::tools::policy_ext::ShellPolicyDefault::parse(
            &config.tools.shell_default_mode,
        ),
        external_skills: crate::tools::runtime_config::ExternalSkillsRuntimePolicy {
            enabled: config.external_skills.enabled,
            require_download_approval: config.external_skills.require_download_approval,
            allowed_domains: config
                .external_skills
                .normalized_allowed_domains()
                .into_iter()
                .collect(),
            blocked_domains: config
                .external_skills
                .normalized_blocked_domains()
                .into_iter()
                .collect(),
            install_root: config.external_skills.resolved_install_root(),
            auto_expose_installed: config.external_skills.auto_expose_installed,
        },
    };
    let _ = crate::tools::runtime_config::init_tool_runtime_config(tool_rt);

    // Populate the typed memory runtime config (same pattern as tool config).
    let memory_rt =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let _ = crate::memory::runtime_config::init_memory_runtime_config(memory_rt);
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
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
            self.sent
                .lock()
                .expect("sent log")
                .push((target.clone(), text.to_owned()));
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
        assert_eq!(*adapter.completed_batches.lock().expect("completed"), 1);
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
            operation_id: "serve",
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
                    "serve",
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
            "serve",
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
            "serve",
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
                operation_id: "serve",
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
            "serve",
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
                operation_id: "serve",
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
            "serve",
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
