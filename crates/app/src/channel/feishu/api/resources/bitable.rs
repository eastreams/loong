use serde_json::{Value, json};

use crate::CliResult;

use super::super::client::FeishuClient;
use super::types::{FeishuBitableRecord, FeishuBitableRecordPage, FeishuBitableTableListPage};

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

fn create_record_query_pairs() -> Vec<(String, String)> {
    vec![("user_id_type".to_owned(), "open_id".to_owned())]
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
}
