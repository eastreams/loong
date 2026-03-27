use std::collections::BTreeMap;
#[cfg(feature = "tool-file")]
use std::fs;
use std::future::Future;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::CliResult;
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

const FEISHU_MESSAGE_RESOURCE_ACCEPTED_SCOPES: &[&str] = &[
    "im:message:readonly",
    "im:message.group_msg:readonly",
    "im:message",
    "im:message:send_as_bot",
    "im:message:send",
];
const FEISHU_DOC_READ_ACCEPTED_SCOPES: &[&str] = &["docx:document:readonly", "docx:document"];
const FEISHU_DOC_WRITE_REQUIRED_SCOPES: &[&str] = &["docx:document"];
const FEISHU_CARD_UPDATE_CALLBACK_TOKEN_USE_LIMIT: usize = 2;

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
];

#[cfg(all(test, not(feature = "tool-file")))]
const FEISHU_TOOL_ALIAS_PAIRS: &[(&str, &str)] = &[
    ("feishu_whoami", "feishu.whoami"),
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
struct LoongclawInternalToolPayload {
    ingress: Option<FeishuInternalIngressPayload>,
    feishu_callback: Option<FeishuInternalCallbackPayload>,
}

impl LoongclawInternalToolPayload {
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
    #[serde(default, rename = "_loongclaw")]
    internal: LoongclawInternalToolPayload,
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
    #[serde(default, rename = "_loongclaw")]
    internal: LoongclawInternalToolPayload,
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
    #[serde(default, rename = "_loongclaw")]
    internal: LoongclawInternalToolPayload,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FeishuDocReadPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    url: String,
    lang: Option<u8>,
    #[serde(default, rename = "_loongclaw")]
    internal: LoongclawInternalToolPayload,
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
    #[serde(default, rename = "_loongclaw")]
    internal: LoongclawInternalToolPayload,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FeishuMessagesGetPayload {
    #[serde(flatten)]
    selector: GrantSelectorPayload,
    message_id: String,
    #[serde(default, rename = "_loongclaw")]
    internal: LoongclawInternalToolPayload,
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
    #[serde(default, rename = "_loongclaw")]
    internal: LoongclawInternalToolPayload,
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
    #[serde(default, rename = "_loongclaw")]
    internal: LoongclawInternalToolPayload,
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
    #[serde(default, rename = "_loongclaw")]
    internal: LoongclawInternalToolPayload,
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
    #[serde(default, rename = "_loongclaw")]
    internal: LoongclawInternalToolPayload,
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
    #[serde(default, rename = "_loongclaw")]
    internal: LoongclawInternalToolPayload,
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
            internal: LoongclawInternalToolPayload::default(),
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
    #[serde(default, rename = "_loongclaw")]
    internal: LoongclawInternalToolPayload,
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
    #[serde(default, rename = "_loongclaw")]
    internal: LoongclawInternalToolPayload,
}

#[cfg(test)]
pub(super) fn feishu_tool_alias_pairs() -> &'static [(&'static str, &'static str)] {
    FEISHU_TOOL_ALIAS_PAIRS
}

pub(super) fn canonical_feishu_tool_name(raw: &str) -> Option<&'static str> {
    match raw {
        "feishu.whoami" | "feishu_whoami" => Some("feishu.whoami"),
        "feishu.doc.create" | "feishu_doc_create" => Some("feishu.doc.create"),
        "feishu.doc.append" | "feishu_doc_append" => Some("feishu.doc.append"),
        "feishu.doc.read" | "feishu_doc_read" => Some("feishu.doc.read"),
        "feishu.messages.history" | "feishu_messages_history" => Some("feishu.messages.history"),
        "feishu.messages.get" | "feishu_messages_get" => Some("feishu.messages.get"),
        #[cfg(feature = "tool-file")]
        "feishu.messages.resource.get" | "feishu_messages_resource_get" => {
            Some("feishu.messages.resource.get")
        }
        "feishu.messages.search" | "feishu_messages_search" => Some("feishu.messages.search"),
        "feishu.messages.send" | "feishu_messages_send" => Some("feishu.messages.send"),
        "feishu.messages.reply" | "feishu_messages_reply" => Some("feishu.messages.reply"),
        "feishu.card.update" | "feishu_card_update" => Some("feishu.card.update"),
        "feishu.calendar.list" | "feishu_calendar_list" => Some("feishu.calendar.list"),
        "feishu.calendar.freebusy" | "feishu_calendar_freebusy" => Some("feishu.calendar.freebusy"),
        _ => None,
    }
}

pub(super) fn is_known_feishu_tool_name(raw: &str) -> bool {
    canonical_feishu_tool_name(raw).is_some()
}

pub(super) fn feishu_tool_registry_entries() -> Vec<super::ToolRegistryEntry> {
    let mut entries = Vec::new();
    push_feishu_registry_entry(
        &mut entries,
        "feishu.calendar.freebusy",
        "Query Feishu calendar free/busy for the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.doc.create",
        "Create a Feishu docx document and optionally insert initial markdown or html content with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.doc.append",
        "Append markdown or html content to an existing Feishu docx document with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.doc.read",
        "Read Feishu Doc raw content with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.messages.get",
        "Read one Feishu message detail using a tenant token resolved from the selected account grant",
    );
    #[cfg(feature = "tool-file")]
    push_feishu_registry_entry(
        &mut entries,
        "feishu.messages.resource.get",
        "Download one Feishu message image or file resource under the configured file root, with safe ingress defaults when the current Feishu turn carries exactly one resource reference",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.messages.history",
        "List Feishu message history using a tenant token resolved from the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.messages.search",
        "Search Feishu messages with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.messages.send",
        "Send a Feishu text, post, image, file, or markdown card message with a tenant token resolved from the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.messages.reply",
        "Reply to a Feishu message with text, post, image, file, or a markdown card using a tenant token resolved from the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.card.update",
        "Update a Feishu interactive card through the delayed callback API, using the current callback token when available",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.calendar.list",
        "List Feishu calendars or primary calendars for the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.whoami",
        "Inspect the active Feishu grant principal and profile",
    );
    entries.sort_by_key(|entry| entry.name);
    entries
}

