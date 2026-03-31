use serde_json::{Value, json};

use crate::CliResult;

use super::super::client::FeishuClient;
use super::types::{
    FeishuBitableApp, FeishuBitableAppListPage, FeishuBitableDeletedField,
    FeishuBitableDeletedRecord, FeishuBitableField, FeishuBitableFieldListPage,
    FeishuBitableRecord, FeishuBitableRecordPage, FeishuBitableTableListPage, FeishuBitableView,
    FeishuBitableViewListPage,
};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BitableRecordSearchQuery {
    pub page_token: Option<String>,
    pub page_size: Option<usize>,
    pub view_id: Option<String>,
    pub filter: Option<Value>,
    pub sort: Option<Value>,
    pub field_names: Option<Vec<String>>,
}

impl BitableRecordSearchQuery {
    fn query_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = vec![("user_id_type".to_owned(), "open_id".to_owned())];
        if let Some(value) = self.page_token.as_ref() {
            pairs.push(("page_token".to_owned(), value.clone()));
        }
        if let Some(value) = self.page_size {
            pairs.push(("page_size".to_owned(), value.to_string()));
        }
        if let Some(value) = self.view_id.as_ref() {
            pairs.push(("view_id".to_owned(), value.clone()));
        }
        pairs
    }

    fn request_body(&self) -> Value {
        let mut body = serde_json::Map::new();
        if let Some(value) = self.filter.as_ref() {
            body.insert("filter".to_owned(), normalize_bitable_filter(value));
        }
        if let Some(value) = self.sort.as_ref() {
            body.insert("sort".to_owned(), value.clone());
        }
        if let Some(value) = self.field_names.as_ref() {
            body.insert("field_names".to_owned(), json!(value));
        }
        Value::Object(body)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BitableAppListQuery {
    pub folder_token: Option<String>,
    pub page_size: Option<usize>,
    pub page_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BitableFieldListQuery {
    pub view_id: Option<String>,
    pub page_size: Option<usize>,
    pub page_token: Option<String>,
}

impl BitableFieldListQuery {
    fn query_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        if let Some(value) = self.view_id.as_ref() {
            pairs.push(("view_id".to_owned(), value.clone()));
        }
        if let Some(value) = self.page_size {
            pairs.push(("page_size".to_owned(), value.to_string()));
        }
        if let Some(value) = self.page_token.as_ref() {
            pairs.push(("page_token".to_owned(), value.clone()));
        }
        pairs
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BitableViewListQuery {
    pub page_size: Option<usize>,
    pub page_token: Option<String>,
}

impl BitableViewListQuery {
    fn query_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        if let Some(value) = self.page_size {
            pairs.push(("page_size".to_owned(), value.to_string()));
        }
        if let Some(value) = self.page_token.as_ref() {
            pairs.push(("page_token".to_owned(), value.clone()));
        }
        pairs
    }
}

impl BitableAppListQuery {
    fn query_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        if let Some(value) = self.folder_token.as_ref() {
            pairs.push(("folder_token".to_owned(), value.clone()));
        }
        if let Some(value) = self.page_size {
            pairs.push(("page_size".to_owned(), value.to_string()));
        }
        if let Some(value) = self.page_token.as_ref() {
            pairs.push(("page_token".to_owned(), value.clone()));
        }
        pairs
    }
}

pub async fn create_bitable_app(
    client: &FeishuClient,
    access_token: &str,
    name: &str,
    folder_token: Option<&str>,
) -> CliResult<FeishuBitableApp> {
    let mut body = serde_json::Map::new();
    body.insert("name".to_owned(), json!(name));
    if let Some(value) = folder_token {
        body.insert("folder_token".to_owned(), json!(value));
    }
    let payload = client
        .post_json(
            "/open-apis/bitable/v1/apps",
            Some(access_token),
            &[],
            &Value::Object(body),
        )
        .await?;
    parse_bitable_app_response(&payload, "bitable app create")
}

pub async fn get_bitable_app(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
) -> CliResult<FeishuBitableApp> {
    let path = format!("/open-apis/bitable/v1/apps/{app_token}");
    let payload = client.get_json(&path, Some(access_token), &[]).await?;
    parse_bitable_app_response(&payload, "bitable app get")
}

pub async fn list_bitable_apps(
    client: &FeishuClient,
    access_token: &str,
    query: &BitableAppListQuery,
) -> CliResult<FeishuBitableAppListPage> {
    let payload = client
        .get_json(
            "/open-apis/drive/v1/files",
            Some(access_token),
            &query.query_pairs(),
        )
        .await?;
    parse_bitable_app_list_response(&payload)
}

pub async fn patch_bitable_app(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    name: Option<&str>,
    is_advanced: Option<bool>,
) -> CliResult<FeishuBitableApp> {
    let mut body = serde_json::Map::new();
    if let Some(value) = name {
        body.insert("name".to_owned(), json!(value));
    }
    if let Some(value) = is_advanced {
        body.insert("is_advanced".to_owned(), json!(value));
    }
    let path = format!("/open-apis/bitable/v1/apps/{app_token}");
    let payload = client
        .patch_json(&path, Some(access_token), &[], &Value::Object(body))
        .await?;
    parse_bitable_app_response(&payload, "bitable app patch")
}

pub async fn copy_bitable_app(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    name: &str,
    folder_token: Option<&str>,
) -> CliResult<FeishuBitableApp> {
    let mut body = serde_json::Map::new();
    body.insert("name".to_owned(), json!(name));
    if let Some(value) = folder_token {
        body.insert("folder_token".to_owned(), json!(value));
    }
    let path = format!("/open-apis/bitable/v1/apps/{app_token}/copy");
    let payload = client
        .post_json(&path, Some(access_token), &[], &Value::Object(body))
        .await?;
    parse_bitable_app_response(&payload, "bitable app copy")
}

pub async fn list_bitable_tables(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    page_token: Option<&str>,
    page_size: Option<usize>,
) -> CliResult<FeishuBitableTableListPage> {
    let mut query = Vec::new();
    if let Some(value) = page_token {
        query.push(("page_token".to_owned(), value.to_owned()));
    }
    if let Some(value) = page_size {
        query.push(("page_size".to_owned(), value.to_string()));
    }
    let path = format!("/open-apis/bitable/v1/apps/{app_token}/tables");
    let payload = client.get_json(&path, Some(access_token), &query).await?;
    parse_bitable_table_list_response(&payload)
}

pub async fn create_bitable_table(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    name: &str,
    default_view_name: Option<&str>,
    fields: Option<Vec<Value>>,
) -> CliResult<Value> {
    let mut table = serde_json::Map::new();
    table.insert("name".to_owned(), json!(name));
    if let Some(value) = default_view_name {
        table.insert("default_view_name".to_owned(), json!(value));
    }
    if let Some(value) = fields {
        table.insert(
            "fields".to_owned(),
            Value::Array(sanitize_table_fields(value)),
        );
    }
    let path = format!("/open-apis/bitable/v1/apps/{app_token}/tables");
    let payload = client
        .post_json(
            &path,
            Some(access_token),
            &[],
            &json!({ "table": Value::Object(table) }),
        )
        .await?;
    parse_bitable_data_response(&payload, "bitable table create")
}

pub async fn patch_bitable_table(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    name: &str,
) -> CliResult<Value> {
    let path = format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}");
    let payload = client
        .patch_json(&path, Some(access_token), &[], &json!({ "name": name }))
        .await?;
    parse_bitable_data_response(&payload, "bitable table patch")
}

