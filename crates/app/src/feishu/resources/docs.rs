use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::CliResult;

use super::types::{FeishuDocumentContent, FeishuDocumentMetadata};
use crate::feishu::client::FeishuClient;

const FEISHU_DOC_NESTED_BLOCK_LIMIT: usize = 1000;
const FEISHU_DOC_BLOCK_TYPE_CALLOUT: i64 = 19;
const FEISHU_DOC_BLOCK_TYPE_GRID: i64 = 24;
const FEISHU_DOC_BLOCK_TYPE_GRID_COLUMN: i64 = 25;
const FEISHU_DOC_BLOCK_TYPE_TABLE: i64 = 31;
const FEISHU_DOC_BLOCK_TYPE_TABLE_CELL: i64 = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuConvertedDocumentBlocks {
    pub first_level_block_ids: Vec<String>,
    pub descendants: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuNestedBlockInsertSummary {
    pub inserted_block_count: usize,
    pub batch_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FeishuCreatedContainerBlock {
    block_id: String,
    child_block_ids: Vec<String>,
}

pub async fn fetch_document_content(
    client: &FeishuClient,
    user_access_token: &str,
    document_id_or_url: &str,
    lang: Option<u8>,
) -> CliResult<FeishuDocumentContent> {
    let document_id = extract_document_id(document_id_or_url)
        .ok_or_else(|| "failed to resolve Feishu document id".to_owned())?;
    let mut query = Vec::new();
    if let Some(lang) = lang {
        query.push(("lang".to_owned(), lang.to_string()));
    }
    let payload = client
        .get_json(
            format!("/open-apis/docx/v1/documents/{document_id}/raw_content").as_str(),
            Some(user_access_token),
            &query,
        )
        .await?;
    parse_raw_content_response(document_id.as_str(), &payload)
}

pub async fn create_document(
    client: &FeishuClient,
    user_access_token: &str,
    title: Option<&str>,
    folder_token: Option<&str>,
) -> CliResult<FeishuDocumentMetadata> {
    let mut body = serde_json::Map::new();
    if let Some(title) = trimmed_opt(title) {
        body.insert("title".to_owned(), Value::String(title.to_owned()));
    }
    if let Some(folder_token) = trimmed_opt(folder_token) {
        body.insert(
            "folder_token".to_owned(),
            Value::String(folder_token.to_owned()),
        );
    }
    let payload = client
        .post_json(
            "/open-apis/docx/v1/documents",
            Some(user_access_token),
            &[],
            &Value::Object(body),
        )
        .await?;
    parse_document_create_response(&payload)
}

pub async fn convert_content_to_blocks(
    client: &FeishuClient,
    user_access_token: &str,
    content_type: &str,
    content: &str,
) -> CliResult<FeishuConvertedDocumentBlocks> {
    let content_type = require_content_type(content_type)?;
    let content = content
        .trim()
        .strip_prefix('\u{feff}')
        .unwrap_or(content.trim());
    if content.is_empty() {
        return Err("feishu document content conversion requires non-empty content".to_owned());
    }
    let payload = client
        .post_json(
            "/open-apis/docx/v1/documents/blocks/convert",
            Some(user_access_token),
            &[],
            &serde_json::json!({
                "content_type": content_type,
                "content": content,
            }),
        )
        .await?;
    parse_convert_blocks_response(&payload)
}

pub async fn create_nested_blocks(
    client: &FeishuClient,
    user_access_token: &str,
    document_id: &str,
    blocks: &FeishuConvertedDocumentBlocks,
) -> CliResult<FeishuNestedBlockInsertSummary> {
    let document_id = document_id.trim();
    if document_id.is_empty() {
        return Err("feishu document nested block creation requires document_id".to_owned());
    }
    if blocks.first_level_block_ids.is_empty() {
        return Ok(FeishuNestedBlockInsertSummary {
            inserted_block_count: 0,
            batch_count: 0,
        });
    }
    let descendants_by_id = descendants_by_id(&blocks.descendants)?;

    insert_child_subtrees(
        client,
        user_access_token,
        document_id,
        document_id.to_owned(),
        blocks.first_level_block_ids.clone(),
        &descendants_by_id,
        &blocks.descendants,
        FEISHU_DOC_NESTED_BLOCK_LIMIT,
    )
    .await
}

pub fn extract_document_id(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("dox") {
        return Some(trimmed.to_owned());
    }

    let parsed = reqwest::Url::parse(trimmed).ok()?;
    let mut segments = parsed.path_segments()?;
    while let Some(segment) = segments.next() {
        if segment == "docx" {
            return segments
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
        }
    }
    None
}

pub fn parse_raw_content_response(
    document_id: &str,
    payload: &Value,
) -> CliResult<FeishuDocumentContent> {
    let content = payload
        .get("data")
        .and_then(|value| value.get("content"))
        .and_then(Value::as_str)
        .map(str::trim)
        .ok_or_else(|| "feishu document payload missing data.content".to_owned())?;

    Ok(FeishuDocumentContent {
        document_id: document_id.trim().to_owned(),
        content: content.to_owned(),
        url: Some(format!("https://open.feishu.cn/docx/{document_id}")),
    })
}

pub fn parse_document_create_response(payload: &Value) -> CliResult<FeishuDocumentMetadata> {
    let document = payload
        .get("data")
        .and_then(|value| value.get("document"))
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu document create payload missing data.document".to_owned())?;
    let document_id = object_string(document, "document_id").ok_or_else(|| {
        "feishu document create payload missing data.document.document_id".to_owned()
    })?;

    Ok(FeishuDocumentMetadata {
        document_id: document_id.clone(),
        title: object_string(document, "title"),
        revision_id: document.get("revision_id").and_then(Value::as_i64),
        url: Some(format!("https://open.feishu.cn/docx/{document_id}")),
    })
}

pub fn parse_convert_blocks_response(payload: &Value) -> CliResult<FeishuConvertedDocumentBlocks> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu document convert payload missing data object".to_owned())?;
    let first_level_block_ids = data
        .get("first_level_block_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "feishu document convert payload missing data.first_level_block_ids".to_owned()
        })?
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if first_level_block_ids.is_empty() {
        return Err(
            "feishu document convert payload did not return any first_level_block_ids".to_owned(),
        );
    }

    let mut descendants = data
        .get("blocks")
        .and_then(Value::as_array)
        .ok_or_else(|| "feishu document convert payload missing data.blocks".to_owned())?
        .clone();
    if descendants.is_empty() {
        return Err("feishu document convert payload did not return any blocks".to_owned());
    }
    strip_table_merge_info(&mut descendants);

    Ok(FeishuConvertedDocumentBlocks {
        first_level_block_ids,
        descendants,
    })
}