pub(super) fn feishu_provider_tool_definitions() -> Vec<Value> {
    let mut tools = Vec::new();
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_calendar_freebusy",
        "Query Feishu calendar free/busy for the selected account grant or an explicit user/room.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "user_id_type": {
                    "type": "string",
                    "description": "Optional Feishu calendar user id type. Defaults to `open_id` when user_id is inferred from the selected grant."
                },
                "time_min": {
                    "type": "string",
                    "description": "Inclusive time window start, typically RFC3339."
                },
                "time_max": {
                    "type": "string",
                    "description": "Exclusive time window end, typically RFC3339."
                },
                "user_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user id. Defaults to the selected grant open_id when room_id is omitted."
                },
                "room_id": {
                    "type": "string",
                    "description": "Optional meeting room id to query instead of a user calendar."
                },
                "include_external_calendar": {
                    "type": "boolean",
                    "description": "Whether to include external calendars."
                },
                "only_busy": {
                    "type": "boolean",
                    "description": "Whether to return only busy slots."
                },
                "need_rsvp_status": {
                    "type": "boolean",
                    "description": "Whether to include RSVP status in each slot."
                }
            },
            "required": ["time_min", "time_max"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_card_update",
        "Update a Feishu interactive card after a card callback. Pass markdown for a standard markdown card or card for full Feishu card JSON. When called from a Feishu callback turn, LoongClaw can infer account_id, callback_token, and a default exclusive open_ids target from internal callback context. Set shared=true for shared-card updates so callback operator defaults are suppressed. Callback tokens expire after 30 minutes and can be used at most twice.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through. Defaults from the current Feishu ingress when available."
                },
                "callback_token": {
                    "type": "string",
                    "description": "Optional callback token for delayed card updates. Usually inferred from the current Feishu card callback turn."
                },
                "card": {
                    "type": "object",
                    "description": "Optional full Feishu card JSON object to apply to the existing card. Mutually exclusive with `markdown`."
                },
                "markdown": {
                    "type": "string",
                    "description": "Optional markdown text to wrap in a standard markdown card. Mutually exclusive with `card`."
                },
                "shared": {
                    "type": "boolean",
                    "description": "Set true for shared-card updates. Shared-card updates must not send non-empty open_ids, and in callback turns this suppresses the default operator open_id target."
                },
                "open_ids": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Optional explicit open_id targets for non-shared cards. For shared cards, either omit open_ids or set shared=true. When omitted in a callback turn without shared=true, LoongClaw can default to the callback operator open_id."
                }
            },
            "required": [],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_calendar_list",
        "List Feishu calendars or primary calendars for the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "primary": {
                    "type": "boolean",
                    "description": "When true, list primary calendars for the selected user."
                },
                "user_id_type": {
                    "type": "string",
                    "description": "Optional Feishu user id type for primary calendar lookup."
                },
                "page_size": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100
                },
                "page_token": {
                    "type": "string"
                },
                "sync_token": {
                    "type": "string"
                }
            },
            "required": [],
            "additionalProperties": false
        }),
    );
    let mut doc_create_parameters = json!({
        "type": "object",
        "properties": {
            "account_id": {
                "type": "string",
                "description": "Optional Feishu configured account id to route through."
            },
            "open_id": {
                "type": "string",
                "description": "Optional explicit Feishu user open_id grant selector."
            },
            "title": {
                "type": "string",
                "description": "Optional plain-text Feishu document title."
            },
            "folder_token": {
                "type": "string",
                "description": "Optional folder token where the new document should be created."
            },
            "content": {
                "type": "string",
                "description": "Optional initial content to convert and insert into the new document. Mutually exclusive with `content_path`."
            },
            "content_type": {
                "type": "string",
                "enum": ["markdown", "html"],
                "description": "Optional content format for `content` or `content_path`. Defaults to the file extension for `content_path` (`.md`/`.markdown` => markdown, `.html`/`.htm` => html) and otherwise `markdown`."
            }
        },
        "required": [],
        "additionalProperties": false
    });
    #[cfg(feature = "tool-file")]
    if let Some(properties) = doc_create_parameters
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        properties.insert(
            "content_path".to_owned(),
            json!({
                "type": "string",
                "description": "Optional relative or rooted local UTF-8 text file path resolved under the configured tool file root and inserted into the new document. Mutually exclusive with `content`."
            }),
        );
    }
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_doc_create",
        "Create a Feishu document with the selected account grant and optionally insert initial markdown or html content into the new doc.",
        doc_create_parameters,
    );
    let mut doc_append_parameters = json!({
        "type": "object",
        "properties": {
            "account_id": {
                "type": "string",
                "description": "Optional Feishu configured account id to route through."
            },
            "open_id": {
                "type": "string",
                "description": "Optional explicit Feishu user open_id grant selector."
            },
            "url": {
                "type": "string",
                "description": "Feishu docx URL or document id of the existing document to append to."
            },
            "content": {
                "type": "string",
                "description": "Markdown or html content to convert and append to the document. Mutually exclusive with `content_path`."
            },
            "content_type": {
                "type": "string",
                "enum": ["markdown", "html"],
                "description": "Optional content format for `content` or `content_path`. Defaults to the file extension for `content_path` (`.md`/`.markdown` => markdown, `.html`/`.htm` => html) and otherwise `markdown`."
            }
        },
        "required": ["url", "content"],
        "additionalProperties": false
    });
    #[cfg(feature = "tool-file")]
    if let Some(parameters) = doc_append_parameters.as_object_mut() {
        if let Some(properties) = parameters
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        {
            properties.insert(
                "content_path".to_owned(),
                json!({
                    "type": "string",
                    "description": "Relative or rooted local UTF-8 text file path resolved under the configured tool file root and appended to the document. Mutually exclusive with `content`."
                }),
            );
        }
        parameters.insert("required".to_owned(), json!(["url"]));
        parameters.insert(
            "anyOf".to_owned(),
            json!([
                { "required": ["content"] },
                { "required": ["content_path"] }
            ]),
        );
    }
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_doc_append",
        "Append markdown or html content to an existing Feishu document identified by docx url or document id using the selected account grant.",
        doc_append_parameters,
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_doc_read",
        "Read a Feishu document by docx url or document id using the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "url": {
                    "type": "string",
                    "description": "Feishu docx URL or document id."
                },
                "lang": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 255,
                    "description": "Optional Feishu language selector."
                }
            },
            "required": ["url"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_messages_get",
        "Fetch one Feishu message detail using a tenant token resolved from the selected account grant. When called from a Feishu conversation, LoongClaw can infer the account and current message from ingress context.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "message_id": {
                    "type": "string",
                    "description": "Feishu message id to fetch. Optional when current Feishu ingress already provides the source message id."
                }
            },
            "required": [],
            "additionalProperties": false
        }),
    );
    #[cfg(feature = "tool-file")]
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_messages_resource_get",
        "Explicitly download one Feishu message image or file resource using a tenant token resolved from the selected account grant and save it under the configured file root. When called from a Feishu conversation, LoongClaw can infer the source message from ingress context and can infer the resource key or type when the current Feishu ingress carries exactly one Feishu message resource or when either payload.file_key or payload.type uniquely identifies one current ingress resource for the same message, as long as payload.message_id is omitted or matches the current ingress message. If the current Feishu ingress summary exposes resource_inventory, choose one entry and copy its file_key plus payload_type into this tool call when multiple resources are present. Outside the current ingress turn, also pass the source message_id explicitly. This does not perform implicit webhook binary downloads.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "message_id": {
                    "type": "string",
                    "description": "Feishu message id that owns the resource. Optional when current Feishu ingress already identifies the source message. Outside the current ingress turn, provide this explicitly. If you override it to a different message, current ingress resource defaults no longer apply."
                },
                "file_key": {
                    "type": "string",
                    "description": "Feishu message resource key paired with the source message id. Optional when the current Feishu ingress carries exactly one Feishu message resource or when payload.type uniquely selects one current ingress resource for the same message, as long as payload.message_id is omitted or matches the current ingress message. If the current Feishu ingress summary includes resource_inventory and multiple resources are present, choose one entry and copy its file_key explicitly."
                },
                "type": {
                    "type": "string",
                    "enum": ["image", "file", "audio", "media"],
                    "description": "Feishu message resource type. Use `image` for image resources, preview images from media messages, and image resource keys; use `file`, `audio`, or `media` for binary file resources. `audio` and `media` aliases normalize to the Feishu file transport type. If the current Feishu ingress summary includes resource_inventory, copy the selected entry's payload_type here. Optional when the current Feishu ingress carries exactly one Feishu message resource or when payload.file_key uniquely selects one current ingress resource for the same message, as long as payload.message_id is omitted or matches the current ingress message."
                },
                "save_as": {
                    "type": "string",
                    "description": "Relative file path to write under the configured file root."
                }
            },
            "required": ["save_as"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_messages_history",
        "List Feishu message history using a tenant token resolved from the selected account grant. When called from a Feishu conversation, LoongClaw can infer the current chat or thread container from ingress context.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "container_id_type": {
                    "type": "string",
                    "description": "Feishu message container id type, for example `chat` or `thread`. Optional when current Feishu conversation ingress can infer the active chat or thread."
                },
                "container_id": {
                    "type": "string",
                    "description": "Feishu message container id. Optional when current Feishu conversation ingress can infer the active chat or thread id."
                },
                "start_time": {
                    "type": "string"
                },
                "end_time": {
                    "type": "string"
                },
                "sort_type": {
                    "type": "string"
                },
                "page_size": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100
                },
                "page_token": {
                    "type": "string"
                }
            },
            "required": [],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_messages_search",
        "Search Feishu messages using the selected account grant. When called from the current Feishu conversation, LoongClaw can infer the account and default chat scope from ingress context.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "user_id_type": {
                    "type": "string",
                    "description": "Optional Feishu search user id type."
                },
                "page_size": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100
                },
                "page_token": {
                    "type": "string"
                },
                "query": {
                    "type": "string",
                    "description": "Search query string."
                },
                "from_ids": {
                    "type": "array",
                    "items": {"type": "string"}
                },
                "chat_ids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional Feishu chat ids to scope the search. When omitted inside the current Feishu conversation, LoongClaw can default this to the active conversation."
                },
                "message_type": {
                    "type": "string"
                },
                "at_chatter_ids": {
                    "type": "array",
                    "items": {"type": "string"}
                },
                "from_type": {
                    "type": "string"
                },
                "chat_type": {
                    "type": "string"
                },
                "start_time": {
                    "type": "string"
                },
                "end_time": {
                    "type": "string"
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    );
    let mut send_parameters = json!({
        "type": "object",
        "properties": {
            "account_id": {
                "type": "string",
                "description": "Optional Feishu configured account id to route through."
            },
            "open_id": {
                "type": "string",
                "description": "Optional explicit Feishu user open_id grant selector."
            },
            "receive_id_type": {
                "type": "string",
                "description": "Optional Feishu receive_id_type override. Defaults to the configured account receive_id_type."
            },
            "receive_id": {
                "type": "string",
                "description": "Feishu receive id to send to. Optional when current Feishu conversation ingress already identifies the active chat."
            },
            "text": {
                "type": "string",
                "description": "Optional plain-text body to send. Mutually exclusive with `post`, `image_key`, `image_path`, `file_key`, and `file_path`."
            },
            "post": {
                "type": "object",
                "description": "Optional Feishu post rich-text content JSON object. Mutually exclusive with `text`, `image_key`, `image_path`, `file_key`, and `file_path`; incompatible with `as_card`."
            },
            "image_key": {
                "type": "string",
                "description": "Optional uploaded Feishu image_key for an image message. Mutually exclusive with `text`, `post`, `image_path`, `file_key`, `file_path`, and `as_card`."
            },
            "file_key": {
                "type": "string",
                "description": "Optional uploaded Feishu file_key for a file message. Mutually exclusive with `text`, `post`, `image_key`, `image_path`, `file_path`, and `as_card`."
            },
            "as_card": {
                "type": "boolean",
                "description": "When true, wrap `text` in a markdown interactive card instead of sending a plain text message. Not allowed with `post`, `image_key`, `image_path`, `file_key`, or `file_path`."
            },
            "uuid": {
                "type": "string",
                "description": "Optional Feishu request UUID used for one-hour message deduplication."
            }
        },
        "required": [],
        "additionalProperties": false
    });
    #[cfg(feature = "tool-file")]
    if let Some(properties) = send_parameters
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        properties.insert(
            "image_path".to_owned(),
            json!({
                "type": "string",
                "description": "Optional relative or rooted local image path resolved under the configured tool file root, uploaded to Feishu before sending. Mutually exclusive with `image_key`, `text`, `post`, `file_key`, `file_path`, and `as_card`."
            }),
        );
        properties.insert(
            "file_path".to_owned(),
            json!({
                "type": "string",
                "description": "Optional relative or rooted local file path resolved under the configured tool file root, uploaded to Feishu before sending. Mutually exclusive with `file_key`, `text`, `post`, `image_key`, `image_path`, and `as_card`."
            }),
        );
        properties.insert(
            "file_type".to_owned(),
            json!({
                "type": "string",
                "description": "Optional Feishu upload file_type used only with `file_path`. Defaults to `stream`."
            }),
        );
    }
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_messages_send",
        "Send a Feishu text, post, image, file, or markdown card message using a tenant token resolved from the selected account grant. When called from the current Feishu conversation, LoongClaw can infer the account and receive_id from ingress context.",
        send_parameters,
    );
    let mut reply_parameters = json!({
        "type": "object",
        "properties": {
            "account_id": {
                "type": "string",
                "description": "Optional Feishu configured account id to route through."
            },
            "open_id": {
                "type": "string",
                "description": "Optional explicit Feishu user open_id grant selector."
            },
            "message_id": {
                "type": "string",
                "description": "Feishu message id to reply to. Optional when current Feishu ingress already identifies the source Feishu message."
            },
            "text": {
                "type": "string",
                "description": "Optional plain-text reply body. Mutually exclusive with `post`, `image_key`, `image_path`, `file_key`, and `file_path`."
            },
            "post": {
                "type": "object",
                "description": "Optional Feishu post rich-text content JSON object. Mutually exclusive with `text`, `image_key`, `image_path`, `file_key`, and `file_path`; incompatible with `as_card`."
            },
            "image_key": {
                "type": "string",
                "description": "Optional uploaded Feishu image_key for an image reply. Mutually exclusive with `text`, `post`, `image_path`, `file_key`, `file_path`, and `as_card`."
            },
            "file_key": {
                "type": "string",
                "description": "Optional uploaded Feishu file_key for a file reply. Mutually exclusive with `text`, `post`, `image_key`, `image_path`, `file_path`, and `as_card`."
            },
            "as_card": {
                "type": "boolean",
                "description": "When true, wrap `text` in a markdown interactive card. Not allowed with `post`, `image_key`, `image_path`, `file_key`, or `file_path`."
            },
            "reply_in_thread": {
                "type": "boolean",
                "description": "When true, force the reply to be posted in thread form. When omitted, LoongClaw defaults to thread form if internal Feishu ingress metadata indicates the source message is already in a thread/topic."
            },
            "uuid": {
                "type": "string",
                "description": "Optional Feishu request UUID used for one-hour reply deduplication."
            }
        },
        "required": [],
        "additionalProperties": false
    });
    #[cfg(feature = "tool-file")]
    if let Some(properties) = reply_parameters
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        properties.insert(
            "image_path".to_owned(),
            json!({
                "type": "string",
                "description": "Optional relative or rooted local image path resolved under the configured tool file root, uploaded to Feishu before replying. Mutually exclusive with `image_key`, `text`, `post`, `file_key`, `file_path`, and `as_card`."
            }),
        );
        properties.insert(
            "file_path".to_owned(),
            json!({
                "type": "string",
                "description": "Optional relative or rooted local file path resolved under the configured tool file root, uploaded to Feishu before replying. Mutually exclusive with `file_key`, `text`, `post`, `image_key`, `image_path`, and `as_card`."
            }),
        );
        properties.insert(
            "file_type".to_owned(),
            json!({
                "type": "string",
                "description": "Optional Feishu upload file_type used only with `file_path`. Defaults to `stream`."
            }),
        );
    }
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_messages_reply",
        "Reply to a Feishu message with text, post, image, file, or a markdown card using a tenant token resolved from the selected account grant. When called from a Feishu conversation, LoongClaw can infer the account and source Feishu message from ingress context.",
        reply_parameters,
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_whoami",
        "Resolve the currently selected Feishu OAuth grant and fetch the live user profile.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                }
            },
            "required": [],
            "additionalProperties": false
        }),
    );
    tools.sort_by(|left, right| {
        feishu_provider_tool_function_name(left).cmp(feishu_provider_tool_function_name(right))
    });
    tools
}

