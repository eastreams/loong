#[cfg(feature = "tool-file")]
use std::fs;
use std::sync::{Mutex, OnceLock};

use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::CliResult;
use crate::channel::ChannelOutboundTarget;
use crate::channel::feishu::api::resources::calendar::{
    self, FeishuCalendarFreebusyQuery, FeishuCalendarListQuery,
};
use crate::channel::feishu::api::resources::cards;
use crate::channel::feishu::api::resources::docs;
use crate::channel::feishu::api::resources::media;
use crate::channel::feishu::api::resources::messages::{
    self, FeishuMessageHistoryQuery, FeishuSearchMessagesQuery,
};
use crate::channel::feishu::api::{
    FeishuClient, FeishuGrant, FeishuMessageResourceType, FeishuTokenStore, FeishuUserPrincipal,
    map_user_info_to_principal,
};
use crate::channel::feishu::send::deliver_feishu_message_body;

const FEISHU_MESSAGE_RESOURCE_ACCEPTED_SCOPES: &[&str] = &[
    "im:message:readonly",
    "im:message.group_msg",
    "im:message",
    "im:message:send_as_bot",
    "im:message:send",
];
const FEISHU_DOC_READ_ACCEPTED_SCOPES: &[&str] = &["docx:document:readonly", "docx:document"];
const FEISHU_DOC_WRITE_REQUIRED_SCOPES: &[&str] = &["docx:document"];
const FEISHU_CARD_UPDATE_CALLBACK_TOKEN_USE_LIMIT: usize = 2;

mod bitable_execution;
mod metadata;
mod support;

use bitable_execution::*;
use support::*;

#[cfg(test)]
pub(super) fn feishu_tool_alias_pairs() -> &'static [(&'static str, &'static str)] {
    metadata::feishu_tool_alias_pairs()
}

pub(super) fn canonical_feishu_tool_name(raw: &str) -> Option<&'static str> {
    metadata::canonical_feishu_tool_name(raw)
}

pub(super) fn is_known_feishu_tool_name(raw: &str) -> bool {
    metadata::is_known_feishu_tool_name(raw)
}

#[cfg(test)]
pub(super) fn feishu_tool_registry_entries() -> Vec<super::ToolRegistryEntry> {
    metadata::feishu_tool_registry_entries()
}

#[cfg(test)]
pub(super) fn feishu_provider_tool_definitions() -> Vec<Value> {
    metadata::feishu_provider_tool_definitions()
}

pub(super) fn feishu_provider_tool_definition(tool_name: &str) -> Option<Value> {
    metadata::feishu_provider_tool_definition(tool_name)
}

#[cfg(test)]
pub(super) fn feishu_shape_examples() -> std::collections::BTreeMap<&'static str, Value> {
    metadata::feishu_shape_examples()
}

#[derive(Debug, Clone)]
pub(crate) struct DeferredFeishuCardUpdate {
    pub configured_account_id: String,
    pub token: String,
    pub card: Value,
    pub open_ids: Vec<String>,
}

fn deferred_feishu_card_update_store()
-> &'static Mutex<std::collections::HashMap<String, Vec<DeferredFeishuCardUpdate>>> {
    static STORE: OnceLock<
        Mutex<std::collections::HashMap<String, Vec<DeferredFeishuCardUpdate>>>,
    > = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn enqueue_deferred_feishu_card_update(
    context_id: &str,
    update: DeferredFeishuCardUpdate,
) -> CliResult<usize> {
    let context_id = context_id.trim();
    if context_id.is_empty() {
        return Err("feishu card update missing deferred callback context id".to_owned());
    }
    let mut store = deferred_feishu_card_update_store()
        .lock()
        .map_err(|error| format!("lock deferred feishu card update store failed: {error}"))?;
    let entry = store.entry(context_id.to_owned()).or_default();
    if entry.len() >= FEISHU_CARD_UPDATE_CALLBACK_TOKEN_USE_LIMIT {
        return Err(format!(
            "feishu card update callback token can only be used twice per callback turn; deferred context `{context_id}` already queued {} updates",
            entry.len()
        ));
    }
    entry.push(update);
    Ok(entry.len())
}

pub(crate) fn drain_deferred_feishu_card_updates(
    context_id: &str,
) -> Vec<DeferredFeishuCardUpdate> {
    let Ok(mut store) = deferred_feishu_card_update_store().lock() else {
        return Vec::new();
    };
    store.remove(context_id.trim()).unwrap_or_default()
}

#[cfg(all(test, feature = "tool-file"))]
const FEISHU_TOOL_ALIAS_PAIRS: &[(&str, &str)] = &[
    ("feishu_whoami", "feishu.whoami"),
    ("feishu_bitable_app_create", "feishu.bitable.app.create"),
    ("feishu_bitable_app_get", "feishu.bitable.app.get"),
    ("feishu_bitable_app_list", "feishu.bitable.app.list"),
    ("feishu_bitable_app_patch", "feishu.bitable.app.patch"),
    ("feishu_bitable_app_copy", "feishu.bitable.app.copy"),
    ("feishu_bitable_list", "feishu.bitable.list"),
    ("feishu_bitable_table_create", "feishu.bitable.table.create"),
    ("feishu_bitable_table_patch", "feishu.bitable.table.patch"),
    (
        "feishu_bitable_table_batch_create",
        "feishu.bitable.table.batch_create",
    ),
    (
        "feishu_bitable_record_create",
        "feishu.bitable.record.create",
    ),
    (
        "feishu_bitable_record_update",
        "feishu.bitable.record.update",
    ),
    (
        "feishu_bitable_record_delete",
        "feishu.bitable.record.delete",
    ),
    (
        "feishu_bitable_record_batch_create",
        "feishu.bitable.record.batch_create",
    ),
    (
        "feishu_bitable_record_batch_update",
        "feishu.bitable.record.batch_update",
    ),
    (
        "feishu_bitable_record_batch_delete",
        "feishu.bitable.record.batch_delete",
    ),
    ("feishu_bitable_field_create", "feishu.bitable.field.create"),
    ("feishu_bitable_field_list", "feishu.bitable.field.list"),
    ("feishu_bitable_field_update", "feishu.bitable.field.update"),
    ("feishu_bitable_field_delete", "feishu.bitable.field.delete"),
    ("feishu_bitable_view_create", "feishu.bitable.view.create"),
    ("feishu_bitable_view_get", "feishu.bitable.view.get"),
    ("feishu_bitable_view_list", "feishu.bitable.view.list"),
    ("feishu_bitable_view_patch", "feishu.bitable.view.patch"),
    (
        "feishu_bitable_record_search",
        "feishu.bitable.record.search",
    ),
    ("feishu_doc_create", "feishu.doc.create"),
    ("feishu_doc_append", "feishu.doc.append"),
    ("feishu_doc_read", "feishu.doc.read"),
    ("feishu_messages_history", "feishu.messages.history"),
    ("feishu_messages_get", "feishu.messages.get"),
    (
        "feishu_messages_resource_get",
        "feishu.messages.resource.get",
    ),
    ("feishu_messages_search", "feishu.messages.search"),
    ("feishu_messages_send", "feishu.messages.send"),
    ("feishu_messages_reply", "feishu.messages.reply"),
    ("feishu_card_update", "feishu.card.update"),
    ("feishu_calendar_list", "feishu.calendar.list"),
    ("feishu_calendar_freebusy", "feishu.calendar.freebusy"),
    ("feishu_calendar_primary_get", "feishu.calendar.primary.get"),
];