pub fn parse_nested_blocks_create_response(payload: &Value) -> CliResult<HashMap<String, String>> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu nested block create payload missing data object".to_owned())?;
    let mut relations = HashMap::new();
    for relation in data
        .get("block_id_relations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let relation = relation.as_object().ok_or_else(|| {
            "feishu nested block create payload contains non-object block_id_relations entry"
                .to_owned()
        })?;
        let temporary_block_id =
            object_string(relation, "temporary_block_id").ok_or_else(|| {
                "feishu nested block create payload missing block_id_relations.temporary_block_id"
                    .to_owned()
            })?;
        let block_id = object_string(relation, "block_id").ok_or_else(|| {
            "feishu nested block create payload missing block_id_relations.block_id".to_owned()
        })?;
        if relations
            .insert(temporary_block_id.clone(), block_id.clone())
            .is_some()
        {
            return Err(format!(
                "feishu nested block create payload contains duplicate temporary_block_id `{temporary_block_id}`"
            ));
        }
    }
    Ok(relations)
}

fn parse_single_created_child_block<'a>(
    payload: &'a Value,
    expected_block_type: i64,
    label: &str,
) -> CliResult<&'a serde_json::Map<String, Value>> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu block create payload missing data object".to_owned())?;
    let created_children = data
        .get("children")
        .and_then(Value::as_array)
        .ok_or_else(|| "feishu block create payload missing data.children".to_owned())?;
    if created_children.len() != 1 {
        return Err(format!(
            "feishu {label} block create payload expected exactly one created child block, got {}",
            created_children.len(),
        ));
    }
    let child_block = created_children
        .first()
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu block create payload child must be a JSON object".to_owned())?;
    if child_block.get("block_type").and_then(Value::as_i64) != Some(expected_block_type) {
        return Err(format!(
            "feishu {label} block create payload child is not a {label} block"
        ));
    }
    Ok(child_block)
}

fn parse_created_container_child_ids(value: &Value, label: &str) -> CliResult<Vec<String>> {
    let child_block_ids = value
        .as_array()
        .ok_or_else(|| format!("feishu {label} block create payload child ids must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    format!(
                        "feishu {label} block create payload contains non-string or empty child id"
                    )
                })
        })
        .collect::<CliResult<Vec<_>>>()?;
    if child_block_ids.is_empty() {
        return Err(format!(
            "feishu {label} block create payload did not return any child ids"
        ));
    }
    Ok(child_block_ids)
}

fn parse_table_block_create_response(payload: &Value) -> CliResult<FeishuCreatedContainerBlock> {
    let table_block =
        parse_single_created_child_block(payload, FEISHU_DOC_BLOCK_TYPE_TABLE, "table")?;
    let block_id = object_string(table_block, "block_id")
        .ok_or_else(|| "feishu table block create payload missing child.block_id".to_owned())?;
    let child_block_ids = parse_created_container_child_ids(
        table_block
            .get("children")
            .or_else(|| {
                table_block
                    .get("table")
                    .and_then(|value| value.get("cells"))
            })
            .ok_or_else(|| {
                "feishu table block create payload missing child.children or child.table.cells"
                    .to_owned()
            })?,
        "table",
    )?;

    Ok(FeishuCreatedContainerBlock {
        block_id,
        child_block_ids,
    })
}

