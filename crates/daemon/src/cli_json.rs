use serde_json::Value;

use crate::CliResult;

pub(crate) fn parse_json_payload(raw: &str, context: &str) -> CliResult<Value> {
    serde_json::from_str(raw).map_err(|error| format!("invalid JSON for {context}: {error}"))
}