#[cfg(all(test, not(feature = "tool-file")))]
const FEISHU_TOOL_ALIAS_PAIRS: &[(&str, &str)] = &[
    ("feishu_whoami", "feishu.whoami"),
    ("feishu_bitable_app_create", "feishu.bitable.app.create"),
    ("feishu_bitable_app_get", "feishu.bitable.app.get"),
    ("feishu_bitable_app_list", "feishu.bitable.app.list"),
    ("feishu_bitable_app_patch", "feishu.bitable.app.patch"),
    ("feishu_bitable_app_copy", "feishu.bitable.app.copy"),
    ("feishu_bitable_list", "feishu.bitable.list"),
    ("feishu_bitable_table_create", "feishu.bitable.table.create"),
    ("feishu_bitable_table_patch", "feishu.bitable.table.patch"),
    (
        "feishu_bitable_table_batch_create",
        "feishu.bitable.table.batch_create",
    ),
    (
        "feishu_bitable_record_create",
        "feishu.bitable.record.create",
    ),
    (
        "feishu_bitable_record_update",
        "feishu.bitable.record.update",
    ),
    (
        "feishu_bitable_record_delete",
        "feishu.bitable.record.delete",
    ),
    (
        "feishu_bitable_record_batch_create",
        "feishu.bitable.record.batch_create",
    ),
    (
        "feishu_bitable_record_batch_update",
        "feishu.bitable.record.batch_update",
    ),
    (
        "feishu_bitable_record_batch_delete",
        "feishu.bitable.record.batch_delete",
    ),
    ("feishu_bitable_field_create", "feishu.bitable.field.create"),
    ("feishu_bitable_field_list", "feishu.bitable.field.list"),
    ("feishu_bitable_field_update", "feishu.bitable.field.update"),
    ("feishu_bitable_field_delete", "feishu.bitable.field.delete"),
    ("feishu_bitable_view_create", "feishu.bitable.view.create"),
    ("feishu_bitable_view_get", "feishu.bitable.view.get"),
    ("feishu_bitable_view_list", "feishu.bitable.view.list"),
    ("feishu_bitable_view_patch", "feishu.bitable.view.patch"),
    (
        "feishu_bitable_record_search",
        "feishu.bitable.record.search",
    ),
    ("feishu_doc_create", "feishu.doc.create"),
    ("feishu_doc_append", "feishu.doc.append"),
    ("feishu_doc_read", "feishu.doc.read"),
    ("feishu_messages_history", "feishu.messages.history"),
    ("feishu_messages_get", "feishu.messages.get"),
    ("feishu_messages_search", "feishu.messages.search"),
    ("feishu_messages_send", "feishu.messages.send"),
    ("feishu_messages_reply", "feishu.messages.reply"),
    ("feishu_card_update", "feishu.card.update"),
    ("feishu_calendar_list", "feishu.calendar.list"),
    ("feishu_calendar_freebusy", "feishu.calendar.freebusy"),
    ("feishu_calendar_primary_get", "feishu.calendar.primary.get"),
];

