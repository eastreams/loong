pub(crate) fn resolve_raw_configured_account_key<'a>(
    keys: impl IntoIterator<Item = &'a String>,
    configured_account_id: &str,
) -> Option<String> {
    let requested_account_id = configured_account_id.trim();
    if requested_account_id.is_empty() {
        return None;
    }

    let normalized_requested_account_id =
        crate::mvp::config::normalize_channel_account_id(requested_account_id);

    keys.into_iter().find_map(|raw_key| {
        let normalized_key = crate::mvp::config::normalize_channel_account_id(raw_key.as_str());
        if normalized_key == normalized_requested_account_id {
            return Some(raw_key.clone());
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn resolve_raw_configured_account_key_matches_display_label_account() {
        let mut accounts = BTreeMap::new();
        accounts.insert("Ops Team".to_owned(), 1_u8);

        let resolved = resolve_raw_configured_account_key(accounts.keys(), "ops-team");

        assert_eq!(resolved.as_deref(), Some("Ops Team"));
    }

    #[test]
    fn resolve_raw_configured_account_key_returns_none_for_missing_account() {
        let mut accounts = BTreeMap::new();
        accounts.insert("Ops Team".to_owned(), 1_u8);

        let resolved = resolve_raw_configured_account_key(accounts.keys(), "alerts");

        assert_eq!(resolved, None);
    }
}
