use super::*;
use crate::channel::feishu::api::resources::bitable;

pub(super) fn execute_feishu_bitable_list_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload =
        parse_payload::<FeishuBitableListPayload>("feishu.bitable.list", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty("feishu.bitable.list", "app_token", &payload.app_token)?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_required_scopes(&grant, &["base:table:read"], tool_name.as_str())?;
        let result = bitable::list_bitable_tables(
            &context.client,
            &grant.access_token,
            &app_token,
            payload.page_token.as_deref(),
            payload.page_size,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "tables": result.items,
                "has_more": result.has_more,
                "page_token": result.page_token,
            }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_app_create_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableAppCreatePayload>(
        "feishu.bitable.app.create",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let name = require_non_empty("feishu.bitable.app.create", "name", &payload.name)?;
    let folder_token = payload.folder_token;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let app = bitable::create_bitable_app(
            &context.client,
            &grant.access_token,
            &name,
            folder_token.as_deref(),
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "app": app }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_app_get_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload =
        parse_payload::<FeishuBitableAppGetPayload>("feishu.bitable.app.get", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty("feishu.bitable.app.get", "app_token", &payload.app_token)?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let app =
            bitable::get_bitable_app(&context.client, &grant.access_token, &app_token).await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "app": app }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_app_list_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload =
        parse_payload::<FeishuBitableAppListPayload>("feishu.bitable.app.list", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let query = bitable::BitableAppListQuery {
        folder_token: payload.folder_token,
        page_token: payload.page_token,
        page_size: payload.page_size,
    };
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_required_scopes(&grant, &["drive:drive:readonly"], tool_name.as_str())?;
        let result =
            bitable::list_bitable_apps(&context.client, &grant.access_token, &query).await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "apps": result.apps,
                "page_token": result.page_token,
                "has_more": result.has_more,
            }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_app_patch_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload =
        parse_payload::<FeishuBitableAppPatchPayload>("feishu.bitable.app.patch", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty("feishu.bitable.app.patch", "app_token", &payload.app_token)?;
    let name = payload.name;
    let is_advanced = payload.is_advanced;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let app = bitable::patch_bitable_app(
            &context.client,
            &grant.access_token,
            &app_token,
            name.as_deref(),
            is_advanced,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "app": app }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_app_copy_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload =
        parse_payload::<FeishuBitableAppCopyPayload>("feishu.bitable.app.copy", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty("feishu.bitable.app.copy", "app_token", &payload.app_token)?;
    let name = require_non_empty("feishu.bitable.app.copy", "name", &payload.name)?;
    let folder_token = payload.folder_token;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let app = bitable::copy_bitable_app(
            &context.client,
            &grant.access_token,
            &app_token,
            &name,
            folder_token.as_deref(),
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "app": app }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_record_create_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableRecordCreatePayload>(
        "feishu.bitable.record.create",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.record.create",
        "app_token",
        &payload.app_token,
    )?;
    let table_id = require_non_empty(
        "feishu.bitable.record.create",
        "table_id",
        &payload.table_id,
    )?;
    if !payload.fields.is_object() {
        return Err(format!(
            "feishu.bitable.record.create: `fields` must be a JSON object, got {}",
            payload.fields
        ));
    }
    let fields = payload.fields;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_required_scopes(&grant, &["base:record:create"], tool_name.as_str())?;
        let record = bitable::create_bitable_record(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            fields,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "record": record,
            }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_table_create_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableTableCreatePayload>(
        "feishu.bitable.table.create",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.table.create",
        "app_token",
        &payload.app_token,
    )?;
    let name = require_non_empty("feishu.bitable.table.create", "name", &payload.name)?;
    let default_view_name = payload.default_view_name;
    let fields = payload.fields;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let result = bitable::create_bitable_table(
            &context.client,
            &grant.access_token,
            &app_token,
            &name,
            default_view_name.as_deref(),
            fields,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "result": result }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_table_patch_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableTablePatchPayload>(
        "feishu.bitable.table.patch",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.table.patch",
        "app_token",
        &payload.app_token,
    )?;
    let table_id = require_non_empty("feishu.bitable.table.patch", "table_id", &payload.table_id)?;
    let name = require_non_empty("feishu.bitable.table.patch", "name", &payload.name)?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let result = bitable::patch_bitable_table(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            &name,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "result": result }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_table_batch_create_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableTableBatchCreatePayload>(
        "feishu.bitable.table.batch_create",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.table.batch_create",
        "app_token",
        &payload.app_token,
    )?;
    let tables = payload.tables;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let result = bitable::batch_create_bitable_tables(
            &context.client,
            &grant.access_token,
            &app_token,
            tables,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "result": result }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_record_search_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableRecordSearchPayload>(
        "feishu.bitable.record.search",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.record.search",
        "app_token",
        &payload.app_token,
    )?;
    let table_id = require_non_empty(
        "feishu.bitable.record.search",
        "table_id",
        &payload.table_id,
    )?;
    let query = bitable::BitableRecordSearchQuery {
        page_token: payload.page_token,
        page_size: payload.page_size,
        view_id: payload.view_id,
        filter: payload.filter,
        sort: payload.sort,
        field_names: payload.field_names,
        automatic_fields: payload.automatic_fields,
    };
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_required_scopes(&grant, &["base:record:retrieve"], tool_name.as_str())?;
        let result = bitable::search_bitable_records(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            &query,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "result": result,
            }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_record_update_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableRecordUpdatePayload>(
        "feishu.bitable.record.update",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.record.update",
        "app_token",
        &payload.app_token,
    )?;
    let table_id = require_non_empty(
        "feishu.bitable.record.update",
        "table_id",
        &payload.table_id,
    )?;
    let record_id = require_non_empty(
        "feishu.bitable.record.update",
        "record_id",
        &payload.record_id,
    )?;
    if !payload.fields.is_object() {
        return Err(format!(
            "feishu.bitable.record.update: `fields` must be a JSON object, got {}",
            payload.fields
        ));
    }
    let fields = payload.fields;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["base:record:write"], tool_name.as_str())?;
        let record = bitable::update_bitable_record(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            &record_id,
            fields,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "record": record }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_record_delete_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableRecordDeletePayload>(
        "feishu.bitable.record.delete",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.record.delete",
        "app_token",
        &payload.app_token,
    )?;
    let table_id = require_non_empty(
        "feishu.bitable.record.delete",
        "table_id",
        &payload.table_id,
    )?;
    let record_id = require_non_empty(
        "feishu.bitable.record.delete",
        "record_id",
        &payload.record_id,
    )?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["base:record:write"], tool_name.as_str())?;
        let result = bitable::delete_bitable_record(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            &record_id,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "deleted": result.deleted,
                "record_id": result.record_id,
            }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_record_batch_create_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableRecordBatchCreatePayload>(
        "feishu.bitable.record.batch_create",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.record.batch_create",
        "app_token",
        &payload.app_token,
    )?;
    let table_id = require_non_empty(
        "feishu.bitable.record.batch_create",
        "table_id",
        &payload.table_id,
    )?;
    let records = payload.records;
    bitable::ensure_bitable_batch_limit("feishu.bitable.record.batch_create", records.len())?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["base:record:write"], tool_name.as_str())?;
        let result = bitable::batch_create_bitable_records(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            records,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "result": result }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_record_batch_update_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableRecordBatchUpdatePayload>(
        "feishu.bitable.record.batch_update",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.record.batch_update",
        "app_token",
        &payload.app_token,
    )?;
    let table_id = require_non_empty(
        "feishu.bitable.record.batch_update",
        "table_id",
        &payload.table_id,
    )?;
    let records = payload.records;
    bitable::ensure_bitable_batch_limit("feishu.bitable.record.batch_update", records.len())?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["base:record:write"], tool_name.as_str())?;
        let result = bitable::batch_update_bitable_records(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            records,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "result": result }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_record_batch_delete_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableRecordBatchDeletePayload>(
        "feishu.bitable.record.batch_delete",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.record.batch_delete",
        "app_token",
        &payload.app_token,
    )?;
    let table_id = require_non_empty(
        "feishu.bitable.record.batch_delete",
        "table_id",
        &payload.table_id,
    )?;
    let records = payload.records;
    bitable::ensure_bitable_batch_limit("feishu.bitable.record.batch_delete", records.len())?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["base:record:write"], tool_name.as_str())?;
        let result = bitable::batch_delete_bitable_records(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            records,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "result": result }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_field_create_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableFieldCreatePayload>(
        "feishu.bitable.field.create",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.field.create",
        "app_token",
        &payload.app_token,
    )?;
    let table_id = require_non_empty("feishu.bitable.field.create", "table_id", &payload.table_id)?;
    let field_name = require_non_empty(
        "feishu.bitable.field.create",
        "field_name",
        &payload.field_name,
    )?;
    let field_type =
        require_positive_i64("feishu.bitable.field.create", "type", payload.field_type)?;
    let property = payload.property;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let field = bitable::create_bitable_field(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            &field_name,
            field_type,
            property,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "field": field }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_field_list_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableFieldListPayload>(
        "feishu.bitable.field.list",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token =
        require_non_empty("feishu.bitable.field.list", "app_token", &payload.app_token)?;
    let table_id = require_non_empty("feishu.bitable.field.list", "table_id", &payload.table_id)?;
    let query = bitable::BitableFieldListQuery {
        view_id: payload.view_id,
        page_size: payload.page_size,
        page_token: payload.page_token,
    };
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let result = bitable::list_bitable_fields(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            &query,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "fields": result.items,
                "page_token": result.page_token,
                "has_more": result.has_more,
                "total": result.total,
            }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_field_update_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableFieldUpdatePayload>(
        "feishu.bitable.field.update",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.field.update",
        "app_token",
        &payload.app_token,
    )?;
    let table_id = require_non_empty("feishu.bitable.field.update", "table_id", &payload.table_id)?;
    let field_id = require_non_empty("feishu.bitable.field.update", "field_id", &payload.field_id)?;
    let field_name = require_non_empty(
        "feishu.bitable.field.update",
        "field_name",
        &payload.field_name,
    )?;
    let field_type =
        require_positive_i64("feishu.bitable.field.update", "type", payload.field_type)?;
    let property = payload.property;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let field = bitable::update_bitable_field(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            &field_id,
            &field_name,
            field_type,
            property,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "field": field }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_field_delete_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableFieldDeletePayload>(
        "feishu.bitable.field.delete",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.field.delete",
        "app_token",
        &payload.app_token,
    )?;
    let table_id = require_non_empty("feishu.bitable.field.delete", "table_id", &payload.table_id)?;
    let field_id = require_non_empty("feishu.bitable.field.delete", "field_id", &payload.field_id)?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let result = bitable::delete_bitable_field(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            &field_id,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "deleted": result.deleted,
                "field_id": result.field_id,
            }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_view_create_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableViewCreatePayload>(
        "feishu.bitable.view.create",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty(
        "feishu.bitable.view.create",
        "app_token",
        &payload.app_token,
    )?;
    let table_id = require_non_empty("feishu.bitable.view.create", "table_id", &payload.table_id)?;
    let view_name = require_non_empty(
        "feishu.bitable.view.create",
        "view_name",
        &payload.view_name,
    )?;
    let view_type = payload.view_type;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let view = bitable::create_bitable_view(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            &view_name,
            view_type.as_deref(),
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "view": view }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_view_get_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload =
        parse_payload::<FeishuBitableViewGetPayload>("feishu.bitable.view.get", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty("feishu.bitable.view.get", "app_token", &payload.app_token)?;
    let table_id = require_non_empty("feishu.bitable.view.get", "table_id", &payload.table_id)?;
    let view_id = require_non_empty("feishu.bitable.view.get", "view_id", &payload.view_id)?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let view = bitable::get_bitable_view(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            &view_id,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "view": view }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_view_list_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload =
        parse_payload::<FeishuBitableViewListPayload>("feishu.bitable.view.list", request.payload)?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token = require_non_empty("feishu.bitable.view.list", "app_token", &payload.app_token)?;
    let table_id = require_non_empty("feishu.bitable.view.list", "table_id", &payload.table_id)?;
    let query = bitable::BitableViewListQuery {
        page_size: payload.page_size,
        page_token: payload.page_token,
    };
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let result = bitable::list_bitable_views(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            &query,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({
                "views": result.items,
                "page_token": result.page_token,
                "has_more": result.has_more,
                "total": result.total,
            }),
        ))
    })
}