pub async fn batch_create_bitable_tables(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    tables: Vec<Value>,
) -> CliResult<Value> {
    let tables = tables
        .into_iter()
        .map(|table| {
            let name = table.get("name").and_then(Value::as_str).ok_or_else(|| {
                "bitable table batch create: each table requires `name`".to_owned()
            })?;
            Ok(json!({ "name": name }))
        })
        .collect::<CliResult<Vec<_>>>()?;
    let path = format!("/open-apis/bitable/v1/apps/{app_token}/tables/batch_create");
    let payload = client
        .post_json(&path, Some(access_token), &[], &json!({ "tables": tables }))
        .await?;
    parse_bitable_data_response(&payload, "bitable table batch create")
}

pub async fn create_bitable_record(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    fields: Value,
) -> CliResult<FeishuBitableRecord> {
    let path = format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records");
    let payload = client
        .post_json(
            &path,
            Some(access_token),
            &create_record_query_pairs(),
            &json!({ "fields": fields }),
        )
        .await?;
    parse_bitable_record_response(&payload)
}

pub async fn update_bitable_record(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    record_id: &str,
    fields: Value,
) -> CliResult<FeishuBitableRecord> {
    let path =
        format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records/{record_id}");
    let payload = client
        .put_json(
            &path,
            Some(access_token),
            &create_record_query_pairs(),
            &json!({ "fields": fields }),
        )
        .await?;
    parse_bitable_record_response(&payload)
}

