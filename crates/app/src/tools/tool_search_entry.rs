use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::query_support::{
    SearchSignalSet, extract_concepts_and_categories, identifier_phrase_variant,
};

#[derive(Debug, Clone)]
pub(crate) struct SearchableToolEntry {
    pub(crate) tool_id: String,
    pub(crate) canonical_name: String,
    pub(crate) summary: String,
    pub(crate) search_hint: String,
    pub(crate) argument_hint: String,
    pub(crate) required_fields: Vec<String>,
    pub(crate) required_field_groups: Vec<Vec<String>>,
    pub(crate) schema_preview: Value,
    pub(crate) tags: Vec<String>,
    pub(crate) surface_id: Option<String>,
    pub(crate) usage_guidance: Option<String>,
    pub(crate) requires_lease: bool,
    pub(super) search_document: SearchDocument,
}

#[derive(Debug, Clone)]
pub(super) struct SearchDocument {
    pub(super) name: SearchSignalSet,
    pub(super) summary: SearchSignalSet,
    pub(super) arguments: SearchSignalSet,
    pub(super) schema: SearchSignalSet,
    pub(super) tags: SearchSignalSet,
    pub(super) concepts: BTreeSet<String>,
    pub(super) categories: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct SchemaArgumentField {
    name: String,
    schema_type: String,
    required: bool,
    preferred_index: usize,
}

impl SchemaArgumentField {
    fn format(self) -> String {
        let suffix = if self.required { "" } else { "?" };
        format!("{}{}:{}", self.name, suffix, self.schema_type)
    }
}

impl SearchDocument {
    pub(super) fn new(
        name_fragments: Vec<String>,
        summary_fragments: Vec<String>,
        argument_fragments: Vec<String>,
        schema_fragments: Vec<String>,
        tag_fragments: Vec<String>,
    ) -> Self {
        let name = SearchSignalSet::from_fragments(&name_fragments);
        let summary = SearchSignalSet::from_fragments(&summary_fragments);
        let arguments = SearchSignalSet::from_fragments(&argument_fragments);
        let schema = SearchSignalSet::from_fragments(&schema_fragments);
        let tags = SearchSignalSet::from_fragments(&tag_fragments);

        let mut all_fragments = Vec::new();
        all_fragments.extend(name_fragments);
        all_fragments.extend(summary_fragments);
        all_fragments.extend(argument_fragments);
        all_fragments.extend(schema_fragments);
        all_fragments.extend(tag_fragments);

        let all_signals = SearchSignalSet::from_fragments(&all_fragments);
        let (concepts, categories) = extract_concepts_and_categories(&all_signals);

        Self {
            name,
            summary,
            arguments,
            schema,
            tags,
            concepts,
            categories,
        }
    }
}

pub(crate) fn searchable_entry_from_provider_definition(
    canonical_name: &str,
    provider_name: &str,
    aliases: &[&str],
    tool_id: String,
    summary: String,
    search_hint: String,
    parameters: &Value,
    preferred_parameter_order: &[(&str, &str)],
    argument_hint_override: Option<String>,
    tags: Vec<String>,
    surface_id: Option<String>,
    usage_guidance: Option<String>,
    requires_lease: bool,
) -> SearchableToolEntry {
    let required_fields = schema_required_fields(parameters);
    let required_field_groups = schema_required_field_groups(parameters);
    let required_field_groups =
        default_required_field_groups(&required_fields, required_field_groups);
    let schema_argument_hint =
        search_argument_hint_from_provider_definition(parameters, preferred_parameter_order);
    let argument_hint = argument_hint_override.unwrap_or(schema_argument_hint);
    let schema_preview = build_schema_preview(&required_fields, &required_field_groups, parameters);

    let name_fragments = build_name_fragments(canonical_name, provider_name, aliases);
    let mut summary_fragments = vec![summary.clone()];

    if search_hint.trim() != summary.trim() {
        summary_fragments.push(search_hint.clone());
    }

    let argument_fragments = build_argument_fragments(
        argument_hint.as_str(),
        &required_fields,
        &required_field_groups,
    );
    let schema_fragments = collect_schema_search_terms(parameters);
    let tag_fragments = tags.clone();
    let search_document = SearchDocument::new(
        name_fragments,
        summary_fragments,
        argument_fragments,
        schema_fragments,
        tag_fragments,
    );

    let (surface_id, usage_guidance) =
        enrich_discovery_prompt_metadata(canonical_name, surface_id, usage_guidance);

    SearchableToolEntry {
        tool_id,
        canonical_name: canonical_name.to_owned(),
        summary,
        search_hint,
        argument_hint,
        required_fields,
        required_field_groups,
        schema_preview,
        tags,
        surface_id,
        usage_guidance,
        requires_lease,
        search_document,
    }
}

fn enrich_discovery_prompt_metadata(
    canonical_name: &str,
    surface_id: Option<String>,
    usage_guidance: Option<String>,
) -> (Option<String>, Option<String>) {
    (
        surface_id.or_else(|| discovery_surface_id(canonical_name)),
        usage_guidance.or_else(|| discovery_usage_guidance(canonical_name)),
    )
}

fn discovery_surface_id(canonical_name: &str) -> Option<String> {
    super::super::tool_surface::tool_surface_id_for_name(canonical_name).map(str::to_owned)
}

fn discovery_usage_guidance(canonical_name: &str) -> Option<String> {
    super::super::tool_surface::tool_surface_usage_guidance(canonical_name).map(str::to_owned)
}

fn build_schema_preview(
    required_fields: &[String],
    required_field_groups: &[Vec<String>],
    parameters: &Value,
) -> Value {
    let properties = parameters
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut required_field_names = BTreeSet::new();

    for required_field in required_fields {
        required_field_names.insert(required_field.clone());
    }

    for group in required_field_groups {
        for field_name in group {
            required_field_names.insert(field_name.clone());
        }
    }

    let mut common_optional_fields = Vec::new();

    for field_name in properties.keys() {
        if !required_field_names.contains(field_name) {
            common_optional_fields.push(field_name.clone());
        }
    }

    json!({
        "required_fields": required_fields,
        "required_field_groups": required_field_groups,
        "common_optional_fields": common_optional_fields,
    })
}

pub(crate) fn searchable_entry_from_manual_definition(
    canonical_name: &str,
    summary: &str,
    argument_hint: &str,
    required_fields: Vec<String>,
    required_field_groups: Vec<Vec<String>>,
    tags: Vec<String>,
) -> SearchableToolEntry {
    let tool_id = super::super::tool_surface::discovery_tool_name_for_tool_name(canonical_name);
    let mut name_fragments = vec![canonical_name.to_owned(), tool_id.clone()];
    let canonical_name_variant = identifier_phrase_variant(canonical_name);
    if canonical_name_variant != canonical_name {
        name_fragments.push(canonical_name_variant);
    }

    let summary_text = summary.to_owned();
    let search_hint = summary.to_owned();
    let argument_hint_text = argument_hint.to_owned();
    let argument_fragments =
        build_argument_fragments(argument_hint, &required_fields, &required_field_groups);
    let schema_preview = json!({
        "required_fields": required_fields,
        "required_field_groups": required_field_groups,
        "common_optional_fields": []
    });

    let mut schema_fragments = required_fields.clone();
    for required_field_group in &required_field_groups {
        schema_fragments.push(required_field_group.join(" "));
    }

    let search_document = SearchDocument::new(
        name_fragments,
        vec![summary_text.clone()],
        argument_fragments,
        schema_fragments,
        tags.clone(),
    );

    SearchableToolEntry {
        tool_id,
        canonical_name: canonical_name.to_owned(),
        summary: summary_text,
        search_hint,
        argument_hint: argument_hint_text,
        required_fields,
        required_field_groups,
        schema_preview,
        tags,
        surface_id: discovery_surface_id(canonical_name),
        usage_guidance: discovery_usage_guidance(canonical_name),
        requires_lease: true,
        search_document,
    }
}

pub(crate) fn search_argument_hint_from_provider_definition(
    parameters: &Value,
    preferred_parameter_order: &[(&str, &str)],
) -> String {
    let Some(properties) = parameters.get("properties").and_then(Value::as_object) else {
        return String::new();
    };

    let required = schema_required_fields(parameters)
        .into_iter()
        .collect::<BTreeSet<_>>();

    let mut fields = Vec::new();
    for (name, schema) in properties {
        fields.push(SchemaArgumentField {
            name: name.to_owned(),
            schema_type: schema_argument_type(schema),
            required: required.contains(name.as_str()),
            preferred_index: preferred_parameter_index(name.as_str(), preferred_parameter_order),
        });
    }

    fields.sort_by(|left, right| {
        let left_required_rank = if left.required { 0usize } else { 1usize };
        let right_required_rank = if right.required { 0usize } else { 1usize };

        left_required_rank
            .cmp(&right_required_rank)
            .then_with(|| left.preferred_index.cmp(&right.preferred_index))
            .then_with(|| left.name.cmp(&right.name))
    });

    let total_field_count = fields.len();
    let compact_fields = compact_argument_hint_fields(fields);
    let omitted_field_count = total_field_count.saturating_sub(compact_fields.len());
    let mut fragments = compact_fields
        .into_iter()
        .map(|field| field.format())
        .collect::<Vec<_>>();

    if omitted_field_count > 0 {
        fragments.push(format!("+{omitted_field_count} more"));
    }

    fragments.join(",")
}

pub(crate) fn schema_required_fields(parameters: &Value) -> Vec<String> {
    parameters
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn schema_required_field_groups(parameters: &Value) -> Vec<Vec<String>> {
    let root_required_fields = schema_required_fields(parameters);
    let mut groups = Vec::new();

    for key in ["anyOf", "oneOf"] {
        let Some(options) = parameters.get(key).and_then(Value::as_array) else {
            continue;
        };

        for schema in options {
            let branch_required_fields = schema_required_fields(schema);
            let merged_required_fields = merge_required_field_group(
                root_required_fields.as_slice(),
                branch_required_fields.as_slice(),
            );
            if !groups.iter().any(|group| group == &merged_required_fields) {
                groups.push(merged_required_fields);
            }
        }
    }

    groups
}

fn merge_required_field_group(
    root_required_fields: &[String],
    branch_required_fields: &[String],
) -> Vec<String> {
    let mut merged_required_fields = root_required_fields.to_vec();

    for field_name in branch_required_fields {
        if !merged_required_fields
            .iter()
            .any(|existing_name| existing_name == field_name)
        {
            merged_required_fields.push(field_name.clone());
        }
    }

    merged_required_fields
}

pub(crate) fn default_required_field_groups(
    required_fields: &[String],
    mut required_field_groups: Vec<Vec<String>>,
) -> Vec<Vec<String>> {
    if required_field_groups.is_empty() && !required_fields.is_empty() {
        required_field_groups.push(required_fields.to_vec());
    }

    required_field_groups
}

fn build_name_fragments(
    canonical_name: &str,
    provider_name: &str,
    aliases: &[&str],
) -> Vec<String> {
    let canonical_name_fragment = canonical_name.to_owned();
    let canonical_name_variant = identifier_phrase_variant(canonical_name);
    let provider_name_fragment = provider_name.to_owned();
    let provider_name_variant = identifier_phrase_variant(provider_name);
    let mut fragments = Vec::from([
        canonical_name_fragment,
        canonical_name_variant,
        provider_name_fragment,
        provider_name_variant,
    ]);

    for alias in aliases {
        fragments.push((*alias).to_owned());
        fragments.push(identifier_phrase_variant(alias));
    }

    fragments
}

pub(crate) fn build_argument_fragments(
    argument_hint: &str,
    required_fields: &[String],
    required_field_groups: &[Vec<String>],
) -> Vec<String> {
    let mut fragments = Vec::new();

    if !argument_hint.is_empty() {
        fragments.push(argument_hint.to_owned());
    }

    if !required_fields.is_empty() {
        fragments.push(required_fields.join(" "));
    }

    for group in required_field_groups {
        fragments.push(group.join(" "));
    }

    fragments
}

fn collect_schema_search_terms(schema: &Value) -> Vec<String> {
    let mut fragments = Vec::new();
    collect_schema_search_terms_into(schema, &mut fragments);
    fragments
}

fn collect_schema_search_terms_into(schema: &Value, fragments: &mut Vec<String>) {
    let Value::Object(map) = schema else {
        return;
    };

    for key in ["title", "description"] {
        if let Some(text) = map.get(key).and_then(Value::as_str) {
            fragments.push(text.to_owned());
        }
    }

    if let Some(property_names) = map.get("properties").and_then(Value::as_object) {
        for (name, property_schema) in property_names {
            fragments.push(name.to_owned());
            collect_schema_search_terms_into(property_schema, fragments);
        }
    }

    for key in [
        "items",
        "additionalProperties",
        "contains",
        "if",
        "then",
        "else",
        "not",
    ] {
        if let Some(nested_schema) = map.get(key) {
            collect_schema_search_terms_into(nested_schema, fragments);
        }
    }

    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(nested_schemas) = map.get(key).and_then(Value::as_array) {
            for nested_schema in nested_schemas {
                collect_schema_search_terms_into(nested_schema, fragments);
            }
        }
    }

    if let Some(enum_values) = map.get("enum").and_then(Value::as_array) {
        for enum_value in enum_values {
            if let Some(text) = enum_value.as_str() {
                fragments.push(text.to_owned());
            }
        }
    }

    if let Some(example_values) = map.get("examples").and_then(Value::as_array) {
        for example_value in example_values {
            if let Some(text) = example_value.as_str() {
                fragments.push(text.to_owned());
            }
        }
    }

    if let Some(const_value) = map.get("const").and_then(Value::as_str) {
        fragments.push(const_value.to_owned());
    }
}

fn schema_argument_type(schema: &Value) -> String {
    let schema_type = schema
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("value");

    if schema_type != "array" {
        return schema_type.to_owned();
    }

    let item_type = schema
        .get("items")
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str);

    let Some(item_type) = item_type else {
        return "array".to_owned();
    };

    format!("{item_type}[]")
}

fn preferred_parameter_index(
    parameter_name: &str,
    preferred_parameter_order: &[(&str, &str)],
) -> usize {
    for (index, (preferred_name, _)) in preferred_parameter_order.iter().enumerate() {
        if *preferred_name == parameter_name {
            return index;
        }
    }

    usize::MAX
}

fn compact_argument_hint_fields(fields: Vec<SchemaArgumentField>) -> Vec<SchemaArgumentField> {
    if fields.len() <= 4 {
        return fields;
    }

    let mut compacted = Vec::new();
    let mut required_fields = 0usize;
    let mut optional_fields = 0usize;

    for field in fields {
        if field.required {
            if required_fields >= 2 {
                continue;
            }

            required_fields += 1;
            compacted.push(field);
            continue;
        }

        if optional_fields >= 1 {
            continue;
        }

        optional_fields += 1;
        compacted.push(field);
    }

    compacted
}