pub(super) fn feishu_shape_examples() -> BTreeMap<&'static str, Value> {
    let mut shapes = BTreeMap::new();
    shapes.insert(
        "feishu.doc.create",
        json!({
            "title": "Release Plan",
            "content": "# Release Plan",
            "content_type": "markdown"
        }),
    );
    shapes.insert(
        "feishu.doc.read",
        json!({
            "url": "https://open.feishu.cn/docx/doxcnDemo"
        }),
    );
    shapes.insert(
        "feishu.doc.append",
        json!({
            "url": "https://open.feishu.cn/docx/doxcnDemo",
            "content": "Follow-up note"
        }),
    );
    shapes.insert(
        "feishu.messages.search",
        json!({
            "query": "release note",
            "chat_ids": ["oc_demo_chat"]
        }),
    );
    shapes.insert(
        "feishu.messages.history",
        json!({
            "container_id_type": "chat",
            "container_id": "oc_demo_chat",
            "page_size": 20
        }),
    );
    shapes.insert(
        "feishu.messages.get",
        json!({
            "message_id": "om_123"
        }),
    );
    #[cfg(feature = "tool-file")]
    shapes.insert(
        "feishu.messages.send",
        json!({
            "receive_id": "oc_demo_chat",
            "image_path": "uploads/demo.png"
        }),
    );
    #[cfg(feature = "tool-file")]
    shapes.insert(
        "feishu.messages.reply",
        json!({
            "message_id": "om_123",
            "file_path": "uploads/spec-sheet.pdf",
            "file_type": "stream"
        }),
    );
    #[cfg(feature = "tool-file")]
    shapes.insert(
        "feishu.messages.resource.get",
        json!({
            "message_id": "om_123",
            "file_key": "img_from_resource_inventory",
            "type": "image",
            "save_as": "downloads/preview.png"
        }),
    );
    shapes.insert(
        "feishu.card.update",
        json!({
            "shared": true,
            "markdown": "Approved for everyone"
        }),
    );
    shapes.insert(
        "feishu.calendar.list",
        json!({
            "primary": true
        }),
    );
    shapes.insert(
        "feishu.calendar.freebusy",
        json!({
            "time_min": "2026-03-12T09:00:00+08:00",
            "time_max": "2026-03-12T18:00:00+08:00"
        }),
    );
    shapes.insert("feishu.whoami", json!({}));
    shapes
}

