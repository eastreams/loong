#[cfg(feature = "memory-sqlite")]
use crate::session::repository::SessionRepository;
use crate::session::store::SessionStoreConfig;
#[cfg(feature = "memory-sqlite")]
use crate::task_progress::resolve_canonical_task_id_for_session;
use loong_contracts::{KernelError, ToolPlaneError};

use super::{AugmentedToolPayload, SessionContext};

pub(crate) fn render_kernel_error_reason(error: &KernelError) -> String {
    #[allow(clippy::wildcard_enum_match_arm)]
    match error {
        KernelError::ToolPlane(ToolPlaneError::Execution(reason)) => format!(
            "tool execution failed: {}",
            reason.strip_prefix("policy_denied: ").unwrap_or(reason)
        ),
        _ => format!("{error}"),
    }
}

pub(super) fn augment_tool_payload_for_kernel(
    canonical_tool_name: &str,
    payload: serde_json::Value,
    session_context: &SessionContext,
    memory_config: &SessionStoreConfig,
) -> AugmentedToolPayload {
    let nested_browser_scope_required = if canonical_tool_name == "tool.invoke" {
        payload
            .as_object()
            .and_then(|object| object.get("tool_id"))
            .and_then(serde_json::Value::as_str)
            .map(crate::tools::canonical_tool_name)
            .is_some_and(browser_scope_injection_required)
    } else {
        false
    };
    let augmented_runtime_narrowing =
        inject_runtime_narrowing_context_trusted(payload, session_context, false);
    let payload_after_runtime_narrowing = augmented_runtime_narrowing.payload;
    let runtime_narrowing_trusted = augmented_runtime_narrowing.trusted_internal_context;
    let augmented_active_skill_workspace_root = inject_active_skill_workspace_root_context_trusted(
        canonical_tool_name,
        payload_after_runtime_narrowing,
        session_context,
        runtime_narrowing_trusted,
    );
    let payload_after_active_skill_workspace_root = augmented_active_skill_workspace_root.payload;
    let active_skill_workspace_root_trusted =
        augmented_active_skill_workspace_root.trusted_internal_context;
    let augmented_workspace_root = inject_workspace_root_context_trusted(
        payload_after_active_skill_workspace_root,
        session_context,
        active_skill_workspace_root_trusted,
    );
    let mut payload = augmented_workspace_root.payload;
    let trusted_internal_context = augmented_workspace_root.trusted_internal_context;
    let canonical_task_id = resolve_canonical_task_id_for_runtime(session_context, memory_config);

    if task_scope_injection_required(canonical_tool_name) {
        payload = inject_task_scope_field(payload, canonical_task_id.as_str());
        return AugmentedToolPayload {
            payload,
            trusted_internal_context,
        };
    }

    if browser_scope_injection_required(canonical_tool_name) {
        payload = inject_browser_scope_field(payload, &session_context.session_id);
        return AugmentedToolPayload {
            payload,
            trusted_internal_context,
        };
    }

    if nested_browser_scope_required {
        payload = inject_nested_browser_scope_field(payload, &session_context.session_id);
        return AugmentedToolPayload {
            payload,
            trusted_internal_context,
        };
    }

    AugmentedToolPayload {
        payload,
        trusted_internal_context,
    }
}

fn inject_active_skill_workspace_root_context_trusted(
    canonical_tool_name: &str,
    payload: serde_json::Value,
    session_context: &SessionContext,
    preserve_existing_internal_context: bool,
) -> AugmentedToolPayload {
    let workspace_root = active_skill_workspace_root_for_tool_payload(
        canonical_tool_name,
        &payload,
        session_context,
    )
    .or_else(|| {
        visible_skill_workspace_root_for_tool_payload(
            canonical_tool_name,
            &payload,
            session_context,
        )
    });
    let Some(workspace_root) = workspace_root else {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    };

    inject_workspace_root_path_context_trusted(
        payload,
        &workspace_root,
        preserve_existing_internal_context,
    )
}

fn active_skill_workspace_root_for_tool_payload(
    canonical_tool_name: &str,
    payload: &serde_json::Value,
    session_context: &SessionContext,
) -> Option<std::path::PathBuf> {
    if session_context.active_skill_roots.is_empty() {
        return None;
    }

    if crate::tools::user_visible_tool_name(canonical_tool_name) != "read" {
        return None;
    }

    let requested_path = requested_file_tool_path(canonical_tool_name, payload)?;
    if requested_path.is_absolute() {
        let normalized_requested_path = if requested_path.exists() {
            std::fs::canonicalize(&requested_path).unwrap_or(requested_path)
        } else {
            requested_path
        };

        return session_context
            .active_skill_roots
            .iter()
            .find(|root| normalized_requested_path.starts_with(root))
            .cloned();
    }

    resolve_active_skill_root_for_relative_path(
        &session_context.active_skill_roots,
        requested_path.as_path(),
    )
}

fn visible_skill_workspace_root_for_tool_payload(
    canonical_tool_name: &str,
    payload: &serde_json::Value,
    session_context: &SessionContext,
) -> Option<std::path::PathBuf> {
    if session_context.visible_skill_roots.is_empty() {
        return None;
    }

    if crate::tools::user_visible_tool_name(canonical_tool_name) != "read" {
        return None;
    }

    let requested_path = requested_file_tool_path(canonical_tool_name, payload)?;
    if !requested_path.is_absolute() {
        return None;
    }

    let normalized_requested_path = if requested_path.exists() {
        std::fs::canonicalize(&requested_path).unwrap_or(requested_path)
    } else {
        requested_path
    };

    session_context
        .visible_skill_roots
        .iter()
        .find(|root| normalized_requested_path.starts_with(root))
        .cloned()
}

