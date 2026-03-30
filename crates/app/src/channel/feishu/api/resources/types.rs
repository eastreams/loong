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

#[cfg(test)]
mod tests {
    use super::*;

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
}