pub async fn delete_bitable_record(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    record_id: &str,
) -> CliResult<FeishuBitableDeletedRecord> {
    let path =
        format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records/{record_id}");
    let payload = client.delete_json(&path, Some(access_token), &[]).await?;
    parse_bitable_deleted_record_response(&payload)
}

pub async fn batch_create_bitable_records(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    records: Vec<Value>,
) -> CliResult<Value> {
    let path =
        format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records/batch_create");
    let payload = client
        .post_json(
            &path,
            Some(access_token),
            &create_record_query_pairs(),
            &json!({ "records": records }),
        )
        .await?;
    parse_bitable_data_response(&payload, "bitable record batch create")
}

pub async fn batch_update_bitable_records(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    records: Vec<Value>,
) -> CliResult<Value> {
    let path =
        format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records/batch_update");
    let payload = client
        .post_json(
            &path,
            Some(access_token),
            &create_record_query_pairs(),
            &json!({ "records": records }),
        )
        .await?;
    parse_bitable_data_response(&payload, "bitable record batch update")
}

pub async fn batch_delete_bitable_records(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    records: Vec<String>,
) -> CliResult<Value> {
    let path =
        format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records/batch_delete");
    let payload = client
        .post_json(
            &path,
            Some(access_token),
            &[],
            &json!({ "records": records }),
        )
        .await?;
    parse_bitable_data_response(&payload, "bitable record batch delete")
}

pub async fn create_bitable_field(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    field_name: &str,
    field_type: i64,
    property: Option<Value>,
) -> CliResult<FeishuBitableField> {
    let mut body = serde_json::Map::new();
    body.insert("field_name".to_owned(), json!(field_name));
    body.insert("type".to_owned(), json!(field_type));
    if let Some(property) = omit_field_property_for_unsupported_types(field_type, property) {
        body.insert("property".to_owned(), property);
    }
    let path = format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/fields");
    let payload = client
        .post_json(&path, Some(access_token), &[], &Value::Object(body))
        .await?;
    parse_bitable_field_response(&payload, "bitable field create")
}

pub async fn list_bitable_fields(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    query: &BitableFieldListQuery,
) -> CliResult<FeishuBitableFieldListPage> {
    let path = format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/fields");
    let payload = client
        .get_json(&path, Some(access_token), &query.query_pairs())
        .await?;
    parse_bitable_field_page_response(&payload)
}

pub async fn update_bitable_field(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    field_id: &str,
    field_name: &str,
    field_type: i64,
    property: Option<Value>,
) -> CliResult<FeishuBitableField> {
    let mut body = serde_json::Map::new();
    body.insert("field_name".to_owned(), json!(field_name));
    body.insert("type".to_owned(), json!(field_type));
    if let Some(property) = omit_field_property_for_unsupported_types(field_type, property) {
        body.insert("property".to_owned(), property);
    }
    let path =
        format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/fields/{field_id}");
    let payload = client
        .put_json(&path, Some(access_token), &[], &Value::Object(body))
        .await?;
    parse_bitable_field_response(&payload, "bitable field update")
}

pub async fn delete_bitable_field(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    field_id: &str,
) -> CliResult<FeishuBitableDeletedField> {
    let path =
        format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/fields/{field_id}");
    let payload = client.delete_json(&path, Some(access_token), &[]).await?;
    parse_bitable_deleted_field_response(&payload)
}