fn parse_callout_block_create_response(payload: &Value) -> CliResult<String> {
    let callout_block =
        parse_single_created_child_block(payload, FEISHU_DOC_BLOCK_TYPE_CALLOUT, "callout")?;
    object_string(callout_block, "block_id")
        .ok_or_else(|| "feishu callout block create payload missing child.block_id".to_owned())
}

fn parse_grid_block_create_response(payload: &Value) -> CliResult<FeishuCreatedContainerBlock> {
    let grid_block = parse_single_created_child_block(payload, FEISHU_DOC_BLOCK_TYPE_GRID, "grid")?;
    let block_id = object_string(grid_block, "block_id")
        .ok_or_else(|| "feishu grid block create payload missing child.block_id".to_owned())?;
    let child_block_ids = parse_created_container_child_ids(
        grid_block
            .get("children")
            .ok_or_else(|| "feishu grid block create payload missing child.children".to_owned())?,
        "grid",
    )?;

    Ok(FeishuCreatedContainerBlock {
        block_id,
        child_block_ids,
    })
}

#[cfg(test)]
fn partition_nested_block_batches_with_limit(
    blocks: &FeishuConvertedDocumentBlocks,
    limit: usize,
) -> CliResult<Vec<FeishuConvertedDocumentBlocks>> {
    if limit == 0 {
        return Err("feishu document nested block insertion limit must be positive".to_owned());
    }
    if blocks.first_level_block_ids.is_empty() {
        return Ok(Vec::new());
    }

    let descendants_by_id = descendants_by_id(&blocks.descendants)?;
    let mut assigned = HashSet::new();
    let mut subtrees = Vec::new();

    for root_id in &blocks.first_level_block_ids {
        let subtree_ids = collect_subtree_block_ids(root_id.as_str(), &descendants_by_id)?;
        if subtree_ids.len() > limit {
            return Err(format!(
                "feishu document top-level subtree `{root_id}` expands to {} blocks, exceeding the current nested block insertion limit of {limit} blocks per request",
                subtree_ids.len()
            ));
        }
        let subtree_id_set = subtree_ids.iter().cloned().collect::<HashSet<_>>();
        for block_id in &subtree_ids {
            if !assigned.insert(block_id.clone()) {
                return Err(format!(
                    "feishu document converted blocks contain duplicate subtree membership for block `{block_id}`"
                ));
            }
        }
        let subtree_descendants = blocks
            .descendants
            .iter()
            .filter(|value| {
                block_id_from_value(value).is_some_and(|block_id| subtree_id_set.contains(block_id))
            })
            .cloned()
            .collect::<Vec<_>>();
        if subtree_descendants.len() != subtree_ids.len() {
            return Err(format!(
                "feishu document converted subtree `{root_id}` could not be reconstructed from descendant order"
            ));
        }
        subtrees.push(FeishuConvertedDocumentBlocks {
            first_level_block_ids: vec![root_id.clone()],
            descendants: subtree_descendants,
        });
    }

    if assigned.len() != descendants_by_id.len() {
        let unreachable = descendants_by_id
            .keys()
            .find(|block_id| !assigned.contains((*block_id).as_str()))
            .cloned()
            .unwrap_or_else(|| "unknown".to_owned());
        return Err(format!(
            "feishu document converted blocks contain unreachable descendant `{unreachable}` outside first_level_block_ids"
        ));
    }

    let mut batches = Vec::new();
    let mut current_roots = Vec::new();
    let mut current_descendants = Vec::new();

    for subtree in subtrees {
        if !current_descendants.is_empty()
            && current_descendants.len() + subtree.descendants.len() > limit
        {
            batches.push(FeishuConvertedDocumentBlocks {
                first_level_block_ids: std::mem::take(&mut current_roots),
                descendants: std::mem::take(&mut current_descendants),
            });
        }

        current_roots.extend(subtree.first_level_block_ids);
        current_descendants.extend(subtree.descendants);
    }

    if !current_descendants.is_empty() {
        batches.push(FeishuConvertedDocumentBlocks {
            first_level_block_ids: current_roots,
            descendants: current_descendants,
        });
    }

    Ok(batches)
}

fn require_content_type(value: &str) -> CliResult<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "markdown" => Ok("markdown"),
        "html" => Ok("html"),
        other => Err(format!(
            "unsupported feishu document content_type `{other}`; expected `markdown` or `html`"
        )),
    }
}

fn build_subtree_blocks(
    root_id: &str,
    descendants_by_id: &HashMap<String, Value>,
    ordered_descendants: &[Value],
) -> CliResult<FeishuConvertedDocumentBlocks> {
    let subtree_ids = collect_subtree_block_ids(root_id, descendants_by_id)?;
    let subtree_id_set = subtree_ids.iter().cloned().collect::<HashSet<_>>();
    let descendants = ordered_descendants
        .iter()
        .filter(|value| {
            block_id_from_value(value).is_some_and(|block_id| subtree_id_set.contains(block_id))
        })
        .cloned()
        .collect::<Vec<_>>();
    if descendants.len() != subtree_ids.len() {
        return Err(format!(
            "feishu document converted subtree `{root_id}` could not be reconstructed from descendant order"
        ));
    }
    Ok(FeishuConvertedDocumentBlocks {
        first_level_block_ids: vec![root_id.to_owned()],
        descendants,
    })
}

