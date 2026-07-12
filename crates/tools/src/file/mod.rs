use loong_kernel::access::fs::FsAccessError;
use serde_json::Value;

mod read;
mod search;
mod write;

pub use read::{FileReadRequest, ReadFileOutput, ReadOutput, ReadRequest, ReadTool};
pub use search::{
    ContentSearchReadOutput, ContentSearchReadRequest, ContentSearchTool, GlobReadOutput,
    GlobReadRequest, GlobSearchTool,
};
pub use write::{WriteOutput, WriteRequest, WriteTool};

// Boundary conversion: access keeps typed errors, while the legacy app-facing
// tool result still carries string reasons. Keep policy denials recognizable
// until the outer error envelope becomes typed end to end.
fn fs_access_error_reason(error: FsAccessError) -> String {
    let rendered = error.to_string();
    if matches!(error, FsAccessError::Authorization(_)) {
        format!("policy_denied: {rendered}")
    } else {
        rendered
    }
}

fn required_trimmed_string_field<'a>(
    payload: &'a serde_json::Map<String, Value>,
    field_name: &str,
    tool_name: &str,
) -> Result<&'a str, String> {
    optional_trimmed_string_field(payload.get(field_name))
        .ok_or_else(|| format!("{tool_name} requires payload.{field_name}"))
}

// `offset` and `limit` intentionally share one parser: both fields use the same
// positive-integer contract, and a separate type would not add a stronger boundary.
fn optional_positive_usize_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    tool_name: &str,
) -> Result<Option<usize>, String> {
    let Some(value) = payload.get(field_name) else {
        return Ok(None);
    };

    let raw_value = value
        .as_u64()
        .ok_or_else(|| format!("{tool_name} payload.{field_name} must be a positive integer"))?;
    if raw_value == 0 {
        return Err(format!(
            "{tool_name} payload.{field_name} must be a positive integer"
        ));
    }

    usize::try_from(raw_value)
        .map(Some)
        .map_err(|conversion_error| {
            format!("{tool_name} payload.{field_name} is too large: {conversion_error}")
        })
}

fn optional_trimmed_string_field(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn optional_bounded_usize_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    default_value: usize,
    minimum: usize,
    maximum: usize,
    tool_name: &str,
) -> Result<usize, String> {
    let Some(value) = payload.get(field_name) else {
        return Ok(default_value);
    };
    let parsed_value_u64 = value
        .as_u64()
        .ok_or_else(|| format!("{tool_name} payload.{field_name} must be an integer"))?;
    let parsed_value = usize::try_from(parsed_value_u64).map_err(|conversion_error| {
        format!("{tool_name} payload.{field_name} is out of range: {conversion_error}")
    })?;
    if parsed_value < minimum || parsed_value > maximum {
        return Err(format!(
            "{tool_name} payload.{field_name} must be between {minimum} and {maximum}"
        ));
    }
    Ok(parsed_value)
}

#[cfg(test)]
mod tests;