#[derive(Debug, Clone)]
struct FeishuToolContext {
    configured_account_id: String,
    configured_account_label: String,
    account_id: String,
    receive_id_type: String,
    client: FeishuClient,
    store: FeishuTokenStore,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct GrantSelectorPayload {
    account_id: Option<String>,
    open_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LoongInternalToolPayload {
    ingress: Option<FeishuInternalIngressPayload>,
    feishu_callback: Option<FeishuInternalCallbackPayload>,
}

impl LoongInternalToolPayload {
    fn ingress_requested_account_id(&self) -> Option<&str> {
        self.ingress_configured_account_id()
            .or_else(|| self.ingress_account_id())
    }

    fn ingress_configured_account_id(&self) -> Option<&str> {
        self.ingress
            .as_ref()
            .and_then(FeishuInternalIngressPayload::configured_account_id)
    }

    fn ingress_account_id(&self) -> Option<&str> {
        self.ingress
            .as_ref()
            .and_then(FeishuInternalIngressPayload::account_id)
    }

    fn ingress_conversation_id(&self) -> Option<&str> {
        self.ingress
            .as_ref()
            .and_then(FeishuInternalIngressPayload::conversation_id)
    }

    fn ingress_thread_id(&self) -> Option<&str> {
        self.ingress
            .as_ref()
            .and_then(FeishuInternalIngressPayload::thread_id)
    }

    fn ingress_history_container_id_type(&self) -> Option<&'static str> {
        self.ingress_thread_id()
            .map(|_| "thread")
            .or_else(|| self.ingress_conversation_id().map(|_| "chat"))
    }

    fn ingress_history_container_id(&self) -> Option<&str> {
        self.ingress_thread_id()
            .or_else(|| self.ingress_conversation_id())
    }

    fn ingress_message_id(&self) -> Option<&str> {
        self.ingress_reply_message_id()
    }

    fn ingress_reply_message_id(&self) -> Option<&str> {
        self.ingress
            .as_ref()
            .and_then(FeishuInternalIngressPayload::reply_message_id)
    }

    fn ingress_reply_in_thread(&self) -> bool {
        self.ingress
            .as_ref()
            .is_some_and(FeishuInternalIngressPayload::reply_in_thread)
    }

    fn ingress_resources(&self) -> Vec<FeishuInternalIngressResolvedResource> {
        self.ingress
            .as_ref()
            .map(FeishuInternalIngressPayload::resolved_resources)
            .unwrap_or_default()
    }

    fn feishu_callback_token(&self) -> Option<&str> {
        self.feishu_callback
            .as_ref()
            .and_then(FeishuInternalCallbackPayload::callback_token)
    }

    fn feishu_callback_open_message_id(&self) -> Option<&str> {
        self.feishu_callback
            .as_ref()
            .and_then(FeishuInternalCallbackPayload::open_message_id)
    }

    fn feishu_callback_open_chat_id(&self) -> Option<&str> {
        self.feishu_callback
            .as_ref()
            .and_then(FeishuInternalCallbackPayload::open_chat_id)
    }

    fn feishu_callback_operator_open_id(&self) -> Option<&str> {
        self.feishu_callback
            .as_ref()
            .and_then(FeishuInternalCallbackPayload::operator_open_id)
    }

    fn feishu_callback_deferred_context_id(&self) -> Option<&str> {
        self.feishu_callback
            .as_ref()
            .and_then(FeishuInternalCallbackPayload::deferred_context_id)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuInternalIngressPayload {
    source: Option<String>,
    channel: Option<FeishuInternalIngressChannelPayload>,
    delivery: Option<FeishuInternalIngressDeliveryPayload>,
}

impl FeishuInternalIngressPayload {
    fn is_feishu_channel(&self) -> bool {
        self.channel
            .as_ref()
            .and_then(|channel| trimmed_opt(channel.platform.as_deref()))
            .is_some_and(|platform| platform.eq_ignore_ascii_case("feishu"))
    }

    fn configured_account_id(&self) -> Option<&str> {
        if !self.is_feishu_channel() {
            return None;
        }
        self.channel
            .as_ref()
            .and_then(|channel| trimmed_opt(channel.configured_account_id.as_deref()))
    }

    fn account_id(&self) -> Option<&str> {
        if !self.is_feishu_channel() {
            return None;
        }
        self.channel
            .as_ref()
            .and_then(|channel| trimmed_opt(channel.account_id.as_deref()))
    }

    fn conversation_id(&self) -> Option<&str> {
        if !self.is_feishu_channel() {
            return None;
        }
        self.channel
            .as_ref()
            .and_then(|channel| trimmed_opt(channel.conversation_id.as_deref()))
    }

    fn thread_id(&self) -> Option<&str> {
        if !self.is_feishu_channel() {
            return None;
        }
        self.channel
            .as_ref()
            .and_then(|channel| trimmed_opt(channel.thread_id.as_deref()))
            .or_else(|| {
                self.delivery
                    .as_ref()
                    .and_then(|delivery| trimmed_opt(delivery.thread_root_id.as_deref()))
            })
    }

    fn reply_message_id(&self) -> Option<&str> {
        if !self.is_feishu_channel() {
            return None;
        }
        self.delivery.as_ref().and_then(|delivery| {
            trimmed_opt(delivery.source_message_id.as_deref())
                .or_else(|| trimmed_opt(delivery.parent_message_id.as_deref()))
        })
    }

    fn reply_in_thread(&self) -> bool {
        if !self.is_feishu_channel() {
            return false;
        }
        self.channel
            .as_ref()
            .and_then(|channel| trimmed_opt(channel.thread_id.as_deref()))
            .is_some()
            || self
                .delivery
                .as_ref()
                .and_then(|delivery| trimmed_opt(delivery.thread_root_id.as_deref()))
                .is_some()
    }

    fn resolved_resources(&self) -> Vec<FeishuInternalIngressResolvedResource> {
        if !self.is_feishu_channel() {
            return Vec::new();
        }
        self.delivery
            .as_ref()
            .map(FeishuInternalIngressDeliveryPayload::resolved_resources)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuInternalIngressChannelPayload {
    platform: Option<String>,
    configured_account_id: Option<String>,
    account_id: Option<String>,
    conversation_id: Option<String>,
    participant_id: Option<String>,
    thread_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuInternalIngressDeliveryPayload {
    source_message_id: Option<String>,
    sender_identity_key: Option<String>,
    thread_root_id: Option<String>,
    parent_message_id: Option<String>,
    resources: Vec<FeishuInternalIngressResourcePayload>,
}

impl FeishuInternalIngressDeliveryPayload {
    fn resolved_resources(&self) -> Vec<FeishuInternalIngressResolvedResource> {
        self.resources
            .iter()
            .filter_map(FeishuInternalIngressResourcePayload::resolved)
            .collect()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuInternalIngressResourcePayload {
    #[serde(rename = "type")]
    resource_type: Option<String>,
    file_key: Option<String>,
    file_name: Option<String>,
}

impl FeishuInternalIngressResourcePayload {
    fn resolved(&self) -> Option<FeishuInternalIngressResolvedResource> {
        Some(FeishuInternalIngressResolvedResource {
            resource_type: trimmed_opt(self.resource_type.as_deref())?.to_owned(),
            file_key: trimmed_opt(self.file_key.as_deref())?.to_owned(),
            file_name: trimmed_opt(self.file_name.as_deref()).map(str::to_owned),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FeishuInternalIngressResolvedResource {
    resource_type: String,
    file_key: String,
    file_name: Option<String>,
}

fn describe_ingress_resource(resource: &FeishuInternalIngressResolvedResource) -> String {
    let mut parts = vec![
        format!("type={}", resource.resource_type),
        format!("file_key={}", resource.file_key),
    ];
    if let Some(file_name) = resource.file_name.as_deref() {
        parts.push(format!("file_name={file_name}"));
    }
    parts.join(" ")
}

fn describe_ingress_resources(resources: &[FeishuInternalIngressResolvedResource]) -> String {
    resources
        .iter()
        .map(describe_ingress_resource)
        .collect::<Vec<_>>()
        .join("; ")
}

fn describe_ingress_resource_matches(
    resources: &[&FeishuInternalIngressResolvedResource],
) -> String {
    resources
        .iter()
        .map(|resource| describe_ingress_resource(resource))
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuInternalCallbackPayload {
    callback_token: Option<String>,
    open_message_id: Option<String>,
    open_chat_id: Option<String>,
    operator_open_id: Option<String>,
    deferred_context_id: Option<String>,
}

impl FeishuInternalCallbackPayload {
    fn callback_token(&self) -> Option<&str> {
        trimmed_opt(self.callback_token.as_deref())
    }

    fn open_message_id(&self) -> Option<&str> {
        trimmed_opt(self.open_message_id.as_deref())
    }

    fn open_chat_id(&self) -> Option<&str> {
        trimmed_opt(self.open_chat_id.as_deref())
    }

    fn operator_open_id(&self) -> Option<&str> {
        trimmed_opt(self.operator_open_id.as_deref())
    }

    fn deferred_context_id(&self) -> Option<&str> {
        trimmed_opt(self.deferred_context_id.as_deref())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuWhoamiPayload {
    account_id: Option<String>,
    open_id: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuDocCreatePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    title: Option<String>,
    folder_token: Option<String>,
    content: Option<String>,
    content_path: Option<String>,
    content_type: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuDocAppendPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    url: String,
    content: Option<String>,
    content_path: Option<String>,
    content_type: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FeishuDocReadPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    url: String,
    lang: Option<u8>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuMessagesHistoryPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    container_id_type: String,
    container_id: String,
    start_time: Option<String>,
    end_time: Option<String>,
    sort_type: Option<String>,
    page_size: Option<usize>,
    page_token: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuMessagesGetPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    message_id: String,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuMessagesSearchPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    user_id_type: Option<String>,
    page_size: Option<usize>,
    page_token: Option<String>,
    query: String,
    from_ids: Vec<String>,
    chat_ids: Vec<String>,
    message_type: Option<String>,
    at_chatter_ids: Vec<String>,
    from_type: Option<String>,
    chat_type: Option<String>,
    start_time: Option<String>,
    end_time: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuMessagesResourceGetPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    message_id: String,
    file_key: String,
    #[serde(rename = "type")]
    resource_type: String,
    save_as: String,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuMessagesSendPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    receive_id_type: Option<String>,
    receive_id: String,
    text: String,
    as_card: bool,
    post: Option<Value>,
    image_key: Option<String>,
    image_path: Option<String>,
    file_key: Option<String>,
    file_path: Option<String>,
    file_type: Option<String>,
    uuid: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuMessagesReplyPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    message_id: String,
    text: String,
    as_card: bool,
    post: Option<Value>,
    image_key: Option<String>,
    image_path: Option<String>,
    file_key: Option<String>,
    file_path: Option<String>,
    file_type: Option<String>,
    reply_in_thread: Option<bool>,
    uuid: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuCardUpdatePayload {
    account_id: Option<String>,
    callback_token: Option<String>,
    card: Value,
    markdown: Option<String>,
    shared: bool,
    open_ids: Option<Vec<String>>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

impl Default for FeishuCardUpdatePayload {
    fn default() -> Self {
        Self {
            account_id: None,
            callback_token: None,
            card: Value::Null,
            markdown: None,
            shared: false,
            open_ids: None,
            internal: LoongInternalToolPayload::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct PreparedFeishuToolMedia {
    image_key: Option<String>,
    image_upload: Option<PreparedFeishuToolUpload>,
    file_key: Option<String>,
    file_upload: Option<PreparedFeishuToolFileUpload>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedFeishuToolUpload {
    file_name: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedFeishuToolFileUpload {
    file_name: String,
    bytes: Vec<u8>,
    file_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedFeishuDocContent {
    content: String,
    content_type: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResolvedFeishuToolMedia {
    image_key: Option<String>,
    file_key: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuCalendarListPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    primary: bool,
    user_id_type: Option<String>,
    page_size: Option<usize>,
    page_token: Option<String>,
    sync_token: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuCalendarPrimaryGetPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    user_id_type: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loongclaw")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableListPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    page_token: Option<String>,
    page_size: Option<usize>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableAppCreatePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    name: String,
    folder_token: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableAppGetPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableAppListPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    folder_token: Option<String>,
    page_token: Option<String>,
    page_size: Option<usize>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableAppPatchPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    name: Option<String>,
    is_advanced: Option<bool>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableAppCopyPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    name: String,
    folder_token: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableRecordCreatePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    fields: Value,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableRecordUpdatePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    record_id: String,
    fields: Value,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableRecordDeletePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    record_id: String,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableRecordBatchCreatePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    records: Vec<Value>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableRecordBatchUpdatePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    records: Vec<Value>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableRecordBatchDeletePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    records: Vec<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableFieldCreatePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    field_name: String,
    #[serde(rename = "type")]
    field_type: i64,
    property: Option<Value>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableFieldListPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    view_id: Option<String>,
    page_size: Option<usize>,
    page_token: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableFieldUpdatePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    field_id: String,
    field_name: String,
    #[serde(rename = "type")]
    field_type: i64,
    property: Option<Value>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableFieldDeletePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    field_id: String,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableViewCreatePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    view_name: String,
    view_type: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableViewGetPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    view_id: String,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableViewListPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    page_size: Option<usize>,
    page_token: Option<String>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableViewPatchPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    view_id: String,
    view_name: String,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableTableCreatePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    name: String,
    default_view_name: Option<String>,
    fields: Option<Vec<Value>>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableTablePatchPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    name: String,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableTableBatchCreatePayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    tables: Vec<Value>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuBitableRecordSearchPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    app_token: String,
    table_id: String,
    page_token: Option<String>,
    page_size: Option<usize>,
    view_id: Option<String>,
    filter: Option<Value>,
    sort: Option<Value>,
    field_names: Option<Vec<String>>,
    automatic_fields: Option<bool>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuCalendarFreebusyPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    user_id_type: Option<String>,
    time_min: String,
    time_max: String,
    user_id: Option<String>,
    room_id: Option<String>,
    include_external_calendar: Option<bool>,
    only_busy: Option<bool>,
    need_rsvp_status: Option<bool>,
    #[serde(default, rename = "_loong", alias = "_loong")]
    internal: LoongInternalToolPayload,
}

pub(super) fn execute_feishu_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    match request.tool_name.as_str() {
        "feishu.whoami" => execute_feishu_whoami_tool_with_config(request, config),
        "feishu.bitable.app.create" => {
            execute_feishu_bitable_app_create_tool_with_config(request, config)
        }
        "feishu.bitable.app.get" => {
            execute_feishu_bitable_app_get_tool_with_config(request, config)
        }
        "feishu.bitable.app.list" => {
            execute_feishu_bitable_app_list_tool_with_config(request, config)
        }
        "feishu.bitable.app.patch" => {
            execute_feishu_bitable_app_patch_tool_with_config(request, config)
        }
        "feishu.bitable.app.copy" => {
            execute_feishu_bitable_app_copy_tool_with_config(request, config)
        }
        "feishu.bitable.list" => execute_feishu_bitable_list_tool_with_config(request, config),
        "feishu.bitable.table.create" => {
            execute_feishu_bitable_table_create_tool_with_config(request, config)
        }
        "feishu.bitable.table.patch" => {
            execute_feishu_bitable_table_patch_tool_with_config(request, config)
        }
        "feishu.bitable.table.batch_create" => {
            execute_feishu_bitable_table_batch_create_tool_with_config(request, config)
        }
        "feishu.bitable.record.create" => {
            execute_feishu_bitable_record_create_tool_with_config(request, config)
        }
        "feishu.bitable.record.update" => {
            execute_feishu_bitable_record_update_tool_with_config(request, config)
        }
        "feishu.bitable.record.delete" => {
            execute_feishu_bitable_record_delete_tool_with_config(request, config)
        }
        "feishu.bitable.record.batch_create" => {
            execute_feishu_bitable_record_batch_create_tool_with_config(request, config)
        }
        "feishu.bitable.record.batch_update" => {
            execute_feishu_bitable_record_batch_update_tool_with_config(request, config)
        }
        "feishu.bitable.record.batch_delete" => {
            execute_feishu_bitable_record_batch_delete_tool_with_config(request, config)
        }
        "feishu.bitable.field.create" => {
            execute_feishu_bitable_field_create_tool_with_config(request, config)
        }
        "feishu.bitable.field.list" => {
            execute_feishu_bitable_field_list_tool_with_config(request, config)
        }
        "feishu.bitable.field.update" => {
            execute_feishu_bitable_field_update_tool_with_config(request, config)
        }
        "feishu.bitable.field.delete" => {
            execute_feishu_bitable_field_delete_tool_with_config(request, config)
        }
        "feishu.bitable.view.create" => {
            execute_feishu_bitable_view_create_tool_with_config(request, config)
        }
        "feishu.bitable.view.get" => {
            execute_feishu_bitable_view_get_tool_with_config(request, config)
        }
        "feishu.bitable.view.list" => {
            execute_feishu_bitable_view_list_tool_with_config(request, config)
        }
        "feishu.bitable.view.patch" => {
            execute_feishu_bitable_view_patch_tool_with_config(request, config)
        }
        "feishu.bitable.record.search" => {
            execute_feishu_bitable_record_search_tool_with_config(request, config)
        }
        "feishu.doc.create" => execute_feishu_doc_create_tool_with_config(request, config),
        "feishu.doc.append" => execute_feishu_doc_append_tool_with_config(request, config),
        "feishu.doc.read" => execute_feishu_doc_read_tool_with_config(request, config),
        "feishu.messages.history" => {
            execute_feishu_messages_history_tool_with_config(request, config)
        }
        "feishu.messages.get" => execute_feishu_messages_get_tool_with_config(request, config),
        "feishu.messages.resource.get" => {
            execute_feishu_messages_resource_get_tool_with_config(request, config)
        }
        "feishu.messages.search" => {
            execute_feishu_messages_search_tool_with_config(request, config)
        }
        "feishu.messages.send" => execute_feishu_messages_send_tool_with_config(request, config),
        "feishu.messages.reply" => execute_feishu_messages_reply_tool_with_config(request, config),
        "feishu.card.update" => execute_feishu_card_update_tool_with_config(request, config),
        "feishu.calendar.list" => execute_feishu_calendar_list_tool_with_config(request, config),
        "feishu.calendar.freebusy" => {
            execute_feishu_calendar_freebusy_tool_with_config(request, config)
        }
        "feishu.calendar.primary.get" => {
            execute_feishu_calendar_primary_get_tool_with_config(request, config)
        }
        other => Err(format!("tool_not_found: unknown feishu tool `{other}`")),
    }
}

fn execute_feishu_whoami_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuWhoamiPayload>("feishu.whoami", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.open_id.as_deref())?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        let user_info = context.client.get_user_info(&grant.access_token).await?;
        let principal = map_user_info_to_principal(context.account_id.as_str(), &user_info)?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &principal,
            json!({
                "user_info": user_info,
                "grant_scopes": grant.scopes.as_slice(),
            }),
        ))
    })
}

fn execute_feishu_doc_create_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuDocCreatePayload>("feishu.doc.create", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let initial_content = prepare_feishu_doc_tool_content(
        "feishu.doc.create",
        payload.content.as_deref(),
        payload.content_path.as_deref(),
        payload.content_type.as_deref(),
        false,
        config,
    )?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_required_scopes(&grant, FEISHU_DOC_WRITE_REQUIRED_SCOPES, tool_name.as_str())?;
        let document = docs::create_document(
            &context.client,
            &grant.access_token,
            payload.title.as_deref(),
            payload.folder_token.as_deref(),
        )
        .await?;

        let mut content_inserted = false;
        let mut inserted_block_count = 0_usize;
        let mut insert_batch_count = 0_usize;
        if let Some(initial_content) = initial_content.as_ref() {
            let converted = docs::convert_content_to_blocks(
                &context.client,
                &grant.access_token,
                initial_content.content_type,
                initial_content.content.as_str(),
            )
            .await?;
            let insert_summary = docs::create_nested_blocks(
                &context.client,
                &grant.access_token,
                document.document_id.as_str(),
                &converted,
            )
            .await?;
            inserted_block_count = insert_summary.inserted_block_count;
            insert_batch_count = insert_summary.batch_count;
            content_inserted = true;
        }

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "document": document,
                "content_inserted": content_inserted,
                "inserted_block_count": inserted_block_count,
                "insert_batch_count": insert_batch_count,
                "content_type": initial_content.as_ref().map(|content| content.content_type),
            }),
        ))
    })
}

fn execute_feishu_doc_append_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuDocAppendPayload>("feishu.doc.append", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let url = require_non_empty("feishu.doc.append", "url", &payload.url)?;
    let prepared_content = prepare_feishu_doc_tool_content(
        "feishu.doc.append",
        payload.content.as_deref(),
        payload.content_path.as_deref(),
        payload.content_type.as_deref(),
        true,
        config,
    )?
    .ok_or_else(|| {
        "feishu.doc.append requires payload.content or payload.content_path".to_owned()
    })?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_required_scopes(&grant, FEISHU_DOC_WRITE_REQUIRED_SCOPES, tool_name.as_str())?;
        let document_id = docs::extract_document_id(url.as_str())
            .ok_or_else(|| "failed to resolve Feishu document id".to_owned())?;
        let converted = docs::convert_content_to_blocks(
            &context.client,
            &grant.access_token,
            prepared_content.content_type,
            prepared_content.content.as_str(),
        )
        .await?;
        let insert_summary = docs::create_nested_blocks(
            &context.client,
            &grant.access_token,
            document_id.as_str(),
            &converted,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "document": {
                    "document_id": document_id.clone(),
                    "url": format!("https://open.feishu.cn/docx/{document_id}")
                },
                "inserted_block_count": insert_summary.inserted_block_count,
                "insert_batch_count": insert_summary.batch_count,
                "content_type": prepared_content.content_type,
            }),
        ))
    })
}

fn execute_feishu_doc_read_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuDocReadPayload>("feishu.doc.read", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let url = require_non_empty("feishu.doc.read", "url", &payload.url)?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, FEISHU_DOC_READ_ACCEPTED_SCOPES, tool_name.as_str())?;
        let document = docs::fetch_document_content(
            &context.client,
            &grant.access_token,
            url.as_str(),
            payload.lang,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "document": document,
            }),
        ))
    })
}

fn execute_feishu_messages_search_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload =
        parse_payload::<FeishuMessagesSearchPayload>("feishu.messages.search", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let query = require_non_empty("feishu.messages.search", "query", &payload.query)?;
    let chat_ids = search_chat_scope(&payload);
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_required_scopes(&grant, &["search:message"], tool_name.as_str())?;
        let page = messages::search_messages(
            &context.client,
            &grant.access_token,
            &FeishuSearchMessagesQuery {
                user_id_type: payload.user_id_type.clone(),
                page_size: payload.page_size,
                page_token: payload.page_token.clone(),
                query,
                from_ids: payload.from_ids.clone(),
                chat_ids,
                message_type: payload.message_type.clone(),
                at_chatter_ids: payload.at_chatter_ids.clone(),
                from_type: payload.from_type.clone(),
                chat_type: payload.chat_type.clone(),
                start_time: payload.start_time.clone(),
                end_time: payload.end_time.clone(),
            },
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "page": page,
            }),
        ))
    })
}