pub(super) fn execute_feishu_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    match request.tool_name.as_str() {
        "feishu.whoami" => execute_feishu_whoami_tool_with_config(request, config),
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
            &["im:message:readonly", "im:message.group_msg:readonly"],
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
        let delivery = messages::send_outbound_message(
            &context.client,
            &tenant_access_token,
            &receive_id_type,
            &receive_id,
            &body,
            uuid.as_deref(),
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
        let delivery = messages::reply_outbound_message(
            &context.client,
            &tenant_access_token,
            &message_id,
            &body,
            reply_in_thread,
            uuid.as_deref(),
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
            &["im:message:readonly", "im:message.group_msg:readonly"],
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
                payload.user_id_type.as_deref().or(Some("open_id")),
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

fn parse_payload<T>(tool_name: &str, payload: serde_json::Value) -> Result<T, String>
where
    T: DeserializeOwned,
{
    serde_json::from_value(payload)
        .map_err(|error| format!("{tool_name} payload validation failed: {error}"))
}

fn load_feishu_tool_context(
    config: &super::runtime_config::ToolRuntimeConfig,
    requested_account_id: Option<&str>,
) -> CliResult<FeishuToolContext> {
    let Some(runtime) = config.feishu.as_ref() else {
        return Err(
            "feishu tool runtime is unavailable; configure feishu credentials and integration storage first"
                .to_owned(),
        );
    };
    let resolved = crate::channel::feishu::api::resolve_requested_feishu_account(
        &runtime.channel,
        trimmed_opt(requested_account_id),
        "set payload.account_id to one of those configured accounts to disambiguate the Feishu tool request",
    )?;
    let client = FeishuClient::from_configs(&resolved, &runtime.integration)?;
    let configured_account_id = resolved.configured_account_id.clone();
    let configured_account_label = resolved.configured_account_label.clone();
    let account_id = resolved.account.id.clone();
    let receive_id_type = resolved.receive_id_type;
    let store = FeishuTokenStore::new(runtime.integration.resolved_sqlite_path());

    Ok(FeishuToolContext {
        configured_account_id,
        configured_account_label,
        account_id,
        receive_id_type,
        client,
        store,
    })
}

fn require_selected_grant(
    context: &FeishuToolContext,
    open_id: Option<&str>,
) -> CliResult<FeishuGrant> {
    let resolution = crate::channel::feishu::api::resolve_grant_selection(
        &context.store,
        context.account_id.as_str(),
        trimmed_opt(open_id),
    )?;
    if let Some(grant) = resolution.selected_grant().cloned() {
        return Ok(grant);
    }
    Err(
        crate::channel::feishu::api::describe_grant_selection_error_for_display(
            context.account_id.as_str(),
            context.configured_account_id.as_str(),
            &resolution,
        )
        .unwrap_or_else(|| {
            format!(
                "no stored Feishu grant for account `{}`; run `loongclaw feishu auth start --account {}` first",
                context.configured_account_id, context.configured_account_id
            )
        }),
    )
}

fn require_non_empty(tool_name: &str, field: &str, value: &str) -> CliResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{tool_name} requires payload.{field}"));
    }
    Ok(trimmed.to_owned())
}

fn resolve_feishu_doc_content_type(
    tool_name: &str,
    has_content: bool,
    raw: Option<&str>,
) -> CliResult<Option<&'static str>> {
    match trimmed_opt(raw) {
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "markdown" => Ok(Some("markdown")),
            "html" => Ok(Some("html")),
            other => Err(format!(
                "unsupported feishu document content_type `{other}`; expected `markdown` or `html`"
            )),
        },
        None if !has_content && raw.is_some() => Err(format!(
            "{tool_name} payload.content_type requires payload.content or payload.content_path"
        )),
        None => Ok(None),
    }
}

