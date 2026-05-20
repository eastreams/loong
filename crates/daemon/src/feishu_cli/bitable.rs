use super::*;

pub async fn execute_feishu_bitable_list_tables(
    args: &FeishuBitableListTablesArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:table:read"],
        "loong feishu bitable list-tables",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::list_bitable_tables(
        &client,
        &grant.access_token,
        &args.app_token,
        args.page_token.as_deref(),
        args.page_size,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "tables": result.items,
        "has_more": result.has_more,
        "page_token": result.page_token,
    }))
}

pub async fn execute_feishu_bitable_app_create(
    args: &FeishuBitableAppCreateArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable app-create",
    )?;
    let client = context.build_client()?;
    let app = mvp::channel::feishu::api::resources::bitable::create_bitable_app(
        &client,
        &grant.access_token,
        &args.name,
        args.folder_token.as_deref(),
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "app": app,
    }))
}

pub async fn execute_feishu_bitable_app_get(args: &FeishuBitableAppGetArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable app-get",
    )?;
    let client = context.build_client()?;
    let app = mvp::channel::feishu::api::resources::bitable::get_bitable_app(
        &client,
        &grant.access_token,
        &args.app_token,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "app": app,
    }))
}

pub async fn execute_feishu_bitable_app_list(args: &FeishuBitableAppListArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["drive:drive:readonly"],
        "loong feishu bitable app-list",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::list_bitable_apps(
        &client,
        &grant.access_token,
        &mvp::channel::feishu::api::resources::bitable::BitableAppListQuery {
            folder_token: args.folder_token.clone(),
            page_size: args.page_size,
            page_token: args.page_token.clone(),
        },
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "apps": result.apps,
        "page_token": result.page_token,
        "has_more": result.has_more,
    }))
}

pub async fn execute_feishu_bitable_app_patch(
    args: &FeishuBitableAppPatchArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable app-patch",
    )?;
    let client = context.build_client()?;
    let app = mvp::channel::feishu::api::resources::bitable::patch_bitable_app(
        &client,
        &grant.access_token,
        &args.app_token,
        args.name.as_deref(),
        args.is_advanced,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "app": app,
    }))
}

pub async fn execute_feishu_bitable_app_copy(args: &FeishuBitableAppCopyArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable app-copy",
    )?;
    let client = context.build_client()?;
    let app = mvp::channel::feishu::api::resources::bitable::copy_bitable_app(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.name,
        args.folder_token.as_deref(),
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "app": app,
    }))
}

pub async fn execute_feishu_bitable_create_record(
    args: &FeishuBitableCreateRecordArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:create"],
        "loong feishu bitable create-record",
    )?;
    let client = context.build_client()?;
    let fields = serde_json::from_str::<Value>(&args.fields)
        .map_err(|error| format!("invalid --fields JSON: {error}"))?;
    if !fields.is_object() {
        return Err("--fields must be a JSON object (e.g. '{\"Name\": \"value\"}')".to_owned());
    }
    let record = mvp::channel::feishu::api::resources::bitable::create_bitable_record(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        fields,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "record": record,
    }))
}