fn execute_feishu_messages_history_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload =
        parse_payload::<FeishuMessagesHistoryPayload>("feishu.messages.history", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let container_id_type = require_non_empty_with_fallback(
        "feishu.messages.history",
        "container_id_type",
        Some(payload.container_id_type.as_str()),
        payload.internal.ingress_history_container_id_type(),
    )?;
    let container_id = require_non_empty_with_fallback(
        "feishu.messages.history",
        "container_id",
        Some(payload.container_id.as_str()),
        payload.internal.ingress_history_container_id(),
    )?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(
            &grant,
            &["im:message:readonly", "im:message.group_msg"],
            tool_name.as_str(),
        )?;
        let tenant_access_token = context.client.get_tenant_access_token().await?;
        let page = messages::fetch_message_history(
            &context.client,
            &tenant_access_token,
            &FeishuMessageHistoryQuery {
                container_id_type,
                container_id,
                start_time: payload.start_time.clone(),
                end_time: payload.end_time.clone(),
                sort_type: payload.sort_type.clone(),
                page_size: payload.page_size,
                page_token: payload.page_token.clone(),
            },
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "page": page,
            }),
        ))
    })
}

fn execute_feishu_messages_send_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload =
        parse_payload::<FeishuMessagesSendPayload>("feishu.messages.send", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let receive_id = require_non_empty_with_fallback(
        "feishu.messages.send",
        "receive_id",
        Some(payload.receive_id.as_str()),
        payload.internal.ingress_conversation_id(),
    )?;
    let prepared_media = prepare_feishu_tool_media(
        "feishu.messages.send",
        payload.image_key.as_deref(),
        payload.image_path.as_deref(),
        payload.file_key.as_deref(),
        payload.file_path.as_deref(),
        payload.file_type.as_deref(),
        config,
    )?;
    validate_feishu_tool_message_body_fields(
        "feishu.messages.send",
        Some(payload.text.as_str()),
        payload.as_card,
        payload.post.as_ref(),
        payload.image_key.as_deref(),
        payload.image_path.as_deref(),
        payload.file_key.as_deref(),
        payload.file_path.as_deref(),
    )?;
    let receive_id_type = trimmed_opt(payload.receive_id_type.as_deref())
        .unwrap_or(context.receive_id_type.as_str())
        .to_owned();
    let text = payload.text;
    let as_card = payload.as_card;
    let post = payload.post;
    let uuid = trimmed_opt(payload.uuid.as_deref()).map(ToOwned::to_owned);
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(
            &grant,
            crate::channel::feishu::api::FEISHU_MESSAGE_WRITE_ACCEPTED_SCOPES,
            tool_name.as_str(),
        )?;
        let tenant_access_token = context.client.get_tenant_access_token().await?;
        let media = resolve_prepared_feishu_tool_media(
            &context.client,
            &tenant_access_token,
            prepared_media,
        )
        .await?;
        let body = messages::resolve_outbound_message_body(
            "feishu.messages.send",
            "payload.text",
            "payload.as_card",
            "payload.post",
            "payload.image_key/payload.image_path",
            "payload.file_key/payload.file_path",
            Some(text.as_str()),
            as_card,
            post.as_ref(),
            media.image_key.as_deref(),
            media.file_key.as_deref(),
        )?;
        let msg_type = body.msg_type().to_owned();
        let mut target = ChannelOutboundTarget::feishu_receive_id(receive_id.clone())
            .with_feishu_receive_id_type(receive_id_type.clone());
        if let Some(uuid) = uuid.as_ref() {
            target = target.with_idempotency_key(uuid.clone());
        }
        let delivery = deliver_feishu_message_body(
            &context.client,
            &tenant_access_token,
            context.receive_id_type.as_str(),
            &target,
            &body,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "delivery": {
                    "mode": "send",
                    "receive_id_type": receive_id_type,
                    "receive_id": receive_id,
                    "msg_type": msg_type,
                    "message_id": delivery.message_id,
                    "root_id": delivery.root_id,
                    "parent_id": delivery.parent_id,
                    "uuid": uuid,
                },
            }),
        ))
    })
}