fn child_block_ids_from_block(block: &Value) -> CliResult<Vec<String>> {
    let child_ids =
        block
            .get("children")
            .and_then(Value::as_array)
            .map(|children| {
                children
                    .iter()
                    .map(|child| {
                        child.as_str().map(str::trim).filter(|value| !value.is_empty()).ok_or_else(
                        || {
                            "feishu document converted block contains non-string or empty child id"
                                .to_owned()
                        },
                    )
                    })
                    .collect::<CliResult<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
    Ok(child_ids.into_iter().map(ToOwned::to_owned).collect())
}

fn clone_block_with_children(block: &Value, child_ids: &[String]) -> CliResult<Value> {
    let mut cloned = block
        .as_object()
        .cloned()
        .ok_or_else(|| "feishu document converted block must be a JSON object".to_owned())?;
    cloned.insert(
        "children".to_owned(),
        Value::Array(child_ids.iter().cloned().map(Value::String).collect()),
    );
    Ok(Value::Object(cloned))
}

fn container_block_create_body(
    block: &Value,
    expected_block_type: i64,
    payload_key: &str,
    label: &str,
) -> CliResult<Value> {
    let object = block
        .as_object()
        .ok_or_else(|| "feishu document converted block must be a JSON object".to_owned())?;
    if object.get("block_type").and_then(Value::as_i64) != Some(expected_block_type) {
        return Err(format!(
            "feishu {label} block create body requires a {label} block"
        ));
    }
    let payload = object
        .get(payload_key)
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| format!("feishu {label} block create body missing {payload_key} payload"))?;
    Ok(serde_json::json!({
        "block_type": expected_block_type,
        payload_key: payload,
    }))
}

fn table_block_create_body(block: &Value) -> CliResult<Value> {
    container_block_create_body(block, FEISHU_DOC_BLOCK_TYPE_TABLE, "table", "table")
}

fn callout_block_create_body(block: &Value) -> CliResult<Value> {
    container_block_create_body(block, FEISHU_DOC_BLOCK_TYPE_CALLOUT, "callout", "callout")
}

fn grid_block_create_body(block: &Value) -> CliResult<Value> {
    container_block_create_body(block, FEISHU_DOC_BLOCK_TYPE_GRID, "grid", "grid")
}

fn ensure_block_supports_deferred_child_insertion(
    root_id: &str,
    block: &Value,
    limit: usize,
) -> CliResult<()> {
    if block.get("block_type").and_then(Value::as_i64) == Some(FEISHU_DOC_BLOCK_TYPE_TABLE) {
        return Err(format!(
            "feishu document subtree `{root_id}` expands beyond the nested block insertion limit of {limit} blocks and cannot currently be split because table blocks require their cell structure to be inserted atomically"
        ));
    }
    Ok(())
}

fn insert_child_subtrees<'a>(
    client: &'a FeishuClient,
    user_access_token: &'a str,
    document_id: &'a str,
    parent_block_id: String,
    child_root_ids: Vec<String>,
    descendants_by_id: &'a HashMap<String, Value>,
    ordered_descendants: &'a [Value],
    limit: usize,
) -> Pin<Box<dyn Future<Output = CliResult<FeishuNestedBlockInsertSummary>> + Send + 'a>> {
    Box::pin(async move {
        let mut summary = FeishuNestedBlockInsertSummary {
            inserted_block_count: 0,
            batch_count: 0,
        };
        let mut current_roots = Vec::new();
        let mut current_descendants = Vec::new();

        for child_root_id in child_root_ids {
            let subtree = build_subtree_blocks(
                child_root_id.as_str(),
                descendants_by_id,
                ordered_descendants,
            )?;
            if subtree.descendants.len() > limit {
                let flushed = flush_nested_block_batch(
                    client,
                    user_access_token,
                    document_id,
                    parent_block_id.as_str(),
                    &mut current_roots,
                    &mut current_descendants,
                )
                .await?;
                accumulate_insert_summary(&mut summary, flushed);

                let inserted = insert_oversized_subtree(
                    client,
                    user_access_token,
                    document_id,
                    parent_block_id.as_str(),
                    child_root_id.as_str(),
                    descendants_by_id,
                    ordered_descendants,
                    limit,
                )
                .await?;
                accumulate_insert_summary(&mut summary, inserted);
                continue;
            }

            if !current_descendants.is_empty()
                && current_descendants.len() + subtree.descendants.len() > limit
            {
                let flushed = flush_nested_block_batch(
                    client,
                    user_access_token,
                    document_id,
                    parent_block_id.as_str(),
                    &mut current_roots,
                    &mut current_descendants,
                )
                .await?;
                accumulate_insert_summary(&mut summary, flushed);
            }

            current_roots.extend(subtree.first_level_block_ids);
            current_descendants.extend(subtree.descendants);
        }

        let flushed = flush_nested_block_batch(
            client,
            user_access_token,
            document_id,
            parent_block_id.as_str(),
            &mut current_roots,
            &mut current_descendants,
        )
        .await?;
        accumulate_insert_summary(&mut summary, flushed);

        Ok(summary)
    })
}

