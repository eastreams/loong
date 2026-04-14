use std::collections::BTreeSet;

use serde::Serialize;

const STRING_ACCESS_WILDCARD_SENTINEL: &str = "*";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelAccessRestrictionMode {
    Open,
    ExactAllowlist,
    WildcardAllowlist,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelInboundAccessPolicySummary {
    pub conversation_mode: ChannelAccessRestrictionMode,
    pub sender_mode: ChannelAccessRestrictionMode,
    pub allowed_conversations: Vec<String>,
    pub allowed_senders: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelInboundAccessPolicy<T> {
    allowed_conversations: AccessMatcher<T>,
    allowed_senders: AccessMatcher<T>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AccessMatcher<T> {
    allows_all: bool,
    values: BTreeSet<T>,
}

impl<T> Default for AccessMatcher<T>
where
    T: Ord,
{
    fn default() -> Self {
        Self {
            allows_all: false,
            values: BTreeSet::new(),
        }
    }
}

impl<T> AccessMatcher<T>
where
    T: Ord,
{
    fn new(values: BTreeSet<T>) -> Self {
        Self {
            allows_all: false,
            values,
        }
    }

    fn with_all_allowed(mut self) -> Self {
        self.allows_all = true;
        self
    }

    fn is_configured(&self) -> bool {
        if self.allows_all {
            return true;
        }
        !self.values.is_empty()
    }

    fn allows(&self, value: &T) -> bool {
        if self.allows_all {
            return true;
        }
        self.values.contains(value)
    }
}

impl<T> Default for ChannelInboundAccessPolicy<T>
where
    T: Ord,
{
    fn default() -> Self {
        Self {
            allowed_conversations: AccessMatcher::default(),
            allowed_senders: AccessMatcher::default(),
        }
    }
}

impl<T> ChannelInboundAccessPolicy<T>
where
    T: Ord,
{
    fn new(allowed_conversations: AccessMatcher<T>, allowed_senders: AccessMatcher<T>) -> Self {
        Self {
            allowed_conversations,
            allowed_senders,
        }
    }

    pub(crate) fn allows(&self, conversation_id: &T, sender_id: Option<&T>) -> bool {
        let conversation_allowed = self.allowed_conversations.allows(conversation_id);
        if !conversation_allowed {
            return false;
        }
        self.sender_allowed(sender_id)
    }

    pub(crate) fn has_conversation_restrictions(&self) -> bool {
        self.allowed_conversations.is_configured()
    }

    pub(crate) fn has_sender_restrictions(&self) -> bool {
        self.allowed_senders.is_configured()
    }

    fn sender_allowed(&self, sender_id: Option<&T>) -> bool {
        if !self.allowed_senders.is_configured() {
            return true;
        }
        let Some(sender_id) = sender_id else {
            return false;
        };
        self.allowed_senders.allows(sender_id)
    }
}

impl ChannelInboundAccessPolicy<i64> {
    pub(crate) fn from_i64_lists(
        allowed_conversation_ids: &[i64],
        allowed_sender_ids: &[i64],
    ) -> Self {
        let allowed_conversations = numeric_matcher(allowed_conversation_ids);
        let allowed_senders = numeric_matcher(allowed_sender_ids);
        Self::new(allowed_conversations, allowed_senders)
    }

    pub(crate) fn summary(&self) -> ChannelInboundAccessPolicySummary {
        let allowed_conversations = self
            .allowed_conversations
            .values
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>();
        let allowed_senders = self
            .allowed_senders
            .values
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>();

        ChannelInboundAccessPolicySummary {
            conversation_mode: numeric_matcher_mode(&self.allowed_conversations),
            sender_mode: numeric_matcher_mode(&self.allowed_senders),
            allowed_conversations,
            allowed_senders,
        }
    }
}

impl ChannelInboundAccessPolicy<String> {
    pub(crate) fn from_string_lists(
        allowed_conversation_ids: &[String],
        allowed_sender_ids: &[String],
        allow_wildcards: bool,
    ) -> Self {
        let allowed_conversations = string_matcher(allowed_conversation_ids, allow_wildcards);
        let allowed_senders = string_matcher(allowed_sender_ids, allow_wildcards);
        Self::new(allowed_conversations, allowed_senders)
    }

    pub(crate) fn allows_str(&self, conversation_id: &str, sender_id: Option<&str>) -> bool {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return false;
        }
        let sender_id = normalize_optional_str(sender_id);
        self.allowed_conversations_allow_str(conversation_id)
            && self.allowed_senders_allow_str(sender_id)
    }

    pub(crate) fn allows_conversation_str(&self, conversation_id: &str) -> bool {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return false;
        }
        self.allowed_conversations_allow_str(conversation_id)
    }

    pub(crate) fn allows_optional_conversation_str(
        &self,
        conversation_id: Option<&str>,
        sender_id: Option<&str>,
    ) -> bool {
        let Some(conversation_id) = normalize_optional_str(conversation_id) else {
            return false;
        };
        self.allows_str(conversation_id, sender_id)
    }

    pub(crate) fn string_conversations(&self) -> Option<Vec<String>> {
        let has_conversation_restrictions = self.has_conversation_restrictions();
        if !has_conversation_restrictions {
            return None;
        }
        let mut values = Vec::new();
        for value in &self.allowed_conversations.values {
            values.push(value.clone());
        }
        if self.allowed_conversations.allows_all {
            values.push(STRING_ACCESS_WILDCARD_SENTINEL.to_owned());
        }
        Some(values)
    }

    pub(crate) fn string_senders(&self) -> Option<Vec<String>> {
        let has_sender_restrictions = self.has_sender_restrictions();
        if !has_sender_restrictions {
            return None;
        }
        let mut values = Vec::new();
        for value in &self.allowed_senders.values {
            values.push(value.clone());
        }
        if self.allowed_senders.allows_all {
            values.push(STRING_ACCESS_WILDCARD_SENTINEL.to_owned());
        }
        Some(values)
    }

    pub(crate) fn summary(&self) -> ChannelInboundAccessPolicySummary {
        let allowed_conversations = self.string_conversations().unwrap_or_default();
        let allowed_senders = self.string_senders().unwrap_or_default();

        ChannelInboundAccessPolicySummary {
            conversation_mode: string_matcher_mode(&self.allowed_conversations),
            sender_mode: string_matcher_mode(&self.allowed_senders),
            allowed_conversations,
            allowed_senders,
        }
    }

    fn allowed_conversations_allow_str(&self, conversation_id: &str) -> bool {
        if self.allowed_conversations.allows_all {
            return true;
        }
        self.allowed_conversations.values.contains(conversation_id)
    }

    fn allowed_senders_allow_str(&self, sender_id: Option<&str>) -> bool {
        if !self.allowed_senders.is_configured() {
            return true;
        }
        let Some(sender_id) = sender_id else {
            return false;
        };
        if self.allowed_senders.allows_all {
            return true;
        }
        self.allowed_senders.values.contains(sender_id)
    }
}

fn normalize_optional_str(value: Option<&str>) -> Option<&str> {
    let value = value?;
    let trimmed_value = value.trim();
    if trimmed_value.is_empty() {
        return None;
    }
    Some(trimmed_value)
}

fn numeric_matcher_mode<T>(matcher: &AccessMatcher<T>) -> ChannelAccessRestrictionMode
where
    T: Ord,
{
    if !matcher.is_configured() {
        return ChannelAccessRestrictionMode::Open;
    }
    ChannelAccessRestrictionMode::ExactAllowlist
}

fn string_matcher_mode(matcher: &AccessMatcher<String>) -> ChannelAccessRestrictionMode {
    if !matcher.is_configured() {
        return ChannelAccessRestrictionMode::Open;
    }
    if matcher.allows_all {
        return ChannelAccessRestrictionMode::WildcardAllowlist;
    }
    ChannelAccessRestrictionMode::ExactAllowlist
}

fn numeric_matcher(values: &[i64]) -> AccessMatcher<i64> {
    let mut normalized = BTreeSet::new();
    for value in values {
        normalized.insert(*value);
    }
    AccessMatcher::new(normalized)
}

fn string_matcher(values: &[String], allow_wildcards: bool) -> AccessMatcher<String> {
    let mut normalized = BTreeSet::new();
    let mut allows_all = false;

    for raw_value in values {
        let trimmed_value = raw_value.trim();
        if trimmed_value.is_empty() {
            continue;
        }
        if allow_wildcards && trimmed_value == STRING_ACCESS_WILDCARD_SENTINEL {
            allows_all = true;
            continue;
        }
        normalized.insert(trimmed_value.to_owned());
    }

    let matcher = AccessMatcher::new(normalized);
    if allows_all {
        return matcher.with_all_allowed();
    }
    matcher
}

#[cfg(test)]
mod tests {
    use super::{
        ChannelAccessRestrictionMode, ChannelInboundAccessPolicy, ChannelInboundAccessPolicySummary,
    };

    #[test]
    fn i64_policy_requires_conversation_allowlist_match() {
        let policy = ChannelInboundAccessPolicy::from_i64_lists(&[1001], &[]);

        assert!(policy.allows(&1001, None));
        assert!(!policy.allows(&2002, None));
    }

    #[test]
    fn i64_policy_enforces_sender_match_only_when_sender_allowlist_is_configured() {
        let unrestricted_sender_policy = ChannelInboundAccessPolicy::from_i64_lists(&[1001], &[]);
        assert!(unrestricted_sender_policy.allows(&1001, None));

        let restricted_sender_policy = ChannelInboundAccessPolicy::from_i64_lists(&[1001], &[42]);
        assert!(restricted_sender_policy.allows(&1001, Some(&42)));
        assert!(!restricted_sender_policy.allows(&1001, Some(&7)));
        assert!(!restricted_sender_policy.allows(&1001, None));
    }

    #[test]
    fn string_policy_supports_wildcards_when_requested() {
        let policy = ChannelInboundAccessPolicy::from_string_lists(
            &["*".to_owned()],
            &["ou_admin".to_owned()],
            true,
        );

        assert!(policy.allows(&"oc_demo".to_owned(), Some(&"ou_admin".to_owned())));
        assert!(!policy.allows(&"oc_demo".to_owned(), Some(&"ou_guest".to_owned())));
    }

    #[test]
    fn string_policy_trims_and_filters_empty_entries() {
        let policy = ChannelInboundAccessPolicy::from_string_lists(
            &["  !ops:example.org  ".to_owned(), " ".to_owned()],
            &[],
            false,
        );

        assert!(policy.allows(&"!ops:example.org".to_owned(), None));
        assert!(!policy.allows(&"!other:example.org".to_owned(), None));
    }

    #[test]
    fn optional_conversation_gate_rejects_missing_conversation_ids() {
        let policy = ChannelInboundAccessPolicy::from_string_lists(
            &["oc_demo".to_owned()],
            &["ou_admin".to_owned()],
            true,
        );

        assert!(policy.allows_optional_conversation_str(Some("oc_demo"), Some("ou_admin"),));
        assert!(!policy.allows_optional_conversation_str(None, Some("ou_admin")));
    }

    #[test]
    fn numeric_policy_summary_reports_open_sender_mode_when_sender_allowlist_is_empty() {
        let policy = ChannelInboundAccessPolicy::from_i64_lists(&[1001], &[]);

        assert_eq!(
            policy.summary(),
            ChannelInboundAccessPolicySummary {
                conversation_mode: ChannelAccessRestrictionMode::ExactAllowlist,
                sender_mode: ChannelAccessRestrictionMode::Open,
                allowed_conversations: vec!["1001".to_owned()],
                allowed_senders: Vec::new(),
            }
        );
    }

    #[test]
    fn string_policy_summary_reports_wildcard_conversation_mode() {
        let policy = ChannelInboundAccessPolicy::from_string_lists(
            &["*".to_owned()],
            &["ou_admin".to_owned()],
            true,
        );

        assert_eq!(
            policy.summary(),
            ChannelInboundAccessPolicySummary {
                conversation_mode: ChannelAccessRestrictionMode::WildcardAllowlist,
                sender_mode: ChannelAccessRestrictionMode::ExactAllowlist,
                allowed_conversations: vec!["*".to_owned()],
                allowed_senders: vec!["ou_admin".to_owned()],
            }
        );
    }

    #[test]
    fn string_policy_can_validate_conversation_without_sender_context() {
        let policy = ChannelInboundAccessPolicy::from_string_lists(
            &["oc_demo".to_owned()],
            &["ou_admin".to_owned()],
            true,
        );

        assert!(policy.allows_conversation_str("oc_demo"));
        assert!(!policy.allows_conversation_str("oc_other"));
        assert!(!policy.allows_conversation_str(" "));
    }
}