pub async fn create_bitable_view(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    view_name: &str,
    view_type: Option<&str>,
) -> CliResult<FeishuBitableView> {
    let mut body = serde_json::Map::new();
    body.insert("view_name".to_owned(), json!(view_name));
    body.insert("view_type".to_owned(), json!(view_type.unwrap_or("grid")));
    let path = format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/views");
    let payload = client
        .post_json(&path, Some(access_token), &[], &Value::Object(body))
        .await?;
    parse_bitable_view_response(&payload, "bitable view create")
}

pub async fn get_bitable_view(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    view_id: &str,
) -> CliResult<FeishuBitableView> {
    let path = format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/views/{view_id}");
    let payload = client.get_json(&path, Some(access_token), &[]).await?;
    parse_bitable_view_response(&payload, "bitable view get")
}

pub async fn list_bitable_views(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    query: &BitableViewListQuery,
) -> CliResult<FeishuBitableViewListPage> {
    let path = format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/views");
    let payload = client
        .get_json(&path, Some(access_token), &query.query_pairs())
        .await?;
    parse_bitable_view_page_response(&payload)
}

pub async fn patch_bitable_view(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    view_id: &str,
    view_name: &str,
) -> CliResult<FeishuBitableView> {
    let path = format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/views/{view_id}");
    let payload = client
        .patch_json(
            &path,
            Some(access_token),
            &[],
            &json!({ "view_name": view_name }),
        )
        .await?;
    parse_bitable_view_response(&payload, "bitable view patch")
}

pub async fn search_bitable_records(
    client: &FeishuClient,
    access_token: &str,
    app_token: &str,
    table_id: &str,
    query: &BitableRecordSearchQuery,
) -> CliResult<FeishuBitableRecordPage> {
    let path = format!("/open-apis/bitable/v1/apps/{app_token}/tables/{table_id}/records/search");
    let payload = client
        .post_json(
            &path,
            Some(access_token),
            &query.query_pairs(),
            &query.request_body(),
        )
        .await?;
    parse_bitable_record_page_response(&payload)
}

pub fn parse_bitable_table_list_response(payload: &Value) -> CliResult<FeishuBitableTableListPage> {
    let data = payload
        .get("data")
        .ok_or_else(|| "bitable table list: missing `data` in response".to_owned())?;
    serde_json::from_value(data.clone())
        .map_err(|error| format!("bitable table list: failed to parse response: {error}"))
}

pub fn parse_bitable_record_response(payload: &Value) -> CliResult<FeishuBitableRecord> {
    let data = payload
        .get("data")
        .ok_or_else(|| "bitable record create: missing `data` in response".to_owned())?;
    let record = data
        .get("record")
        .ok_or_else(|| "bitable record create: missing `data.record` in response".to_owned())?;
    serde_json::from_value(record.clone())
        .map_err(|error| format!("bitable record create: failed to parse record: {error}"))
}

pub fn parse_bitable_record_page_response(payload: &Value) -> CliResult<FeishuBitableRecordPage> {
    let data = payload
        .get("data")
        .ok_or_else(|| "bitable record search: missing `data` in response".to_owned())?;
    serde_json::from_value(data.clone())
        .map_err(|error| format!("bitable record search: failed to parse response: {error}"))
}

pub fn parse_bitable_app_response(payload: &Value, action: &str) -> CliResult<FeishuBitableApp> {
    let data = payload
        .get("data")
        .ok_or_else(|| format!("{action}: missing `data` in response"))?;
    let app = data
        .get("app")
        .ok_or_else(|| format!("{action}: missing `data.app` in response"))?;
    serde_json::from_value(app.clone())
        .map_err(|error| format!("{action}: failed to parse app: {error}"))
}