async fn insert_oversized_subtree(
    client: &FeishuClient,
    user_access_token: &str,
    document_id: &str,
    parent_block_id: &str,
    root_id: &str,
    descendants_by_id: &HashMap<String, Value>,
    ordered_descendants: &[Value],
    limit: usize,
) -> CliResult<FeishuNestedBlockInsertSummary> {
    let root_block = descendants_by_id.get(root_id).ok_or_else(|| {
        format!("feishu document converted subtree references missing descendant block `{root_id}`")
    })?;
    match root_block.get("block_type").and_then(Value::as_i64) {
        Some(FEISHU_DOC_BLOCK_TYPE_TABLE) => {
            return insert_oversized_table_subtree(
                client,
                user_access_token,
                document_id,
                parent_block_id,
                root_id,
                root_block,
                descendants_by_id,
                ordered_descendants,
                limit,
            )
            .await;
        }
        Some(FEISHU_DOC_BLOCK_TYPE_CALLOUT) => {
            return insert_oversized_callout_subtree(
                client,
                user_access_token,
                document_id,
                parent_block_id,
                root_block,
                descendants_by_id,
                ordered_descendants,
                limit,
            )
            .await;
        }
        Some(FEISHU_DOC_BLOCK_TYPE_GRID) => {
            return insert_oversized_grid_subtree(
                client,
                user_access_token,
                document_id,
                parent_block_id,
                root_id,
                root_block,
                descendants_by_id,
                ordered_descendants,
                limit,
            )
            .await;
        }
        _ => {}
    }
    ensure_block_supports_deferred_child_insertion(root_id, root_block, limit)?;
    let child_root_ids = child_block_ids_from_block(root_block)?;
    let root_only_block = clone_block_with_children(root_block, &[])?;
    let relations = post_nested_block_batch(
        client,
        user_access_token,
        document_id,
        parent_block_id,
        &FeishuConvertedDocumentBlocks {
            first_level_block_ids: vec![root_id.to_owned()],
            descendants: vec![root_only_block],
        },
    )
    .await?;
    let real_root_id = relations.get(root_id).cloned().ok_or_else(|| {
        format!(
            "feishu nested block create response did not return a real block_id mapping for temporary block `{root_id}`"
        )
    })?;

    let mut summary = FeishuNestedBlockInsertSummary {
        inserted_block_count: 1,
        batch_count: 1,
    };
    if !child_root_ids.is_empty() {
        let child_summary = insert_child_subtrees(
            client,
            user_access_token,
            document_id,
            real_root_id,
            child_root_ids,
            descendants_by_id,
            ordered_descendants,
            limit,
        )
        .await?;
        accumulate_insert_summary(&mut summary, child_summary);
    }
    Ok(summary)
}

async fn insert_oversized_table_subtree(
    client: &FeishuClient,
    user_access_token: &str,
    document_id: &str,
    parent_block_id: &str,
    root_id: &str,
    root_block: &Value,
    descendants_by_id: &HashMap<String, Value>,
    ordered_descendants: &[Value],
    limit: usize,
) -> CliResult<FeishuNestedBlockInsertSummary> {
    let temporary_cell_ids = child_block_ids_from_block(root_block)?;
    let created_table = create_table_block(
        client,
        user_access_token,
        document_id,
        parent_block_id,
        root_block,
    )
    .await?;
    if created_table.child_block_ids.len() != temporary_cell_ids.len() {
        return Err(format!(
            "feishu oversized table subtree `{root_id}` expected {} created table cells, got {}",
            temporary_cell_ids.len(),
            created_table.child_block_ids.len()
        ));
    }

    let mut summary = FeishuNestedBlockInsertSummary {
        inserted_block_count: 1 + created_table.child_block_ids.len(),
        batch_count: 1,
    };

    for (temporary_cell_id, actual_cell_id) in temporary_cell_ids
        .iter()
        .zip(created_table.child_block_ids.iter())
    {
        let cell_block = descendants_by_id.get(temporary_cell_id.as_str()).ok_or_else(|| {
            format!(
                "feishu oversized table subtree `{root_id}` references missing table cell `{temporary_cell_id}`"
            )
        })?;
        if cell_block.get("block_type").and_then(Value::as_i64)
            != Some(FEISHU_DOC_BLOCK_TYPE_TABLE_CELL)
        {
            return Err(format!(
                "feishu oversized table subtree `{root_id}` expected child `{temporary_cell_id}` to be a table cell block"
            ));
        }
        let child_root_ids = child_block_ids_from_block(cell_block)?;
        if child_root_ids.is_empty() {
            continue;
        }
        let child_summary = insert_child_subtrees(
            client,
            user_access_token,
            document_id,
            actual_cell_id.clone(),
            child_root_ids,
            descendants_by_id,
            ordered_descendants,
            limit,
        )
        .await?;
        accumulate_insert_summary(&mut summary, child_summary);
    }

    Ok(summary)
}