fn execute_feishu_messages_reply_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload =
        parse_payload::<FeishuMessagesReplyPayload>("feishu.messages.reply", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let message_id = require_non_empty_with_fallback(
        "feishu.messages.reply",
        "message_id",
        Some(payload.message_id.as_str()),
        payload.internal.ingress_reply_message_id(),
    )?;
    let prepared_media = prepare_feishu_tool_media(
        "feishu.messages.reply",
        payload.image_key.as_deref(),
        payload.image_path.as_deref(),
        payload.file_key.as_deref(),
        payload.file_path.as_deref(),
        payload.file_type.as_deref(),
        config,
    )?;
    validate_feishu_tool_message_body_fields(
        "feishu.messages.reply",
        Some(payload.text.as_str()),
        payload.as_card,
        payload.post.as_ref(),
        payload.image_key.as_deref(),
        payload.image_path.as_deref(),
        payload.file_key.as_deref(),
        payload.file_path.as_deref(),
    )?;
    let text = payload.text;
    let as_card = payload.as_card;
    let post = payload.post;
    let uuid = trimmed_opt(payload.uuid.as_deref()).map(ToOwned::to_owned);
    let reply_in_thread = payload
        .reply_in_thread
        .unwrap_or_else(|| payload.internal.ingress_reply_in_thread());
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(
            &grant,
            crate::channel::feishu::api::FEISHU_MESSAGE_WRITE_ACCEPTED_SCOPES,
            tool_name.as_str(),
        )?;
        let tenant_access_token = context.client.get_tenant_access_token().await?;
        let media = resolve_prepared_feishu_tool_media(
            &context.client,
            &tenant_access_token,
            prepared_media,
        )
        .await?;
        let body = messages::resolve_outbound_message_body(
            "feishu.messages.reply",
            "payload.text",
            "payload.as_card",
            "payload.post",
            "payload.image_key/payload.image_path",
            "payload.file_key/payload.file_path",
            Some(text.as_str()),
            as_card,
            post.as_ref(),
            media.image_key.as_deref(),
            media.file_key.as_deref(),
        )?;
        let msg_type = body.msg_type().to_owned();
        let mut target = ChannelOutboundTarget::feishu_message_reply(message_id.clone())
            .with_feishu_reply_in_thread(reply_in_thread);
        if let Some(uuid) = uuid.as_ref() {
            target = target.with_idempotency_key(uuid.clone());
        }
        let delivery = deliver_feishu_message_body(
            &context.client,
            &tenant_access_token,
            context.receive_id_type.as_str(),
            &target,
            &body,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "delivery": {
                    "mode": "reply",
                    "message_id": delivery.message_id,
                    "reply_to_message_id": message_id,
                    "reply_in_thread": reply_in_thread,
                    "msg_type": msg_type,
                    "root_id": delivery.root_id,
                    "parent_id": delivery.parent_id,
                    "uuid": uuid,
                },
            }),
        ))
    })
}

