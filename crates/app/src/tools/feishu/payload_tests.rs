use super::*;

#[test]
fn feishu_calendar_primary_get_payload_accepts_selector_and_user_id_type() {
    let payload: FeishuCalendarPrimaryGetPayload = serde_json::from_value(json!({
        "account_id": "acct-001",
        "open_id": "ou_abc",
        "user_id_type": "union_id"
    }))
    .expect("primary get payload parses");
    assert_eq!(payload.selector.account_id.as_deref(), Some("acct-001"));
    assert_eq!(payload.selector.open_id.as_deref(), Some("ou_abc"));
    assert_eq!(payload.user_id_type.as_deref(), Some("union_id"));
}

#[test]
fn feishu_calendar_primary_get_payload_defaults_to_empty() {
    let payload: FeishuCalendarPrimaryGetPayload =
        serde_json::from_value(json!({})).expect("empty primary get payload parses");
    assert!(payload.selector.account_id.is_none());
    assert!(payload.selector.open_id.is_none());
    assert!(payload.user_id_type.is_none());
}

#[test]
fn feishu_calendar_primary_get_payload_rejects_unknown_fields() {
    let err = serde_json::from_value::<FeishuCalendarPrimaryGetPayload>(json!({
        "unexpected_field": true
    }))
    .expect_err("unknown fields must be rejected");
    assert!(
        err.to_string().contains("unexpected_field"),
        "error should mention the unknown field, got: {err}"
    );
}