fn prepare_feishu_doc_tool_content(
    tool_name: &str,
    content: Option<&str>,
    content_path: Option<&str>,
    content_type: Option<&str>,
    required: bool,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> CliResult<Option<PreparedFeishuDocContent>> {
    let inline_content = trimmed_opt(content).map(ToOwned::to_owned);
    let file_path = trimmed_opt(content_path);
    if inline_content.is_some() && file_path.is_some() {
        return Err(format!(
            "{tool_name} accepts either payload.content or payload.content_path, not both"
        ));
    }

    let has_content = inline_content.is_some() || file_path.is_some();
    let explicit_content_type =
        resolve_feishu_doc_content_type(tool_name, has_content, content_type)?;

    match (inline_content, file_path) {
        (Some(content), None) => Ok(Some(PreparedFeishuDocContent {
            content,
            content_type: explicit_content_type.unwrap_or("markdown"),
        })),
        (None, Some(path)) => {
            let content =
                read_safe_tool_text_file(tool_name, "payload.content_path", path, config)?;
            Ok(Some(PreparedFeishuDocContent {
                content,
                content_type: explicit_content_type
                    .unwrap_or_else(|| infer_feishu_doc_content_type_from_path(Path::new(path))),
            }))
        }
        (None, None) if required => Err(format!(
            "{tool_name} requires payload.content or payload.content_path"
        )),
        (None, None) => Ok(None),
        (Some(_), Some(_)) => Err(format!(
            "{tool_name} accepts either payload.content or payload.content_path, not both"
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

fn require_non_empty_with_fallback(
    tool_name: &str,
    field: &str,
    value: Option<&str>,
    fallback: Option<&str>,
) -> CliResult<String> {
    value
        .and_then(|value| trimmed_opt(Some(value)))
        .or_else(|| fallback.and_then(|value| trimmed_opt(Some(value))))
        .map(str::to_owned)
        .ok_or_else(|| format!("{tool_name} requires payload.{field}"))
}

fn normalize_open_ids<'a, I>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut seen = std::collections::BTreeSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_owned()) {
            continue;
        }
        normalized.push(trimmed.to_owned());
    }
    normalized
}

fn requested_account_id<'a>(
    explicit: Option<&'a str>,
    internal: &'a LoongclawInternalToolPayload,
) -> Option<&'a str> {
    explicit.or_else(|| internal.ingress_requested_account_id())
}