pub fn parse_bitable_app_list_response(payload: &Value) -> CliResult<FeishuBitableAppListPage> {
    let data = payload
        .get("data")
        .ok_or_else(|| "bitable app list: missing `data` in response".to_owned())?;
    let files = data
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| "bitable app list: missing `data.files` in response".to_owned())?;
    let apps = files
        .iter()
        .filter(|file| file.get("type").and_then(Value::as_str) == Some("bitable"))
        .cloned()
        .collect::<Vec<_>>();

    Ok(FeishuBitableAppListPage {
        apps,
        has_more: data.get("has_more").and_then(Value::as_bool),
        page_token: data
            .get("page_token")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

pub fn parse_bitable_deleted_record_response(
    payload: &Value,
) -> CliResult<FeishuBitableDeletedRecord> {
    let data = payload
        .get("data")
        .ok_or_else(|| "bitable record delete: missing `data` in response".to_owned())?;
    serde_json::from_value(data.clone())
        .map_err(|error| format!("bitable record delete: failed to parse response: {error}"))
}

pub fn parse_bitable_field_response(
    payload: &Value,
    action: &str,
) -> CliResult<FeishuBitableField> {
    let data = payload
        .get("data")
        .ok_or_else(|| format!("{action}: missing `data` in response"))?;
    let field = data
        .get("field")
        .ok_or_else(|| format!("{action}: missing `data.field` in response"))?;
    serde_json::from_value(field.clone())
        .map_err(|error| format!("{action}: failed to parse field: {error}"))
}

pub fn parse_bitable_field_page_response(payload: &Value) -> CliResult<FeishuBitableFieldListPage> {
    let data = payload
        .get("data")
        .ok_or_else(|| "bitable field list: missing `data` in response".to_owned())?;
    serde_json::from_value(data.clone())
        .map_err(|error| format!("bitable field list: failed to parse response: {error}"))
}

pub fn parse_bitable_deleted_field_response(
    payload: &Value,
) -> CliResult<FeishuBitableDeletedField> {
    let data = payload
        .get("data")
        .ok_or_else(|| "bitable field delete: missing `data` in response".to_owned())?;
    serde_json::from_value(data.clone())
        .map_err(|error| format!("bitable field delete: failed to parse response: {error}"))
}

pub fn parse_bitable_view_response(payload: &Value, action: &str) -> CliResult<FeishuBitableView> {
    let data = payload
        .get("data")
        .ok_or_else(|| format!("{action}: missing `data` in response"))?;
    let view = data
        .get("view")
        .ok_or_else(|| format!("{action}: missing `data.view` in response"))?;
    serde_json::from_value(view.clone())
        .map_err(|error| format!("{action}: failed to parse view: {error}"))
}

pub fn parse_bitable_view_page_response(payload: &Value) -> CliResult<FeishuBitableViewListPage> {
    let data = payload
        .get("data")
        .ok_or_else(|| "bitable view list: missing `data` in response".to_owned())?;
    serde_json::from_value(data.clone())
        .map_err(|error| format!("bitable view list: failed to parse response: {error}"))
}

pub fn parse_bitable_data_response(payload: &Value, action: &str) -> CliResult<Value> {
    payload
        .get("data")
        .cloned()
        .ok_or_else(|| format!("{action}: missing `data` in response"))
}

fn create_record_query_pairs() -> Vec<(String, String)> {
    vec![("user_id_type".to_owned(), "open_id".to_owned())]
}

#[allow(dead_code)]
fn omit_field_property_for_unsupported_types(
    field_type: i64,
    property: Option<Value>,
) -> Option<Value> {
    if matches!(field_type, 7 | 15) {
        None
    } else {
        property
    }
}

fn sanitize_table_fields(fields: Vec<Value>) -> Vec<Value> {
    fields
        .into_iter()
        .map(|field| {
            let Some(mut field_object) = field.as_object().cloned() else {
                return field;
            };
            let field_type = field_object.get("type").and_then(Value::as_i64);
            let property = field_object.remove("property");
            if let Some(field_type) = field_type {
                if let Some(property) =
                    omit_field_property_for_unsupported_types(field_type, property)
                {
                    field_object.insert("property".to_owned(), property);
                }
            } else if let Some(property) = property {
                field_object.insert("property".to_owned(), property);
            }
            Value::Object(field_object)
        })
        .collect()
}

fn normalize_bitable_filter(value: &Value) -> Value {
    let Some(filter) = value.as_object() else {
        return value.clone();
    };
    let mut normalized = filter.clone();
    if let Some(conditions) = normalized
        .get_mut("conditions")
        .and_then(Value::as_array_mut)
    {
        for condition in conditions.iter_mut() {
            let Some(condition_object) = condition.as_object_mut() else {
                continue;
            };
            let operator = condition_object
                .get("operator")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if matches!(operator, "isEmpty" | "isNotEmpty")
                && !condition_object.contains_key("value")
            {
                condition_object.insert("value".to_owned(), json!([]));
            }
        }
    }
    Value::Object(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_bitable_table_list_response_extracts_items() {
        let payload = json!({
            "code": 0,
            "data": {
                "items": [{"table_id": "tblXXX", "name": "Sheet1", "revision": 1}],
                "has_more": false,
                "total": 1
            }
        });

        let result = parse_bitable_table_list_response(&payload).expect("table list should parse");
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].table_id.as_deref(), Some("tblXXX"));
        assert_eq!(result.items[0].name.as_deref(), Some("Sheet1"));
    }

    #[test]
    fn parse_bitable_record_response_extracts_record() {
        let payload = json!({
            "code": 0,
            "data": {
                "record": {
                    "record_id": "recABC",
                    "fields": {"Name": "test value"}
                }
            }
        });

        let result = parse_bitable_record_response(&payload).expect("record should parse");
        assert_eq!(result.record_id.as_deref(), Some("recABC"));
    }

    #[test]
    fn parse_bitable_record_page_response_extracts_items() {
        let payload = json!({
            "code": 0,
            "data": {
                "items": [{"record_id": "recABC", "fields": {}}],
                "has_more": false,
                "total": 1
            }
        });

        let result =
            parse_bitable_record_page_response(&payload).expect("record page should parse");
        assert_eq!(result.items.len(), 1);
    }

    #[test]
    fn create_record_query_pairs_default_to_open_id_user_scope() {
        assert_eq!(
            create_record_query_pairs(),
            vec![("user_id_type".to_owned(), "open_id".to_owned())]
        );
    }

    #[test]
    fn search_query_pairs_include_open_id_and_optional_paging_and_view() {
        let query = BitableRecordSearchQuery {
            page_token: Some("page_123".to_owned()),
            page_size: Some(50),
            view_id: Some("vew_123".to_owned()),
            filter: None,
            sort: None,
            field_names: None,
        };

        assert_eq!(
            query.query_pairs(),
            vec![
                ("user_id_type".to_owned(), "open_id".to_owned()),
                ("page_token".to_owned(), "page_123".to_owned()),
                ("page_size".to_owned(), "50".to_owned()),
                ("view_id".to_owned(), "vew_123".to_owned()),
            ]
        );
    }

    #[test]
    fn search_request_body_adds_empty_value_for_is_empty_operators() {
        let query = BitableRecordSearchQuery {
            filter: Some(json!({
                "conjunction": "and",
                "conditions": [
                    {
                        "field_name": "Name",
                        "operator": "isEmpty"
                    }
                ]
            })),
            ..BitableRecordSearchQuery::default()
        };

        let body = query.request_body();
        assert_eq!(body["filter"]["conditions"][0]["value"], json!([]));
    }

    #[test]
    fn omit_field_property_for_unsupported_types_drops_checkbox_and_url_property() {
        assert_eq!(
            omit_field_property_for_unsupported_types(7, Some(json!({"color": "green"}))),
            None
        );
        assert_eq!(
            omit_field_property_for_unsupported_types(
                15,
                Some(json!({"link": "https://example.com"}))
            ),
            None
        );
        assert_eq!(
            omit_field_property_for_unsupported_types(3, Some(json!({"options": []}))),
            Some(json!({"options": []}))
        );
    }

    #[test]
    fn parse_bitable_delete_record_response_extracts_deleted_status() {
        let payload = json!({
            "code": 0,
            "data": {
                "deleted": true,
                "record_id": "rec_123"
            }
        });

        let result =
            parse_bitable_deleted_record_response(&payload).expect("delete response should parse");
        assert!(result.deleted);
        assert_eq!(result.record_id, "rec_123");
    }
}