pub async fn execute_feishu_bitable_create_table(
    args: &FeishuBitableCreateTableArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable create-table",
    )?;
    let client = context.build_client()?;
    let fields = args
        .fields
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| format!("invalid --fields JSON: {error}"))?;
    let fields = match fields {
        Some(Value::Array(items)) => Some(items),
        Some(_) => return Err("--fields must be a JSON array".to_owned()),
        None => None,
    };
    let result = mvp::channel::feishu::api::resources::bitable::create_bitable_table(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.name,
        args.default_view_name.as_deref(),
        fields,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_patch_table(
    args: &FeishuBitablePatchTableArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable patch-table",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::patch_bitable_table(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.name,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_batch_create_tables(
    args: &FeishuBitableBatchCreateTablesArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable batch-create-tables",
    )?;
    let client = context.build_client()?;
    let tables = serde_json::from_str::<Value>(&args.tables)
        .map_err(|error| format!("invalid --tables JSON: {error}"))?;
    let tables = match tables {
        Value::Array(items) => items,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_) => {
            return Err("--tables must be a JSON array".to_owned());
        }
    };
    let result = mvp::channel::feishu::api::resources::bitable::batch_create_bitable_tables(
        &client,
        &grant.access_token,
        &args.app_token,
        tables,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_search_records(
    args: &FeishuBitableSearchRecordsArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:retrieve"],
        "loong feishu bitable search-records",
    )?;
    let client = context.build_client()?;
    let filter = args
        .filter
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| format!("invalid --filter JSON: {error}"))?;
    let sort = args
        .sort
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| format!("invalid --sort JSON: {error}"))?;
    let result = mvp::channel::feishu::api::resources::bitable::search_bitable_records(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &mvp::channel::feishu::api::resources::bitable::BitableRecordSearchQuery {
            page_token: args.page_token.clone(),
            page_size: args.page_size,
            view_id: args.view_id.clone(),
            filter,
            sort,
            field_names: (!args.field_names.is_empty()).then(|| args.field_names.clone()),
            automatic_fields: args.automatic_fields.then_some(true),
        },
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_update_record(
    args: &FeishuBitableUpdateRecordArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:write"],
        "loong feishu bitable update-record",
    )?;
    let client = context.build_client()?;
    let fields = serde_json::from_str::<Value>(&args.fields)
        .map_err(|error| format!("invalid --fields JSON: {error}"))?;
    if !fields.is_object() {
        return Err("--fields must be a JSON object (e.g. '{\"Name\": \"value\"}')".to_owned());
    }
    let record = mvp::channel::feishu::api::resources::bitable::update_bitable_record(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.record_id,
        fields,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "record": record,
    }))
}

pub async fn execute_feishu_bitable_delete_record(
    args: &FeishuBitableDeleteRecordArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:write"],
        "loong feishu bitable delete-record",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::delete_bitable_record(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.record_id,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "deleted": result.deleted,
        "record_id": result.record_id,
    }))
}

pub async fn execute_feishu_bitable_batch_create_records(
    args: &FeishuBitableBatchCreateRecordsArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:write"],
        "loong feishu bitable batch-create-records",
    )?;
    let client = context.build_client()?;
    let records = serde_json::from_str::<Value>(&args.records)
        .map_err(|error| format!("invalid --records JSON: {error}"))?;
    let records = match records {
        Value::Array(items) => items,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_) => {
            return Err("--records must be a JSON array".to_owned());
        }
    };
    if records.len() > 500 {
        return Err(format!(
            "feishu.bitable.record.batch_create: batch size must be <= 500, got {}",
            records.len()
        ));
    }
    let result = mvp::channel::feishu::api::resources::bitable::batch_create_bitable_records(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        records,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_batch_update_records(
    args: &FeishuBitableBatchUpdateRecordsArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:write"],
        "loong feishu bitable batch-update-records",
    )?;
    let client = context.build_client()?;
    let records = serde_json::from_str::<Value>(&args.records)
        .map_err(|error| format!("invalid --records JSON: {error}"))?;
    let records = match records {
        Value::Array(items) => items,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_) => {
            return Err("--records must be a JSON array".to_owned());
        }
    };
    if records.len() > 500 {
        return Err(format!(
            "feishu.bitable.record.batch_update: batch size must be <= 500, got {}",
            records.len()
        ));
    }
    let result = mvp::channel::feishu::api::resources::bitable::batch_update_bitable_records(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        records,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_batch_delete_records(
    args: &FeishuBitableBatchDeleteRecordsArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["base:record:write"],
        "loong feishu bitable batch-delete-records",
    )?;
    let client = context.build_client()?;
    let records = serde_json::from_str::<Value>(&args.records)
        .map_err(|error| format!("invalid --records JSON: {error}"))?;
    let records = match records {
        Value::Array(items) => items
            .into_iter()
            .map(|item| {
                item.as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| "--records must be a JSON array of strings".to_owned())
            })
            .collect::<CliResult<Vec<_>>>()?,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_) => {
            return Err("--records must be a JSON array".to_owned());
        }
    };
    if records.len() > 500 {
        return Err(format!(
            "feishu.bitable.record.batch_delete: batch size must be <= 500, got {}",
            records.len()
        ));
    }
    let result = mvp::channel::feishu::api::resources::bitable::batch_delete_bitable_records(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        records,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "result": result,
    }))
}

