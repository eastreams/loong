use std::collections::BTreeMap;

use serde_json::Value;

use crate::mvp::config::LoongConfig;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GatewayToolSettings {
    pub(crate) requested_tool_ids: Option<Vec<String>>,
    pub(crate) disable_tools: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GatewayToolChoice {
    Auto,
    None,
    Required,
    Specific,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GatewayToolValidationError {
    pub(crate) param: &'static str,
    pub(crate) message: String,
}

pub(crate) fn resolve_gateway_tool_settings(
    config: &LoongConfig,
    tools: Option<&Value>,
    tool_choice: Option<&Value>,
) -> Result<GatewayToolSettings, GatewayToolValidationError> {
    let exposed_tools = exposed_provider_tools(config)?;
    let requested_tools = parse_requested_tools(tools, &exposed_tools)?;
    let parsed_tool_choice = parse_tool_choice(tool_choice, &exposed_tools)?;

    let requested_tool_ids = match parsed_tool_choice {
        GatewayToolChoice::Auto => requested_tools,
        GatewayToolChoice::Required => {
            let Some(requested_tools) = requested_tools else {
                return Err(GatewayToolValidationError {
                    param: "tool_choice",
                    message: "tool_choice=`required` requires an explicit tools array on this gateway surface"
                        .to_owned(),
                });
            };
            if requested_tools.is_empty() {
                return Ok(GatewayToolSettings {
                    requested_tool_ids: None,
                    disable_tools: true,
                });
            }
            Some(requested_tools)
        }
        GatewayToolChoice::None => {
            return Ok(GatewayToolSettings {
                requested_tool_ids: None,
                disable_tools: true,
            });
        }
        GatewayToolChoice::Specific => {
            let specific_tool_name = extract_specific_tool_name(
                tool_choice.expect("specific tool choice payload"),
                &exposed_tools,
            )?;
            if let Some(requested_tools) = requested_tools.as_ref()
                && !requested_tools.contains(&specific_tool_name)
            {
                return Err(GatewayToolValidationError {
                    param: "tool_choice",
                    message: format!(
                        "tool_choice requested `{}` but the tools array does not include it",
                        specific_tool_name
                    ),
                });
            }
            Some(vec![specific_tool_name])
        }
    };

    if requested_tool_ids.as_ref().is_some_and(Vec::is_empty) {
        return Ok(GatewayToolSettings {
            requested_tool_ids: None,
            disable_tools: true,
        });
    }

    Ok(GatewayToolSettings {
        requested_tool_ids,
        disable_tools: false,
    })
}

pub(crate) fn apply_gateway_tool_settings(
    config: &LoongConfig,
    session_id: &str,
    tool_settings: &GatewayToolSettings,
) -> Result<(), String> {
    if tool_settings.disable_tools {
        return clear_gateway_session_tool_policy(config, session_id);
    }
    let Some(requested_tool_ids) = tool_settings.requested_tool_ids.as_ref() else {
        return Ok(());
    };
    ensure_gateway_session(config, session_id)?;
    let store_config =
        crate::mvp::session::store::session_store_config_from_memory_config(&config.memory);
    let repo = crate::mvp::session::repository::SessionRepository::new(&store_config)?;
    repo.upsert_session_tool_policy(
        crate::mvp::session::repository::NewSessionToolPolicyRecord {
            session_id: session_id.to_owned(),
            requested_tool_ids: requested_tool_ids.clone(),
            runtime_narrowing: crate::mvp::tools::runtime_config::ToolRuntimeNarrowing::default(),
        },
    )?;
    Ok(())
}

pub(crate) fn clear_gateway_session_tool_policy(
    config: &LoongConfig,
    session_id: &str,
) -> Result<(), String> {
    let store_config =
        crate::mvp::session::store::session_store_config_from_memory_config(&config.memory);
    let repo = crate::mvp::session::repository::SessionRepository::new(&store_config)?;
    let _ = repo.delete_session_tool_policy(session_id)?;
    Ok(())
}

fn ensure_gateway_session(config: &LoongConfig, session_id: &str) -> Result<(), String> {
    let store_config =
        crate::mvp::session::store::session_store_config_from_memory_config(&config.memory);
    let repo = crate::mvp::session::repository::SessionRepository::new(&store_config)?;
    let _ = repo.ensure_session(crate::mvp::session::repository::NewSessionRecord {
        session_id: session_id.to_owned(),
        kind: crate::mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("gateway-webui".to_owned()),
        state: crate::mvp::session::repository::SessionState::Ready,
    })?;
    Ok(())
}

fn exposed_provider_tools(
    _config: &LoongConfig,
) -> Result<BTreeMap<String, String>, GatewayToolValidationError> {
    let catalog = crate::mvp::tools::tool_catalog();
    let mut exposed = BTreeMap::new();
    for descriptor in catalog.descriptors() {
        if !descriptor.is_gateway() {
            continue;
        }
        exposed.insert(
            descriptor.provider_name.to_owned(),
            descriptor.name.to_owned(),
        );
        exposed.insert(descriptor.name.to_owned(), descriptor.name.to_owned());
    }
    Ok(exposed)
}

fn parse_requested_tools(
    tools: Option<&Value>,
    exposed_tools: &BTreeMap<String, String>,
) -> Result<Option<Vec<String>>, GatewayToolValidationError> {
    let Some(tools) = tools else {
        return Ok(None);
    };
    let Some(items) = tools.as_array() else {
        return Err(GatewayToolValidationError {
            param: "tools",
            message: "tools must be an array".to_owned(),
        });
    };
    let mut requested_tool_ids = BTreeMap::<String, String>::new();
    for item in items {
        let item_type = item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("function");
        if item_type != "function" {
            return Err(GatewayToolValidationError {
                param: "tools",
                message: format!("unsupported tool type `{item_type}`"),
            });
        }
        let Some(name) = item
            .get("function")
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
        else {
            return Err(GatewayToolValidationError {
                param: "tools",
                message: "tool entries must include function.name".to_owned(),
            });
        };
        let canonical_name = resolve_exposed_tool_name(name, exposed_tools).ok_or_else(|| {
            GatewayToolValidationError {
                param: "tools",
                message: format!("unknown or unavailable tool `{name}`"),
            }
        })?;
        requested_tool_ids.insert(canonical_name.clone(), canonical_name);
    }
    Ok(Some(requested_tool_ids.into_values().collect()))
}

fn parse_tool_choice(
    tool_choice: Option<&Value>,
    exposed_tools: &BTreeMap<String, String>,
) -> Result<GatewayToolChoice, GatewayToolValidationError> {
    let Some(tool_choice) = tool_choice else {
        return Ok(GatewayToolChoice::Auto);
    };
    if let Some(choice) = tool_choice.as_str() {
        return match choice {
            "auto" => Ok(GatewayToolChoice::Auto),
            "none" => Ok(GatewayToolChoice::None),
            "required" => Ok(GatewayToolChoice::Required),
            other => {
                if resolve_exposed_tool_name(other, exposed_tools).is_some() {
                    Ok(GatewayToolChoice::Specific)
                } else {
                    Err(GatewayToolValidationError {
                        param: "tool_choice",
                        message: format!("unsupported tool_choice `{other}`"),
                    })
                }
            }
        };
    }
    let Some(choice_type) = tool_choice.get("type").and_then(Value::as_str) else {
        return Err(GatewayToolValidationError {
            param: "tool_choice",
            message: "tool_choice must be a string or object with type".to_owned(),
        });
    };
    match choice_type {
        "auto" => Ok(GatewayToolChoice::Auto),
        "none" => Ok(GatewayToolChoice::None),
        "required" => Ok(GatewayToolChoice::Required),
        "function" => {
            let _ = extract_specific_tool_name(tool_choice, exposed_tools)?;
            Ok(GatewayToolChoice::Specific)
        }
        other => Err(GatewayToolValidationError {
            param: "tool_choice",
            message: format!("unsupported tool_choice type `{other}`"),
        }),
    }
}

fn extract_specific_tool_name(
    tool_choice: &Value,
    exposed_tools: &BTreeMap<String, String>,
) -> Result<String, GatewayToolValidationError> {
    let Some(name) = tool_choice
        .get("function")
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
    else {
        return Err(GatewayToolValidationError {
            param: "tool_choice",
            message: "function tool_choice requires function.name".to_owned(),
        });
    };
    resolve_exposed_tool_name(name, exposed_tools).ok_or_else(|| GatewayToolValidationError {
        param: "tool_choice",
        message: format!("unknown or unavailable tool `{name}`"),
    })
}

fn resolve_exposed_tool_name(
    raw_name: &str,
    exposed_tools: &BTreeMap<String, String>,
) -> Option<String> {
    if let Some(canonical_name) = exposed_tools.get(raw_name) {
        return Some(canonical_name.clone());
    }
    let descriptor = crate::mvp::tools::tool_catalog().resolve(raw_name)?;
    let canonical_name = descriptor.name.to_owned();
    exposed_tools
        .values()
        .any(|value| value == &canonical_name)
        .then_some(canonical_name)
}
