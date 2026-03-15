use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuApiError {
    pub code: i64,
    pub message: String,
}

impl FeishuApiError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for FeishuApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "feishu api error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for FeishuApiError {}