fn resolve_message_resource_selection(
    tool_name: &str,
    effective_message_id: &str,
    payload_file_key: &str,
    payload_resource_type: &str,
    internal: &LoongclawInternalToolPayload,
) -> CliResult<(String, String)> {
    let explicit_file_key = trimmed_opt(Some(payload_file_key));
    let explicit_resource_type = trimmed_opt(Some(payload_resource_type))
        .map(normalize_message_resource_type_alias)
        .transpose()
        .map_err(|error| format!("{tool_name} invalid payload.type: {error}"))?;
    let ingress_message_override = ingress_message_override_reason(internal, effective_message_id);
    let ingress_resources = ingress_resources_for_effective_message(internal, effective_message_id);
    let ingress_resource = match (explicit_file_key, explicit_resource_type.as_deref()) {
        (None, None) => {
            single_ingress_resource_for_selection(tool_name, ingress_resources.as_slice())?
        }
        (Some(explicit_file_key), None) => infer_ingress_resource_from_file_key(
            tool_name,
            explicit_file_key,
            ingress_resources.as_slice(),
        )?,
        (None, Some(explicit_resource_type)) => infer_ingress_resource_from_type(
            tool_name,
            explicit_resource_type,
            ingress_resources.as_slice(),
        )?,
        (Some(explicit_file_key), Some(explicit_resource_type)) => {
            validate_explicit_ingress_resource_pair(
                tool_name,
                explicit_file_key,
                explicit_resource_type,
                ingress_resources.as_slice(),
            )?;
            None
        }
    };

    if let Some(current_ingress_message_id) = ingress_message_override {
        match (explicit_file_key, explicit_resource_type.as_deref()) {
            (None, None) => {
                return Err(format!(
                    "{tool_name} requires payload.file_key and payload.type because payload.message_id `{effective_message_id}` differs from current Feishu ingress message `{current_ingress_message_id}`; current ingress resource defaults only apply when payload.message_id is omitted or matches the current message"
                ));
            }
            (None, Some(_)) => {
                return Err(format!(
                    "{tool_name} requires payload.file_key because payload.message_id `{effective_message_id}` differs from current Feishu ingress message `{current_ingress_message_id}`; current ingress resource defaults only apply when payload.message_id is omitted or matches the current message"
                ));
            }
            (Some(_), None) => {
                return Err(format!(
                    "{tool_name} requires payload.type because payload.message_id `{effective_message_id}` differs from current Feishu ingress message `{current_ingress_message_id}`; current ingress resource defaults only apply when payload.message_id is omitted or matches the current message"
                ));
            }
            (Some(_), Some(_)) => {}
        }
    }

    let file_key = require_non_empty_with_fallback(
        tool_name,
        "file_key",
        explicit_file_key,
        ingress_resource
            .as_ref()
            .map(|resource| resource.file_key.as_str()),
    )?;
    let resource_type = require_non_empty_with_fallback(
        tool_name,
        "type",
        explicit_resource_type.as_deref(),
        ingress_resource
            .as_ref()
            .map(|resource| resource.resource_type.as_str()),
    )?;
    Ok((file_key, resource_type))
}

fn ingress_resources_for_effective_message(
    internal: &LoongclawInternalToolPayload,
    effective_message_id: &str,
) -> Vec<FeishuInternalIngressResolvedResource> {
    if internal
        .ingress_message_id()
        .is_some_and(|message_id| message_id == effective_message_id)
    {
        return internal.ingress_resources();
    }
    Vec::new()
}

fn ingress_message_override_reason<'a>(
    internal: &'a LoongclawInternalToolPayload,
    effective_message_id: &str,
) -> Option<&'a str> {
    internal
        .ingress_message_id()
        .filter(|message_id| *message_id != effective_message_id)
}

fn single_ingress_resource_for_selection(
    tool_name: &str,
    ingress_resources: &[FeishuInternalIngressResolvedResource],
) -> CliResult<Option<FeishuInternalIngressResolvedResource>> {
    match ingress_resources {
        [] => Ok(None),
        [resource] => Ok(Some(resource.clone())),
        _ => Err(format!(
            "{tool_name} requires payload.file_key and payload.type when current Feishu ingress carries multiple Feishu message resources; available ingress resources: {}. If the current Feishu ingress summary includes resource_inventory, choose one entry and copy its file_key plus payload_type.",
            describe_ingress_resources(ingress_resources)
        )),
    }
}

fn validate_explicit_ingress_resource_pair(
    tool_name: &str,
    explicit_file_key: &str,
    explicit_resource_type: &str,
    ingress_resources: &[FeishuInternalIngressResolvedResource],
) -> CliResult<()> {
    if ingress_resources.is_empty() {
        return Ok(());
    }

    if ingress_resources.iter().any(|resource| {
        resource.file_key == explicit_file_key
            && normalize_ingress_message_resource_type(tool_name, resource)
                .is_ok_and(|resource_type| resource_type == explicit_resource_type)
    }) {
        return Ok(());
    }

    let matching_file_key = ingress_resources
        .iter()
        .find(|resource| resource.file_key == explicit_file_key);
    if let Some(resource) = matching_file_key {
        return Err(format!(
            "{tool_name} payload.type conflicts with the current Feishu ingress resource selected by payload.file_key ({}); choose one entry from resource_inventory and copy its payload_type, or override both payload.message_id and payload.file_key when targeting a different Feishu message resource",
            describe_ingress_resource(resource)
        ));
    }

    let matching_type = ingress_resources
        .iter()
        .filter(|resource| {
            normalize_ingress_message_resource_type(tool_name, resource)
                .is_ok_and(|resource_type| resource_type == explicit_resource_type)
        })
        .collect::<Vec<_>>();
    if !matching_type.is_empty() {
        return Err(format!(
            "{tool_name} payload.file_key `{explicit_file_key}` does not match the current Feishu ingress resource(s) selected by payload.type: {}. Choose one entry from resource_inventory and copy its file_key, or override both payload.message_id and payload.file_key when targeting a different Feishu message resource",
            describe_ingress_resource_matches(matching_type.as_slice())
        ));
    }

    Err(format!(
        "{tool_name} payload.file_key `{explicit_file_key}` and payload.type `{explicit_resource_type}` did not match any current Feishu ingress resource; available ingress resources: {}. Choose one entry from resource_inventory and copy its file_key plus payload_type, or override payload.message_id when targeting a different Feishu message resource",
        describe_ingress_resources(ingress_resources)
    ))
}

fn infer_ingress_resource_from_file_key(
    tool_name: &str,
    explicit_file_key: &str,
    ingress_resources: &[FeishuInternalIngressResolvedResource],
) -> CliResult<Option<FeishuInternalIngressResolvedResource>> {
    match ingress_resources {
        [] => Ok(None),
        [resource] => {
            if explicit_file_key != resource.file_key {
                return Err(format!(
                    "{tool_name} payload.file_key conflicts with the current Feishu ingress resource ({}); provide payload.type explicitly to override ingress defaults or omit payload.file_key to use ingress defaults",
                    describe_ingress_resource(resource)
                ));
            }
            Ok(Some(resource.clone()))
        }
        _ => {
            let matches = ingress_resources
                .iter()
                .filter(|resource| resource.file_key == explicit_file_key)
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [] => Err(format!(
                    "{tool_name} payload.file_key `{explicit_file_key}` did not match any current Feishu ingress resource; available ingress resources: {}. Provide payload.type explicitly to override ingress defaults or choose one entry from resource_inventory and copy its file_key plus payload_type.",
                    describe_ingress_resources(ingress_resources)
                )),
                [resource] => Ok(Some((*resource).clone())),
                _ => Err(format!(
                    "{tool_name} payload.file_key matches multiple current Feishu ingress resources: {}. Provide payload.type explicitly to disambiguate or choose one entry from resource_inventory and copy its payload_type.",
                    describe_ingress_resource_matches(matches.as_slice())
                )),
            }
        }
    }
}