fn execute_feishu_card_update_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuCardUpdatePayload>("feishu.card.update", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.account_id.as_deref(), &payload.internal),
    )?;
    let callback_token = require_non_empty_with_fallback(
        "feishu.card.update",
        "callback_token",
        payload.callback_token.as_deref(),
        payload.internal.feishu_callback_token(),
    )?;
    let explicit_open_ids = payload
        .open_ids
        .as_ref()
        .map(|values| normalize_open_ids(values.iter().map(String::as_str)));
    if payload.shared
        && explicit_open_ids
            .as_ref()
            .is_some_and(|values| !values.is_empty())
    {
        return Err(
            "feishu.card.update payload.shared=true cannot be combined with non-empty payload.open_ids"
                .to_owned(),
        );
    }
    let effective_open_ids = if payload.shared {
        Vec::new()
    } else {
        explicit_open_ids.unwrap_or_else(|| {
            payload
                .internal
                .feishu_callback_operator_open_id()
                .map(|value| vec![value.to_owned()])
                .unwrap_or_default()
        })
    };
    let callback_open_message_id = payload
        .internal
        .feishu_callback_open_message_id()
        .map(ToOwned::to_owned);
    let callback_open_chat_id = payload
        .internal
        .feishu_callback_open_chat_id()
        .map(ToOwned::to_owned);
    let operator_open_id = payload
        .internal
        .feishu_callback_operator_open_id()
        .map(ToOwned::to_owned);
    let deferred_context_id = payload
        .internal
        .feishu_callback_deferred_context_id()
        .map(ToOwned::to_owned);
    let card = resolve_feishu_card_update_card(
        "feishu.card.update",
        payload.card,
        payload.markdown.as_deref(),
    )?;
    let update_request = cards::FeishuCardUpdateRequest {
        token: callback_token,
        card,
        open_ids: effective_open_ids.clone(),
    };
    update_request.validate()?;
    let tool_name = request.tool_name;
    let configured_account_id = context.configured_account_id.clone();

    if let Some(deferred_context_id) = deferred_context_id {
        let cards::FeishuCardUpdateRequest {
            token,
            card,
            open_ids,
        } = update_request;
        let callback_token_use_count = enqueue_deferred_feishu_card_update(
            deferred_context_id.as_str(),
            DeferredFeishuCardUpdate {
                configured_account_id,
                token,
                card,
                open_ids,
            },
        )?;
        return Ok(ok_outcome_without_principal(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            json!({
                    "update": {
                        "mode": "deferred",
                        "message": "queued_for_post_callback_dispatch",
                        "shared": payload.shared,
                        "open_ids": effective_open_ids,
                        "callback_token_use_count": callback_token_use_count,
                        "callback_token_use_limit": FEISHU_CARD_UPDATE_CALLBACK_TOKEN_USE_LIMIT,
                        "callback_open_message_id": callback_open_message_id,
                        "callback_open_chat_id": callback_open_chat_id,
                        "operator_open_id": operator_open_id,
                    },
            }),
        ));
    }

    run_feishu_future(async move {
        let tenant_access_token = context.client.get_tenant_access_token().await?;
        let receipt = cards::delay_update_message_card(
            &context.client,
            &tenant_access_token,
            &update_request,
        )
        .await?;

        Ok(ok_outcome_without_principal(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            json!({
                "update": {
                    "mode": "immediate",
                    "message": receipt.message,
                    "shared": payload.shared,
                    "open_ids": effective_open_ids,
                    "callback_open_message_id": callback_open_message_id,
                    "callback_open_chat_id": callback_open_chat_id,
                    "operator_open_id": operator_open_id,
                },
            }),
        ))
    })
}

