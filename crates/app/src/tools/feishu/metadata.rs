use super::support::*;
use super::*;
#[cfg(test)]
use std::collections::BTreeMap;

#[cfg(test)]
pub(in super::super) fn feishu_tool_alias_pairs() -> &'static [(&'static str, &'static str)] {
    FEISHU_TOOL_ALIAS_PAIRS
}

pub(in super::super) fn canonical_feishu_tool_name(raw: &str) -> Option<&'static str> {
    match raw {
        "feishu.whoami" | "feishu_whoami" => Some("feishu.whoami"),
        "feishu.bitable.app.create" | "feishu_bitable_app_create" => {
            Some("feishu.bitable.app.create")
        }
        "feishu.bitable.app.get" | "feishu_bitable_app_get" => Some("feishu.bitable.app.get"),
        "feishu.bitable.app.list" | "feishu_bitable_app_list" => Some("feishu.bitable.app.list"),
        "feishu.bitable.app.patch" | "feishu_bitable_app_patch" => Some("feishu.bitable.app.patch"),
        "feishu.bitable.app.copy" | "feishu_bitable_app_copy" => Some("feishu.bitable.app.copy"),
        "feishu.bitable.list" | "feishu_bitable_list" => Some("feishu.bitable.list"),
        "feishu.bitable.table.create" | "feishu_bitable_table_create" => {
            Some("feishu.bitable.table.create")
        }
        "feishu.bitable.table.patch" | "feishu_bitable_table_patch" => {
            Some("feishu.bitable.table.patch")
        }
        "feishu.bitable.table.batch_create" | "feishu_bitable_table_batch_create" => {
            Some("feishu.bitable.table.batch_create")
        }
        "feishu.bitable.record.create" | "feishu_bitable_record_create" => {
            Some("feishu.bitable.record.create")
        }
        "feishu.bitable.record.update" | "feishu_bitable_record_update" => {
            Some("feishu.bitable.record.update")
        }
        "feishu.bitable.record.delete" | "feishu_bitable_record_delete" => {
            Some("feishu.bitable.record.delete")
        }
        "feishu.bitable.record.batch_create" | "feishu_bitable_record_batch_create" => {
            Some("feishu.bitable.record.batch_create")
        }
        "feishu.bitable.record.batch_update" | "feishu_bitable_record_batch_update" => {
            Some("feishu.bitable.record.batch_update")
        }
        "feishu.bitable.record.batch_delete" | "feishu_bitable_record_batch_delete" => {
            Some("feishu.bitable.record.batch_delete")
        }
        "feishu.bitable.field.create" | "feishu_bitable_field_create" => {
            Some("feishu.bitable.field.create")
        }
        "feishu.bitable.field.list" | "feishu_bitable_field_list" => {
            Some("feishu.bitable.field.list")
        }
        "feishu.bitable.field.update" | "feishu_bitable_field_update" => {
            Some("feishu.bitable.field.update")
        }
        "feishu.bitable.field.delete" | "feishu_bitable_field_delete" => {
            Some("feishu.bitable.field.delete")
        }
        "feishu.bitable.view.create" | "feishu_bitable_view_create" => {
            Some("feishu.bitable.view.create")
        }
        "feishu.bitable.view.get" | "feishu_bitable_view_get" => Some("feishu.bitable.view.get"),
        "feishu.bitable.view.list" | "feishu_bitable_view_list" => Some("feishu.bitable.view.list"),
        "feishu.bitable.view.patch" | "feishu_bitable_view_patch" => {
            Some("feishu.bitable.view.patch")
        }
        "feishu.bitable.record.search" | "feishu_bitable_record_search" => {
            Some("feishu.bitable.record.search")
        }
        "feishu.doc.create" | "feishu_doc_create" => Some("feishu.doc.create"),
        "feishu.doc.append" | "feishu_doc_append" => Some("feishu.doc.append"),
        "feishu.doc.read" | "feishu_doc_read" => Some("feishu.doc.read"),
        "feishu.messages.history" | "feishu_messages_history" => Some("feishu.messages.history"),
        "feishu.messages.get" | "feishu_messages_get" => Some("feishu.messages.get"),
        #[cfg(feature = "tool-file")]
        "feishu.messages.resource.get" | "feishu_messages_resource_get" => {
            Some("feishu.messages.resource.get")
        }
        "feishu.messages.search" | "feishu_messages_search" => Some("feishu.messages.search"),
        "feishu.messages.send" | "feishu_messages_send" => Some("feishu.messages.send"),
        "feishu.messages.reply" | "feishu_messages_reply" => Some("feishu.messages.reply"),
        "feishu.card.update" | "feishu_card_update" => Some("feishu.card.update"),
        "feishu.calendar.list" | "feishu_calendar_list" => Some("feishu.calendar.list"),
        "feishu.calendar.freebusy" | "feishu_calendar_freebusy" => Some("feishu.calendar.freebusy"),
        "feishu.calendar.primary.get" | "feishu_calendar_primary_get" => {
            Some("feishu.calendar.primary.get")
        }
        _ => None,
    }
}