fn infer_ingress_resource_from_type(
    tool_name: &str,
    explicit_resource_type: &str,
    ingress_resources: &[FeishuInternalIngressResolvedResource],
) -> CliResult<Option<FeishuInternalIngressResolvedResource>> {
    match ingress_resources {
        [] => Ok(None),
        [resource] => {
            let ingress_resource_type =
                normalize_ingress_message_resource_type(tool_name, resource)?;
            if explicit_resource_type != ingress_resource_type {
                return Err(format!(
                    "{tool_name} payload.type conflicts with the current Feishu ingress resource ({}); provide payload.file_key explicitly to override ingress defaults or omit payload.type to use ingress defaults",
                    describe_ingress_resource(resource)
                ));
            }
            Ok(Some(resource.clone()))
        }
        _ => {
            let mut matches = Vec::new();
            for resource in ingress_resources {
                if explicit_resource_type
                    == normalize_ingress_message_resource_type(tool_name, resource)?
                {
                    matches.push(resource);
                }
            }
            match matches.as_slice() {
                [] => Err(format!(
                    "{tool_name} payload.type `{explicit_resource_type}` did not match any current Feishu ingress resource; available ingress resources: {}. Provide payload.file_key explicitly to override ingress defaults or choose one entry from resource_inventory and copy its file_key plus payload_type.",
                    describe_ingress_resources(ingress_resources)
                )),
                [resource] => Ok(Some((*resource).clone())),
                _ => Err(format!(
                    "{tool_name} payload.type matches multiple current Feishu ingress resources: {}. Provide payload.file_key explicitly to disambiguate and choose one entry from resource_inventory.",
                    describe_ingress_resource_matches(matches.as_slice())
                )),
            }
        }
    }
}

fn normalize_ingress_message_resource_type(
    tool_name: &str,
    resource: &FeishuInternalIngressResolvedResource,
) -> CliResult<String> {
    normalize_message_resource_type_alias(resource.resource_type.as_str())
        .map_err(|error| format!("{tool_name} invalid ingress resource type: {error}"))
}

fn normalize_message_resource_type_alias(value: &str) -> CliResult<String> {
    value
        .parse::<FeishuMessageResourceType>()
        .map(|resource_type| resource_type.as_api_value().to_owned())
}

fn prepare_feishu_tool_media(
    tool_name: &str,
    image_key: Option<&str>,
    image_path: Option<&str>,
    file_key: Option<&str>,
    file_path: Option<&str>,
    file_type: Option<&str>,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> CliResult<PreparedFeishuToolMedia> {
    ensure_tool_media_source_exclusive(
        tool_name,
        "payload.image_key",
        image_key,
        "payload.image_path",
        image_path,
    )?;
    ensure_tool_media_source_exclusive(
        tool_name,
        "payload.file_key",
        file_key,
        "payload.file_path",
        file_path,
    )?;
    if trimmed_opt(file_type).is_some() && trimmed_opt(file_path).is_none() {
        return Err(format!(
            "{tool_name} only allows payload.file_type with payload.file_path"
        ));
    }

    let image_key = trimmed_opt(image_key).map(ToOwned::to_owned);
    let file_key = trimmed_opt(file_key).map(ToOwned::to_owned);
    let image_upload = match trimmed_opt(image_path) {
        Some(path) => Some(read_safe_tool_media_file(
            tool_name,
            "payload.image_path",
            path,
            config,
        )?),
        None => None,
    };
    let file_upload = match trimmed_opt(file_path) {
        Some(path) => {
            let upload = read_safe_tool_media_file(tool_name, "payload.file_path", path, config)?;
            Some(PreparedFeishuToolFileUpload {
                file_name: upload.file_name,
                bytes: upload.bytes,
                file_type: trimmed_opt(file_type)
                    .unwrap_or(media::FEISHU_DEFAULT_MESSAGE_FILE_TYPE)
                    .to_owned(),
            })
        }
        None => None,
    };

    Ok(PreparedFeishuToolMedia {
        image_key,
        image_upload,
        file_key,
        file_upload,
    })
}

fn validate_feishu_tool_message_body_fields(
    tool_name: &str,
    text: Option<&str>,
    as_card: bool,
    post: Option<&Value>,
    image_key: Option<&str>,
    image_path: Option<&str>,
    file_key: Option<&str>,
    file_path: Option<&str>,
) -> CliResult<()> {
    messages::resolve_outbound_message_body(
        tool_name,
        "payload.text",
        "payload.as_card",
        "payload.post",
        "payload.image_key/payload.image_path",
        "payload.file_key/payload.file_path",
        text,
        as_card,
        post,
        trimmed_opt(image_key).or_else(|| trimmed_opt(image_path).map(|_| "__image_path__")),
        trimmed_opt(file_key).or_else(|| trimmed_opt(file_path).map(|_| "__file_path__")),
    )
    .map(|_| ())
}

async fn resolve_prepared_feishu_tool_media(
    client: &FeishuClient,
    tenant_access_token: &str,
    prepared: PreparedFeishuToolMedia,
) -> CliResult<ResolvedFeishuToolMedia> {
    let image_key = match (prepared.image_key, prepared.image_upload) {
        (Some(image_key), None) => Some(image_key),
        (None, Some(upload)) => Some(
            media::upload_message_image(
                client,
                tenant_access_token,
                upload.file_name.as_str(),
                upload.bytes,
            )
            .await?
            .image_key,
        ),
        (Some(_), Some(_)) => {
            return Err(
                "feishu tool media preparation allowed both image_key and image_upload".to_owned(),
            );
        }
        (None, None) => None,
    };
    let file_key = match (prepared.file_key, prepared.file_upload) {
        (Some(file_key), None) => Some(file_key),
        (None, Some(upload)) => Some(
            media::upload_message_file(
                client,
                tenant_access_token,
                upload.file_name.as_str(),
                upload.bytes,
                upload.file_type.as_str(),
                None,
            )
            .await?
            .file_key,
        ),
        (Some(_), Some(_)) => {
            return Err(
                "feishu tool media preparation allowed both file_key and file_upload".to_owned(),
            );
        }
        (None, None) => None,
    };

    Ok(ResolvedFeishuToolMedia {
        image_key,
        file_key,
    })
}

fn ensure_tool_media_source_exclusive(
    tool_name: &str,
    key_field: &str,
    key: Option<&str>,
    path_field: &str,
    path: Option<&str>,
) -> CliResult<()> {
    if trimmed_opt(key).is_some() && trimmed_opt(path).is_some() {
        return Err(format!(
            "{tool_name} accepts either {key_field} or {path_field}, not both"
        ));
    }
    Ok(())
}

#[cfg(feature = "tool-file")]
fn read_safe_tool_text_file(
    tool_name: &str,
    field: &str,
    raw_path: &str,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> CliResult<String> {
    let resolved = super::file::resolve_safe_file_path_with_config(raw_path, config)?;
    let bytes = fs::read(&resolved).map_err(|error| {
        format!(
            "{tool_name} failed to read {} `{}`: {error}",
            field,
            resolved.display()
        )
    })?;
    if bytes.is_empty() {
        return Err(format!(
            "{tool_name} requires {} `{}` to be non-empty UTF-8 text",
            field,
            resolved.display()
        ));
    }
    let content = String::from_utf8(bytes).map_err(|error| {
        format!(
            "{tool_name} requires {} `{}` to contain valid UTF-8 text: {error}",
            field,
            resolved.display()
        )
    })?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "{tool_name} requires {} `{}` to be non-empty UTF-8 text",
            field,
            resolved.display()
        ));
    }
    Ok(trimmed.to_owned())
}

