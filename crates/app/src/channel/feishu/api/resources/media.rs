use reqwest::multipart::{Form, Part};
use serde_json::Value;

use crate::CliResult;

use super::super::client::FeishuClient;
use super::types::{
    FeishuDownloadedMessageResource, FeishuMessageResourceType, FeishuUploadedFile,
    FeishuUploadedImage,
};

pub const FEISHU_DEFAULT_MESSAGE_FILE_TYPE: &str = "stream";

pub async fn upload_message_image(
    client: &FeishuClient,
    tenant_access_token: &str,
    file_name: &str,
    bytes: Vec<u8>,
) -> CliResult<FeishuUploadedImage> {
    let file_name = require_non_empty("feishu image upload", "file_name", file_name)?;
    if bytes.is_empty() {
        return Err("feishu image upload requires non-empty file bytes".to_owned());
    }

    let payload = client
        .post_multipart(
            "/open-apis/im/v1/images",
            Some(tenant_access_token),
            &[],
            move || {
                Form::new().text("image_type", "message").part(
                    "image",
                    Part::bytes(bytes.clone()).file_name(file_name.clone()),
                )
            },
        )
        .await?;
    parse_uploaded_image_response(&payload)
}

pub async fn upload_message_file(
    client: &FeishuClient,
    tenant_access_token: &str,
    file_name: &str,
    bytes: Vec<u8>,
    file_type: &str,
    duration_ms: Option<u64>,
) -> CliResult<FeishuUploadedFile> {
    let file_name = require_non_empty("feishu file upload", "file_name", file_name)?;
    let file_type = require_non_empty("feishu file upload", "file_type", file_type)?;
    if bytes.is_empty() {
        return Err("feishu file upload requires non-empty file bytes".to_owned());
    }

    let payload = client
        .post_multipart(
            "/open-apis/im/v1/files",
            Some(tenant_access_token),
            &[],
            move || {
                let mut form = Form::new()
                    .text("file_type", file_type.clone())
                    .text("file_name", file_name.clone())
                    .part(
                        "file",
                        Part::bytes(bytes.clone()).file_name(file_name.clone()),
                    );
                if let Some(duration_ms) = duration_ms.filter(|value| *value > 0) {
                    form = form.text("duration", duration_ms.to_string());
                }
                form
            },
        )
        .await?;
    parse_uploaded_file_response(&payload)
}

pub async fn download_message_resource(
    client: &FeishuClient,
    tenant_access_token: &str,
    message_id: &str,
    file_key: &str,
    resource_type: FeishuMessageResourceType,
) -> CliResult<FeishuDownloadedMessageResource> {
    let message_id =
        require_non_empty("feishu message resource download", "message_id", message_id)?;
    let file_key = require_non_empty("feishu message resource download", "file_key", file_key)?;
    let payload = client
        .get_binary(
            format!("/open-apis/im/v1/messages/{message_id}/resources/{file_key}").as_str(),
            Some(tenant_access_token),
            &[("type".to_owned(), resource_type.as_api_value().to_owned())],
        )
        .await?;
    Ok(FeishuDownloadedMessageResource {
        message_id,
        file_key,
        resource_type,
        content_type: trimmed_header_value(payload.content_type.as_deref()),
        file_name: parse_content_disposition_filename(payload.content_disposition.as_deref()),
        bytes: payload.bytes,
    })
}

pub fn parse_uploaded_image_response(payload: &Value) -> CliResult<FeishuUploadedImage> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu image upload payload missing data object".to_owned())?;
    let image_key = data
        .get("image_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| "feishu image upload payload missing data.image_key".to_owned())?;
    Ok(FeishuUploadedImage { image_key })
}

pub fn parse_uploaded_file_response(payload: &Value) -> CliResult<FeishuUploadedFile> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu file upload payload missing data object".to_owned())?;
    let file_key = data
        .get("file_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| "feishu file upload payload missing data.file_key".to_owned())?;
    Ok(FeishuUploadedFile { file_key })
}