async fn insert_oversized_callout_subtree(
    client: &FeishuClient,
    user_access_token: &str,
    document_id: &str,
    parent_block_id: &str,
    root_block: &Value,
    descendants_by_id: &HashMap<String, Value>,
    ordered_descendants: &[Value],
    limit: usize,
) -> CliResult<FeishuNestedBlockInsertSummary> {
    let real_callout_id = create_callout_block(
        client,
        user_access_token,
        document_id,
        parent_block_id,
        root_block,
    )
    .await?;
    let child_root_ids = child_block_ids_from_block(root_block)?;

    let mut summary = FeishuNestedBlockInsertSummary {
        inserted_block_count: 1,
        batch_count: 1,
    };
    if !child_root_ids.is_empty() {
        let child_summary = insert_child_subtrees(
            client,
            user_access_token,
            document_id,
            real_callout_id,
            child_root_ids,
            descendants_by_id,
            ordered_descendants,
            limit,
        )
        .await?;
        accumulate_insert_summary(&mut summary, child_summary);
    }
    Ok(summary)
}

async fn insert_oversized_grid_subtree(
    client: &FeishuClient,
    user_access_token: &str,
    document_id: &str,
    parent_block_id: &str,
    root_id: &str,
    root_block: &Value,
    descendants_by_id: &HashMap<String, Value>,
    ordered_descendants: &[Value],
    limit: usize,
) -> CliResult<FeishuNestedBlockInsertSummary> {
    let temporary_column_ids = child_block_ids_from_block(root_block)?;
    let created_grid = create_grid_block(
        client,
        user_access_token,
        document_id,
        parent_block_id,
        root_block,
    )
    .await?;
    if created_grid.child_block_ids.len() != temporary_column_ids.len() {
        return Err(format!(
            "feishu oversized grid subtree `{root_id}` expected {} created grid columns, got {}",
            temporary_column_ids.len(),
            created_grid.child_block_ids.len()
        ));
    }

    let mut summary = FeishuNestedBlockInsertSummary {
        inserted_block_count: 1 + created_grid.child_block_ids.len(),
        batch_count: 1,
    };

    for (temporary_column_id, actual_column_id) in temporary_column_ids
        .iter()
        .zip(created_grid.child_block_ids.iter())
    {
        let column_block = descendants_by_id
            .get(temporary_column_id.as_str())
            .ok_or_else(|| {
                format!(
                    "feishu oversized grid subtree `{root_id}` references missing grid column `{temporary_column_id}`"
                )
            })?;
        if column_block.get("block_type").and_then(Value::as_i64)
            != Some(FEISHU_DOC_BLOCK_TYPE_GRID_COLUMN)
        {
            return Err(format!(
                "feishu oversized grid subtree `{root_id}` expected child `{temporary_column_id}` to be a grid column block"
            ));
        }
        let child_root_ids = child_block_ids_from_block(column_block)?;
        if child_root_ids.is_empty() {
            continue;
        }
        let child_summary = insert_child_subtrees(
            client,
            user_access_token,
            document_id,
            actual_column_id.clone(),
            child_root_ids,
            descendants_by_id,
            ordered_descendants,
            limit,
        )
        .await?;
        accumulate_insert_summary(&mut summary, child_summary);
    }

    Ok(summary)
}

async fn flush_nested_block_batch(
    client: &FeishuClient,
    user_access_token: &str,
    document_id: &str,
    parent_block_id: &str,
    current_roots: &mut Vec<String>,
    current_descendants: &mut Vec<Value>,
) -> CliResult<FeishuNestedBlockInsertSummary> {
    if current_descendants.is_empty() {
        return Ok(FeishuNestedBlockInsertSummary {
            inserted_block_count: 0,
            batch_count: 0,
        });
    }

    let batch = FeishuConvertedDocumentBlocks {
        first_level_block_ids: std::mem::take(current_roots),
        descendants: std::mem::take(current_descendants),
    };
    post_nested_block_batch(
        client,
        user_access_token,
        document_id,
        parent_block_id,
        &batch,
    )
    .await?;
    Ok(FeishuNestedBlockInsertSummary {
        inserted_block_count: batch.descendants.len(),
        batch_count: 1,
    })
}

async fn post_nested_block_batch(
    client: &FeishuClient,
    user_access_token: &str,
    document_id: &str,
    parent_block_id: &str,
    batch: &FeishuConvertedDocumentBlocks,
) -> CliResult<HashMap<String, String>> {
    let payload = client
        .post_json(
            format!(
                "/open-apis/docx/v1/documents/{document_id}/blocks/{parent_block_id}/descendant"
            )
            .as_str(),
            Some(user_access_token),
            &[("document_revision_id".to_owned(), "-1".to_owned())],
            &serde_json::json!({
                "children_id": batch.first_level_block_ids,
                "descendants": batch.descendants,
                "index": -1,
            }),
        )
        .await?;
    parse_nested_blocks_create_response(&payload)
}