pub(in super::super) fn is_known_feishu_tool_name(raw: &str) -> bool {
    canonical_feishu_tool_name(raw).is_some()
}

#[cfg(test)]
pub(in super::super) fn feishu_tool_registry_entries() -> Vec<super::super::ToolRegistryEntry> {
    let mut entries = Vec::new();
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.app.create",
        "Create a Feishu Bitable app with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.app.get",
        "Fetch Feishu Bitable app metadata with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.app.list",
        "List Feishu Bitable apps through the Drive API with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.app.patch",
        "Update Feishu Bitable app metadata with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.app.copy",
        "Copy a Feishu Bitable app with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.list",
        "List data tables in a Feishu Bitable app with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.table.create",
        "Create a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.table.patch",
        "Rename a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.table.batch_create",
        "Batch create Feishu Bitable tables with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.record.create",
        "Create a record in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.record.update",
        "Update a record in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.record.delete",
        "Delete a record in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.record.batch_create",
        "Batch create records in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.record.batch_update",
        "Batch update records in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.record.batch_delete",
        "Batch delete records in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.field.create",
        "Create a field in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.field.list",
        "List fields in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.field.update",
        "Update a field in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.field.delete",
        "Delete a field in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.view.create",
        "Create a view in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.view.get",
        "Fetch a view in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.view.list",
        "List views in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.view.patch",
        "Patch a view in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.bitable.record.search",
        "Search or list records in a Feishu Bitable table with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.calendar.freebusy",
        "Query Feishu calendar free/busy for the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.doc.create",
        "Create a Feishu docx document and optionally insert initial markdown or html content with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.doc.append",
        "Append markdown or html content to an existing Feishu docx document with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.doc.read",
        "Read Feishu Doc raw content with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.messages.get",
        "Read one Feishu message detail using a tenant token resolved from the selected account grant",
    );
    #[cfg(feature = "tool-file")]
    push_feishu_registry_entry(
        &mut entries,
        "feishu.messages.resource.get",
        "Download one Feishu message image or file resource under the configured file root, with safe ingress defaults when the current Feishu turn carries exactly one resource reference",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.messages.history",
        "List Feishu message history using a tenant token resolved from the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.messages.search",
        "Search Feishu messages with the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.messages.send",
        "Send a Feishu text, post, image, file, or markdown card message with a tenant token resolved from the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.messages.reply",
        "Reply to a Feishu message with text, post, image, file, or a markdown card using a tenant token resolved from the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.card.update",
        "Update a Feishu interactive card through the delayed callback API, using the current callback token when available",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.calendar.list",
        "List Feishu calendars or primary calendars for the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.calendar.primary.get",
        "Fetch the Feishu primary calendar entry for the selected account grant",
    );
    push_feishu_registry_entry(
        &mut entries,
        "feishu.whoami",
        "Inspect the active Feishu grant principal and profile",
    );
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries
}

