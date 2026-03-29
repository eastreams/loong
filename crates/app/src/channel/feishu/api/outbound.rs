use std::path::Path;

use serde_json::Value;

use crate::CliResult;

use super::client::FeishuClient;
use super::resources::media::{
    FEISHU_DEFAULT_MESSAGE_FILE_TYPE, upload_message_file, upload_message_image,
};
use super::resources::messages::{FeishuOutboundMessageBody, resolve_outbound_message_body};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeishuOperatorOutboundMessageInput {
    pub text: Option<String>,
    pub card: bool,
    pub post_json: Option<String>,
    pub image_key: Option<String>,
    pub image_path: Option<String>,
    pub file_key: Option<String>,
    pub file_path: Option<String>,
    pub file_type: Option<String>,
}

pub fn parse_post_json_argument(action: &str, value: Option<&str>) -> CliResult<Option<Value>> {
    let Some(value) = trimmed_opt(value) else {
        return Ok(None);
    };
    serde_json::from_str::<Value>(value)
        .map(Some)
        .map_err(|error| format!("{action} requires --post-json to be valid JSON: {error}"))
}

pub fn validate_operator_outbound_message_input(
    action: &str,
    input: &FeishuOperatorOutboundMessageInput,
) -> CliResult<()> {
    ensure_media_source_exclusive(
        action,
        "--image-key",
        input.image_key.as_deref(),
        "--image-path",
        input.image_path.as_deref(),
    )?;
    ensure_media_source_exclusive(
        action,
        "--file-key",
        input.file_key.as_deref(),
        "--file-path",
        input.file_path.as_deref(),
    )?;
    if trimmed_opt(input.file_type.as_deref()).is_some()
        && trimmed_opt(input.file_path.as_deref()).is_none()
    {
        return Err(format!("{action} only allows --file-type with --file-path"));
    }

    let post = parse_post_json_argument(action, input.post_json.as_deref())?;
    resolve_outbound_message_body(
        action,
        "--text",
        "--card",
        "--post-json",
        "--image-key/--image-path",
        "--file-key/--file-path",
        input.text.as_deref(),
        input.card,
        post.as_ref(),
        trimmed_opt(input.image_key.as_deref())
            .or_else(|| trimmed_opt(input.image_path.as_deref()).map(|_| "__image_path__")),
        trimmed_opt(input.file_key.as_deref())
            .or_else(|| trimmed_opt(input.file_path.as_deref()).map(|_| "__file_path__")),
    )
    .map(|_| ())
}

pub async fn resolve_operator_outbound_message_body(
    action: &str,
    client: &FeishuClient,
    tenant_access_token: &str,
    input: &FeishuOperatorOutboundMessageInput,
) -> CliResult<FeishuOutboundMessageBody> {
    validate_operator_outbound_message_input(action, input)?;
    let post = parse_post_json_argument(action, input.post_json.as_deref())?;
    let media = resolve_operator_media_keys(
        action,
        client,
        tenant_access_token,
        input.image_key.as_deref(),
        input.image_path.as_deref(),
        input.file_key.as_deref(),
        input.file_path.as_deref(),
        input.file_type.as_deref(),
    )
    .await?;
    resolve_outbound_message_body(
        action,
        "--text",
        "--card",
        "--post-json",
        "--image-key",
        "--file-key",
        input.text.as_deref(),
        input.card,
        post.as_ref(),
        media.image_key.as_deref(),
        media.file_key.as_deref(),
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResolvedOperatorMediaKeys {
    image_key: Option<String>,
    file_key: Option<String>,
}

async fn resolve_operator_media_keys(
    action: &str,
    client: &FeishuClient,
    tenant_access_token: &str,
    image_key: Option<&str>,
    image_path: Option<&str>,
    file_key: Option<&str>,
    file_path: Option<&str>,
    file_type: Option<&str>,
) -> CliResult<ResolvedOperatorMediaKeys> {
    let image_key = match trimmed_opt(image_key) {
        Some(image_key) => Some(image_key.to_owned()),
        None => match trimmed_opt(image_path) {
            Some(image_path) => {
                let (file_name, bytes) = read_local_media_file(action, "--image-path", image_path)?;
                Some(
                    upload_message_image(client, tenant_access_token, file_name.as_str(), bytes)
                        .await?
                        .image_key,
                )
            }
            None => None,
        },
    };

    let file_key = match trimmed_opt(file_key) {
        Some(file_key) => Some(file_key.to_owned()),
        None => match trimmed_opt(file_path) {
            Some(file_path) => {
                let (file_name, bytes) = read_local_media_file(action, "--file-path", file_path)?;
                let file_type = trimmed_opt(file_type).unwrap_or(FEISHU_DEFAULT_MESSAGE_FILE_TYPE);
                Some(
                    upload_message_file(
                        client,
                        tenant_access_token,
                        file_name.as_str(),
                        bytes,
                        file_type,
                        None,
                    )
                    .await?
                    .file_key,
                )
            }
            None => None,
        },
    };

    Ok(ResolvedOperatorMediaKeys {
        image_key,
        file_key,
    })
}

fn ensure_media_source_exclusive(
    action: &str,
    key_field: &str,
    key: Option<&str>,
    path_field: &str,
    path: Option<&str>,
) -> CliResult<()> {
    if trimmed_opt(key).is_some() && trimmed_opt(path).is_some() {
        return Err(format!(
            "{action} accepts either {key_field} or {path_field}, not both"
        ));
    }
    Ok(())
}

fn read_local_media_file(action: &str, field: &str, path: &str) -> CliResult<(String, Vec<u8>)> {
    let path = Path::new(path.trim());
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{action} requires {field} to include a file name"))?;
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "{action} failed to read {} `{}`: {error}",
            field,
            path.display()
        )
    })?;
    if bytes.is_empty() {
        return Err(format!(
            "{action} requires {} `{}` to be non-empty",
            field,
            path.display()
        ));
    }
    Ok((file_name, bytes))
}

fn trimmed_opt(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_operator_outbound_message_input_rejects_mixed_image_key_and_path() {
        let error = validate_operator_outbound_message_input(
            "loongclaw feishu send",
            &FeishuOperatorOutboundMessageInput {
                image_key: Some("img_v2_demo".to_owned()),
                image_path: Some("/tmp/demo.png".to_owned()),
                ..FeishuOperatorOutboundMessageInput::default()
            },
        )
        .expect_err("mixed image key and path should fail");

        assert_eq!(
            error,
            "loongclaw feishu send accepts either --image-key or --image-path, not both"
        );
    }

    #[test]
    fn validate_operator_outbound_message_input_rejects_file_type_without_path() {
        let error = validate_operator_outbound_message_input(
            "loongclaw feishu send",
            &FeishuOperatorOutboundMessageInput {
                file_key: Some("file_v2_demo".to_owned()),
                file_type: Some("stream".to_owned()),
                ..FeishuOperatorOutboundMessageInput::default()
            },
        )
        .expect_err("file type without file path should fail");

        assert_eq!(
            error,
            "loongclaw feishu send only allows --file-type with --file-path"
        );
    }
}
