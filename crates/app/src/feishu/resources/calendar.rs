use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::CliResult;

use super::types::{
    FeishuCalendarEntry, FeishuCalendarFreebusyResult, FeishuCalendarFreebusySlot,
    FeishuCalendarListPage, FeishuPrimaryCalendarEntry, FeishuPrimaryCalendarList,
};
use crate::feishu::client::FeishuClient;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FeishuCalendarListQuery {
    pub page_size: Option<usize>,
    pub page_token: Option<String>,
    pub sync_token: Option<String>,
}

impl FeishuCalendarListQuery {
    fn query_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        if let Some(page_size) = self.page_size {
            pairs.push(("page_size".to_owned(), page_size.to_string()));
        }
        push_optional_query(&mut pairs, "page_token", self.page_token.as_deref());
        push_optional_query(&mut pairs, "sync_token", self.sync_token.as_deref());
        pairs
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuCalendarFreebusyQuery {
    pub user_id_type: Option<String>,
    pub time_min: String,
    pub time_max: String,
    pub user_id: Option<String>,
    pub room_id: Option<String>,
    pub include_external_calendar: Option<bool>,
    pub only_busy: Option<bool>,
    pub need_rsvp_status: Option<bool>,
}

impl FeishuCalendarFreebusyQuery {
    pub fn validate(&self) -> CliResult<()> {
        if self.time_min.trim().is_empty() {
            return Err("feishu calendar freebusy requires time_min".to_owned());
        }
        if self.time_max.trim().is_empty() {
            return Err("feishu calendar freebusy requires time_max".to_owned());
        }
        if self
            .user_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
            && self
                .room_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
        {
            return Err("feishu calendar freebusy requires user_id or room_id".to_owned());
        }
        Ok(())
    }

    fn query_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        push_optional_query(&mut pairs, "user_id_type", self.user_id_type.as_deref());
        pairs
    }

    fn request_body(&self) -> Value {
        let mut body = serde_json::Map::new();
        body.insert(
            "time_min".to_owned(),
            Value::String(self.time_min.trim().to_owned()),
        );
        body.insert(
            "time_max".to_owned(),
            Value::String(self.time_max.trim().to_owned()),
        );
        insert_optional_string(&mut body, "user_id", self.user_id.as_deref());
        insert_optional_string(&mut body, "room_id", self.room_id.as_deref());
        insert_optional_bool(
            &mut body,
            "include_external_calendar",
            self.include_external_calendar,
        );
        insert_optional_bool(&mut body, "only_busy", self.only_busy);
        insert_optional_bool(&mut body, "need_rsvp_status", self.need_rsvp_status);
        Value::Object(body)
    }
}

pub async fn list_calendars(
    client: &FeishuClient,
    access_token: &str,
    query: &FeishuCalendarListQuery,
) -> CliResult<FeishuCalendarListPage> {
    let payload = client
        .get_json(
            "/open-apis/calendar/v4/calendars",
            Some(access_token),
            &query.query_pairs(),
        )
        .await?;
    parse_calendar_list_response(&payload)
}

pub async fn get_primary_calendars(
    client: &FeishuClient,
    access_token: &str,
    user_id_type: Option<&str>,
) -> CliResult<FeishuPrimaryCalendarList> {
    let mut query = Vec::new();
    push_optional_query(&mut query, "user_id_type", user_id_type);
    let payload = client
        .post_json(
            "/open-apis/calendar/v4/calendars/primary",
            Some(access_token),
            &query,
            &Value::Object(serde_json::Map::new()),
        )
        .await?;
    parse_primary_calendar_response(&payload)
}

pub async fn get_freebusy(
    client: &FeishuClient,
    access_token: &str,
    query: &FeishuCalendarFreebusyQuery,
) -> CliResult<FeishuCalendarFreebusyResult> {
    query.validate()?;
    let payload = client
        .post_json(
            "/open-apis/calendar/v4/freebusy/list",
            Some(access_token),
            &query.query_pairs(),
            &query.request_body(),
        )
        .await?;
    parse_freebusy_response(&payload)
}