async fn create_single_child_block(
    client: &FeishuClient,
    user_access_token: &str,
    document_id: &str,
    parent_block_id: &str,
    block_body: Value,
) -> CliResult<Value> {
    client
        .post_json(
            format!("/open-apis/docx/v1/documents/{document_id}/blocks/{parent_block_id}/children")
                .as_str(),
            Some(user_access_token),
            &[("document_revision_id".to_owned(), "-1".to_owned())],
            &serde_json::json!({
                "children": [block_body],
                "index": -1,
            }),
        )
        .await
}

async fn create_table_block(
    client: &FeishuClient,
    user_access_token: &str,
    document_id: &str,
    parent_block_id: &str,
    table_block: &Value,
) -> CliResult<FeishuCreatedContainerBlock> {
    let payload = create_single_child_block(
        client,
        user_access_token,
        document_id,
        parent_block_id,
        table_block_create_body(table_block)?,
    )
    .await?;
    parse_table_block_create_response(&payload)
}

async fn create_callout_block(
    client: &FeishuClient,
    user_access_token: &str,
    document_id: &str,
    parent_block_id: &str,
    callout_block: &Value,
) -> CliResult<String> {
    let payload = create_single_child_block(
        client,
        user_access_token,
        document_id,
        parent_block_id,
        callout_block_create_body(callout_block)?,
    )
    .await?;
    parse_callout_block_create_response(&payload)
}

async fn create_grid_block(
    client: &FeishuClient,
    user_access_token: &str,
    document_id: &str,
    parent_block_id: &str,
    grid_block: &Value,
) -> CliResult<FeishuCreatedContainerBlock> {
    let payload = create_single_child_block(
        client,
        user_access_token,
        document_id,
        parent_block_id,
        grid_block_create_body(grid_block)?,
    )
    .await?;
    parse_grid_block_create_response(&payload)
}

fn accumulate_insert_summary(
    total: &mut FeishuNestedBlockInsertSummary,
    added: FeishuNestedBlockInsertSummary,
) {
    total.inserted_block_count += added.inserted_block_count;
    total.batch_count += added.batch_count;
}

fn strip_table_merge_info(blocks: &mut [Value]) {
    for block in blocks {
        strip_table_merge_info_from_block(block);
    }
}

fn strip_table_merge_info_from_block(block: &mut Value) {
    let Some(object) = block.as_object_mut() else {
        return;
    };
    if let Some(table) = object.get_mut("table").and_then(Value::as_object_mut)
        && let Some(property) = table.get_mut("property").and_then(Value::as_object_mut)
    {
        property.remove("merge_info");
    }
    if let Some(children) = object.get_mut("children").and_then(Value::as_array_mut) {
        strip_table_merge_info(children);
    }
}

