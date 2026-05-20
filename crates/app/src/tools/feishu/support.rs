use super::*;
use serde::de::DeserializeOwned;
#[cfg(feature = "tool-file")]
use std::fs;
use std::future::Future;
use std::path::Path;

pub(super) fn parse_payload<T>(tool_name: &str, payload: serde_json::Value) -> Result<T, String>
where
    T: DeserializeOwned,
{
    serde_json::from_value(payload)
        .map_err(|error| format!("{tool_name} payload validation failed: {error}"))
}

pub(super) fn load_feishu_tool_context(
    config: &super::super::runtime_config::ToolRuntimeConfig,
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

pub(super) fn require_selected_grant(
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
                "no stored Feishu grant for account `{}`; run `{} feishu auth start --account {}` first",
                context.configured_account_id,
                crate::config::active_cli_command_name(),
                context.configured_account_id
            )
        }),
    )
}

pub(super) fn require_non_empty(tool_name: &str, field: &str, value: &str) -> CliResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{tool_name} requires payload.{field}"));
    }
    Ok(trimmed.to_owned())
}

pub(super) fn require_positive_i64(tool_name: &str, field: &str, value: i64) -> CliResult<i64> {
    if value > 0 {
        return Ok(value);
    }

    Err(format!(
        "{tool_name} invalid payload.{field}: expected positive integer, got {value}"
    ))
}

pub(super) fn resolve_feishu_doc_content_type(
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

pub(super) fn prepare_feishu_doc_tool_content(
    tool_name: &str,
    content: Option<&str>,
    content_path: Option<&str>,
    content_type: Option<&str>,
    required: bool,
    config: &super::super::runtime_config::ToolRuntimeConfig,
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

pub(super) fn infer_feishu_doc_content_type_from_path(path: &Path) -> &'static str {
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

pub(super) fn require_non_empty_with_fallback(
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

pub(super) fn normalize_open_ids<'a, I>(values: I) -> Vec<String>
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

pub(super) fn requested_account_id<'a>(
    explicit: Option<&'a str>,
    internal: &'a LoongInternalToolPayload,
) -> Option<&'a str> {
    explicit.or_else(|| internal.ingress_requested_account_id())
}

pub(super) fn resolve_message_resource_selection(
    tool_name: &str,
    effective_message_id: &str,
    payload_file_key: &str,
    payload_resource_type: &str,
    internal: &LoongInternalToolPayload,
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

pub(super) fn ingress_resources_for_effective_message(
    internal: &LoongInternalToolPayload,
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

pub(super) fn ingress_message_override_reason<'a>(
    internal: &'a LoongInternalToolPayload,
    effective_message_id: &str,
) -> Option<&'a str> {
    internal
        .ingress_message_id()
        .filter(|message_id| *message_id != effective_message_id)
}

pub(super) fn single_ingress_resource_for_selection(
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

pub(super) fn validate_explicit_ingress_resource_pair(
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

pub(super) fn infer_ingress_resource_from_file_key(
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

pub(super) fn infer_ingress_resource_from_type(
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

pub(super) fn normalize_ingress_message_resource_type(
    tool_name: &str,
    resource: &FeishuInternalIngressResolvedResource,
) -> CliResult<String> {
    normalize_message_resource_type_alias(resource.resource_type.as_str())
        .map_err(|error| format!("{tool_name} invalid ingress resource type: {error}"))
}

pub(super) fn normalize_message_resource_type_alias(value: &str) -> CliResult<String> {
    value
        .parse::<FeishuMessageResourceType>()
        .map(|resource_type| resource_type.as_api_value().to_owned())
}

pub(super) fn prepare_feishu_tool_media(
    tool_name: &str,
    image_key: Option<&str>,
    image_path: Option<&str>,
    file_key: Option<&str>,
    file_path: Option<&str>,
    file_type: Option<&str>,
    config: &super::super::runtime_config::ToolRuntimeConfig,
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

pub(super) fn validate_feishu_tool_message_body_fields(
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

pub(super) async fn resolve_prepared_feishu_tool_media(
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

pub(super) fn ensure_tool_media_source_exclusive(
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
pub(super) fn read_safe_tool_text_file(
    tool_name: &str,
    field: &str,
    raw_path: &str,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> CliResult<String> {
    let resolved = super::super::file::resolve_safe_file_path_with_config(raw_path, config)?;
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
pub(super) fn read_safe_tool_text_file(
    tool_name: &str,
    field: &str,
    raw_path: &str,
    _config: &super::super::runtime_config::ToolRuntimeConfig,
) -> CliResult<String> {
    let _ = raw_path;
    Err(format!(
        "{tool_name} does not support {field} unless feature `tool-file` is enabled"
    ))
}

#[cfg(feature = "tool-file")]
pub(super) fn read_safe_tool_media_file(
    tool_name: &str,
    field: &str,
    raw_path: &str,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> CliResult<PreparedFeishuToolUpload> {
    let resolved = super::super::file::resolve_safe_file_path_with_config(raw_path, config)?;
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
pub(super) fn read_safe_tool_media_file(
    tool_name: &str,
    field: &str,
    raw_path: &str,
    _config: &super::super::runtime_config::ToolRuntimeConfig,
) -> CliResult<PreparedFeishuToolUpload> {
    let _ = raw_path;
    Err(format!(
        "{tool_name} does not support {field} unless feature `tool-file` is enabled"
    ))
}

pub(super) fn search_chat_scope(payload: &FeishuMessagesSearchPayload) -> Vec<String> {
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

#[cfg(test)]
pub(super) fn push_feishu_registry_entry(
    entries: &mut Vec<super::super::ToolRegistryEntry>,
    name: &'static str,
    description: &'static str,
) {
    let entry = super::super::ToolRegistryEntry {
        name: name.to_owned(),
        description: description.to_owned(),
    };
    entries.push(entry);
}

pub(super) fn push_feishu_provider_tool_definition(
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

pub(super) fn feishu_provider_tool_function_name(tool: &Value) -> &str {
    tool.get("function")
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

pub(super) fn ensure_required_scopes(
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
        "{tool_name} requires Feishu scopes [{}] for `{}`; update Feishu config if needed and rerun `loong feishu auth start --account <account>`",
        missing.join(", "),
        grant.principal.storage_key()
    ))
}

pub(super) fn ensure_any_required_scope(
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
        "{tool_name} requires at least one Feishu scope [{}] for `{}`; update Feishu config if needed and rerun `loong feishu auth start --account <account>`",
        accepted.join(", "),
        grant.principal.storage_key()
    ))
}

pub(super) fn ok_outcome(
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

pub(super) fn ok_outcome_without_principal(
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

pub(super) fn run_feishu_future<F>(future: F) -> CliResult<ToolCoreOutcome>
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

pub(super) fn trimmed_opt(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}