pub async fn execute_feishu_bitable_create_field(
    args: &FeishuBitableCreateFieldArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable create-field",
    )?;
    let client = context.build_client()?;
    let property = args
        .property
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| format!("invalid --property JSON: {error}"))?;
    let field = mvp::channel::feishu::api::resources::bitable::create_bitable_field(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.field_name,
        args.field_type,
        property,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "field": field,
    }))
}

pub async fn execute_feishu_bitable_list_fields(
    args: &FeishuBitableListFieldsArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable list-fields",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::list_bitable_fields(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &mvp::channel::feishu::api::resources::bitable::BitableFieldListQuery {
            view_id: args.view_id.clone(),
            page_size: args.page_size,
            page_token: args.page_token.clone(),
        },
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "fields": result.items,
        "page_token": result.page_token,
        "has_more": result.has_more,
        "total": result.total,
    }))
}

pub async fn execute_feishu_bitable_update_field(
    args: &FeishuBitableUpdateFieldArgs,
) -> CliResult<Value> {
    let field_name = args
        .field_name
        .as_deref()
        .ok_or_else(|| "--field-name and --type are required for field update".to_owned())?;
    let field_type = args
        .field_type
        .ok_or_else(|| "--field-name and --type are required for field update".to_owned())?;

    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable update-field",
    )?;
    let client = context.build_client()?;
    let property = args
        .property
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| format!("invalid --property JSON: {error}"))?;
    let field = mvp::channel::feishu::api::resources::bitable::update_bitable_field(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.field_id,
        field_name,
        field_type,
        property,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "field": field,
    }))
}

pub async fn execute_feishu_bitable_delete_field(
    args: &FeishuBitableDeleteFieldArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable delete-field",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::delete_bitable_field(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.field_id,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "deleted": result.deleted,
        "field_id": result.field_id,
    }))
}

pub async fn execute_feishu_bitable_create_view(
    args: &FeishuBitableCreateViewArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable create-view",
    )?;
    let client = context.build_client()?;
    let view = mvp::channel::feishu::api::resources::bitable::create_bitable_view(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.view_name,
        args.view_type.as_deref(),
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "view": view,
    }))
}

pub async fn execute_feishu_bitable_get_view(args: &FeishuBitableGetViewArgs) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable get-view",
    )?;
    let client = context.build_client()?;
    let view = mvp::channel::feishu::api::resources::bitable::get_bitable_view(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.view_id,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "view": view,
    }))
}

pub async fn execute_feishu_bitable_list_views(
    args: &FeishuBitableListViewsArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable list-views",
    )?;
    let client = context.build_client()?;
    let result = mvp::channel::feishu::api::resources::bitable::list_bitable_views(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &mvp::channel::feishu::api::resources::bitable::BitableViewListQuery {
            page_size: args.page_size,
            page_token: args.page_token.clone(),
        },
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "views": result.items,
        "page_token": result.page_token,
        "has_more": result.has_more,
        "total": result.total,
    }))
}

pub async fn execute_feishu_bitable_patch_view(
    args: &FeishuBitablePatchViewArgs,
) -> CliResult<Value> {
    let (context, grant) = load_context_and_fresh_grant(&args.grant).await?;
    ensure_grant_has_any_scope(
        &grant,
        context.resolved.configured_account_id.as_str(),
        &["bitable:app"],
        "loong feishu bitable patch-view",
    )?;
    let client = context.build_client()?;
    let view = mvp::channel::feishu::api::resources::bitable::patch_bitable_view(
        &client,
        &grant.access_token,
        &args.app_token,
        &args.table_id,
        &args.view_id,
        &args.view_name,
    )
    .await?;

    Ok(json!({
        "account_id": context.account_id(),
        "configured_account": context.resolved.configured_account_label,
        "principal": grant.principal,
        "view": view,
    }))
}