pub(super) fn execute_feishu_bitable_view_patch_tool_with_config(
    request: ToolCoreRequest,
    config: &super::super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = parse_payload::<FeishuBitableViewPatchPayload>(
        "feishu.bitable.view.patch",
        request.payload,
    )?;
    let context = load_feishu_tool_context(
        config,
        requested_account_id(payload.selector.account_id.as_deref(), &payload.internal),
    )?;
    let grant = require_selected_grant(&context, payload.selector.open_id.as_deref())?;
    let app_token =
        require_non_empty("feishu.bitable.view.patch", "app_token", &payload.app_token)?;
    let table_id = require_non_empty("feishu.bitable.view.patch", "table_id", &payload.table_id)?;
    let view_id = require_non_empty("feishu.bitable.view.patch", "view_id", &payload.view_id)?;
    let view_name =
        require_non_empty("feishu.bitable.view.patch", "view_name", &payload.view_name)?;
    let tool_name = request.tool_name;

    run_feishu_future(async move {
        let grant = crate::channel::feishu::api::ensure_fresh_user_grant(
            &context.client,
            &context.store,
            &grant,
        )
        .await?;
        ensure_any_required_scope(&grant, &["bitable:app"], tool_name.as_str())?;
        let view = bitable::patch_bitable_view(
            &context.client,
            &grant.access_token,
            &app_token,
            &table_id,
            &view_id,
            &view_name,
        )
        .await?;

        Ok(ok_outcome(
            tool_name.as_str(),
            context.configured_account_label.as_str(),
            context.account_id.as_str(),
            &grant.principal,
            json!({ "view": view }),
        ))
    })
}
