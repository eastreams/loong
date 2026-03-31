use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuDocumentContent {
    pub document_id: String,
    pub content: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuDocumentMetadata {
    pub document_id: String,
    pub title: Option<String>,
    pub revision_id: Option<i64>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuMessageSummary {
    pub message_id: String,
    pub chat_id: Option<String>,
    pub root_id: Option<String>,
    pub parent_id: Option<String>,
    pub message_type: Option<String>,
    pub create_time: Option<String>,
    pub update_time: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuMessageDetail {
    pub message_id: String,
    pub chat_id: Option<String>,
    pub root_id: Option<String>,
    pub parent_id: Option<String>,
    pub message_type: Option<String>,
    pub create_time: Option<String>,
    pub update_time: Option<String>,
    pub deleted: Option<bool>,
    pub updated: Option<bool>,
    pub sender_id: Option<String>,
    pub sender_type: Option<String>,
    pub body: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuMessageWriteReceipt {
    pub message_id: String,
    pub root_id: Option<String>,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuCardUpdateReceipt {
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuUploadedImage {
    pub image_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuUploadedFile {
    pub file_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeishuMessageResourceType {
    Image,
    File,
}

impl FeishuMessageResourceType {
    pub fn as_api_value(&self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::File => "file",
        }
    }
}

impl FromStr for FeishuMessageResourceType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "image" => Ok(Self::Image),
            "file" | "audio" | "media" => Ok(Self::File),
            other => Err(format!(
                "unsupported Feishu message resource type `{other}`; expected `image`, `file`, `audio`, or `media`"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuDownloadedMessageResource {
    pub message_id: String,
    pub file_key: String,
    pub resource_type: FeishuMessageResourceType,
    pub content_type: Option<String>,
    pub file_name: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuMessageHistoryPage {
    pub has_more: bool,
    pub page_token: Option<String>,
    pub items: Vec<FeishuMessageDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuSearchMessagePage {
    pub has_more: bool,
    pub page_token: Option<String>,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuCalendarEntry {
    pub calendar_id: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub permissions: Option<String>,
    pub color: Option<i64>,
    pub calendar_type: Option<String>,
    pub summary_alias: Option<String>,
    pub is_deleted: Option<bool>,
    pub is_third_party: Option<bool>,
    pub role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuCalendarListPage {
    pub has_more: bool,
    pub page_token: Option<String>,
    pub sync_token: Option<String>,
    pub calendar_list: Vec<FeishuCalendarEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuPrimaryCalendarEntry {
    pub calendar: FeishuCalendarEntry,
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuPrimaryCalendarList {
    pub calendars: Vec<FeishuPrimaryCalendarEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuCalendarFreebusySlot {
    pub start_time: String,
    pub end_time: String,
    pub rsvp_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuCalendarFreebusyResult {
    pub freebusy_list: Vec<FeishuCalendarFreebusySlot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuBitableTable {
    pub table_id: Option<String>,
    pub name: Option<String>,
    pub revision: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuBitableApp {
    pub app_token: Option<String>,
    pub name: Option<String>,
    pub revision: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeishuBitableField {
    pub field_id: Option<String>,
    pub field_name: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<i64>,
    pub property: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuBitableView {
    pub view_id: Option<String>,
    pub view_name: Option<String>,
    pub view_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeishuBitableTableListPage {
    pub items: Vec<FeishuBitableTable>,
    pub page_token: Option<String>,
    pub has_more: Option<bool>,
    pub total: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeishuBitableRecord {
    pub record_id: Option<String>,
    pub fields: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeishuBitableRecordPage {
    pub items: Vec<FeishuBitableRecord>,
    pub page_token: Option<String>,
    pub has_more: Option<bool>,
    pub total: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeishuBitableAppListPage {
    pub apps: Vec<Value>,
    pub page_token: Option<String>,
    pub has_more: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeishuBitableFieldListPage {
    pub items: Vec<FeishuBitableField>,
    pub page_token: Option<String>,
    pub has_more: Option<bool>,
    pub total: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeishuBitableViewListPage {
    pub items: Vec<FeishuBitableView>,
    pub page_token: Option<String>,
    pub has_more: Option<bool>,
    pub total: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuBitableDeletedRecord {
    pub deleted: bool,
    pub record_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuBitableDeletedField {
    pub deleted: bool,
    pub field_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn feishu_message_resource_type_accepts_audio_and_media_aliases() {
        assert_eq!(
            "audio"
                .parse::<FeishuMessageResourceType>()
                .expect("audio alias should parse"),
            FeishuMessageResourceType::File
        );
        assert_eq!(
            "media"
                .parse::<FeishuMessageResourceType>()
                .expect("media alias should parse"),
            FeishuMessageResourceType::File
        );
        assert_eq!(
            "image"
                .parse::<FeishuMessageResourceType>()
                .expect("image should parse"),
            FeishuMessageResourceType::Image
        );
    }

    #[test]
    fn feishu_bitable_field_deserializes_type_and_property() {
        let field: FeishuBitableField = serde_json::from_value(json!({
            "field_id": "fld_123",
            "field_name": "Status",
            "type": 3,
            "property": {
                "options": [{"name": "Open"}]
            }
        }))
        .expect("field should deserialize");

        assert_eq!(field.field_id.as_deref(), Some("fld_123"));
        assert_eq!(field.field_name.as_deref(), Some("Status"));
        assert_eq!(field.r#type, Some(3));
        assert_eq!(field.property, Some(json!({"options": [{"name": "Open"}]})));
    }

    #[test]
    fn feishu_bitable_view_list_page_deserializes_items() {
        let page: FeishuBitableViewListPage = serde_json::from_value(json!({
            "items": [
                {
                    "view_id": "vew_123",
                    "view_name": "All Tasks",
                    "view_type": "grid"
                }
            ],
            "has_more": false,
            "page_token": "page_1",
            "total": 1
        }))
        .expect("view page should deserialize");

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].view_id.as_deref(), Some("vew_123"));
        assert_eq!(page.items[0].view_type.as_deref(), Some("grid"));
        assert_eq!(page.total, Some(1));
    }
}