fn require_non_empty(action: &str, field: &str, value: &str) -> CliResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{action} requires {field}"));
    }
    Ok(trimmed.to_owned())
}

fn trimmed_header_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_content_disposition_filename(value: Option<&str>) -> Option<String> {
    let value = value.map(str::trim).filter(|value| !value.is_empty())?;
    let mut fallback = None;
    for part in value.split(';').map(str::trim) {
        let Some((name, raw_value)) = part.split_once('=') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let raw_value = raw_value.trim().trim_matches('"');
        if raw_value.is_empty() {
            continue;
        }
        if name == "filename*" {
            if let Some(decoded) = decode_rfc5987_filename(raw_value) {
                return Some(decoded);
            }
            continue;
        }
        if name == "filename" && fallback.is_none() {
            fallback = Some(raw_value.to_owned());
        }
    }
    fallback
}

fn decode_rfc5987_filename(value: &str) -> Option<String> {
    let (_, encoded) = value.split_once("''").unwrap_or(("", value));
    let mut bytes = Vec::with_capacity(encoded.len());
    let raw = encoded.as_bytes();
    let mut index = 0;
    while let Some(&byte) = raw.get(index) {
        if byte == b'%'
            && let (Some(&high), Some(&low)) = (raw.get(index + 1), raw.get(index + 2))
            && let (Some(high), Some(low)) = (decode_hex(high), decode_hex(low))
        {
            bytes.push(high * 16 + low);
            index += 3;
            continue;
        }
        bytes.push(byte);
        index += 1;
    }
    String::from_utf8(bytes)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn decode_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use crate::channel::feishu::api::client::FeishuClient;
    use axum::{
        Json, Router,
        body::Body,
        http::{Response, StatusCode},
        routing::{get, post},
    };
    use serde_json::json;

    async fn spawn_mock_feishu_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock feishu listener");
        let addr = listener.local_addr().expect("mock feishu listener addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve mock feishu api");
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn parse_uploaded_image_response_reads_image_key() {
        let payload = json!({
            "code": 0,
            "data": {
                "image_key": "img_v2_demo"
            }
        });

        let image = parse_uploaded_image_response(&payload).expect("parse uploaded image");
        assert_eq!(image.image_key, "img_v2_demo");
    }

    #[test]
    fn parse_uploaded_file_response_reads_file_key() {
        let payload = json!({
            "code": 0,
            "data": {
                "file_key": "file_v2_demo"
            }
        });

        let file = parse_uploaded_file_response(&payload).expect("parse uploaded file");
        assert_eq!(file.file_key, "file_v2_demo");
    }

    #[test]
    fn parse_content_disposition_filename_prefers_extended_filename() {
        let file_name = parse_content_disposition_filename(Some(
            "attachment; filename=\"fallback.txt\"; filename*=UTF-8''design%20doc.pdf",
        ));

        assert_eq!(file_name.as_deref(), Some("design doc.pdf"));
    }

    #[test]
    fn parse_content_disposition_filename_uses_basic_filename_when_needed() {
        let file_name =
            parse_content_disposition_filename(Some("attachment; filename=\"spec-sheet.pdf\""));

        assert_eq!(file_name.as_deref(), Some("spec-sheet.pdf"));
    }

    #[test]
    fn download_message_resource_maps_binary_response_metadata() {
        let response = crate::channel::feishu::api::client::FeishuBinaryResponse {
            bytes: b"demo".to_vec(),
            content_type: Some("application/pdf".to_owned()),
            content_disposition: Some("attachment; filename=\"spec-sheet.pdf\"".to_owned()),
        };

        let mapped = FeishuDownloadedMessageResource {
            message_id: "om_123".to_owned(),
            file_key: "file_456".to_owned(),
            resource_type: FeishuMessageResourceType::File,
            content_type: trimmed_header_value(response.content_type.as_deref()),
            file_name: parse_content_disposition_filename(response.content_disposition.as_deref()),
            bytes: response.bytes,
        };

        assert_eq!(mapped.content_type.as_deref(), Some("application/pdf"));
        assert_eq!(mapped.file_name.as_deref(), Some("spec-sheet.pdf"));
        assert_eq!(mapped.bytes, b"demo".to_vec());
    }

    #[tokio::test]
    async fn upload_message_image_retries_retryable_payload_error_and_returns_image_key() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let router = Router::new().route(
            "/open-apis/im/v1/images",
            post({
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                        if attempt == 0 {
                            return Json(json!({
                                "code": 2200,
                                "msg": "internal error"
                            }));
                        }
                        Json(json!({
                            "code": 0,
                            "msg": "ok",
                            "data": {
                                "image_key": "img_v2_retry"
                            }
                        }))
                    }
                }
            }),
        );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let client = FeishuClient::new(base_url, "cli_xxx", "secret_xxx", 20).expect("client");

        let image = upload_message_image(
            &client,
            "tenant-token",
            "diagram.png",
            b"png-binary".to_vec(),
        )
        .await
        .expect("upload image should retry and succeed");

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(image.image_key, "img_v2_retry");

        server.abort();
    }

    #[tokio::test]
    async fn upload_message_file_retries_retryable_payload_error_and_returns_file_key() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let router = Router::new().route(
            "/open-apis/im/v1/files",
            post({
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                        if attempt == 0 {
                            return Json(json!({
                                "code": 2200,
                                "msg": "internal error"
                            }));
                        }
                        Json(json!({
                            "code": 0,
                            "msg": "ok",
                            "data": {
                                "file_key": "file_v2_retry"
                            }
                        }))
                    }
                }
            }),
        );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let client = FeishuClient::new(base_url, "cli_xxx", "secret_xxx", 20).expect("client");

        let file = upload_message_file(
            &client,
            "tenant-token",
            "report.pdf",
            b"pdf-binary".to_vec(),
            FEISHU_DEFAULT_MESSAGE_FILE_TYPE,
            Some(1_000),
        )
        .await
        .expect("upload file should retry and succeed");

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(file.file_key, "file_v2_retry");

        server.abort();
    }

    #[tokio::test]
    async fn download_message_resource_retries_rate_limit_and_preserves_metadata() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let router = Router::new().route(
            "/open-apis/im/v1/messages/om_123/resources/file_456",
            get({
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                        if attempt == 0 {
                            return Response::builder()
                                .status(StatusCode::TOO_MANY_REQUESTS)
                                .header("content-type", "application/json")
                                .header("retry-after", "0")
                                .body(Body::from(
                                    json!({
                                        "code": 99991400,
                                        "msg": "request trigger frequency limit"
                                    })
                                    .to_string(),
                                ))
                                .expect("build retry response");
                        }

                        Response::builder()
                            .status(StatusCode::OK)
                            .header("content-type", "application/pdf")
                            .header(
                                "content-disposition",
                                "attachment; filename=\"spec-sheet.pdf\"",
                            )
                            .body(Body::from(b"demo-pdf".to_vec()))
                            .expect("build binary response")
                    }
                }
            }),
        );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let client = FeishuClient::new(base_url, "cli_xxx", "secret_xxx", 20).expect("client");

        let resource = download_message_resource(
            &client,
            "tenant-token",
            "om_123",
            "file_456",
            FeishuMessageResourceType::File,
        )
        .await
        .expect("download resource should retry and succeed");

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(resource.message_id, "om_123");
        assert_eq!(resource.file_key, "file_456");
        assert_eq!(resource.file_name.as_deref(), Some("spec-sheet.pdf"));
        assert_eq!(resource.content_type.as_deref(), Some("application/pdf"));
        assert_eq!(resource.bytes, b"demo-pdf".to_vec());

        server.abort();
    }
}