#[cfg(not(feature = "tool-file"))]
fn read_safe_tool_text_file(
    tool_name: &str,
    field: &str,
    raw_path: &str,
    _config: &super::runtime_config::ToolRuntimeConfig,
) -> CliResult<String> {
    let _ = raw_path;
    Err(format!(
        "{tool_name} does not support {field} unless feature `tool-file` is enabled"
    ))
}

#[cfg(feature = "tool-file")]
fn read_safe_tool_media_file(
    tool_name: &str,
    field: &str,
    raw_path: &str,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> CliResult<PreparedFeishuToolUpload> {
    let resolved = super::file::resolve_safe_file_path_with_config(raw_path, config)?;
    let file_name = resolved
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{tool_name} requires {field} to include a file name"))?;
    let bytes = fs::read(&resolved).map_err(|error| {
        format!(
            "{tool_name} failed to read {} `{}`: {error}",
            field,
            resolved.display()
        )
    })?;
    if bytes.is_empty() {
        return Err(format!(
            "{tool_name} requires {} `{}` to be non-empty",
            field,
            resolved.display()
        ));
    }
    Ok(PreparedFeishuToolUpload { file_name, bytes })
}

#[cfg(not(feature = "tool-file"))]
fn read_safe_tool_media_file(
    tool_name: &str,
    field: &str,
    raw_path: &str,
    _config: &super::runtime_config::ToolRuntimeConfig,
) -> CliResult<PreparedFeishuToolUpload> {
    let _ = raw_path;
    Err(format!(
        "{tool_name} does not support {field} unless feature `tool-file` is enabled"
    ))
}

fn search_chat_scope(payload: &FeishuMessagesSearchPayload) -> Vec<String> {
    let explicit = payload
        .chat_ids
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if !explicit.is_empty() {
        return explicit;
    }

    payload
        .internal
        .ingress_conversation_id()
        .map(|conversation_id| vec![conversation_id.to_owned()])
        .unwrap_or_default()
}

fn push_feishu_registry_entry(
    entries: &mut Vec<super::ToolRegistryEntry>,
    name: &'static str,
    description: &'static str,
) {
    entries.push(super::ToolRegistryEntry { name, description });
}

fn push_feishu_provider_tool_definition(
    tools: &mut Vec<Value>,
    name: &'static str,
    description: &'static str,
    parameters: Value,
) {
    tools.push(json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    }));
}

fn feishu_provider_tool_function_name(tool: &Value) -> &str {
    tool.get("function")
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn ensure_required_scopes(
    grant: &FeishuGrant,
    required: &[&str],
    tool_name: &str,
) -> CliResult<()> {
    let missing = required
        .iter()
        .copied()
        .filter(|scope| !grant.scopes.contains(scope))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }

    Err(format!(
        "{tool_name} requires Feishu scopes [{}] for `{}`; rerun the OAuth flow with the required scopes",
        missing.join(", "),
        grant.principal.storage_key()
    ))
}

fn ensure_any_required_scope(
    grant: &FeishuGrant,
    accepted: &[&str],
    tool_name: &str,
) -> CliResult<()> {
    if accepted
        .iter()
        .copied()
        .any(|scope| grant.scopes.contains(scope))
    {
        return Ok(());
    }

    Err(format!(
        "{tool_name} requires at least one Feishu scope [{}] for `{}`; rerun the OAuth flow with the required scopes",
        accepted.join(", "),
        grant.principal.storage_key()
    ))
}

fn ok_outcome(
    tool_name: &str,
    configured_account: &str,
    account_id: &str,
    principal: &FeishuUserPrincipal,
    payload: serde_json::Value,
) -> ToolCoreOutcome {
    let mut body = json!({
        "adapter": "core-tools",
        "tool_name": tool_name,
        "configured_account": configured_account,
        "account_id": account_id,
        "principal": principal,
    });
    if let Some(object) = body.as_object_mut()
        && let Some(extra) = payload.as_object()
    {
        for (key, value) in extra {
            object.insert(key.clone(), value.clone());
        }
    }
    ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: body,
    }
}

fn ok_outcome_without_principal(
    tool_name: &str,
    configured_account: &str,
    account_id: &str,
    payload: serde_json::Value,
) -> ToolCoreOutcome {
    let mut body = json!({
        "adapter": "core-tools",
        "tool_name": tool_name,
        "configured_account": configured_account,
        "account_id": account_id,
    });
    if let Some(object) = body.as_object_mut()
        && let Some(extra) = payload.as_object()
    {
        for (key, value) in extra {
            object.insert(key.clone(), value.clone());
        }
    }
    ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: body,
    }
}

fn run_feishu_future<F>(future: F) -> CliResult<ToolCoreOutcome>
where
    F: Future<Output = CliResult<ToolCoreOutcome>> + Send + 'static,
{
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("build feishu tool runtime failed: {error}"))?;
        runtime.block_on(future)
    })
    .join()
    .map_err(|error| format!("feishu tool execution thread panicked: {error:?}"))?
}

fn trimmed_opt(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}
