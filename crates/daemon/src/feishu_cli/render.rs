use super::*;

#[allow(clippy::print_stdout)]
pub(super) fn print_feishu_payload(
    payload: &Value,
    as_json: bool,
    render_text: fn(&Value) -> CliResult<String>,
) -> CliResult<()> {
    if as_json {
        let encoded = serde_json::to_string_pretty(payload)
            .map_err(|error| format!("serialize feishu command output failed: {error}"))?;
        println!("{encoded}");
        return Ok(());
    }
    println!("{}", render_text(payload)?);
    Ok(())
}

pub(super) fn render_onboard_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu onboard".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!("config: {}", required_json_string(payload, "config")?),
        format!(
            "credential_source: {}",
            required_json_string(payload, "credential_source")?
        ),
        format!("domain: {}", required_json_string(payload, "domain")?),
        format!("mode: {}", required_json_string(payload, "mode")?),
    ]);
    if let Some(owner_open_id) = payload.get("owner_open_id").and_then(Value::as_str) {
        lines.push(format!("owner_open_id: {owner_open_id}"));
    }
    if let Some(bot_name) = payload.get("bot_name").and_then(Value::as_str) {
        lines.push(format!("bot_name: {bot_name}"));
    }
    if let Some(bot_open_id) = payload.get("bot_open_id").and_then(Value::as_str) {
        lines.push(format!("bot_open_id: {bot_open_id}"));
    }
    if let Some(qr_url) = payload.get("qr_url").and_then(Value::as_str) {
        lines.push(format!("qr_url: {qr_url}"));
    }
    lines.push(format!(
        "serve_command: {}",
        required_json_string(payload, "serve_command")?
    ));
    lines.push(format!(
        "status_command: {}",
        required_json_string(payload, "status_command")?
    ));
    if let Some(notes) = payload.get("notes").and_then(Value::as_array) {
        for note in notes.iter().filter_map(Value::as_str) {
            lines.push(format!("note: {note}"));
        }
    }
    Ok(lines.join("\n"))
}

pub(super) fn render_auth_start_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu auth start".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!("state: {}", required_json_string(payload, "state")?),
        format!(
            "redirect_uri: {}",
            required_json_string(payload, "redirect_uri")?
        ),
        format!(
            "authorize_url: {}",
            required_json_string(payload, "authorize_url")?
        ),
        format!(
            "capabilities: {}",
            payload
                .get("capabilities")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "-".to_owned())
        ),
        format!(
            "scopes: {}",
            payload
                .get("scopes")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        ),
        format!(
            "sqlite_path: {}",
            required_json_string(payload, "sqlite_path")?
        ),
    ]);
    if let Some(status) = payload.get("status").and_then(Value::as_str) {
        lines.push(format!("status: {status}"));
    }
    Ok(lines.join("\n"))
}