pub(in super::super) fn feishu_provider_tool_definitions() -> Vec<Value> {
    let mut tools = Vec::new();
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_app_create",
        "Create a Feishu Bitable app with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string", "description": "Optional Feishu configured account id to route through." },
                "open_id": { "type": "string", "description": "Optional explicit Feishu user open_id grant selector." },
                "name": { "type": "string", "description": "Bitable app name." },
                "folder_token": { "type": "string", "description": "Optional Drive folder token." }
            },
            "required": ["name"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_app_get",
        "Fetch Feishu Bitable app metadata with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string", "description": "Optional Feishu configured account id to route through." },
                "open_id": { "type": "string", "description": "Optional explicit Feishu user open_id grant selector." },
                "app_token": { "type": "string", "description": "Feishu Bitable app token." }
            },
            "required": ["app_token"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_app_list",
        "List Feishu Bitable apps through the Drive API with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string", "description": "Optional Feishu configured account id to route through." },
                "open_id": { "type": "string", "description": "Optional explicit Feishu user open_id grant selector." },
                "folder_token": { "type": "string", "description": "Optional Drive folder token." },
                "page_size": { "type": "integer", "minimum": 1, "maximum": 200 },
                "page_token": { "type": "string" }
            },
            "required": [],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_app_patch",
        "Update Feishu Bitable app metadata with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string", "description": "Optional Feishu configured account id to route through." },
                "open_id": { "type": "string", "description": "Optional explicit Feishu user open_id grant selector." },
                "app_token": { "type": "string", "description": "Feishu Bitable app token." },
                "name": { "type": "string", "description": "Optional new app name." },
                "is_advanced": { "type": "boolean", "description": "Optional advanced permission toggle." }
            },
            "required": ["app_token"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_app_copy",
        "Copy a Feishu Bitable app with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string", "description": "Optional Feishu configured account id to route through." },
                "open_id": { "type": "string", "description": "Optional explicit Feishu user open_id grant selector." },
                "app_token": { "type": "string", "description": "Source Bitable app token." },
                "name": { "type": "string", "description": "Copied app name." },
                "folder_token": { "type": "string", "description": "Optional target Drive folder token." }
            },
            "required": ["app_token", "name"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_list",
        "List data tables in a Feishu Bitable app with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "app_token": {
                    "type": "string",
                    "description": "Feishu Bitable app token."
                },
                "page_size": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100
                },
                "page_token": {
                    "type": "string"
                }
            },
            "required": ["app_token"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_table_create",
        "Create a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string", "description": "Optional Feishu configured account id to route through." },
                "open_id": { "type": "string", "description": "Optional explicit Feishu user open_id grant selector." },
                "app_token": { "type": "string", "description": "Feishu Bitable app token." },
                "name": { "type": "string", "description": "Bitable table name." },
                "default_view_name": { "type": "string", "description": "Optional default view name." },
                "fields": { "type": "array", "items": { "type": "object" }, "description": "Optional table field definitions." }
            },
            "required": ["app_token", "name"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_table_patch",
        "Rename a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string", "description": "Optional Feishu configured account id to route through." },
                "open_id": { "type": "string", "description": "Optional explicit Feishu user open_id grant selector." },
                "app_token": { "type": "string", "description": "Feishu Bitable app token." },
                "table_id": { "type": "string", "description": "Feishu Bitable table id." },
                "name": { "type": "string", "description": "New table name." }
            },
            "required": ["app_token", "table_id", "name"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_table_batch_create",
        "Batch create Feishu Bitable tables with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string", "description": "Optional Feishu configured account id to route through." },
                "open_id": { "type": "string", "description": "Optional explicit Feishu user open_id grant selector." },
                "app_token": { "type": "string", "description": "Feishu Bitable app token." },
                "tables": { "type": "array", "items": { "type": "object" }, "description": "Tables to create; only `name` is sent upstream." }
            },
            "required": ["app_token", "tables"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_record_create",
        "Create a record in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "app_token": {
                    "type": "string",
                    "description": "Feishu Bitable app token."
                },
                "table_id": {
                    "type": "string",
                    "description": "Feishu Bitable table id."
                },
                "fields": {
                    "type": "object",
                    "description": "Record field values keyed by field name."
                }
            },
            "required": ["app_token", "table_id", "fields"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_record_update",
        "Update a record in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string", "description": "Optional Feishu configured account id to route through." },
                "open_id": { "type": "string", "description": "Optional explicit Feishu user open_id grant selector." },
                "app_token": { "type": "string", "description": "Feishu Bitable app token." },
                "table_id": { "type": "string", "description": "Feishu Bitable table id." },
                "record_id": { "type": "string", "description": "Feishu Bitable record id." },
                "fields": { "type": "object", "description": "Record field values keyed by field name." }
            },
            "required": ["app_token", "table_id", "record_id", "fields"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_record_delete",
        "Delete a record in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string", "description": "Optional Feishu configured account id to route through." },
                "open_id": { "type": "string", "description": "Optional explicit Feishu user open_id grant selector." },
                "app_token": { "type": "string", "description": "Feishu Bitable app token." },
                "table_id": { "type": "string", "description": "Feishu Bitable table id." },
                "record_id": { "type": "string", "description": "Feishu Bitable record id." }
            },
            "required": ["app_token", "table_id", "record_id"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_record_batch_create",
        "Batch create records in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string" },
                "open_id": { "type": "string" },
                "app_token": { "type": "string" },
                "table_id": { "type": "string" },
                "records": { "type": "array", "items": { "type": "object" } }
            },
            "required": ["app_token", "table_id", "records"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_record_batch_update",
        "Batch update records in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string" },
                "open_id": { "type": "string" },
                "app_token": { "type": "string" },
                "table_id": { "type": "string" },
                "records": { "type": "array", "items": { "type": "object" } }
            },
            "required": ["app_token", "table_id", "records"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_record_batch_delete",
        "Batch delete records in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string" },
                "open_id": { "type": "string" },
                "app_token": { "type": "string" },
                "table_id": { "type": "string" },
                "records": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["app_token", "table_id", "records"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_field_create",
        "Create a field in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string" },
                "open_id": { "type": "string" },
                "app_token": { "type": "string" },
                "table_id": { "type": "string" },
                "field_name": { "type": "string" },
                "type": { "type": "integer" },
                "property": {}
            },
            "required": ["app_token", "table_id", "field_name", "type"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_field_list",
        "List fields in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string" },
                "open_id": { "type": "string" },
                "app_token": { "type": "string" },
                "table_id": { "type": "string" },
                "view_id": { "type": "string" },
                "page_size": { "type": "integer" },
                "page_token": { "type": "string" }
            },
            "required": ["app_token", "table_id"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_field_update",
        "Update a field in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string" },
                "open_id": { "type": "string" },
                "app_token": { "type": "string" },
                "table_id": { "type": "string" },
                "field_id": { "type": "string" },
                "field_name": { "type": "string" },
                "type": { "type": "integer" },
                "property": {}
            },
            "required": ["app_token", "table_id", "field_id", "field_name", "type"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_field_delete",
        "Delete a field in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string" },
                "open_id": { "type": "string" },
                "app_token": { "type": "string" },
                "table_id": { "type": "string" },
                "field_id": { "type": "string" }
            },
            "required": ["app_token", "table_id", "field_id"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_view_create",
        "Create a view in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string" },
                "open_id": { "type": "string" },
                "app_token": { "type": "string" },
                "table_id": { "type": "string" },
                "view_name": { "type": "string" },
                "view_type": { "type": "string" }
            },
            "required": ["app_token", "table_id", "view_name"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_view_get",
        "Fetch a view in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string" },
                "open_id": { "type": "string" },
                "app_token": { "type": "string" },
                "table_id": { "type": "string" },
                "view_id": { "type": "string" }
            },
            "required": ["app_token", "table_id", "view_id"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_view_list",
        "List views in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string" },
                "open_id": { "type": "string" },
                "app_token": { "type": "string" },
                "table_id": { "type": "string" },
                "page_size": { "type": "integer" },
                "page_token": { "type": "string" }
            },
            "required": ["app_token", "table_id"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_view_patch",
        "Patch a view in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string" },
                "open_id": { "type": "string" },
                "app_token": { "type": "string" },
                "table_id": { "type": "string" },
                "view_id": { "type": "string" },
                "view_name": { "type": "string" }
            },
            "required": ["app_token", "table_id", "view_id", "view_name"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_bitable_record_search",
        "Search or list records in a Feishu Bitable table with the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "app_token": {
                    "type": "string",
                    "description": "Feishu Bitable app token."
                },
                "table_id": {
                    "type": "string",
                    "description": "Feishu Bitable table id."
                },
                "view_id": {
                    "type": "string",
                    "description": "Optional Bitable view id."
                },
                "filter": {
                    "type": "object",
                    "description": "Optional Feishu Bitable search filter object."
                },
                "sort": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "field_name": {
                                "type": "string"
                            },
                            "desc": {
                                "type": "boolean"
                            }
                        },
                        "required": ["field_name", "desc"],
                        "additionalProperties": false
                    },
                    "description": "Optional Feishu Bitable sort rules."
                },
                "field_names": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Optional subset of field names to return."
                },
                "automatic_fields": {
                    "type": "boolean",
                    "description": "Whether to return automatic fields such as created_time and last_modified_time."
                },
                "page_size": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 500
                },
                "page_token": {
                    "type": "string"
                }
            },
            "required": ["app_token", "table_id"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_calendar_primary_get",
        "Fetch the Feishu primary calendar entry for the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "user_id_type": {
                    "type": "string",
                    "description": "Optional Feishu user id type for the response. Defaults to `open_id`."
                }
            },
            "required": [],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_calendar_freebusy",
        "Query Feishu calendar free/busy for the selected account grant or an explicit user/room.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "user_id_type": {
                    "type": "string",
                    "description": "Optional Feishu calendar user id type. Defaults to `open_id` when user_id is inferred from the selected grant."
                },
                "time_min": {
                    "type": "string",
                    "description": "Inclusive time window start, typically RFC3339."
                },
                "time_max": {
                    "type": "string",
                    "description": "Exclusive time window end, typically RFC3339."
                },
                "user_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user id. Defaults to the selected grant open_id when room_id is omitted."
                },
                "room_id": {
                    "type": "string",
                    "description": "Optional meeting room id to query instead of a user calendar."
                },
                "include_external_calendar": {
                    "type": "boolean",
                    "description": "Whether to include external calendars."
                },
                "only_busy": {
                    "type": "boolean",
                    "description": "Whether to return only busy slots."
                },
                "need_rsvp_status": {
                    "type": "boolean",
                    "description": "Whether to include RSVP status in each slot."
                }
            },
            "required": ["time_min", "time_max"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_card_update",
        "Update a Feishu interactive card after a card callback. Pass markdown for a standard markdown card or card for full Feishu card JSON. When called from a Feishu callback turn, Loong can infer account_id, callback_token, and a default exclusive open_ids target from internal callback context. Set shared=true for shared-card updates so callback operator defaults are suppressed. Callback tokens expire after 30 minutes and can be used at most twice.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through. Defaults from the current Feishu ingress when available."
                },
                "callback_token": {
                    "type": "string",
                    "description": "Optional callback token for delayed card updates. Usually inferred from the current Feishu card callback turn."
                },
                "card": {
                    "type": "object",
                    "description": "Optional full Feishu card JSON object to apply to the existing card. Mutually exclusive with `markdown`."
                },
                "markdown": {
                    "type": "string",
                    "description": "Optional markdown text to wrap in a standard markdown card. Mutually exclusive with `card`."
                },
                "shared": {
                    "type": "boolean",
                    "description": "Set true for shared-card updates. Shared-card updates must not send non-empty open_ids, and in callback turns this suppresses the default operator open_id target."
                },
                "open_ids": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Optional explicit open_id targets for non-shared cards. For shared cards, either omit open_ids or set shared=true. When omitted in a callback turn without shared=true, Loong can default to the callback operator open_id."
                }
            },
            "required": [],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_calendar_list",
        "List Feishu calendars or primary calendars for the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "primary": {
                    "type": "boolean",
                    "description": "When true, list primary calendars for the selected user."
                },
                "user_id_type": {
                    "type": "string",
                    "description": "Optional Feishu user id type for primary calendar lookup."
                },
                "page_size": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100
                },
                "page_token": {
                    "type": "string"
                },
                "sync_token": {
                    "type": "string"
                }
            },
            "required": [],
            "additionalProperties": false
        }),
    );
    let mut doc_create_parameters = json!({
        "type": "object",
        "properties": {
            "account_id": {
                "type": "string",
                "description": "Optional Feishu configured account id to route through."
            },
            "open_id": {
                "type": "string",
                "description": "Optional explicit Feishu user open_id grant selector."
            },
            "title": {
                "type": "string",
                "description": "Optional plain-text Feishu document title."
            },
            "folder_token": {
                "type": "string",
                "description": "Optional folder token where the new document should be created."
            },
            "content": {
                "type": "string",
                "description": "Optional initial content to convert and insert into the new document. Mutually exclusive with `content_path`."
            },
            "content_type": {
                "type": "string",
                "enum": ["markdown", "html"],
                "description": "Optional content format for `content` or `content_path`. Defaults to the file extension for `content_path` (`.md`/`.markdown` => markdown, `.html`/`.htm` => html) and otherwise `markdown`."
            }
        },
        "required": [],
        "additionalProperties": false
    });
    #[cfg(feature = "tool-file")]
    if let Some(properties) = doc_create_parameters
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        properties.insert(
            "content_path".to_owned(),
            json!({
                "type": "string",
                "description": "Optional relative or rooted local UTF-8 text file path resolved under the configured tool file root and inserted into the new document. Mutually exclusive with `content`."
            }),
        );
    }
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_doc_create",
        "Create a Feishu document with the selected account grant and optionally insert initial markdown or html content into the new doc.",
        doc_create_parameters,
    );
    let mut doc_append_parameters = json!({
        "type": "object",
        "properties": {
            "account_id": {
                "type": "string",
                "description": "Optional Feishu configured account id to route through."
            },
            "open_id": {
                "type": "string",
                "description": "Optional explicit Feishu user open_id grant selector."
            },
            "url": {
                "type": "string",
                "description": "Feishu docx URL or document id of the existing document to append to."
            },
            "content": {
                "type": "string",
                "description": "Markdown or html content to convert and append to the document. Mutually exclusive with `content_path`."
            },
            "content_type": {
                "type": "string",
                "enum": ["markdown", "html"],
                "description": "Optional content format for `content` or `content_path`. Defaults to the file extension for `content_path` (`.md`/`.markdown` => markdown, `.html`/`.htm` => html) and otherwise `markdown`."
            }
        },
        "required": ["url", "content"],
        "additionalProperties": false
    });
    #[cfg(feature = "tool-file")]
    if let Some(parameters) = doc_append_parameters.as_object_mut() {
        if let Some(properties) = parameters
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        {
            properties.insert(
                "content_path".to_owned(),
                json!({
                    "type": "string",
                    "description": "Relative or rooted local UTF-8 text file path resolved under the configured tool file root and appended to the document. Mutually exclusive with `content`."
                }),
            );
        }
        parameters.insert("required".to_owned(), json!(["url"]));
        parameters.insert(
            "anyOf".to_owned(),
            json!([
                { "required": ["content"] },
                { "required": ["content_path"] }
            ]),
        );
    }
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_doc_append",
        "Append markdown or html content to an existing Feishu document identified by docx url or document id using the selected account grant.",
        doc_append_parameters,
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_doc_read",
        "Read a Feishu document by docx url or document id using the selected account grant.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "url": {
                    "type": "string",
                    "description": "Feishu docx URL or document id."
                },
                "lang": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 255,
                    "description": "Optional Feishu language selector."
                }
            },
            "required": ["url"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_messages_get",
        "Fetch one Feishu message detail using a tenant token resolved from the selected account grant. When called from a Feishu conversation, Loong can infer the account and current message from ingress context.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "message_id": {
                    "type": "string",
                    "description": "Feishu message id to fetch. Optional when current Feishu ingress already provides the source message id."
                }
            },
            "required": [],
            "additionalProperties": false
        }),
    );
    #[cfg(feature = "tool-file")]
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_messages_resource_get",
        "Explicitly download one Feishu message image or file resource using a tenant token resolved from the selected account grant and save it under the configured file root. When called from a Feishu conversation, Loong can infer the source message from ingress context and can infer the resource key or type when the current Feishu ingress carries exactly one Feishu message resource or when either payload.file_key or payload.type uniquely identifies one current ingress resource for the same message, as long as payload.message_id is omitted or matches the current ingress message. If the current Feishu ingress summary exposes resource_inventory, choose one entry and copy its file_key plus payload_type into this tool call when multiple resources are present. Outside the current ingress turn, also pass the source message_id explicitly. This does not perform implicit webhook binary downloads.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "message_id": {
                    "type": "string",
                    "description": "Feishu message id that owns the resource. Optional when current Feishu ingress already identifies the source message. Outside the current ingress turn, provide this explicitly. If you override it to a different message, current ingress resource defaults no longer apply."
                },
                "file_key": {
                    "type": "string",
                    "description": "Feishu message resource key paired with the source message id. Optional when the current Feishu ingress carries exactly one Feishu message resource or when payload.type uniquely selects one current ingress resource for the same message, as long as payload.message_id is omitted or matches the current ingress message. If the current Feishu ingress summary includes resource_inventory and multiple resources are present, choose one entry and copy its file_key explicitly."
                },
                "type": {
                    "type": "string",
                    "enum": ["image", "file", "audio", "media"],
                    "description": "Feishu message resource type. Use `image` for image resources, preview images from media messages, and image resource keys; use `file`, `audio`, or `media` for binary file resources. `audio` and `media` aliases normalize to the Feishu file transport type. If the current Feishu ingress summary includes resource_inventory, copy the selected entry's payload_type here. Optional when the current Feishu ingress carries exactly one Feishu message resource or when payload.file_key uniquely selects one current ingress resource for the same message, as long as payload.message_id is omitted or matches the current ingress message."
                },
                "save_as": {
                    "type": "string",
                    "description": "Relative file path to write under the configured file root."
                }
            },
            "required": ["save_as"],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_messages_history",
        "List Feishu message history using a tenant token resolved from the selected account grant. When called from a Feishu conversation, Loong can infer the current chat or thread container from ingress context.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "container_id_type": {
                    "type": "string",
                    "description": "Feishu message container id type, for example `chat` or `thread`. Optional when current Feishu conversation ingress can infer the active chat or thread."
                },
                "container_id": {
                    "type": "string",
                    "description": "Feishu message container id. Optional when current Feishu conversation ingress can infer the active chat or thread id."
                },
                "start_time": {
                    "type": "string"
                },
                "end_time": {
                    "type": "string"
                },
                "sort_type": {
                    "type": "string"
                },
                "page_size": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100
                },
                "page_token": {
                    "type": "string"
                }
            },
            "required": [],
            "additionalProperties": false
        }),
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_messages_search",
        "Search Feishu messages using the selected account grant. When called from the current Feishu conversation, Loong can infer the account and default chat scope from ingress context.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                },
                "user_id_type": {
                    "type": "string",
                    "description": "Optional Feishu search user id type."
                },
                "page_size": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100
                },
                "page_token": {
                    "type": "string"
                },
                "query": {
                    "type": "string",
                    "description": "Search query string."
                },
                "from_ids": {
                    "type": "array",
                    "items": {"type": "string"}
                },
                "chat_ids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional Feishu chat ids to scope the search. When omitted inside the current Feishu conversation, Loong can default this to the active conversation."
                },
                "message_type": {
                    "type": "string"
                },
                "at_chatter_ids": {
                    "type": "array",
                    "items": {"type": "string"}
                },
                "from_type": {
                    "type": "string"
                },
                "chat_type": {
                    "type": "string"
                },
                "start_time": {
                    "type": "string"
                },
                "end_time": {
                    "type": "string"
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    );
    let mut send_parameters = json!({
        "type": "object",
        "properties": {
            "account_id": {
                "type": "string",
                "description": "Optional Feishu configured account id to route through."
            },
            "open_id": {
                "type": "string",
                "description": "Optional explicit Feishu user open_id grant selector."
            },
            "receive_id_type": {
                "type": "string",
                "description": "Optional Feishu receive_id_type override. Defaults to the configured account receive_id_type."
            },
            "receive_id": {
                "type": "string",
                "description": "Feishu receive id to send to. Optional when current Feishu conversation ingress already identifies the active chat."
            },
            "text": {
                "type": "string",
                "description": "Optional plain-text body to send. Mutually exclusive with `post`, `image_key`, `image_path`, `file_key`, and `file_path`."
            },
            "post": {
                "type": "object",
                "description": "Optional Feishu post rich-text content JSON object. Mutually exclusive with `text`, `image_key`, `image_path`, `file_key`, and `file_path`; incompatible with `as_card`."
            },
            "image_key": {
                "type": "string",
                "description": "Optional uploaded Feishu image_key for an image message. Mutually exclusive with `text`, `post`, `image_path`, `file_key`, `file_path`, and `as_card`."
            },
            "file_key": {
                "type": "string",
                "description": "Optional uploaded Feishu file_key for a file message. Mutually exclusive with `text`, `post`, `image_key`, `image_path`, `file_path`, and `as_card`."
            },
            "as_card": {
                "type": "boolean",
                "description": "When true, wrap `text` in a markdown interactive card instead of sending a plain text message. Not allowed with `post`, `image_key`, `image_path`, `file_key`, or `file_path`."
            },
            "uuid": {
                "type": "string",
                "description": "Optional Feishu request UUID used for one-hour message deduplication."
            }
        },
        "required": [],
        "additionalProperties": false
    });
    #[cfg(feature = "tool-file")]
    if let Some(properties) = send_parameters
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        properties.insert(
            "image_path".to_owned(),
            json!({
                "type": "string",
                "description": "Optional relative or rooted local image path resolved under the configured tool file root, uploaded to Feishu before sending. Mutually exclusive with `image_key`, `text`, `post`, `file_key`, `file_path`, and `as_card`."
            }),
        );
        properties.insert(
            "file_path".to_owned(),
            json!({
                "type": "string",
                "description": "Optional relative or rooted local file path resolved under the configured tool file root, uploaded to Feishu before sending. Mutually exclusive with `file_key`, `text`, `post`, `image_key`, `image_path`, and `as_card`."
            }),
        );
        properties.insert(
            "file_type".to_owned(),
            json!({
                "type": "string",
                "description": "Optional Feishu upload file_type used only with `file_path`. Defaults to `stream`."
            }),
        );
    }
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_messages_send",
        "Send a Feishu text, post, image, file, or markdown card message using a tenant token resolved from the selected account grant. When called from the current Feishu conversation, Loong can infer the account and receive_id from ingress context.",
        send_parameters,
    );
    let mut reply_parameters = json!({
        "type": "object",
        "properties": {
            "account_id": {
                "type": "string",
                "description": "Optional Feishu configured account id to route through."
            },
            "open_id": {
                "type": "string",
                "description": "Optional explicit Feishu user open_id grant selector."
            },
            "message_id": {
                "type": "string",
                "description": "Feishu message id to reply to. Optional when current Feishu ingress already identifies the source Feishu message."
            },
            "text": {
                "type": "string",
                "description": "Optional plain-text reply body. Mutually exclusive with `post`, `image_key`, `image_path`, `file_key`, and `file_path`."
            },
            "post": {
                "type": "object",
                "description": "Optional Feishu post rich-text content JSON object. Mutually exclusive with `text`, `image_key`, `image_path`, `file_key`, and `file_path`; incompatible with `as_card`."
            },
            "image_key": {
                "type": "string",
                "description": "Optional uploaded Feishu image_key for an image reply. Mutually exclusive with `text`, `post`, `image_path`, `file_key`, `file_path`, and `as_card`."
            },
            "file_key": {
                "type": "string",
                "description": "Optional uploaded Feishu file_key for a file reply. Mutually exclusive with `text`, `post`, `image_key`, `image_path`, `file_path`, and `as_card`."
            },
            "as_card": {
                "type": "boolean",
                "description": "When true, wrap `text` in a markdown interactive card. Not allowed with `post`, `image_key`, `image_path`, `file_key`, or `file_path`."
            },
            "reply_in_thread": {
                "type": "boolean",
                "description": "When true, force the reply to be posted in thread form. When omitted, Loong defaults to thread form if internal Feishu ingress metadata indicates the source message is already in a thread/topic."
            },
            "uuid": {
                "type": "string",
                "description": "Optional Feishu request UUID used for one-hour reply deduplication."
            }
        },
        "required": [],
        "additionalProperties": false
    });
    #[cfg(feature = "tool-file")]
    if let Some(properties) = reply_parameters
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        properties.insert(
            "image_path".to_owned(),
            json!({
                "type": "string",
                "description": "Optional relative or rooted local image path resolved under the configured tool file root, uploaded to Feishu before replying. Mutually exclusive with `image_key`, `text`, `post`, `file_key`, `file_path`, and `as_card`."
            }),
        );
        properties.insert(
            "file_path".to_owned(),
            json!({
                "type": "string",
                "description": "Optional relative or rooted local file path resolved under the configured tool file root, uploaded to Feishu before replying. Mutually exclusive with `file_key`, `text`, `post`, `image_key`, `image_path`, and `as_card`."
            }),
        );
        properties.insert(
            "file_type".to_owned(),
            json!({
                "type": "string",
                "description": "Optional Feishu upload file_type used only with `file_path`. Defaults to `stream`."
            }),
        );
    }
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_messages_reply",
        "Reply to a Feishu message with text, post, image, file, or a markdown card using a tenant token resolved from the selected account grant. When called from a Feishu conversation, Loong can infer the account and source Feishu message from ingress context.",
        reply_parameters,
    );
    push_feishu_provider_tool_definition(
        &mut tools,
        "feishu_whoami",
        "Resolve the currently selected Feishu OAuth grant and fetch the live user profile.",
        json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Optional Feishu configured account id to route through."
                },
                "open_id": {
                    "type": "string",
                    "description": "Optional explicit Feishu user open_id grant selector."
                }
            },
            "required": [],
            "additionalProperties": false
        }),
    );
    tools.sort_by(|left, right| {
        feishu_provider_tool_function_name(left).cmp(feishu_provider_tool_function_name(right))
    });
    tools
}