pub fn parse_calendar_list_response(payload: &Value) -> CliResult<FeishuCalendarListPage> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu calendar list payload missing data object".to_owned())?;
    let calendar_list = data
        .get("calendar_list")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(parse_calendar_entry)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(FeishuCalendarListPage {
        has_more: data
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        page_token: opt_string(data.get("page_token")),
        sync_token: opt_string(data.get("sync_token")),
        calendar_list,
    })
}

pub fn parse_primary_calendar_response(payload: &Value) -> CliResult<FeishuPrimaryCalendarList> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu primary calendar payload missing data object".to_owned())?;
    let calendars = data
        .get("calendars")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let object = entry.as_object()?;
                    let calendar = parse_calendar_entry(object.get("calendar")?)?;
                    Some(FeishuPrimaryCalendarEntry {
                        calendar,
                        user_id: opt_string(object.get("user_id")),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(FeishuPrimaryCalendarList { calendars })
}

pub fn parse_freebusy_response(payload: &Value) -> CliResult<FeishuCalendarFreebusyResult> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu freebusy payload missing data object".to_owned())?;
    let freebusy_list = data
        .get("freebusy_list")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let object = entry.as_object()?;
                    Some(FeishuCalendarFreebusySlot {
                        start_time: opt_string(object.get("start_time"))?,
                        end_time: opt_string(object.get("end_time"))?,
                        rsvp_status: opt_string(object.get("rsvp_status")),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(FeishuCalendarFreebusyResult { freebusy_list })
}

fn parse_calendar_entry(value: &Value) -> Option<FeishuCalendarEntry> {
    let object = value.as_object()?;
    Some(FeishuCalendarEntry {
        calendar_id: opt_string(object.get("calendar_id"))?,
        summary: opt_string(object.get("summary")),
        description: opt_string(object.get("description")),
        permissions: opt_string(object.get("permissions")),
        color: object.get("color").and_then(Value::as_i64),
        calendar_type: opt_string(object.get("type")),
        summary_alias: opt_string(object.get("summary_alias")),
        is_deleted: object.get("is_deleted").and_then(Value::as_bool),
        is_third_party: object.get("is_third_party").and_then(Value::as_bool),
        role: opt_string(object.get("role")),
    })
}

fn opt_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn push_optional_query(pairs: &mut Vec<(String, String)>, key: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        pairs.push((key.to_owned(), value.to_owned()));
    }
}

fn insert_optional_string(
    body: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        body.insert(key.to_owned(), Value::String(value.to_owned()));
    }
}

fn insert_optional_bool(body: &mut serde_json::Map<String, Value>, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        body.insert(key.to_owned(), Value::Bool(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freebusy_query_requires_time_window_and_subject() {
        let query = FeishuCalendarFreebusyQuery {
            user_id_type: Some("open_id".to_owned()),
            time_min: String::new(),
            time_max: String::new(),
            user_id: None,
            room_id: None,
            include_external_calendar: Some(true),
            only_busy: Some(true),
            need_rsvp_status: Some(false),
        };

        let error = query.validate().expect_err("invalid freebusy query");
        assert!(error.contains("time_min"));
    }

    #[test]
    fn list_calendars_response_preserves_sync_token() {
        let payload = serde_json::json!({
            "code": 0,
            "msg": "success",
            "data": {
                "has_more": false,
                "page_token": "",
                "sync_token": "ListCalendarsSyncToken_xxx",
                "calendar_list": [{
                    "calendar_id": "feishu.cn_xxx@group.calendar.feishu.cn",
                    "summary": "Team Calendar",
                    "description": "demo",
                    "permissions": "private",
                    "color": -1,
                    "type": "shared",
                    "summary_alias": "Alias",
                    "is_deleted": false,
                    "is_third_party": false,
                    "role": "owner"
                }]
            }
        });

        let page = parse_calendar_list_response(&payload).expect("parse calendar list");
        assert_eq!(
            page.sync_token.as_deref(),
            Some("ListCalendarsSyncToken_xxx")
        );
        assert_eq!(page.calendar_list.len(), 1);
    }
}