pub(super) fn render_auth_exchange_text(payload: &Value) -> CliResult<String> {
    let principal = payload
        .get("principal")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut lines = vec![
        "feishu auth exchange".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "open_id: {}",
            principal
                .get("open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "name: {}",
            principal.get("name").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "scopes: {}",
            payload
                .get("granted_scopes")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        ),
        format!(
            "selected_open_id: {}",
            payload
                .get("selected_open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "effective_open_id: {}",
            payload
                .get("effective_open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_auth_list_text(payload: &Value) -> CliResult<String> {
    let grants = payload
        .get("grants")
        .and_then(Value::as_array)
        .ok_or_else(|| "feishu auth list payload missing grants".to_owned())?;
    let mut lines = vec![
        "feishu auth list".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
        format!("grant_count: {}", grants.len()),
        format!(
            "selected_open_id: {}",
            payload
                .get("selected_open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.insert(2, format!("configured_account: {configured_account}"));
    }
    if let Some(effective_open_id) = payload.get("effective_open_id").and_then(Value::as_str) {
        lines.push(format!("effective_open_id: {effective_open_id}"));
    }
    if let Some(select_hint) = payload
        .get("recommendations")
        .and_then(|value| value.get("select_command"))
        .and_then(Value::as_str)
    {
        lines.push(format!("select_hint: {select_hint}"));
    }
    if let Some(stale_selected_open_id) = payload
        .get("recommendations")
        .and_then(|value| value.get("stale_selected_open_id"))
        .and_then(Value::as_str)
    {
        lines.push(format!("stale_selected_open_id: {stale_selected_open_id}"));
    }
    if let Some(auth_start_hint) = payload
        .get("recommendations")
        .and_then(|value| value.get("auth_start_command"))
        .and_then(Value::as_str)
    {
        lines.push(format!("auth_start_hint: {auth_start_hint}"));
    }
    for grant in grants {
        lines.push(render_auth_grant_summary_line(grant));
    }
    Ok(lines.join("\n"))
}

pub(super) fn render_auth_select_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu auth select".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(format!(
        "selected_open_id: {}",
        required_json_string(payload, "selected_open_id")?
    ));
    if let Some(grant) = payload.get("grant") {
        lines.push(render_auth_grant_summary_line(grant));
    }
    Ok(lines.join("\n"))
}

pub(super) fn render_auth_status_text(payload: &Value) -> CliResult<String> {
    if payload
        .get("status_scope")
        .and_then(Value::as_str)
        .is_some_and(|scope| scope == "account")
    {
        let grants = payload
            .get("grants")
            .and_then(Value::as_array)
            .ok_or_else(|| "feishu auth status payload missing grants".to_owned())?;
        let mut lines = vec![
            "feishu auth status".to_owned(),
            format!("account: {}", required_json_string(payload, "account_id")?),
            format!("status_scope: account"),
            format!("grant_count: {}", grants.len()),
            format!(
                "selected_open_id: {}",
                payload
                    .get("selected_open_id")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            ),
        ];
        if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str)
        {
            lines.insert(2, format!("configured_account: {configured_account}"));
        }
        if let Some(effective_open_id) = payload.get("effective_open_id").and_then(Value::as_str) {
            lines.push(format!("effective_open_id: {effective_open_id}"));
        }
        if let Some(select_hint) = payload
            .get("recommendations")
            .and_then(|value| value.get("select_command"))
            .and_then(Value::as_str)
        {
            lines.push(format!("select_hint: {select_hint}"));
        }
        if let Some(stale_selected_open_id) = payload
            .get("recommendations")
            .and_then(|value| value.get("stale_selected_open_id"))
            .and_then(Value::as_str)
        {
            lines.push(format!("stale_selected_open_id: {stale_selected_open_id}"));
        }
        for grant in grants {
            lines.push(render_auth_grant_summary_line(grant));
        }
        return Ok(lines.join("\n"));
    }

    let status = payload
        .get("status")
        .ok_or_else(|| "feishu auth status payload missing status".to_owned())?;
    let doc_write_status = payload.get("doc_write_status").unwrap_or(&Value::Null);
    let message_write_status = payload.get("message_write_status").unwrap_or(&Value::Null);
    let auth_start_hint = payload
        .get("recommendations")
        .and_then(|value| value.get("auth_start_command"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let select_hint = payload
        .get("recommendations")
        .and_then(|value| value.get("select_command"))
        .and_then(Value::as_str);
    let selected_open_id = payload
        .get("selected_open_id")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let effective_open_id = payload
        .get("effective_open_id")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let mut lines = vec![
        "feishu auth status".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
        format!(
            "has_grant: {}",
            status
                .get("has_grant")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "access_token_expired: {}",
            status
                .get("access_token_expired")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        ),
        format!(
            "refresh_token_expired: {}",
            status
                .get("refresh_token_expired")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        ),
        format!(
            "missing_scopes: {}",
            status
                .get("missing_scopes")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        ),
        format!(
            "doc_write_ready: {}",
            doc_write_status
                .get("ready")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "matched_doc_write_scopes: {}",
            doc_write_status
                .get("matched_scopes")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "-".to_owned())
        ),
        format!(
            "message_write_ready: {}",
            message_write_status
                .get("ready")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "matched_write_scopes: {}",
            message_write_status
                .get("matched_scopes")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "-".to_owned())
        ),
        format!("auth_start_hint: {auth_start_hint}"),
        format!("selected_open_id: {selected_open_id}"),
        format!("effective_open_id: {effective_open_id}"),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.insert(2, format!("configured_account: {configured_account}"));
    }
    if let Some(requested_open_id) = payload.get("requested_open_id").and_then(Value::as_str) {
        lines.push(format!("requested_open_id: {requested_open_id}"));
    }
    if let Some(select_hint) = select_hint {
        lines.push(format!("select_hint: {select_hint}"));
    }
    if let Some(available_open_ids) = payload
        .get("available_open_ids")
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
    {
        lines.push(format!(
            "available_open_ids: {}",
            available_open_ids
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(lines.join("\n"))
}

pub(super) fn render_auth_grant_summary_line(grant: &Value) -> String {
    let principal = grant
        .get("principal")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let status = grant
        .get("status")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let doc_write_status = grant
        .get("doc_write_status")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let message_write_status = grant
        .get("message_write_status")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let missing_scopes = status
        .get("missing_scopes")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "-".to_owned());
    let matched_doc_write_scopes = doc_write_status
        .get("matched_scopes")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "-".to_owned());
    let matched_write_scopes = message_write_status
        .get("matched_scopes")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "-".to_owned());
    let auth_start_hint = grant
        .get("recommendations")
        .and_then(|value| value.get("auth_start_command"))
        .and_then(Value::as_str)
        .unwrap_or("-");

    format!(
        "open_id: {} | selected: {} | effective_selected: {} | name: {} | access_expired: {} | refresh_expired: {} | missing_scopes: {} | doc_write_ready: {} | matched_doc_write_scopes: {} | message_write_ready: {} | matched_write_scopes: {} | auth_start_hint: {}",
        principal
            .get("open_id")
            .and_then(Value::as_str)
            .unwrap_or("-"),
        grant
            .get("selected")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        grant
            .get("effective_selected")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        principal.get("name").and_then(Value::as_str).unwrap_or("-"),
        status
            .get("access_token_expired")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        status
            .get("refresh_token_expired")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        missing_scopes,
        doc_write_status
            .get("ready")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        matched_doc_write_scopes,
        message_write_status
            .get("ready")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        matched_write_scopes,
        auth_start_hint,
    )
}

pub(super) fn render_auth_revoke_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu auth revoke".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "open_id: {}",
            payload
                .get("open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "deleted: {}",
            payload
                .get("deleted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
    ]);
    if let Some(grant_count) = payload.get("grant_count").and_then(Value::as_u64) {
        lines.push(format!("grant_count: {grant_count}"));
    }
    if payload.get("selected_open_id").is_some() {
        lines.push(format!(
            "selected_open_id: {}",
            payload
                .get("selected_open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ));
    }
    if payload.get("effective_open_id").is_some() {
        lines.push(format!(
            "effective_open_id: {}",
            payload
                .get("effective_open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ));
    }
    if let Some(select_hint) = payload
        .get("recommendations")
        .and_then(|value| value.get("select_command"))
        .and_then(Value::as_str)
    {
        lines.push(format!("select_hint: {select_hint}"));
    }
    if let Some(auth_start_hint) = payload
        .get("recommendations")
        .and_then(|value| value.get("auth_start_command"))
        .and_then(Value::as_str)
    {
        lines.push(format!("auth_start_hint: {auth_start_hint}"));
    }
    Ok(lines.join("\n"))
}

pub(super) fn render_whoami_text(payload: &Value) -> CliResult<String> {
    let principal = payload
        .get("principal")
        .ok_or_else(|| "feishu whoami payload missing principal".to_owned())?;
    let mut lines = vec![
        "feishu whoami".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "open_id: {}",
            principal
                .get("open_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "name: {}",
            principal.get("name").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "email: {}",
            principal
                .get("email")
                .and_then(Value::as_str)
                .or_else(|| principal.get("enterprise_email").and_then(Value::as_str))
                .unwrap_or("-")
        ),
        format!(
            "tenant_key: {}",
            principal
                .get("tenant_key")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_read_doc_text(payload: &Value) -> CliResult<String> {
    let document = payload
        .get("document")
        .ok_or_else(|| "feishu read doc payload missing document".to_owned())?;
    let mut lines = vec![
        "feishu read doc".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(format!(
        "document_id: {}",
        document
            .get("document_id")
            .and_then(Value::as_str)
            .unwrap_or("-")
    ));
    lines.push(String::new());
    lines.push(
        document
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    );
    Ok(lines.join("\n"))
}

pub(super) fn render_doc_create_text(payload: &Value) -> CliResult<String> {
    let document = payload
        .get("document")
        .ok_or_else(|| "feishu doc create payload missing document".to_owned())?;
    let mut lines = vec![
        "feishu doc create".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "document_id: {}",
            document
                .get("document_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "url: {}",
            document.get("url").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "content_inserted: {}",
            payload
                .get("content_inserted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "inserted_block_count: {}",
            payload
                .get("inserted_block_count")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ),
        format!(
            "insert_batch_count: {}",
            payload
                .get("insert_batch_count")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ),
        format!(
            "content_type: {}",
            payload
                .get("content_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_doc_append_text(payload: &Value) -> CliResult<String> {
    let document = payload
        .get("document")
        .ok_or_else(|| "feishu doc append payload missing document".to_owned())?;
    let mut lines = vec![
        "feishu doc append".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "document_id: {}",
            document
                .get("document_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "url: {}",
            document.get("url").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "inserted_block_count: {}",
            payload
                .get("inserted_block_count")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ),
        format!(
            "insert_batch_count: {}",
            payload
                .get("insert_batch_count")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ),
        format!(
            "content_type: {}",
            payload
                .get("content_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_messages_history_text(payload: &Value) -> CliResult<String> {
    let page = payload
        .get("page")
        .ok_or_else(|| "feishu message history payload missing page".to_owned())?;
    let mut lines = vec![
        "feishu messages history".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "items: {}",
            page.get("items")
                .and_then(Value::as_array)
                .map_or(0, std::vec::Vec::len)
        ),
        format!(
            "has_more: {}",
            page.get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "page_token: {}",
            page.get("page_token")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_messages_get_text(payload: &Value) -> CliResult<String> {
    let message = payload
        .get("message")
        .ok_or_else(|| "feishu message get payload missing message".to_owned())?;
    let encoded = serde_json::to_string_pretty(message)
        .map_err(|error| format!("serialize feishu message detail failed: {error}"))?;
    let mut lines = vec![
        "feishu messages get".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(encoded);
    Ok(lines.join("\n"))
}

pub(super) fn render_messages_resource_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu messages resource".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "message_id: {}",
            payload
                .get("message_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "file_key: {}",
            payload
                .get("file_key")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "type: {}",
            payload
                .get("resource_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "path: {}",
            payload.get("path").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "bytes_written: {}",
            payload
                .get("bytes_written")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ),
        format!(
            "content_type: {}",
            payload
                .get("content_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "file_name: {}",
            payload
                .get("file_name")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_send_text(payload: &Value) -> CliResult<String> {
    let delivery = payload
        .get("delivery")
        .ok_or_else(|| "feishu send payload missing delivery".to_owned())?;
    let mut lines = vec![
        "feishu send".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "message_id: {}",
            delivery
                .get("message_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "receive_id_type: {}",
            delivery
                .get("receive_id_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "receive_id: {}",
            delivery
                .get("receive_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "uuid: {}",
            delivery.get("uuid").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "msg_type: {}",
            delivery
                .get("msg_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_reply_text(payload: &Value) -> CliResult<String> {
    let delivery = payload
        .get("delivery")
        .ok_or_else(|| "feishu reply payload missing delivery".to_owned())?;
    let mut lines = vec![
        "feishu reply".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "message_id: {}",
            delivery
                .get("message_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "reply_to_message_id: {}",
            delivery
                .get("reply_to_message_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "reply_in_thread: {}",
            delivery
                .get("reply_in_thread")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "uuid: {}",
            delivery.get("uuid").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "msg_type: {}",
            delivery
                .get("msg_type")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_search_messages_text(payload: &Value) -> CliResult<String> {
    let page = payload
        .get("page")
        .ok_or_else(|| "feishu message search payload missing page".to_owned())?;
    let mut lines = vec![
        "feishu search messages".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "items: {}",
            page.get("items")
                .and_then(Value::as_array)
                .map_or(0, std::vec::Vec::len)
        ),
        format!(
            "has_more: {}",
            page.get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "page_token: {}",
            page.get("page_token")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_calendar_list_text(payload: &Value) -> CliResult<String> {
    if payload
        .get("primary")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let calendars = payload
            .get("calendars")
            .and_then(|value| value.get("calendars"))
            .and_then(Value::as_array)
            .map_or(0, std::vec::Vec::len);
        let mut lines = vec![
            "feishu calendar list".to_owned(),
            format!("account: {}", required_json_string(payload, "account_id")?),
        ];
        if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str)
        {
            lines.push(format!("configured_account: {configured_account}"));
        }
        lines.extend([
            "primary: true".to_owned(),
            format!("calendars: {calendars}"),
        ]);
        return Ok(lines.join("\n"));
    }

    let page = payload
        .get("page")
        .ok_or_else(|| "feishu calendar list payload missing page".to_owned())?;
    let mut lines = vec![
        "feishu calendar list".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        "primary: false".to_owned(),
        format!(
            "calendars: {}",
            page.get("calendar_list")
                .and_then(Value::as_array)
                .map_or(0, std::vec::Vec::len)
        ),
        format!(
            "sync_token: {}",
            page.get("sync_token")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_calendar_freebusy_text(payload: &Value) -> CliResult<String> {
    let result = payload
        .get("result")
        .ok_or_else(|| "feishu calendar freebusy payload missing result".to_owned())?;
    let mut lines = vec![
        "feishu calendar freebusy".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(format!(
        "slots: {}",
        result
            .get("freebusy_list")
            .and_then(Value::as_array)
            .map_or(0, std::vec::Vec::len)
    ));
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_list_tables_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu bitable list-tables".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "tables: {}",
            payload
                .get("tables")
                .and_then(Value::as_array)
                .map_or(0, std::vec::Vec::len)
        ),
        format!(
            "has_more: {}",
            payload
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "page_token: {}",
            payload
                .get("page_token")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_app_text(payload: &Value) -> CliResult<String> {
    let app = payload
        .get("app")
        .ok_or_else(|| "feishu bitable app payload missing app".to_owned())?;
    let mut lines = vec![
        "feishu bitable app".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "app_token: {}",
            app.get("app_token").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "name: {}",
            app.get("name").and_then(Value::as_str).unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_app_list_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu bitable app-list".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "apps: {}",
            payload
                .get("apps")
                .and_then(Value::as_array)
                .map_or(0, std::vec::Vec::len)
        ),
        format!(
            "has_more: {}",
            payload
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "page_token: {}",
            payload
                .get("page_token")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_create_record_text(payload: &Value) -> CliResult<String> {
    let record = payload
        .get("record")
        .ok_or_else(|| "feishu bitable create payload missing record".to_owned())?;
    let mut lines = vec![
        "feishu bitable create-record".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "record_id: {}",
            record
                .get("record_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "fields: {}",
            record
                .get("fields")
                .and_then(Value::as_object)
                .map_or(0, serde_json::Map::len)
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_table_text(payload: &Value) -> CliResult<String> {
    let result = payload
        .get("result")
        .ok_or_else(|| "feishu bitable table payload missing result".to_owned())?;
    let mut lines = vec![
        "feishu bitable table".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "table_id: {}",
            result
                .get("table_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "name: {}",
            result.get("name").and_then(Value::as_str).unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_table_batch_create_text(payload: &Value) -> CliResult<String> {
    let result = payload
        .get("result")
        .ok_or_else(|| "feishu bitable batch create tables payload missing result".to_owned())?;
    let mut lines = vec![
        "feishu bitable batch-create-tables".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(format!(
        "table_ids: {}",
        result
            .get("table_ids")
            .and_then(Value::as_array)
            .map_or(0, std::vec::Vec::len)
    ));
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_search_records_text(payload: &Value) -> CliResult<String> {
    let result = payload
        .get("result")
        .ok_or_else(|| "feishu bitable search payload missing result".to_owned())?;
    let mut lines = vec![
        "feishu bitable search-records".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "records: {}",
            result
                .get("items")
                .and_then(Value::as_array)
                .map_or(0, std::vec::Vec::len)
        ),
        format!(
            "has_more: {}",
            result
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "page_token: {}",
            result
                .get("page_token")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_delete_record_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu bitable delete-record".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "record_id: {}",
            payload
                .get("record_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "deleted: {}",
            payload
                .get("deleted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_batch_records_text(payload: &Value) -> CliResult<String> {
    let result = payload
        .get("result")
        .ok_or_else(|| "feishu bitable batch payload missing result".to_owned())?;
    let mut lines = vec![
        "feishu bitable batch-records".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    if let Some(records) = result.get("records").and_then(Value::as_array) {
        lines.push(format!("records: {}", records.len()));
    }
    if let Some(success) = result.get("success").and_then(Value::as_bool) {
        lines.push(format!("success: {success}"));
    }
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_field_text(payload: &Value) -> CliResult<String> {
    let field = payload
        .get("field")
        .ok_or_else(|| "feishu bitable field payload missing field".to_owned())?;
    let mut lines = vec![
        "feishu bitable field".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "field_id: {}",
            field.get("field_id").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "field_name: {}",
            field
                .get("field_name")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_field_list_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu bitable list-fields".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(format!(
        "fields: {}",
        payload
            .get("fields")
            .and_then(Value::as_array)
            .map_or(0, std::vec::Vec::len)
    ));
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_delete_field_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu bitable delete-field".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "field_id: {}",
            payload
                .get("field_id")
                .and_then(Value::as_str)
                .unwrap_or("-")
        ),
        format!(
            "deleted: {}",
            payload
                .get("deleted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_view_text(payload: &Value) -> CliResult<String> {
    let view = payload
        .get("view")
        .ok_or_else(|| "feishu bitable view payload missing view".to_owned())?;
    let mut lines = vec![
        "feishu bitable view".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.extend([
        format!(
            "view_id: {}",
            view.get("view_id").and_then(Value::as_str).unwrap_or("-")
        ),
        format!(
            "view_name: {}",
            view.get("view_name").and_then(Value::as_str).unwrap_or("-")
        ),
    ]);
    Ok(lines.join("\n"))
}

pub(super) fn render_bitable_view_list_text(payload: &Value) -> CliResult<String> {
    let mut lines = vec![
        "feishu bitable list-views".to_owned(),
        format!("account: {}", required_json_string(payload, "account_id")?),
    ];
    if let Some(configured_account) = payload.get("configured_account").and_then(Value::as_str) {
        lines.push(format!("configured_account: {configured_account}"));
    }
    lines.push(format!(
        "views: {}",
        payload
            .get("views")
            .and_then(Value::as_array)
            .map_or(0, std::vec::Vec::len)
    ));
    Ok(lines.join("\n"))
}
