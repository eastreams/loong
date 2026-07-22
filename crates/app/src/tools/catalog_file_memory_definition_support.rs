use serde_json::{Value, json};

use super::ToolDescriptor;

pub(super) fn memory_search_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural-language lookup query for durable workspace memory and canonical cross-session recall."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 8,
                        "description": "Optional maximum number of memory hits to return. Defaults to 5."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn memory_retrieve_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session identifier whose memory context should drive retrieval."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional natural-language retrieval query. When omitted, retrieval falls back to profile-aware advisory recall."
                    },
                    "intent": {
                        "type": "string",
                        "enum": ["operator_inspection", "prompt_assembly"],
                        "description": "Retrieval intent. Defaults to `operator_inspection`."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 8,
                        "description": "Optional maximum number of retrieval hits to return. Defaults to 5."
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn memory_get_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative or absolute durable memory file path within the configured safe file root."
                    },
                    "from": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional 1-based starting line number. Defaults to 1."
                    },
                    "lines": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "Optional number of lines to read. Defaults to 40."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }
    })
}
