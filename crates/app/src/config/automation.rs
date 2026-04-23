use serde::{Deserialize, Serialize};

const DEFAULT_AUTOMATION_EVENT_PATH: &str = "/automation/events";
const DEFAULT_AUTOMATION_POLL_MS: u64 = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationConfig {
    #[serde(default = "default_automation_event_path")]
    pub event_path: String,
    #[serde(default = "default_automation_poll_ms")]
    pub poll_ms: u64,
    #[serde(default)]
    pub retain_last_sealed_segments: usize,
    #[serde(default)]
    pub retain_min_age_seconds: Option<u64>,
    #[serde(default)]
    pub internal_event_segment_max_bytes: Option<u64>,
}

impl Default for AutomationConfig {
    fn default() -> Self {
        Self {
            event_path: default_automation_event_path(),
            poll_ms: default_automation_poll_ms(),
            retain_last_sealed_segments: 0,
            retain_min_age_seconds: None,
            internal_event_segment_max_bytes: None,
        }
    }
}

impl AutomationConfig {
    #[must_use]
    pub fn is_default(config: &Self) -> bool {
        *config == Self::default()
    }

    #[must_use]
    pub fn resolved_event_path(&self) -> String {
        let trimmed_event_path = self.event_path.trim();
        if trimmed_event_path.is_empty() {
            return default_automation_event_path();
        }
        trimmed_event_path.to_owned()
    }

    #[must_use]
    pub fn resolved_poll_ms(&self) -> u64 {
        if self.poll_ms == 0 {
            return default_automation_poll_ms();
        }
        self.poll_ms
    }
}

fn default_automation_event_path() -> String {
    DEFAULT_AUTOMATION_EVENT_PATH.to_owned()
}

const fn default_automation_poll_ms() -> u64 {
    DEFAULT_AUTOMATION_POLL_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automation_config_defaults_match_operator_surface_defaults() {
        let config = AutomationConfig::default();
        assert_eq!(config.event_path, "/automation/events");
        assert_eq!(config.poll_ms, 1_000);
        assert_eq!(config.retain_last_sealed_segments, 0);
        assert_eq!(config.retain_min_age_seconds, None);
        assert_eq!(config.internal_event_segment_max_bytes, None);
        assert!(AutomationConfig::is_default(&config));
    }

    #[test]
    fn automation_config_blank_event_path_and_zero_poll_fall_back_to_defaults() {
        let config = AutomationConfig {
            event_path: "   ".to_owned(),
            poll_ms: 0,
            ..AutomationConfig::default()
        };
        assert_eq!(config.resolved_event_path(), "/automation/events");
        assert_eq!(config.resolved_poll_ms(), 1_000);
    }
}