fn resolve_feishu_card_update_card(
    tool_name: &str,
    card: Value,
    markdown: Option<&str>,
) -> CliResult<Value> {
    let markdown = markdown.and_then(|value| trimmed_opt(Some(value)));
    let explicit_card = (!card.is_null()).then_some(card);

    match (explicit_card, markdown) {
        (Some(_), Some(_)) => Err(format!(
            "{tool_name} accepts exactly one of payload.card or payload.markdown"
        )),
        (None, None) => Err(format!(
            "{tool_name} requires payload.card or payload.markdown"
        )),
        (Some(card), None) => Ok(card),
        (None, Some(markdown)) => Ok(cards::build_markdown_card(markdown)),
    }
}

fn execute_feishu_messages_get_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload =
        parse_payload::<FeishuMessagesGetPayload>("feishu.messages.get", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let message_id = require_non_empty_with_fallback(
        "feishu.messages.get",
        "message_id",
        Some(payload.message_id.as_str()),
        payload.internal.ingress_message_id(),
    )?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(
            &grant,
            &["im:message:readonly", "im:message.group_msg"],
            tool_name.as_str(),
        )?;
        let tenant_access_token = context.client.get_tenant_access_token().await?;
        let message =
            messages::fetch_message_detail(&context.client, &tenant_access_token, &message_id)
                .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "message": message,
            }),
        ))
    })
}