fn object_string(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn trimmed_opt(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn descendants_by_id(descendants: &[Value]) -> CliResult<HashMap<String, Value>> {
    let mut result = HashMap::with_capacity(descendants.len());
    for block in descendants {
        let block_id = block_id_from_value(block).ok_or_else(|| {
            "feishu document converted block is missing non-empty block_id".to_owned()
        })?;
        if result.insert(block_id.to_owned(), block.clone()).is_some() {
            return Err(format!(
                "feishu document converted blocks contain duplicate block_id `{block_id}`"
            ));
        }
    }
    Ok(result)
}

fn collect_subtree_block_ids(
    root_id: &str,
    descendants_by_id: &HashMap<String, Value>,
) -> CliResult<Vec<String>> {
    let mut ordered = Vec::new();
    let mut stack = vec![root_id.to_owned()];
    let mut visited = HashSet::new();

    while let Some(block_id) = stack.pop() {
        if !visited.insert(block_id.clone()) {
            continue;
        }
        let block = descendants_by_id.get(block_id.as_str()).ok_or_else(|| {
            format!(
                "feishu document converted subtree references missing descendant block `{block_id}`"
            )
        })?;
        ordered.push(block_id.clone());

        let child_ids = block
            .get("children")
            .and_then(Value::as_array)
            .map(|children| {
                children
                    .iter()
                    .rev()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        stack.extend(child_ids);
    }

    Ok(ordered)
}

fn block_id_from_value(value: &Value) -> Option<&str> {
    value
        .get("block_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_document_id_supports_full_docx_url() {
        let id = extract_document_id("https://example.feishu.cn/docx/doxbcmEtbFrbbq10nPNu8gabcef");
        assert_eq!(id.as_deref(), Some("doxbcmEtbFrbbq10nPNu8gabcef"));
    }

    #[test]
    fn parse_raw_content_response_returns_plain_text() {
        let payload = serde_json::json!({
            "code": 0,
            "msg": "success",
            "data": { "content": "hello from docs" }
        });

        let doc =
            parse_raw_content_response("doxbcmEtbFrbbq10nPNu8gabcef", &payload).expect("parse doc");

        assert_eq!(doc.document_id, "doxbcmEtbFrbbq10nPNu8gabcef");
        assert_eq!(doc.content, "hello from docs");
    }

    #[test]
    fn parse_document_create_response_reads_document_metadata() {
        let payload = serde_json::json!({
            "code": 0,
            "msg": "success",
            "data": {
                "document": {
                    "document_id": "doxcnCreated",
                    "revision_id": 1,
                    "title": "Release Plan"
                }
            }
        });

        let document = parse_document_create_response(&payload).expect("parse document create");

        assert_eq!(document.document_id, "doxcnCreated");
        assert_eq!(document.revision_id, Some(1));
        assert_eq!(document.title.as_deref(), Some("Release Plan"));
        assert_eq!(
            document.url.as_deref(),
            Some("https://open.feishu.cn/docx/doxcnCreated")
        );
    }

    #[test]
    fn parse_convert_blocks_response_strips_table_merge_info() {
        let payload = serde_json::json!({
            "code": 0,
            "msg": "success",
            "data": {
                "first_level_block_ids": ["table_1"],
                "blocks": [
                    {
                        "block_id": "table_1",
                        "block_type": 31,
                        "table": {
                            "property": {
                                "row_size": 1,
                                "column_size": 1,
                                "merge_info": [{"row_span": 1, "col_span": 1}]
                            }
                        },
                        "children": []
                    }
                ]
            }
        });

        let converted = parse_convert_blocks_response(&payload).expect("parse converted blocks");

        assert_eq!(converted.first_level_block_ids, vec!["table_1".to_owned()]);
        assert_eq!(
            converted.descendants[0]["table"]["property"].get("merge_info"),
            None
        );
    }

    #[test]
    fn parse_nested_blocks_create_response_reads_block_id_relations() {
        let payload = serde_json::json!({
            "code": 0,
            "msg": "success",
            "data": {
                "block_id_relations": [
                    {
                        "temporary_block_id": "tmp-root",
                        "block_id": "blk_real_root"
                    }
                ]
            }
        });

        let relations =
            parse_nested_blocks_create_response(&payload).expect("parse nested block relations");

        assert_eq!(
            relations.get("tmp-root").map(String::as_str),
            Some("blk_real_root")
        );
    }

    #[test]
    fn parse_table_block_create_response_reads_created_table_and_cells() {
        let payload = serde_json::json!({
            "code": 0,
            "msg": "success",
            "data": {
                "children": [{
                    "block_id": "blk_real_table",
                    "block_type": 31,
                    "children": ["blk_real_cell_1", "blk_real_cell_2"],
                    "table": {
                        "cells": ["blk_real_cell_1", "blk_real_cell_2"],
                        "property": {
                            "row_size": 1,
                            "column_size": 2
                        }
                    }
                }]
            }
        });

        let table = parse_table_block_create_response(&payload).expect("parse created table block");

        assert_eq!(table.block_id, "blk_real_table");
        assert_eq!(
            table.child_block_ids,
            vec!["blk_real_cell_1".to_owned(), "blk_real_cell_2".to_owned()]
        );
    }

    #[test]
    fn partition_nested_block_batches_splits_on_top_level_subtree_boundaries() {
        let blocks = FeishuConvertedDocumentBlocks {
            first_level_block_ids: vec!["root_a".to_owned(), "root_b".to_owned()],
            descendants: vec![
                serde_json::json!({
                    "block_id": "root_a",
                    "children": ["child_a"]
                }),
                serde_json::json!({
                    "block_id": "child_a",
                    "children": []
                }),
                serde_json::json!({
                    "block_id": "root_b",
                    "children": ["child_b"]
                }),
                serde_json::json!({
                    "block_id": "child_b",
                    "children": []
                }),
            ],
        };

        let batches =
            partition_nested_block_batches_with_limit(&blocks, 3).expect("partition batches");

        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].first_level_block_ids, vec!["root_a".to_owned()]);
        assert_eq!(
            batches[0]
                .descendants
                .iter()
                .filter_map(|value| value.get("block_id").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["root_a", "child_a"]
        );
        assert_eq!(batches[1].first_level_block_ids, vec!["root_b".to_owned()]);
        assert_eq!(
            batches[1]
                .descendants
                .iter()
                .filter_map(|value| value.get("block_id").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["root_b", "child_b"]
        );
    }

    #[test]
    fn partition_nested_block_batches_rejects_single_subtree_larger_than_limit() {
        let blocks = FeishuConvertedDocumentBlocks {
            first_level_block_ids: vec!["root_a".to_owned()],
            descendants: vec![
                serde_json::json!({
                    "block_id": "root_a",
                    "children": ["child_a", "child_b"]
                }),
                serde_json::json!({
                    "block_id": "child_a",
                    "children": []
                }),
                serde_json::json!({
                    "block_id": "child_b",
                    "children": []
                }),
            ],
        };

        let error = partition_nested_block_batches_with_limit(&blocks, 2)
            .expect_err("single subtree over limit should fail");

        assert!(error.contains("top-level subtree"));
        assert!(error.contains("root_a"));
    }

    #[test]
    fn ensure_block_supports_deferred_child_insertion_rejects_table_blocks() {
        let block = serde_json::json!({
            "block_id": "table_root",
            "block_type": 31,
            "children": ["cell_1"]
        });

        let error = ensure_block_supports_deferred_child_insertion("table_root", &block, 1000)
            .expect_err("table block should be rejected for deferred insertion");

        assert!(error.contains("table_root"));
        assert!(error.contains("atomically"));
    }
}