pub(in super::super) fn feishu_provider_tool_definition(tool_name: &str) -> Option<Value> {
    feishu_provider_tool_definitions()
        .into_iter()
        .find(|definition| {
            definition
                .get("function")
                .and_then(|value| value.get("name"))
                .and_then(Value::as_str)
                .map(super::super::canonical_tool_name)
                == Some(tool_name)
        })
}

#[cfg(test)]
pub(in super::super) fn feishu_shape_examples() -> BTreeMap<&'static str, Value> {
    let mut shapes = BTreeMap::new();
    shapes.insert(
        "feishu.bitable.list",
        json!({
            "app_token": "bascnDemoAppToken",
            "page_size": 20
        }),
    );
    shapes.insert(
        "feishu.bitable.record.create",
        json!({
            "app_token": "bascnDemoAppToken",
            "table_id": "tblDemo",
            "fields": {
                "Name": "Release note",
                "Status": "Draft"
            }
        }),
    );
    shapes.insert(
        "feishu.bitable.record.search",
        json!({
            "app_token": "bascnDemoAppToken",
            "table_id": "tblDemo",
            "page_size": 20
        }),
    );
    shapes.insert(
        "feishu.doc.create",
        json!({
            "title": "Release Plan",
            "content": "# Release Plan",
            "content_type": "markdown"
        }),
    );
    shapes.insert(
        "feishu.doc.read",
        json!({
            "url": "https://open.feishu.cn/docx/doxcnDemo"
        }),
    );
    shapes.insert(
        "feishu.doc.append",
        json!({
            "url": "https://open.feishu.cn/docx/doxcnDemo",
            "content": "Follow-up note"
        }),
    );
    shapes.insert(
        "feishu.messages.search",
        json!({
            "query": "release note",
            "chat_ids": ["oc_demo_chat"]
        }),
    );
    shapes.insert(
        "feishu.messages.history",
        json!({
            "container_id_type": "chat",
            "container_id": "oc_demo_chat",
            "page_size": 20
        }),
    );
    shapes.insert(
        "feishu.messages.get",
        json!({
            "message_id": "om_123"
        }),
    );
    #[cfg(feature = "tool-file")]
    shapes.insert(
        "feishu.messages.send",
        json!({
            "receive_id": "oc_demo_chat",
            "image_path": "uploads/demo.png"
        }),
    );
    #[cfg(feature = "tool-file")]
    shapes.insert(
        "feishu.messages.reply",
        json!({
            "message_id": "om_123",
            "file_path": "uploads/spec-sheet.pdf",
            "file_type": "stream"
        }),
    );
    #[cfg(feature = "tool-file")]
    shapes.insert(
        "feishu.messages.resource.get",
        json!({
            "message_id": "om_123",
            "file_key": "img_from_resource_inventory",
            "type": "image",
            "save_as": "downloads/preview.png"
        }),
    );
    shapes.insert(
        "feishu.card.update",
        json!({
            "shared": true,
            "markdown": "Approved for everyone"
        }),
    );
    shapes.insert(
        "feishu.calendar.list",
        json!({
            "primary": true
        }),
    );
    shapes.insert(
        "feishu.calendar.freebusy",
        json!({
            "time_min": "2026-03-12T09:00:00+08:00",
            "time_max": "2026-03-12T18:00:00+08:00"
        }),
    );
    shapes.insert("feishu.whoami", json!({}));
    shapes
}
