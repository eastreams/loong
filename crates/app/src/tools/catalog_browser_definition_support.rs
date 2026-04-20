use serde_json::{Value, json};

use super::ToolDescriptor;

pub(super) fn browser_open_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "HTTP or HTTPS URL to open."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": crate::config::MAX_WEB_FETCH_MAX_BYTES,
                        "description": "Optional per-call read limit in bytes. Cannot exceed the configured runtime max."
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn browser_extract_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Bounded browser session identifier returned by browser.open."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["page_text", "title", "links", "selector_text"],
                        "description": "Extraction mode. Defaults to `page_text`."
                    },
                    "selector": {
                        "type": "string",
                        "description": "Optional CSS selector used only with `selector_text` mode."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": crate::config::MAX_BROWSER_MAX_LINKS,
                        "description": "Maximum extracted items when the mode returns a list."
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn browser_click_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Bounded browser session identifier returned by browser.open."
                    },
                    "link_id": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": crate::config::MAX_BROWSER_MAX_LINKS,
                        "description": "One-based link identifier returned in the current page snapshot."
                    }
                },
                "required": ["session_id", "link_id"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn browser_companion_session_start_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "HTTP or HTTPS URL to open in the managed browser companion session."
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn browser_companion_navigate_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Browser companion session identifier returned by browser.companion.session.start."
                    },
                    "url": {
                        "type": "string",
                        "description": "HTTP or HTTPS URL to load next."
                    }
                },
                "required": ["session_id", "url"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn browser_companion_snapshot_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Browser companion session identifier returned by browser.companion.session.start."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["summary", "html", "links"],
                        "description": "Optional snapshot mode. Defaults to `summary`."
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn browser_companion_wait_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Browser companion session identifier returned by browser.companion.session.start."
                    },
                    "condition": {
                        "type": "string",
                        "description": "Optional companion-side wait condition."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 30000,
                        "description": "Optional maximum wait in milliseconds."
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn browser_companion_session_stop_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Browser companion session identifier returned by browser.companion.session.start."
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn browser_companion_click_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Browser companion session identifier returned by browser.companion.session.start."
                    },
                    "selector": {
                        "type": "string",
                        "description": "Selector for the element to click."
                    }
                },
                "required": ["session_id", "selector"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn browser_companion_type_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Browser companion session identifier returned by browser.companion.session.start."
                    },
                    "selector": {
                        "type": "string",
                        "description": "Selector for the element to type into."
                    },
                    "text": {
                        "type": "string",
                        "description": "Text to enter."
                    }
                },
                "required": ["session_id", "selector", "text"],
                "additionalProperties": false
            }
        }
    })
}
