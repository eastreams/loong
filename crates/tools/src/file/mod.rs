use loong_contracts::ToolInputError;
use serde_json::Value;

// Concrete file tools live here, but filesystem effects do not. Each tool only
// parses payload, calls ctx.access().fs(), and shapes typed output; read/write,
// directory traversal, and content search side effects are owned by
// loong_access::fs through granted fs actions.
mod edit;
mod read;
mod search;
mod write;

pub use edit::{EditOutput, EditRequest, EditTool, EditToolError, ExactTextEditBlock};
pub use read::{FileReadRequest, ReadFileOutput, ReadOutput, ReadRequest, ReadTool, ReadToolError};
pub use search::{
    ContentSearchReadOutput, ContentSearchReadRequest, ContentSearchTool, GlobReadOutput,
    GlobReadRequest, GlobSearchTool,
};
pub use write::{WriteOutput, WriteRequest, WriteTool};

// Required non-empty strings share one contract across file tools so missing
// and malformed fields retain the same typed evidence at every entry point.
fn required_trimmed_string_field<'a>(
    payload: &'a serde_json::Map<String, Value>,
    field_name: &str,
) -> Result<&'a str, ToolInputError> {
    let value = payload
        .get(field_name)
        .ok_or_else(|| ToolInputError::missing_field(field_name))?;
    let value = value
        .as_str()
        .ok_or_else(|| ToolInputError::invalid_field(field_name, "must be a string"))?
        .trim();
    if value.is_empty() {
        return Err(ToolInputError::invalid_field(
            field_name,
            "must not be empty",
        ));
    }
    Ok(value)
}

// `offset` and `limit` intentionally share one parser: both fields use the same
// positive-integer contract, and a separate type would not add a stronger boundary.
fn optional_positive_usize_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
) -> Result<Option<usize>, ToolInputError> {
    let Some(value) = payload.get(field_name) else {
        return Ok(None);
    };

    let raw_value = value
        .as_u64()
        .ok_or_else(|| ToolInputError::invalid_field(field_name, "must be a positive integer"))?;
    if raw_value == 0 {
        return Err(ToolInputError::invalid_field(
            field_name,
            "must be a positive integer",
        ));
    }

    usize::try_from(raw_value)
        .map(Some)
        .map_err(|conversion_error| {
            ToolInputError::invalid_field(field_name, format!("is too large: {conversion_error}"))
        })
}

// Mode selection and optional search fields intentionally share trimming and
// blank-as-absent behavior; callers decide which field, if any, is required.
fn optional_trimmed_string_field(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

// Search limit fields use the same default-and-inclusive-range contract, while
// each caller supplies its schema-specific bounds.
fn optional_bounded_usize_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    default_value: usize,
    minimum: usize,
    maximum: usize,
) -> Result<usize, ToolInputError> {
    let Some(value) = payload.get(field_name) else {
        return Ok(default_value);
    };
    let parsed_value_u64 = value
        .as_u64()
        .ok_or_else(|| ToolInputError::invalid_field(field_name, "must be an integer"))?;
    let parsed_value = usize::try_from(parsed_value_u64).map_err(|conversion_error| {
        ToolInputError::invalid_field(field_name, format!("is out of range: {conversion_error}"))
    })?;
    if parsed_value < minimum || parsed_value > maximum {
        return Err(ToolInputError::invalid_field(
            field_name,
            format!("must be between {minimum} and {maximum}"),
        ));
    }
    Ok(parsed_value)
}

#[cfg(test)]
mod tests;