fn resolve_active_skill_root_for_relative_path(
    active_skill_roots: &[std::path::PathBuf],
    requested_path: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let mut matches = active_skill_roots
        .iter()
        .filter_map(|root| {
            let candidate = root.join(requested_path);
            candidate.exists().then(|| root.clone())
        })
        .collect::<Vec<_>>();
    matches.dedup();
    (matches.len() == 1).then(|| matches.remove(0))
}

fn requested_file_tool_path(
    tool_name: &str,
    payload: &serde_json::Value,
) -> Option<std::path::PathBuf> {
    let payload_object = payload.as_object()?;
    if crate::tools::user_visible_tool_name(tool_name) != "read" {
        return None;
    }

    payload_object
        .get("path")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            payload_object
                .get("root")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from)
        })
}

fn task_scope_injection_required(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "task_status" | "task_wait" | "task_history" | "task_events"
    )
}

fn inject_task_scope_field(payload: serde_json::Value, task_id: &str) -> serde_json::Value {
    match payload {
        serde_json::Value::Object(mut object) => {
            let has_task_id = object
                .get("task_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| !value.trim().is_empty());
            if !has_task_id {
                object.insert("task_id".to_owned(), serde_json::json!(task_id));
            }
            serde_json::Value::Object(object)
        }
        other @ serde_json::Value::Null
        | other @ serde_json::Value::Bool(_)
        | other @ serde_json::Value::Number(_)
        | other @ serde_json::Value::String(_)
        | other @ serde_json::Value::Array(_) => other,
    }
}

fn resolve_canonical_task_id_for_runtime(
    session_context: &SessionContext,
    memory_config: &SessionStoreConfig,
) -> String {
    #[cfg(feature = "memory-sqlite")]
    {
        if let Ok(repo) = SessionRepository::new(memory_config)
            && let Some(task_id) =
                resolve_canonical_task_id_for_session(&repo, &session_context.session_id)
        {
            return task_id;
        }
    }

    session_context.session_id.clone()
}

fn inject_runtime_narrowing_context_trusted(
    payload: serde_json::Value,
    session_context: &SessionContext,
    preserve_existing_internal_context: bool,
) -> AugmentedToolPayload {
    let Some(runtime_narrowing) = session_context.resolved_runtime_narrowing() else {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    };
    if runtime_narrowing.is_empty() {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    }

    let serde_json::Value::Object(mut object) = payload else {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    };
    let mut internal = if preserve_existing_internal_context {
        crate::tools::take_trusted_internal_tool_context(&mut object)
    } else {
        serde_json::Map::new()
    };
    internal.insert(
        crate::tools::LOONG_INTERNAL_RUNTIME_NARROWING_KEY.to_owned(),
        serde_json::to_value(runtime_narrowing)
            .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new())),
    );
    object.insert(
        crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY.to_owned(),
        serde_json::Value::Object(internal),
    );
    AugmentedToolPayload {
        payload: serde_json::Value::Object(object),
        trusted_internal_context: true,
    }
}

fn inject_workspace_root_context_trusted(
    payload: serde_json::Value,
    session_context: &SessionContext,
    preserve_existing_internal_context: bool,
) -> AugmentedToolPayload {
    let Some(workspace_root) = session_context.workspace_root.as_ref() else {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    };

    inject_workspace_root_path_context_trusted(
        payload,
        workspace_root.as_path(),
        preserve_existing_internal_context,
    )
}

fn inject_workspace_root_path_context_trusted(
    payload: serde_json::Value,
    workspace_root: &std::path::Path,
    preserve_existing_internal_context: bool,
) -> AugmentedToolPayload {
    let serde_json::Value::Object(mut object) = payload else {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    };
    let mut internal = if preserve_existing_internal_context {
        crate::tools::take_trusted_internal_tool_context(&mut object)
    } else {
        serde_json::Map::new()
    };
    if preserve_existing_internal_context
        && internal.contains_key(crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY)
    {
        object.insert(
            crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY.to_owned(),
            serde_json::Value::Object(internal),
        );
        return AugmentedToolPayload {
            payload: serde_json::Value::Object(object),
            trusted_internal_context: true,
        };
    }
    internal.insert(
        crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY.to_owned(),
        serde_json::Value::String(workspace_root.display().to_string()),
    );
    object.insert(
        crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY.to_owned(),
        serde_json::Value::Object(internal),
    );
    AugmentedToolPayload {
        payload: serde_json::Value::Object(object),
        trusted_internal_context: true,
    }
}

fn browser_scope_injection_required(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "browse" | "browser" | "browser.open" | "browser.extract" | "browser.click"
    )
}

fn inject_browser_scope_field(payload: serde_json::Value, session_id: &str) -> serde_json::Value {
    match payload {
        serde_json::Value::Object(mut object) => {
            object.insert(
                crate::tools::BROWSER_SESSION_SCOPE_FIELD.to_owned(),
                serde_json::json!(session_id),
            );
            serde_json::Value::Object(object)
        }
        other @ (serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_)
        | serde_json::Value::Array(_)) => other,
    }
}

fn inject_nested_browser_scope_field(
    payload: serde_json::Value,
    session_id: &str,
) -> serde_json::Value {
    match payload {
        serde_json::Value::Object(mut object) => match object.remove("arguments") {
            Some(arguments) => {
                object.insert(
                    "arguments".to_owned(),
                    inject_browser_scope_field(arguments, session_id),
                );
                serde_json::Value::Object(object)
            }
            None => serde_json::Value::Object(object),
        },
        other @ serde_json::Value::Null
        | other @ serde_json::Value::Bool(_)
        | other @ serde_json::Value::Number(_)
        | other @ serde_json::Value::String(_)
        | other @ serde_json::Value::Array(_) => other,
    }
}
