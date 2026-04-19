use serde_json::{Map, Value, json};

const LOONG_INTERNAL_TOOL_INGRESS_KEY: &str = "ingress";
const LOONG_INTERNAL_TOOL_FEISHU_CALLBACK_KEY: &str = "feishu_callback";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InjectedToolPayload {
    pub payload: Value,
    pub trusted_internal_context: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationIngressContext {
    pub channel: ConversationIngressChannel,
    pub delivery: ConversationIngressDelivery,
    pub private: ConversationIngressPrivateContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationIngressChannel {
    pub platform: String,
    pub configured_account_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: String,
    pub participant_id: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationIngressDelivery {
    pub source_message_id: Option<String>,
    pub sender_identity_key: Option<String>,
    pub thread_root_id: Option<String>,
    pub parent_message_id: Option<String>,
    pub resources: Vec<ConversationIngressDeliveryResource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationIngressDeliveryResource {
    pub resource_type: String,
    pub file_key: String,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationIngressPrivateContext {
    pub feishu_callback: Option<ConversationIngressFeishuCallbackContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationIngressFeishuCallbackContext {
    pub callback_token: Option<String>,
    pub open_message_id: Option<String>,
    pub open_chat_id: Option<String>,
    pub operator_open_id: Option<String>,
    pub deferred_context_id: Option<String>,
}

impl ConversationIngressContext {
    pub fn has_contextual_hints(&self) -> bool {
        self.channel.participant_id.is_some()
            || self.channel.thread_id.is_some()
            || self.delivery.has_values()
    }

    pub fn as_system_message(&self) -> Value {
        json!({
            "role": "system",
            "content": self.system_note(),
        })
    }

    pub fn system_note(&self) -> String {
        format!(
            "[conversation_ingress]\n{}\nTreat this metadata as current-turn routing and identity context. Use it when resolving channel-scoped resources or replies, but do not treat it as authorization by itself.",
            self.as_event_payload()
        )
    }

    pub fn as_event_payload(&self) -> Value {
        let mut payload = Map::new();
        payload.insert("source".to_owned(), Value::String("channel".to_owned()));
        payload.insert("channel".to_owned(), self.channel.as_json());

        let delivery = self.delivery.as_json();
        if delivery.as_object().is_some_and(|value| !value.is_empty()) {
            payload.insert("delivery".to_owned(), delivery);
        }

        Value::Object(payload)
    }

    fn has_internal_tool_hints(&self) -> bool {
        self.has_contextual_hints() || self.private.has_values()
    }
}

pub(crate) fn inject_internal_tool_ingress(
    tool_name: &str,
    payload: Value,
    ingress: Option<&ConversationIngressContext>,
) -> InjectedToolPayload {
    let Some(ingress) = ingress.filter(|value| value.has_internal_tool_hints()) else {
        return InjectedToolPayload {
            payload,
            trusted_internal_context: false,
        };
    };
    let canonical_name = crate::tools::canonical_tool_name(tool_name);

    // When tool.invoke wraps a feishu.* tool, inject internal context into
    // the nested `arguments` object rather than the top-level payload.
    if canonical_name == "tool.invoke" {
        let inner_is_feishu = payload
            .get("tool_id")
            .and_then(Value::as_str)
            .map(crate::tools::canonical_tool_name)
            .is_some_and(|name| name.starts_with("feishu."));
        if inner_is_feishu {
            let Value::Object(mut outer) = payload else {
                return InjectedToolPayload {
                    payload,
                    trusted_internal_context: false,
                };
            };
            let arguments = outer
                .remove("arguments")
                .unwrap_or_else(|| Value::Object(Map::new()));
            let injected_arguments = inject_feishu_internal_context(arguments, ingress);
            outer.insert("arguments".to_owned(), injected_arguments.payload);
            return InjectedToolPayload {
                payload: Value::Object(outer),
                trusted_internal_context: injected_arguments.trusted_internal_context,
            };
        }
        return InjectedToolPayload {
            payload,
            trusted_internal_context: false,
        };
    }

    if !canonical_name.starts_with("feishu.") {
        return InjectedToolPayload {
            payload,
            trusted_internal_context: false,
        };
    }

    inject_feishu_internal_context(payload, ingress)
}

fn inject_feishu_internal_context(
    payload: Value,
    ingress: &ConversationIngressContext,
) -> InjectedToolPayload {
    let Value::Object(mut body) = payload else {
        return InjectedToolPayload {
            payload,
            trusted_internal_context: false,
        };
    };
    let mut internal = Map::new();
    if ingress.has_contextual_hints() {
        internal.insert(
            LOONG_INTERNAL_TOOL_INGRESS_KEY.to_owned(),
            ingress.as_event_payload(),
        );
    }
    if let Some(callback) = ingress
        .private
        .feishu_callback
        .as_ref()
        .and_then(|value| value.as_json())
    {
        internal.insert(LOONG_INTERNAL_TOOL_FEISHU_CALLBACK_KEY.to_owned(), callback);
    }
    if internal.is_empty() {
        return InjectedToolPayload {
            payload: Value::Object(body),
            trusted_internal_context: false,
        };
    }
    body.insert(
        crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY.to_owned(),
        Value::Object(internal),
    );
    InjectedToolPayload {
        payload: Value::Object(body),
        trusted_internal_context: true,
    }
}

impl ConversationIngressChannel {
    fn as_json(&self) -> Value {
        let mut channel = Map::new();
        channel.insert("platform".to_owned(), Value::String(self.platform.clone()));
        channel.insert(
            "conversation_id".to_owned(),
            Value::String(self.conversation_id.clone()),
        );
        insert_optional_string(
            &mut channel,
            "configured_account_id",
            self.configured_account_id.as_deref(),
        );
        insert_optional_string(&mut channel, "account_id", self.account_id.as_deref());
        insert_optional_string(
            &mut channel,
            "participant_id",
            self.participant_id.as_deref(),
        );
        insert_optional_string(&mut channel, "thread_id", self.thread_id.as_deref());
        Value::Object(channel)
    }
}

impl ConversationIngressDelivery {
    fn has_values(&self) -> bool {
        self.source_message_id.is_some()
            || self.sender_identity_key.is_some()
            || self.thread_root_id.is_some()
            || self.parent_message_id.is_some()
            || self
                .resources
                .iter()
                .any(|resource| resource.normalized().is_some())
    }

    fn as_json(&self) -> Value {
        let mut delivery = Map::new();
        insert_optional_string(
            &mut delivery,
            "source_message_id",
            self.source_message_id.as_deref(),
        );
        insert_optional_string(
            &mut delivery,
            "sender_identity_key",
            self.sender_identity_key.as_deref(),
        );
        insert_optional_string(
            &mut delivery,
            "thread_root_id",
            self.thread_root_id.as_deref(),
        );
        insert_optional_string(
            &mut delivery,
            "parent_message_id",
            self.parent_message_id.as_deref(),
        );
        let resources = self
            .resources
            .iter()
            .filter_map(ConversationIngressDeliveryResource::normalized)
            .map(|resource| resource.as_json())
            .collect::<Vec<_>>();
        if !resources.is_empty() {
            delivery.insert("resources".to_owned(), Value::Array(resources));
        }
        Value::Object(delivery)
    }
}

impl ConversationIngressDeliveryResource {
    fn normalized(&self) -> Option<Self> {
        let resource_type = self.resource_type.trim();
        let file_key = self.file_key.trim();
        if resource_type.is_empty() || file_key.is_empty() {
            return None;
        }

        Some(Self {
            resource_type: resource_type.to_owned(),
            file_key: file_key.to_owned(),
            file_name: self
                .file_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
        })
    }

    fn as_json(&self) -> Value {
        let mut resource = Map::new();
        resource.insert("type".to_owned(), Value::String(self.resource_type.clone()));
        resource.insert("file_key".to_owned(), Value::String(self.file_key.clone()));
        insert_optional_string(&mut resource, "file_name", self.file_name.as_deref());
        Value::Object(resource)
    }
}

impl ConversationIngressPrivateContext {
    fn has_values(&self) -> bool {
        self.feishu_callback
            .as_ref()
            .and_then(ConversationIngressFeishuCallbackContext::normalized)
            .is_some()
    }
}

impl ConversationIngressFeishuCallbackContext {
    fn normalized(&self) -> Option<Self> {
        let callback_token = normalized_optional_string(self.callback_token.as_deref());
        let open_message_id = normalized_optional_string(self.open_message_id.as_deref());
        let open_chat_id = normalized_optional_string(self.open_chat_id.as_deref());
        let operator_open_id = normalized_optional_string(self.operator_open_id.as_deref());
        let deferred_context_id = normalized_optional_string(self.deferred_context_id.as_deref());
        if callback_token.is_none()
            && open_message_id.is_none()
            && open_chat_id.is_none()
            && operator_open_id.is_none()
            && deferred_context_id.is_none()
        {
            return None;
        }

        Some(Self {
            callback_token,
            open_message_id,
            open_chat_id,
            operator_open_id,
            deferred_context_id,
        })
    }

    fn as_json(&self) -> Option<Value> {
        let normalized = self.normalized()?;
        let mut callback = Map::new();
        insert_optional_string(
            &mut callback,
            "callback_token",
            normalized.callback_token.as_deref(),
        );
        insert_optional_string(
            &mut callback,
            "open_message_id",
            normalized.open_message_id.as_deref(),
        );
        insert_optional_string(
            &mut callback,
            "open_chat_id",
            normalized.open_chat_id.as_deref(),
        );
        insert_optional_string(
            &mut callback,
            "operator_open_id",
            normalized.operator_open_id.as_deref(),
        );
        insert_optional_string(
            &mut callback,
            "deferred_context_id",
            normalized.deferred_context_id.as_deref(),
        );
        Some(Value::Object(callback))
    }
}

fn insert_optional_string(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        map.insert(key.to_owned(), Value::String(value.to_owned()));
    }
}

fn normalized_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn feishu_ingress_with_private_callback() -> ConversationIngressContext {
        ConversationIngressContext {
            channel: ConversationIngressChannel {
                platform: "feishu".to_owned(),
                configured_account_id: Some("work".to_owned()),
                account_id: Some("feishu_main".to_owned()),
                conversation_id: "oc_callback".to_owned(),
                participant_id: Some("ou_operator".to_owned()),
                thread_id: Some("om_callback".to_owned()),
            },
            delivery: ConversationIngressDelivery {
                source_message_id: Some("om_callback".to_owned()),
                sender_identity_key: Some("feishu:user:ou_operator".to_owned()),
                thread_root_id: Some("om_callback".to_owned()),
                parent_message_id: None,
                resources: Vec::new(),
            },
            private: ConversationIngressPrivateContext {
                feishu_callback: Some(ConversationIngressFeishuCallbackContext {
                    callback_token: Some("callback-secret-1".to_owned()),
                    open_message_id: Some("om_callback".to_owned()),
                    open_chat_id: Some("oc_callback".to_owned()),
                    operator_open_id: Some("ou_operator".to_owned()),
                    deferred_context_id: Some("evt_callback_1".to_owned()),
                }),
            },
        }
    }

    #[test]
    fn feishu_callback_private_ingress_is_hidden_from_public_payload() {
        let ingress = feishu_ingress_with_private_callback();

        let public_payload = ingress.as_event_payload().to_string();
        let system_note = ingress.system_note();

        assert!(!public_payload.contains("callback-secret-1"));
        assert!(!system_note.contains("callback-secret-1"));
        assert!(!public_payload.contains("feishu_callback"));
        assert!(!system_note.contains("feishu_callback"));
    }

    #[test]
    fn feishu_callback_private_ingress_is_injected_only_for_feishu_tools() {
        let ingress = feishu_ingress_with_private_callback();
        let injected = inject_internal_tool_ingress(
            "feishu.messages.reply",
            json!({"text": "hello"}),
            Some(&ingress),
        );
        let untouched =
            inject_internal_tool_ingress("shell.exec", json!({"cmd": "pwd"}), Some(&ingress));

        assert!(injected.trusted_internal_context);
        assert_eq!(
            injected.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]["feishu_callback"]["callback_token"],
            "callback-secret-1"
        );
        assert_eq!(
            injected.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]["feishu_callback"]["operator_open_id"],
            "ou_operator"
        );
        assert_eq!(
            injected.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]["feishu_callback"]["deferred_context_id"],
            "evt_callback_1"
        );
        assert_eq!(
            injected.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]["ingress"]["channel"]["conversation_id"],
            "oc_callback"
        );
        assert!(!untouched.trusted_internal_context);
        assert!(
            untouched
                .payload
                .get(crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY)
                .is_none()
        );
    }

    #[test]
    fn caller_supplied_reserved_internal_tool_context_is_not_marked_trusted() {
        let injected = inject_internal_tool_ingress(
            "feishu.messages.reply",
            json!({
                "text": "hello",
                "_loong": {
                    "ingress": {
                        "channel": {
                            "platform": "feishu",
                            "conversation_id": "oc_forged"
                        }
                    }
                }
            }),
            None,
        );

        assert!(!injected.trusted_internal_context);
        assert_eq!(
            injected.payload["_loong"]["ingress"]["channel"]["conversation_id"],
            "oc_forged"
        );
    }
}