fn execute_feishu_messages_resource_get_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "tool-file"))]
    {
        let _ = (request, config);
        return Err(
            "feishu message resource tool is disabled in this build (enable feature `tool-file`)"
                .to_owned(),
        );
    }

    #[cfg(feature = "tool-file")]
    {
        let payload = parse_payload::<FeishuMessagesResourceGetPayload>(
            "feishu.messages.resource.get",
            request.payload,
        )?;
        let context = load_feishu_tool_context(
            config,
            requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
        )?;
        let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
        let message_id = require_non_empty_with_fallback(
            "feishu.messages.resource.get",
            "message_id",
            Some(payload.message_id.as_str()),
            payload.internal.ingress_message_id(),
        )?;
        let (file_key, resource_type) = resolve_message_resource_selection(
            "feishu.messages.resource.get",
            message_id.as_str(),
            &payload.file_key,
            &payload.resource_type,
            &payload.internal,
        )?;
        let save_as =
            require_non_empty("feishu.messages.resource.get", "save_as", &payload.save_as)?;
        let resource_type =
            resource_type
                .parse::<FeishuMessageResourceType>()
                .map_err(|error| {
                    format!("feishu.messages.resource.get invalid payload.type: {error}")
                })?;
        let save_path = super::file::resolve_safe_file_path_with_config(save_as.as_str(), config)?;
        let tool_name = request.tool_name;

        run_feishu_future(async move {
            let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
                &context.client,
                &context.store,
                &grant,
            )
            .await?;
            ensure_any_required_scope(
                &grant,
                FEISHU_MESSAGE_RESOURCE_ACCEPTED_SCOPES,
                tool_name.as_str(),
            )?;
            let tenant_access_token = context.client.get_tenant_access_token().await?;
            let resource = media::download_message_resource(
                &context.client,
                &tenant_access_token,
                &message_id,
                &file_key,
                resource_type,
                media::FEISHU_MESSAGE_RESOURCE_DOWNLOAD_MAX_BYTES,
            )
            .await?;
            if let Some(parent) = save_path.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "failed to create parent directory {}: {error}",
                        parent.display()
                    )
                })?;
            }
            fs::write(&save_path, &resource.bytes).map_err(|error| {
                format!(
                    "failed to write Feishu resource file {}: {error}",
                    save_path.display()
                )
            })?;

            Ok(ok_outcome(
                tool_name.as_str(),
                context.configured_account_label.as_str(),
                context.account_id.as_str(),
                &grant.principal,
                json!({
                    "message_id": resource.message_id,
                    "file_key": resource.file_key,
                    "resource_type": resource.resource_type.as_api_value(),
                    "content_type": resource.content_type,
                    "file_name": resource.file_name,
                    "path": save_path.display().to_string(),
                    "bytes_written": resource.bytes.len(),
                }),
            ))
        })
    }
}

fn execute_feishu_calendar_list_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload =
        parse_payload::<FeishuCalendarListPayload>("feishu.calendar.list", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_required_scopes(&grant, &["calendar:calendar:readonly"], tool_name.as_str())?;
        if payload.primary {
            let calendars = calendar::get_primary_calendars(
                &context.client,
                &grant.access_token,
                &calendar::FeishuPrimaryCalendarQuery {
                    user_id_type: Some(
                        payload
                            .user_id_type
                            .clone()
                            .unwrap_or_else(|| "open_id".to_owned()),
                    ),
                },
            )
            .await?;
            return Ok(ok_outcome(
                tool_name.as_str(),
                context.configured_account_label.as_str(),
                context.account_id.as_str(),
                &grant.principal,
                json!({
                    "primary": true,
                    "calendars": calendars,
                }),
            ));
        }

        let page = calendar::list_calendars(
            &context.client,
            &grant.access_token,
            &FeishuCalendarListQuery {
                page_size: payload.page_size,
                page_token: payload.page_token.clone(),
                sync_token: payload.sync_token.clone(),
            },
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "primary": false,
                "page": page,
            }),
        ))
    })
}

fn execute_feishu_calendar_primary_get_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuCalendarPrimaryGetPayload>(
        "feishu.calendar.primary.get",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_required_scopes(&grant, &["calendar:calendar:readonly"], tool_name.as_str())?;
        let calendars = calendar::get_primary_calendars(
            &context.client,
            &grant.access_token,
            &calendar::FeishuPrimaryCalendarQuery {
                user_id_type: Some(
                    payload
                        .user_id_type
                        .clone()
                        .unwrap_or_else(|| "open_id".to_owned()),
                ),
            },
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "calendars": calendars }),
        ))
    })
}

fn execute_feishu_calendar_freebusy_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuCalendarFreebusyPayload>(
        "feishu.calendar.freebusy",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let time_min = require_non_empty("feishu.calendar.freebusy", "time_min", &payload.time_min)?;
    let time_max = require_non_empty("feishu.calendar.freebusy", "time_max", &payload.time_max)?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_required_scopes(&grant, &["calendar:calendar:readonly"], tool_name.as_str())?;
        let effective_user_id = payload.user_id.clone().or_else(|| {
            trimmed_opt(payload.room_id.as_deref())
                .is_none()
                .then(|| grant.principal.open_id.clone())
        });
        let result = calendar::get_freebusy(
            &context.client,
            &grant.access_token,
            &FeishuCalendarFreebusyQuery {
                user_id_type: payload.user_id_type.clone().or_else(|| {
                    effective_user_id
                        .as_deref()
                        .and_then(|value| (!value.trim().is_empty()).then(|| "open_id".to_owned()))
                }),
                time_min,
                time_max,
                user_id: effective_user_id,
                room_id: payload.room_id.clone(),
                include_external_calendar: payload.include_external_calendar,
                only_busy: payload.only_busy,
                need_rsvp_status: payload.need_rsvp_status,
            },
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "result": result,
            }),
        ))
    })
}

#[cfg(test)]
mod payload_tests;
