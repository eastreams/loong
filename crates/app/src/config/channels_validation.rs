use super::*;

pub(super) fn build_email_invalid_value_issue(
    field_path: &str,
    invalid_reason: &str,
    suggested_fix: &str,
) -> ConfigValidationIssue {
    let mut extra_message_variables = BTreeMap::new();
    extra_message_variables.insert("invalid_reason".to_owned(), invalid_reason.to_owned());
    extra_message_variables.insert("suggested_fix".to_owned(), suggested_fix.to_owned());

    ConfigValidationIssue {
        severity: ConfigValidationSeverity::Error,
        code: ConfigValidationCode::InvalidValue,
        field_path: field_path.to_owned(),
        inline_field_path: field_path.to_owned(),
        example_env_name: String::new(),
        suggested_env_name: None,
        extra_message_variables,
    }
}

pub(super) fn validate_telegram_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name: TELEGRAM_BOT_TOKEN_ENV,
            detect_telegram_token_shape: true,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_telegram_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: TELEGRAM_BOT_TOKEN_ENV,
            detect_telegram_token_shape: true,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_feishu_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("app_id_env") {
        FEISHU_APP_ID_ENV
    } else if field_path.ends_with("app_secret_env") {
        FEISHU_APP_SECRET_ENV
    } else if field_path.ends_with("verification_token_env") {
        FEISHU_VERIFICATION_TOKEN_ENV
    } else {
        FEISHU_ENCRYPT_KEY_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_feishu_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("app_id") {
        FEISHU_APP_ID_ENV
    } else if field_path.ends_with("app_secret") {
        FEISHU_APP_SECRET_ENV
    } else if field_path.ends_with("verification_token") {
        FEISHU_VERIFICATION_TOKEN_ENV
    } else {
        FEISHU_ENCRYPT_KEY_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_matrix_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name: MATRIX_ACCESS_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_matrix_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: MATRIX_ACCESS_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_wecom_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("bot_id_env") {
        WECOM_BOT_ID_ENV
    } else {
        WECOM_SECRET_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_wecom_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("bot_id") {
        WECOM_BOT_ID_ENV
    } else {
        WECOM_SECRET_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_discord_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name: DISCORD_BOT_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_line_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("channel_secret_env") {
        LINE_CHANNEL_SECRET_ENV
    } else {
        LINE_CHANNEL_ACCESS_TOKEN_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_line_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("channel_secret") {
        LINE_CHANNEL_SECRET_ENV
    } else {
        LINE_CHANNEL_ACCESS_TOKEN_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_dingtalk_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("secret_env") {
        DINGTALK_SECRET_ENV
    } else {
        DINGTALK_WEBHOOK_URL_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_dingtalk_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("secret") {
        DINGTALK_SECRET_ENV
    } else {
        DINGTALK_WEBHOOK_URL_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_webhook_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("endpoint_url_env") {
        WEBHOOK_ENDPOINT_URL_ENV
    } else if field_path.ends_with("signing_secret_env") {
        WEBHOOK_SIGNING_SECRET_ENV
    } else {
        WEBHOOK_AUTH_TOKEN_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_webhook_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("endpoint_url") {
        WEBHOOK_ENDPOINT_URL_ENV
    } else if field_path.ends_with("signing_secret") {
        WEBHOOK_SIGNING_SECRET_ENV
    } else {
        WEBHOOK_AUTH_TOKEN_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_discord_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: DISCORD_BOT_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_google_chat_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name: GOOGLE_CHAT_WEBHOOK_URL_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_google_chat_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: GOOGLE_CHAT_WEBHOOK_URL_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_nextcloud_talk_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("shared_secret_env") {
        NEXTCLOUD_TALK_SHARED_SECRET_ENV
    } else {
        NEXTCLOUD_TALK_SERVER_URL_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_nextcloud_talk_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: NEXTCLOUD_TALK_SHARED_SECRET_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_synology_chat_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("incoming_url_env") {
        SYNOLOGY_CHAT_INCOMING_URL_ENV
    } else {
        SYNOLOGY_CHAT_TOKEN_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_synology_chat_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("incoming_url") {
        SYNOLOGY_CHAT_INCOMING_URL_ENV
    } else {
        SYNOLOGY_CHAT_TOKEN_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_teams_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("webhook_url_env") {
        TEAMS_WEBHOOK_URL_ENV
    } else if field_path.ends_with("app_password_env") {
        TEAMS_APP_PASSWORD_ENV
    } else if field_path.ends_with("tenant_id_env") {
        TEAMS_TENANT_ID_ENV
    } else {
        TEAMS_APP_ID_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_teams_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("webhook_url") {
        TEAMS_WEBHOOK_URL_ENV
    } else if field_path.ends_with("app_password") {
        TEAMS_APP_PASSWORD_ENV
    } else {
        TEAMS_APP_ID_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_imessage_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("bridge_url_env") {
        IMESSAGE_BRIDGE_URL_ENV
    } else {
        IMESSAGE_BRIDGE_TOKEN_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_imessage_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: IMESSAGE_BRIDGE_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_signal_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("service_url_env") {
        SIGNAL_SERVICE_URL_ENV
    } else {
        SIGNAL_ACCOUNT_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_mattermost_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("server_url_env") {
        MATTERMOST_SERVER_URL_ENV
    } else {
        MATTERMOST_BOT_TOKEN_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_mattermost_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: MATTERMOST_BOT_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_slack_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name: SLACK_BOT_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_slack_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: SLACK_BOT_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_whatsapp_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("access_token_env") {
        WHATSAPP_ACCESS_TOKEN_ENV
    } else if field_path.ends_with("phone_number_id_env") {
        WHATSAPP_PHONE_NUMBER_ID_ENV
    } else if field_path.ends_with("verify_token_env") {
        WHATSAPP_VERIFY_TOKEN_ENV
    } else {
        WHATSAPP_APP_SECRET_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_whatsapp_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("access_token") {
        WHATSAPP_ACCESS_TOKEN_ENV
    } else if field_path.ends_with("verify_token") {
        WHATSAPP_VERIFY_TOKEN_ENV
    } else {
        WHATSAPP_APP_SECRET_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

pub(super) fn validate_channel_account_integrity<'a, I>(
    issues: &mut Vec<ConfigValidationIssue>,
    channel_key: &str,
    default_account: Option<&str>,
    keys: I,
) where
    I: IntoIterator<Item = &'a String>,
{
    let mut normalized_to_labels = BTreeMap::<String, Vec<String>>::new();
    for raw_key in keys {
        let label = raw_key.trim();
        if label.is_empty() {
            continue;
        }
        normalized_to_labels
            .entry(normalize_channel_account_id(label))
            .or_default()
            .push(label.to_owned());
    }

    for (normalized_account_id, labels) in &normalized_to_labels {
        if labels.len() < 2 {
            continue;
        }
        let mut extra_message_variables = BTreeMap::new();
        extra_message_variables.insert(
            "normalized_account_id".to_owned(),
            normalized_account_id.clone(),
        );
        extra_message_variables.insert("raw_account_labels".to_owned(), labels.join(", "));
        issues.push(ConfigValidationIssue {
            severity: ConfigValidationSeverity::Error,
            code: ConfigValidationCode::DuplicateChannelAccountId,
            field_path: format!("{channel_key}.accounts"),
            inline_field_path: format!("{channel_key}.accounts.{normalized_account_id}"),
            example_env_name: String::new(),
            suggested_env_name: None,
            extra_message_variables,
        });
    }

    if normalized_to_labels.is_empty() {
        return;
    }

    let Some(requested_default_account) = default_account
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let normalized_default_account = normalize_channel_account_id(requested_default_account);
    if normalized_to_labels.contains_key(&normalized_default_account) {
        return;
    }

    let mut extra_message_variables = BTreeMap::new();
    extra_message_variables.insert(
        "requested_account_id".to_owned(),
        normalized_default_account,
    );
    extra_message_variables.insert(
        "configured_account_ids".to_owned(),
        normalized_to_labels
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
    );
    issues.push(ConfigValidationIssue {
        severity: ConfigValidationSeverity::Error,
        code: ConfigValidationCode::UnknownChannelDefaultAccount,
        field_path: format!("{channel_key}.default_account"),
        inline_field_path: format!("{channel_key}.accounts"),
        example_env_name: String::new(),
        suggested_env_name: None,
        extra_message_variables,
    });
}
