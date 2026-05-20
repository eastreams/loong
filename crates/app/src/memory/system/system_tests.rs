use super::*;
use rusqlite::Connection;
use serde_json::json;

const TEST_SESSION_ID: &str = "session-123";
const TEST_SECONDARY_SESSION_ID: &str = "session-1";
const RELEASE_FREEZE_TIMING: &str = "release freeze timing";
const DIAGNOSE_RELEASE_FREEZE_TEXT: &str = "diagnose release freeze";
const REGISTRY_REQUEST_ONLY_ID: &str = "result-wrapping-registry";
const PLAN_RESULT_ONLY_ID: &str = "plan-result-only-registry";
const REGISTRY_REQUEST_NOTE: &str = "registry request";
const PLAN_RESULT_ONLY_NOTE: &str = "plan result only";
const ROOT_SESSION_ID: &str = "root-session";
const ROOT_SESSION_LABEL: &str = "Root Session";
const DELEGATE_CHILD_KIND: &str = "delegate_child";
const DELEGATE_CHILD_LABEL: &str = "Release Child";
const WORKFLOW_TASK_TEXT: &str = "investigate release freeze";
const WORKFLOW_PHASE_FAILED: &str = "failed";
const WORKFLOW_PHASE_COMPLETE: &str = "complete";
const WORKFLOW_OPERATION_KIND_APPROVAL: &str = "approval";
const WORKFLOW_OPERATION_SCOPE_SESSION: &str = "session";
const DELEGATE_STARTED_EVENT: &str = "delegate_started";
const DELEGATE_COMPLETED_EVENT: &str = "delegate_completed";
const SHELL_EXEC_TOOL_NAME: &str = "shell.exec";

struct StageAwareRegistryMemorySystem;

impl MemorySystem for StageAwareRegistryMemorySystem {
    fn id(&self) -> &'static str {
        "registry-stage-aware"
    }

    fn metadata(&self) -> MemorySystemMetadata {
        MemorySystemMetadata::new(
            "registry-stage-aware",
            [MemorySystemCapability::PromptHydration],
            "Registry stage-aware test system",
        )
        .with_runtime_fallback_kind(MemorySystemRuntimeFallbackKind::SystemBacked)
        .with_supported_pre_assembly_stage_families([MemoryStageFamily::Retrieve])
    }
}

struct ResultWrappingRegistryMemorySystem;

struct PlanResultOnlyRegistryMemorySystem;

impl MemorySystem for ResultWrappingRegistryMemorySystem {
    fn id(&self) -> &'static str {
        REGISTRY_REQUEST_ONLY_ID
    }

    fn metadata(&self) -> MemorySystemMetadata {
        MemorySystemMetadata::new(
            REGISTRY_REQUEST_ONLY_ID,
            [MemorySystemCapability::PromptHydration],
            "Registry system with direct retrieval request",
        )
    }

    fn build_retrieval_request(
        &self,
        session_id: &str,
        _workspace_root: Option<&Path>,
        _config: &MemoryRuntimeConfig,
        _recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalRequest> {
        Some(MemoryRetrievalRequest {
            session_id: session_id.to_owned(),
            memory_system_id: self.id().to_owned(),
            strategy: MemoryRetrievalStrategy::WorkspaceReferenceOnly,
            planning_notes: vec![REGISTRY_REQUEST_NOTE.to_owned()],
            query: None,
            recall_mode: MemoryRecallMode::PromptAssembly,
            scopes: vec![MemoryScope::Workspace],
            budget_items: 1,
            allowed_kinds: vec![DerivedMemoryKind::Reference],
        })
    }
}

impl MemorySystem for PlanResultOnlyRegistryMemorySystem {
    fn id(&self) -> &'static str {
        PLAN_RESULT_ONLY_ID
    }

    fn metadata(&self) -> MemorySystemMetadata {
        MemorySystemMetadata::new(
            PLAN_RESULT_ONLY_ID,
            [MemorySystemCapability::PromptHydration],
            "Registry system with direct retrieval plan result",
        )
    }

    fn build_retrieval_plan_result(
        &self,
        session_id: &str,
        _workspace_root: Option<&Path>,
        _config: &MemoryRuntimeConfig,
        _recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalPlanResult> {
        Some(
            MemoryRetrievalRequest {
                session_id: session_id.to_owned(),
                memory_system_id: self.id().to_owned(),
                strategy: MemoryRetrievalStrategy::WorkspaceReferenceOnly,
                planning_notes: vec![PLAN_RESULT_ONLY_NOTE.to_owned()],
                query: None,
                recall_mode: MemoryRecallMode::PromptAssembly,
                scopes: vec![MemoryScope::Workspace],
                budget_items: 1,
                allowed_kinds: vec![DerivedMemoryKind::Reference],
            }
            .into_plan_result(),
        )
    }
}

fn release_freeze_recent_window() -> Vec<WindowTurn> {
    vec![WindowTurn {
        role: "user".to_owned(),
        content: RELEASE_FREEZE_TIMING.to_owned(),
        ts: None,
    }]
}

fn release_freeze_test_config() -> MemoryRuntimeConfig {
    MemoryRuntimeConfig::default()
}

fn default_test_config() -> MemoryRuntimeConfig {
    MemoryRuntimeConfig::default()
}

fn workspace_recall_test_config() -> MemoryRuntimeConfig {
    MemoryRuntimeConfig {
        sliding_window: 6,
        ..MemoryRuntimeConfig::default()
    }
}

fn assert_request_adapter_matches_plan_result_for_workspace_recall(system: &dyn MemorySystem) {
    let config = workspace_recall_test_config();
    assert_request_adapter_matches_plan_result(
        system,
        WORKSPACE_RECALL_MEMORY_SYSTEM_ID,
        &config,
        &[],
    );
}

fn assert_request_helper_matches_request_adapter_for_workspace_recall(system: &dyn MemorySystem) {
    let config = workspace_recall_test_config();
    assert_request_helper_matches_request_adapter(system, &config, &[]);
}

fn assert_plan_result_helper_matches_default_for_workspace_recall(system: &dyn MemorySystem) {
    let config = workspace_recall_test_config();
    assert_plan_result_helper_matches_default(system, &config, &[]);
}

fn assert_boxed_request_helper_matches_request_adapter_for_workspace_recall(
    boxed: Box<dyn MemorySystem>,
) {
    let config = workspace_recall_test_config();
    assert_boxed_request_helper_matches_request_adapter(boxed, &config, &[]);
}

fn assert_boxed_request_adapter_matches_plan_result_for_workspace_recall(
    boxed: Box<dyn MemorySystem>,
) {
    let config = workspace_recall_test_config();
    assert_boxed_request_adapter_matches_plan_result(
        boxed,
        WORKSPACE_RECALL_MEMORY_SYSTEM_ID,
        &config,
        &[],
    );
}

fn assert_boxed_plan_result_helper_matches_default_for_workspace_recall(
    boxed: Box<dyn MemorySystem>,
) {
    let config = workspace_recall_test_config();
    assert_boxed_plan_result_helper_matches_default(boxed, &config, &[]);
}

fn assert_boxed_workspace_recall_request_budget_items(
    boxed: Box<dyn MemorySystem>,
    expected_budget_items: usize,
) {
    assert_workspace_recall_request_budget_items(&*boxed, expected_budget_items);
}

fn assert_workspace_recall_request_budget_items(
    system: &dyn MemorySystem,
    expected_budget_items: usize,
) {
    let config = workspace_recall_test_config();
    let request = system
        .build_retrieval_request(TEST_SESSION_ID, Some(test_workspace_root()), &config, &[])
        .expect("workspace recall retrieval request");
    assert_eq!(request.budget_items, expected_budget_items);
}

fn test_workspace_root() -> &'static Path {
    Path::new("/tmp/workspace")
}

fn boxed_request_only_registry_system() -> Box<dyn MemorySystem> {
    Box::new(ResultWrappingRegistryMemorySystem)
}

fn boxed_plan_result_only_registry_system() -> Box<dyn MemorySystem> {
    Box::new(PlanResultOnlyRegistryMemorySystem)
}

fn boxed_builtin_system() -> Box<dyn MemorySystem> {
    Box::new(BuiltinMemorySystem)
}

fn boxed_recall_first_system() -> Box<dyn MemorySystem> {
    Box::new(RecallFirstMemorySystem)
}

fn boxed_workspace_recall_system() -> Box<dyn MemorySystem> {
    Box::new(WorkspaceRecallMemorySystem)
}

fn delegate_child_metadata_hint(
    state: &str,
    workflow_task: Option<&str>,
    workflow_phase: Option<&str>,
    workflow_operation_kind: Option<&str>,
    workflow_operation_scope: Option<&str>,
) -> crate::memory::sqlite::SessionMetadataHint {
    crate::memory::sqlite::SessionMetadataHint {
        kind: DELEGATE_CHILD_KIND.to_owned(),
        state: state.to_owned(),
        parent_session_id: Some(ROOT_SESSION_ID.to_owned()),
        label: Some(DELEGATE_CHILD_LABEL.to_owned()),
        parent_label: Some(ROOT_SESSION_LABEL.to_owned()),
        lineage_root_session_id: Some(ROOT_SESSION_ID.to_owned()),
        lineage_root_label: Some(ROOT_SESSION_LABEL.to_owned()),
        lineage_depth: 1,
        workflow_task: workflow_task.map(str::to_owned),
        workflow_phase: workflow_phase.map(str::to_owned),
        workflow_operation_kind: workflow_operation_kind.map(str::to_owned),
        workflow_operation_scope: workflow_operation_scope.map(str::to_owned),
    }
}

fn expected_delegate_lineage_query() -> String {
    format!("{DELEGATE_CHILD_LABEL}\n{ROOT_SESSION_LABEL}")
}

fn expected_workflow_task_query(
    phase: &str,
    operation_kind: Option<&str>,
    operation_scope: Option<&str>,
) -> String {
    let mut parts = vec![WORKFLOW_TASK_TEXT.to_owned(), format!("phase: {phase}")];
    if let Some(operation_kind) = operation_kind {
        parts.push(format!("operation_kind: {operation_kind}"));
    }
    if let Some(operation_scope) = operation_scope {
        parts.push(format!("operation_scope: {operation_scope}"));
    }
    parts.join("\n")
}

fn expected_structured_signal_query() -> String {
    format!("{DELEGATE_STARTED_EVENT}\n{SHELL_EXEC_TOOL_NAME}")
}

#[cfg(feature = "memory-sqlite")]
fn sqlite_test_workspace() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    MemoryRuntimeConfig,
    Connection,
) {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = temp_dir.path().to_path_buf();
    let db_path = workspace_root.join("memory.sqlite3");
    let config = MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
        ..MemoryRuntimeConfig::default()
    };

    crate::memory::ensure_memory_db_ready(Some(db_path.clone()), &config)
        .expect("ensure memory db ready");
    let conn = Connection::open(&db_path).expect("open sqlite db");

    (temp_dir, workspace_root, config, conn)
}

#[cfg(feature = "memory-sqlite")]
fn insert_root_session(conn: &Connection) {
    conn.execute(
        "INSERT INTO sessions(session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error)
        VALUES (?1, 'root', NULL, ?2, 'ready', 1, 1, NULL)",
        rusqlite::params![ROOT_SESSION_ID, ROOT_SESSION_LABEL],
    )
    .expect("insert root session");
}

#[cfg(feature = "memory-sqlite")]
fn insert_delegate_child_session(conn: &Connection, state: &str) {
    conn.execute(
        "INSERT INTO sessions(session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, 1, NULL)",
        rusqlite::params![
            TEST_SESSION_ID,
            DELEGATE_CHILD_KIND,
            ROOT_SESSION_ID,
            DELEGATE_CHILD_LABEL,
            state,
        ],
    )
    .expect("insert delegate child session");
}

#[cfg(feature = "memory-sqlite")]
fn insert_delegate_task_event(conn: &Connection, event_kind: &str, task: &str) {
    conn.execute(
        "INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, search_text, ts)
         VALUES (?1, ?2, ?3, ?4, '', 1)",
        rusqlite::params![
            TEST_SESSION_ID,
            event_kind,
            ROOT_SESSION_ID,
            serde_json::to_string(&json!({ "task": task }))
                .expect("encode delegate task payload"),
        ],
    )
    .expect("insert delegate task event");
}

fn assert_request_adapter_matches_plan_result(
    system: &dyn MemorySystem,
    expected_system_id: &str,
    config: &MemoryRuntimeConfig,
    recent_window: &[WindowTurn],
) {
    let request = system
        .build_retrieval_request(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            config,
            recent_window,
        )
        .expect("retrieval request");
    let result = system
        .build_retrieval_plan_result(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            config,
            recent_window,
        )
        .expect("retrieval plan");

    assert_eq!(&request, result.request());
    assert_eq!(request.memory_system_id, expected_system_id);
}

fn assert_boxed_request_adapter_matches_plan_result(
    boxed: Box<dyn MemorySystem>,
    expected_system_id: &str,
    config: &MemoryRuntimeConfig,
    recent_window: &[WindowTurn],
) {
    assert_request_adapter_matches_plan_result(&*boxed, expected_system_id, config, recent_window);
}

fn assert_request_helper_matches_request_adapter(
    system: &dyn MemorySystem,
    config: &MemoryRuntimeConfig,
    recent_window: &[WindowTurn],
) {
    let request = system
        .build_retrieval_request(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            config,
            recent_window,
        )
        .expect("retrieval request");
    let helper_request = system
        .build_retrieval_request_via_plan_result(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            config,
            recent_window,
        )
        .expect("retrieval request via plan result");

    assert_eq!(helper_request, request);
}

fn assert_plan_result_helper_matches_default(
    system: &dyn MemorySystem,
    config: &MemoryRuntimeConfig,
    recent_window: &[WindowTurn],
) {
    let default_result = system
        .build_retrieval_plan_result(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            config,
            recent_window,
        )
        .expect("default retrieval plan result");
    let helper_result = system
        .build_retrieval_plan_result_via_request(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            config,
            recent_window,
        )
        .expect("helper retrieval plan result");

    assert_eq!(helper_result, default_result);
}

fn assert_boxed_request_helper_matches_request_adapter(
    boxed: Box<dyn MemorySystem>,
    config: &MemoryRuntimeConfig,
    recent_window: &[WindowTurn],
) {
    assert_request_helper_matches_request_adapter(&*boxed, config, recent_window);
}

fn assert_boxed_plan_result_helper_matches_default(
    boxed: Box<dyn MemorySystem>,
    config: &MemoryRuntimeConfig,
    recent_window: &[WindowTurn],
) {
    assert_plan_result_helper_matches_default(&*boxed, config, recent_window);
}

fn assert_boxed_request_helper_matches_request_adapter_for_release_freeze(
    boxed: Box<dyn MemorySystem>,
) {
    let recent_window = release_freeze_recent_window();
    let config = release_freeze_test_config();
    assert_boxed_request_helper_matches_request_adapter(boxed, &config, recent_window.as_slice());
}

fn assert_request_adapter_matches_plan_result_for_release_freeze(
    system: &dyn MemorySystem,
    expected_system_id: &str,
) {
    let recent_window = release_freeze_recent_window();
    let config = release_freeze_test_config();
    assert_request_adapter_matches_plan_result(
        system,
        expected_system_id,
        &config,
        recent_window.as_slice(),
    );
}

fn assert_request_helper_matches_request_adapter_for_release_freeze(system: &dyn MemorySystem) {
    let recent_window = release_freeze_recent_window();
    let config = release_freeze_test_config();
    assert_request_helper_matches_request_adapter(system, &config, recent_window.as_slice());
}

fn assert_plan_result_helper_matches_default_for_release_freeze(system: &dyn MemorySystem) {
    let recent_window = release_freeze_recent_window();
    let config = release_freeze_test_config();
    assert_plan_result_helper_matches_default(system, &config, recent_window.as_slice());
}

fn assert_release_freeze_request_adapter_matches_plan_result(
    system: &dyn MemorySystem,
    expected_system_id: &str,
) {
    assert_request_adapter_matches_plan_result_for_release_freeze(system, expected_system_id);
}

fn assert_release_freeze_request_helper_matches_request_adapter(system: &dyn MemorySystem) {
    assert_request_helper_matches_request_adapter_for_release_freeze(system);
}

fn assert_release_freeze_plan_result_helper_matches_default(system: &dyn MemorySystem) {
    assert_plan_result_helper_matches_default_for_release_freeze(system);
}

fn assert_boxed_request_adapter_matches_plan_result_for_release_freeze(
    boxed: Box<dyn MemorySystem>,
    expected_system_id: &str,
) {
    let recent_window = release_freeze_recent_window();
    let config = release_freeze_test_config();
    assert_boxed_request_adapter_matches_plan_result(
        boxed,
        expected_system_id,
        &config,
        recent_window.as_slice(),
    );
}

fn assert_boxed_plan_result_helper_matches_default_for_release_freeze(
    boxed: Box<dyn MemorySystem>,
) {
    let recent_window = release_freeze_recent_window();
    let config = release_freeze_test_config();
    assert_boxed_plan_result_helper_matches_default(boxed, &config, recent_window.as_slice());
}

fn assert_boxed_request_only_default_plan_result_matches_explicit_helper(
    boxed: Box<dyn MemorySystem>,
    config: &MemoryRuntimeConfig,
) {
    assert_request_only_default_plan_result_matches_explicit_helper(&*boxed, config);
}

fn assert_boxed_request_only_plan_result_adapter_shape(
    boxed: Box<dyn MemorySystem>,
    config: &MemoryRuntimeConfig,
) {
    assert_request_only_plan_result_adapter_shape(&*boxed, config);
}

fn assert_boxed_plan_result_only_helper_request_matches_default_plan_result_request(
    boxed: Box<dyn MemorySystem>,
    config: &MemoryRuntimeConfig,
) {
    assert_plan_result_only_helper_request_matches_default_plan_result_request(&*boxed, config);
}

fn assert_boxed_plan_result_only_request_adapter_shape(
    boxed: Box<dyn MemorySystem>,
    config: &MemoryRuntimeConfig,
) {
    assert_plan_result_only_request_adapter_shape(&*boxed, config);
}

fn assert_boxed_plan_result_only_request_gap(
    boxed: Box<dyn MemorySystem>,
    config: &MemoryRuntimeConfig,
) {
    assert_plan_result_only_request_gap(&*boxed, config);
}

fn assert_retrieval_plan_shape(
    system: &dyn MemorySystem,
    config: &MemoryRuntimeConfig,
    expected_system_id: &str,
    expected_planning_notes: &[&str],
) {
    let result = system
        .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), config, &[])
        .expect("retrieval plan result");

    let expected_planning_notes = expected_planning_notes
        .iter()
        .map(|note| (*note).to_owned())
        .collect::<Vec<_>>();

    assert_plan_result_identity(&result, expected_system_id);
    assert_eq!(
        result.planner_snapshot.planning_notes,
        expected_planning_notes
    );
}

fn assert_plan_result_identity(result: &MemoryRetrievalPlanResult, expected_system_id: &str) {
    assert_eq!(result.request.memory_system_id, expected_system_id);
    assert_eq!(result.planner_snapshot.memory_system_id, expected_system_id);
}

fn assert_plan_result_snapshot_matches_request(
    result: &MemoryRetrievalPlanResult,
    expected_system_id: &str,
) {
    assert_plan_result_identity(result, expected_system_id);
    assert_eq!(result.planner_snapshot, result.request.planner_snapshot());
}

fn assert_plan_result_snapshot_matches_request_for_release_freeze(
    system: &dyn MemorySystem,
    expected_system_id: &str,
) -> MemoryRetrievalPlanResult {
    let recent_window = release_freeze_recent_window();
    let config = release_freeze_test_config();
    let result = system
        .build_retrieval_plan_result(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            &config,
            recent_window.as_slice(),
        )
        .expect("retrieval plan result");
    assert_plan_result_snapshot_matches_request(&result, expected_system_id);
    result
}

fn assert_plan_result_snapshot_matches_request_for_workspace_recall(
    system: &dyn MemorySystem,
) -> MemoryRetrievalPlanResult {
    let config = workspace_recall_test_config();
    let result = system
        .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), &config, &[])
        .expect("retrieval plan result");
    assert_plan_result_snapshot_matches_request(&result, WORKSPACE_RECALL_MEMORY_SYSTEM_ID);
    result
}

fn assert_boxed_plan_result_snapshot_matches_request(
    boxed: Box<dyn MemorySystem>,
    config: &MemoryRuntimeConfig,
    recent_window: &[WindowTurn],
    expected_system_id: &str,
) -> MemoryRetrievalPlanResult {
    let result = boxed
        .build_retrieval_plan_result(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            config,
            recent_window,
        )
        .expect("boxed retrieval plan result");
    assert_plan_result_snapshot_matches_request(&result, expected_system_id);
    result
}

fn assert_boxed_plan_result_snapshot_matches_request_for_release_freeze(
    boxed: Box<dyn MemorySystem>,
    expected_system_id: &str,
) -> MemoryRetrievalPlanResult {
    let recent_window = release_freeze_recent_window();
    let config = release_freeze_test_config();
    assert_boxed_plan_result_snapshot_matches_request(
        boxed,
        &config,
        recent_window.as_slice(),
        expected_system_id,
    )
}

fn assert_boxed_plan_result_snapshot_matches_request_for_workspace_recall(
    boxed: Box<dyn MemorySystem>,
) -> MemoryRetrievalPlanResult {
    let config = workspace_recall_test_config();
    assert_boxed_plan_result_snapshot_matches_request(
        boxed,
        &config,
        &[],
        WORKSPACE_RECALL_MEMORY_SYSTEM_ID,
    )
}

fn assert_workspace_recall_plan_result_snapshot_matches_request_and_budget(
    result: &MemoryRetrievalPlanResult,
) {
    assert_plan_result_snapshot_matches_request(result, WORKSPACE_RECALL_MEMORY_SYSTEM_ID);
    assert_eq!(result.request.budget_items, 4);
}

fn assert_retrieval_request_shape(
    system: &dyn MemorySystem,
    config: &MemoryRuntimeConfig,
    expected_system_id: &str,
    expected_planning_notes: &[&str],
) {
    let request = system
        .build_retrieval_request_via_plan_result(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            config,
            &[],
        )
        .expect("retrieval request via plan result");

    let expected_planning_notes = expected_planning_notes
        .iter()
        .map(|note| (*note).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(request.memory_system_id, expected_system_id);
    assert_eq!(request.planning_notes, expected_planning_notes);
}

fn assert_request_only_default_plan_result_matches_explicit_helper(
    system: &dyn MemorySystem,
    config: &MemoryRuntimeConfig,
) {
    let default_result = system
        .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), config, &[])
        .expect("default retrieval plan result");
    let helper_result = system
        .build_retrieval_plan_result_via_request(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            config,
            &[],
        )
        .expect("helper retrieval plan result");

    assert_eq!(helper_result, default_result);
}

fn assert_request_only_plan_result_adapter_shape(
    system: &dyn MemorySystem,
    config: &MemoryRuntimeConfig,
) {
    let result = system
        .build_retrieval_plan_result_via_request(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            config,
            &[],
        )
        .expect("retrieval plan result via request");

    assert_retrieval_plan_shape(
        system,
        config,
        REGISTRY_REQUEST_ONLY_ID,
        &[REGISTRY_REQUEST_NOTE],
    );
    assert_eq!(
        result.planner_snapshot.planning_notes,
        vec![REGISTRY_REQUEST_NOTE.to_owned()]
    );
}

fn assert_plan_result_only_helper_request_matches_default_plan_result_request(
    system: &dyn MemorySystem,
    config: &MemoryRuntimeConfig,
) {
    let helper_request = system
        .build_retrieval_request_via_plan_result(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            config,
            &[],
        )
        .expect("retrieval request via plan result");
    let default_result = system
        .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), config, &[])
        .expect("default retrieval plan result");

    assert_eq!(helper_request, default_result.request);
}

fn assert_plan_result_only_request_adapter_shape(
    system: &dyn MemorySystem,
    config: &MemoryRuntimeConfig,
) {
    let request = system
        .build_retrieval_request_via_plan_result(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            config,
            &[],
        )
        .expect("retrieval request via plan result");

    assert_retrieval_request_shape(
        system,
        config,
        PLAN_RESULT_ONLY_ID,
        &[PLAN_RESULT_ONLY_NOTE],
    );
    assert_eq!(
        request.planning_notes,
        vec![PLAN_RESULT_ONLY_NOTE.to_owned()]
    );
}

fn assert_plan_result_only_request_gap(system: &dyn MemorySystem, config: &MemoryRuntimeConfig) {
    let request =
        system.build_retrieval_request(TEST_SESSION_ID, Some(test_workspace_root()), config, &[]);
    let result = system
        .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), config, &[])
        .expect("retrieval plan result");

    assert_eq!(request, None);
    assert_plan_result_identity(&result, PLAN_RESULT_ONLY_ID);
    assert_eq!(
        result.planner_snapshot.planning_notes,
        vec![PLAN_RESULT_ONLY_NOTE.to_owned()]
    );
}

#[test]
fn builtin_memory_system_metadata_is_stable() {
    let metadata = BuiltinMemorySystem.metadata();
    assert_eq!(metadata.id, DEFAULT_MEMORY_SYSTEM_ID);
    assert_eq!(metadata.api_version, MEMORY_SYSTEM_API_VERSION);
    assert_eq!(
        metadata.runtime_fallback_kind,
        MemorySystemRuntimeFallbackKind::MetadataOnly
    );
    assert_eq!(
        metadata.capability_names(),
        vec![
            "canonical_store",
            "deterministic_summary",
            "profile_note_projection",
            "prompt_hydration",
            "retrieval_provenance",
        ]
    );
    assert_eq!(
        metadata.supported_recall_modes,
        vec![
            MemoryRecallMode::PromptAssembly,
            MemoryRecallMode::OperatorInspection
        ]
    );
}

#[test]
fn build_retrieval_plan_result_wraps_non_builtin_request_into_snapshot() {
    let config = default_test_config();

    let result = ResultWrappingRegistryMemorySystem
        .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), &config, &[])
        .expect("retrieval plan result");

    assert_retrieval_plan_shape(
        &ResultWrappingRegistryMemorySystem,
        &config,
        REGISTRY_REQUEST_ONLY_ID,
        &[REGISTRY_REQUEST_NOTE],
    );
    assert_eq!(
        result.planner_snapshot.strategy,
        MemoryRetrievalStrategy::WorkspaceReferenceOnly
    );
    assert_eq!(result.planner_snapshot.budget_items, 1);
    assert!(!result.planner_snapshot.query_present);
}

#[test]
fn build_retrieval_plan_result_via_request_adapts_request_only_system() {
    let config = default_test_config();
    assert_request_only_plan_result_adapter_shape(&ResultWrappingRegistryMemorySystem, &config);
}

#[test]
fn request_only_system_default_plan_result_matches_explicit_helper() {
    let config = default_test_config();
    assert_request_only_default_plan_result_matches_explicit_helper(
        &ResultWrappingRegistryMemorySystem,
        &config,
    );
}

#[test]
fn build_retrieval_request_default_does_not_backfill_from_plan_result() {
    let config = default_test_config();
    assert_plan_result_only_request_gap(&PlanResultOnlyRegistryMemorySystem, &config);
}

#[test]
fn plan_result_only_system_helper_request_matches_default_plan_result_request() {
    let config = default_test_config();
    assert_plan_result_only_helper_request_matches_default_plan_result_request(
        &PlanResultOnlyRegistryMemorySystem,
        &config,
    );
}

#[test]
fn build_retrieval_request_via_plan_result_adapts_plan_result_only_system() {
    let config = default_test_config();
    assert_plan_result_only_request_adapter_shape(&PlanResultOnlyRegistryMemorySystem, &config);
}

#[test]
fn boxed_memory_system_preserves_request_to_plan_result_compatibility() {
    let config = default_test_config();
    let boxed = boxed_request_only_registry_system();

    let result = boxed
        .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), &config, &[])
        .expect("retrieval plan result");

    assert_retrieval_plan_shape(
        &*boxed,
        &config,
        REGISTRY_REQUEST_ONLY_ID,
        &[REGISTRY_REQUEST_NOTE],
    );
    assert_eq!(
        result.planner_snapshot.planning_notes,
        vec![REGISTRY_REQUEST_NOTE.to_owned()]
    );
}

#[test]
fn boxed_memory_system_preserves_request_only_plan_result_adapter() {
    let config = default_test_config();
    assert_boxed_request_only_plan_result_adapter_shape(
        boxed_request_only_registry_system(),
        &config,
    );
}

#[test]
fn boxed_request_only_system_default_plan_result_matches_explicit_helper() {
    let config = default_test_config();
    assert_boxed_request_only_default_plan_result_matches_explicit_helper(
        boxed_request_only_registry_system(),
        &config,
    );
}

#[test]
fn boxed_memory_system_preserves_plan_result_only_request_adapter() {
    let config = default_test_config();
    assert_boxed_plan_result_only_request_adapter_shape(
        boxed_plan_result_only_registry_system(),
        &config,
    );
}

#[test]
fn boxed_plan_result_only_system_helper_request_matches_default_plan_result_request() {
    let config = default_test_config();
    assert_boxed_plan_result_only_helper_request_matches_default_plan_result_request(
        boxed_plan_result_only_registry_system(),
        &config,
    );
}

#[test]
fn boxed_memory_system_preserves_plan_result_only_request_gap() {
    let config = default_test_config();
    assert_boxed_plan_result_only_request_gap(boxed_plan_result_only_registry_system(), &config);
}

#[test]
fn memory_system_field_exposes_builtin_pre_assembly_stage_families() {
    let metadata = BuiltinMemorySystem.metadata();
    assert_eq!(
        metadata.supported_pre_assembly_stage_families,
        builtin_pre_assembly_stage_families()
    );
    assert_eq!(
        metadata.supported_stage_families,
        vec![
            MemoryStageFamily::Derive,
            MemoryStageFamily::Retrieve,
            MemoryStageFamily::Rank,
            MemoryStageFamily::Compact,
        ]
    );
}

#[test]
fn memory_system_field_allows_custom_registry_stage_family_sets() {
    let custom = StageAwareRegistryMemorySystem.metadata();
    assert_eq!(custom.id, "registry-stage-aware");
    assert_eq!(
        custom.supported_pre_assembly_stage_families,
        vec![MemoryStageFamily::Retrieve]
    );
    assert_eq!(
        custom.supported_stage_families,
        vec![MemoryStageFamily::Retrieve]
    );
    assert_eq!(
        custom.runtime_fallback_kind,
        MemorySystemRuntimeFallbackKind::SystemBacked
    );
    assert!(custom.supported_recall_modes.is_empty());
}

#[test]
fn memory_system_registry_includes_builtin_metadata() {
    let metadata = crate::memory::list_memory_system_metadata().expect("list memory systems");
    assert!(
        metadata
            .iter()
            .any(|entry| entry.id == DEFAULT_MEMORY_SYSTEM_ID)
    );
    assert!(
        metadata
            .iter()
            .any(|entry| entry.id == WORKSPACE_RECALL_MEMORY_SYSTEM_ID)
    );
    assert!(
        metadata
            .iter()
            .any(|entry| entry.id == RECALL_FIRST_MEMORY_SYSTEM_ID)
    );
}

#[test]
fn recall_first_memory_system_metadata_is_stable() {
    let metadata = RecallFirstMemorySystem.metadata();

    assert_eq!(metadata.id, RECALL_FIRST_MEMORY_SYSTEM_ID);
    assert_eq!(metadata.api_version, MEMORY_SYSTEM_API_VERSION);
    assert_eq!(
        metadata.runtime_fallback_kind,
        MemorySystemRuntimeFallbackKind::SystemBacked
    );
    assert_eq!(
        metadata.capability_names(),
        vec!["prompt_hydration", "retrieval_provenance"]
    );
    assert_eq!(
        metadata.supported_pre_assembly_stage_families,
        vec![
            MemoryStageFamily::Derive,
            MemoryStageFamily::Retrieve,
            MemoryStageFamily::Rank,
        ]
    );
    assert_eq!(
        metadata.supported_stage_families,
        vec![
            MemoryStageFamily::Derive,
            MemoryStageFamily::Retrieve,
            MemoryStageFamily::Rank,
        ]
    );
    assert_eq!(
        metadata.supported_recall_modes,
        vec![MemoryRecallMode::PromptAssembly]
    );
}

#[test]
fn recall_first_retrieval_plan_result_preserves_selected_system_id() {
    let result = assert_plan_result_snapshot_matches_request_for_release_freeze(
        &RecallFirstMemorySystem,
        RECALL_FIRST_MEMORY_SYSTEM_ID,
    );
    assert_eq!(result.planner_snapshot.strategy, result.request.strategy);
}

#[test]
fn recall_first_plan_result_helper_matches_default_plan_result() {
    assert_release_freeze_plan_result_helper_matches_default(&RecallFirstMemorySystem);
}

#[test]
fn recall_first_request_adapter_matches_plan_result_request() {
    assert_release_freeze_request_adapter_matches_plan_result(
        &RecallFirstMemorySystem,
        RECALL_FIRST_MEMORY_SYSTEM_ID,
    );
}

#[test]
fn recall_first_request_helper_matches_request_adapter() {
    assert_release_freeze_request_helper_matches_request_adapter(&RecallFirstMemorySystem);
}

#[test]
fn boxed_recall_first_request_helper_matches_request_adapter() {
    assert_boxed_request_helper_matches_request_adapter_for_release_freeze(
        boxed_recall_first_system(),
    );
}

#[test]
fn boxed_recall_first_request_adapter_matches_plan_result_request() {
    assert_boxed_request_adapter_matches_plan_result_for_release_freeze(
        boxed_recall_first_system(),
        RECALL_FIRST_MEMORY_SYSTEM_ID,
    );
}

#[test]
fn boxed_recall_first_plan_result_helper_matches_default_plan_result() {
    assert_boxed_plan_result_helper_matches_default_for_release_freeze(boxed_recall_first_system());
}

#[test]
fn boxed_recall_first_retrieval_plan_result_preserves_selected_system_id() {
    let result = assert_boxed_plan_result_snapshot_matches_request_for_release_freeze(
        boxed_recall_first_system(),
        RECALL_FIRST_MEMORY_SYSTEM_ID,
    );

    assert_eq!(result.planner_snapshot.strategy, result.request.strategy);
}

#[test]
fn builtin_request_adapter_matches_plan_result_request() {
    assert_release_freeze_request_adapter_matches_plan_result(
        &BuiltinMemorySystem,
        DEFAULT_MEMORY_SYSTEM_ID,
    );
}

#[test]
fn builtin_plan_result_helper_matches_default_plan_result() {
    assert_release_freeze_plan_result_helper_matches_default(&BuiltinMemorySystem);
}

#[test]
fn builtin_request_helper_matches_request_adapter() {
    assert_release_freeze_request_helper_matches_request_adapter(&BuiltinMemorySystem);
}

#[test]
fn boxed_builtin_request_helper_matches_request_adapter() {
    assert_boxed_request_helper_matches_request_adapter_for_release_freeze(boxed_builtin_system());
}

#[test]
fn boxed_builtin_request_adapter_matches_plan_result_request() {
    assert_boxed_request_adapter_matches_plan_result_for_release_freeze(
        boxed_builtin_system(),
        DEFAULT_MEMORY_SYSTEM_ID,
    );
}

#[test]
fn boxed_builtin_plan_result_helper_matches_default_plan_result() {
    assert_boxed_plan_result_helper_matches_default_for_release_freeze(boxed_builtin_system());
}

#[test]
fn builtin_retrieval_plan_result_matches_request_snapshot() {
    let _result = assert_plan_result_snapshot_matches_request_for_release_freeze(
        &BuiltinMemorySystem,
        DEFAULT_MEMORY_SYSTEM_ID,
    );
}

#[test]
fn boxed_builtin_retrieval_plan_result_matches_request_snapshot() {
    let _result = assert_boxed_plan_result_snapshot_matches_request_for_release_freeze(
        boxed_builtin_system(),
        DEFAULT_MEMORY_SYSTEM_ID,
    );
}

#[test]
fn builtin_retrieval_plan_combines_query_and_workspace_reference_scope() {
    let recent_window = release_freeze_recent_window();
    let config = MemoryRuntimeConfig {
        profile: crate::config::MemoryProfile::WindowPlusSummary,
        mode: crate::config::MemoryMode::WindowPlusSummary,
        sliding_window: 4,
        ..MemoryRuntimeConfig::default()
    };

    let plan = BuiltinRetrievalPlan::from_runtime_inputs(
        TEST_SESSION_ID,
        Some(test_workspace_root()),
        &config,
        recent_window.as_slice(),
    )
    .expect("retrieval plan");

    assert_eq!(
        plan.strategy,
        MemoryRetrievalStrategy::RecentUserQueryWithWorkspace
    );
    assert_eq!(plan.query.as_deref(), Some(RELEASE_FREEZE_TIMING));
    assert_eq!(plan.budget_items, 4);
    assert_eq!(
        plan.scopes,
        vec![
            MemoryScope::Session,
            MemoryScope::Workspace,
            MemoryScope::Agent,
            MemoryScope::User,
        ]
    );
    assert!(plan.allowed_kinds.contains(&DerivedMemoryKind::Reference));
}

#[test]
fn builtin_retrieval_plan_prioritizes_recent_user_query_over_other_hints() {
    let plan = BuiltinRetrievalPlan::from_planner_inputs(BuiltinRetrievalPlannerInputs {
        has_workspace_root: true,
        recent_user_query: Some("recent user ask".to_owned()),
        recent_user_budget_items: 4,
        structured_query: Some("structured seed".to_owned()),
        task_progress_plan: Some(SeededWorkspaceRetrievalPlan {
            strategy: MemoryRetrievalStrategy::TaskProgressIntentQueryWithWorkspace,
            planning_notes: vec!["task progress".to_owned()],
            query: "task progress seed".to_owned(),
            budget_items: 2,
            allowed_kinds: vec![DerivedMemoryKind::Procedure],
        }),
        workflow_task_plan: Some(SeededWorkspaceRetrievalPlan {
            strategy: MemoryRetrievalStrategy::WorkflowTaskQueryWithWorkspace,
            planning_notes: vec!["workflow task".to_owned()],
            query: "workflow seed".to_owned(),
            budget_items: 2,
            allowed_kinds: vec![DerivedMemoryKind::Overview],
        }),
        delegate_lineage_plan: Some(SeededWorkspaceRetrievalPlan::new(
            MemoryRetrievalStrategy::DelegateLabelQueryWithWorkspace,
            vec!["delegate label".to_owned()],
            "delegate label".to_owned(),
            1,
            vec![DerivedMemoryKind::Reference],
        )),
        has_structured_session_signals: true,
    })
    .expect("retrieval plan");

    assert_eq!(
        plan.strategy,
        MemoryRetrievalStrategy::RecentUserQueryWithWorkspace
    );
    assert_eq!(plan.query.as_deref(), Some("recent user ask"));
    assert_eq!(plan.budget_items, 4);
}

#[test]
fn builtin_retrieval_plan_into_result_populates_planner_snapshot() {
    let plan = BuiltinRetrievalPlan::recent_user_query(true, "recent user ask".to_owned(), 4);

    let result = plan.into_result(DEFAULT_MEMORY_SYSTEM_ID, TEST_SESSION_ID);

    assert_eq!(result.request.session_id, TEST_SESSION_ID);
    assert_plan_result_identity(&result, DEFAULT_MEMORY_SYSTEM_ID);
    assert_eq!(
        result.planner_snapshot.strategy,
        MemoryRetrievalStrategy::RecentUserQueryWithWorkspace
    );
    assert_eq!(result.planner_snapshot.budget_items, 4);
    assert!(result.planner_snapshot.query_present);
    assert!(
        result
            .planner_snapshot
            .planning_notes
            .iter()
            .any(|note: &String| note.contains("recent_user_query seed"))
    );
}

#[test]
fn builtin_retrieval_plan_workspace_only_mode_requests_reference_recall() {
    let config = MemoryRuntimeConfig::default();

    let plan = BuiltinRetrievalPlan::from_runtime_inputs(
        TEST_SESSION_ID,
        Some(test_workspace_root()),
        &config,
        &[],
    )
    .expect("retrieval plan");

    assert_eq!(
        plan.strategy,
        MemoryRetrievalStrategy::WorkspaceReferenceOnly
    );
    assert_eq!(plan.query, None);
    assert_eq!(plan.budget_items, 1);
    assert_eq!(
        plan.scopes,
        vec![MemoryScope::Workspace, MemoryScope::Session]
    );
    assert_eq!(plan.allowed_kinds, vec![DerivedMemoryKind::Reference]);
}

#[test]
fn builtin_retrieval_plan_workspace_mode_uses_structured_signal_strategy() {
    let recent_window = vec![WindowTurn {
        role: "assistant".to_owned(),
        content: crate::memory::build_tool_decision_content(
            "turn-1",
            "tool-1",
            json!({"tool_name": SHELL_EXEC_TOOL_NAME}),
        ),
        ts: None,
    }];
    let config = MemoryRuntimeConfig::default();

    let plan = BuiltinRetrievalPlan::from_runtime_inputs(
        TEST_SESSION_ID,
        Some(test_workspace_root()),
        &config,
        recent_window.as_slice(),
    )
    .expect("retrieval plan");

    assert_eq!(
        plan.strategy,
        MemoryRetrievalStrategy::StructuredSignalQueryWithWorkspace
    );
    assert_eq!(plan.query.as_deref(), Some(SHELL_EXEC_TOOL_NAME));
    assert_eq!(
        plan.allowed_kinds,
        vec![
            DerivedMemoryKind::Reference,
            DerivedMemoryKind::Procedure,
            DerivedMemoryKind::Fact,
        ]
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn builtin_retrieval_plan_workspace_mode_uses_task_progress_intent_query() {
    let (_temp_dir, workspace_root, config, conn) = sqlite_test_workspace();
    conn.execute(
        "INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, search_text, ts)
         VALUES (?1, ?2, NULL, ?3, '', 1)",
        rusqlite::params![
            TEST_SESSION_ID,
            crate::task_progress::TASK_PROGRESS_EVENT_KIND,
            serde_json::to_string(&crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: TEST_SESSION_ID.to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Waiting,
                    intent_summary: Some(DIAGNOSE_RELEASE_FREEZE_TEXT.to_owned()),
                    verification_state: None,
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 1,
                },
            ))
            .expect("encode task progress payload"),
        ],
    )
    .expect("insert task progress event");

    let plan = BuiltinRetrievalPlan::from_runtime_inputs(
        TEST_SESSION_ID,
        Some(workspace_root.as_path()),
        &config,
        &[],
    )
    .expect("retrieval plan");

    assert_eq!(
        plan.strategy,
        MemoryRetrievalStrategy::TaskProgressIntentQueryWithWorkspace
    );
    assert_eq!(plan.query.as_deref(), Some(DIAGNOSE_RELEASE_FREEZE_TEXT));
    assert_eq!(plan.budget_items, 2);
    assert_eq!(
        plan.allowed_kinds,
        vec![
            DerivedMemoryKind::Reference,
            DerivedMemoryKind::Procedure,
            DerivedMemoryKind::Overview,
            DerivedMemoryKind::Fact,
        ]
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn builtin_retrieval_plan_workspace_mode_uses_delegate_label_query() {
    let (_temp_dir, workspace_root, config, conn) = sqlite_test_workspace();
    insert_root_session(&conn);
    insert_delegate_child_session(&conn, "ready");

    let plan = BuiltinRetrievalPlan::from_runtime_inputs(
        TEST_SESSION_ID,
        Some(workspace_root.as_path()),
        &config,
        &[],
    )
    .expect("retrieval plan");

    assert_eq!(
        plan.strategy,
        MemoryRetrievalStrategy::DelegateLineageQueryWithWorkspace
    );
    let expected_query = expected_delegate_lineage_query();
    assert_eq!(plan.query.as_deref(), Some(expected_query.as_str()));
    assert_eq!(
        plan.allowed_kinds,
        vec![
            DerivedMemoryKind::Reference,
            DerivedMemoryKind::Procedure,
            DerivedMemoryKind::Overview,
        ]
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn builtin_retrieval_plan_workspace_mode_uses_workflow_task_query() {
    let (_temp_dir, workspace_root, config, conn) = sqlite_test_workspace();
    insert_delegate_child_session(&conn, "ready");
    insert_delegate_task_event(&conn, DELEGATE_STARTED_EVENT, WORKFLOW_TASK_TEXT);

    let plan = BuiltinRetrievalPlan::from_runtime_inputs(
        TEST_SESSION_ID,
        Some(workspace_root.as_path()),
        &config,
        &[],
    )
    .expect("retrieval plan");

    assert_eq!(
        plan.strategy,
        MemoryRetrievalStrategy::WorkflowTaskQueryWithWorkspace
    );
    assert_eq!(plan.query.as_deref(), Some(WORKFLOW_TASK_TEXT));
    assert_eq!(plan.budget_items, 2);
    assert!(
        plan.planning_notes
            .iter()
            .any(|note| note.contains("workflow task seed"))
    );
    assert_eq!(
        plan.allowed_kinds,
        vec![
            DerivedMemoryKind::Reference,
            DerivedMemoryKind::Procedure,
            DerivedMemoryKind::Overview,
            DerivedMemoryKind::Fact,
        ]
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn builtin_retrieval_plan_completed_workflow_task_uses_lower_budget() {
    let (_temp_dir, workspace_root, config, conn) = sqlite_test_workspace();
    insert_delegate_child_session(&conn, "completed");
    insert_delegate_task_event(&conn, DELEGATE_COMPLETED_EVENT, WORKFLOW_TASK_TEXT);

    let plan = BuiltinRetrievalPlan::from_runtime_inputs(
        TEST_SESSION_ID,
        Some(workspace_root.as_path()),
        &config,
        &[],
    )
    .expect("retrieval plan");

    assert_eq!(
        plan.strategy,
        MemoryRetrievalStrategy::WorkflowTaskQueryWithWorkspace
    );
    let expected_query = expected_workflow_task_query(WORKFLOW_PHASE_COMPLETE, None, None);
    assert_eq!(plan.query.as_deref(), Some(expected_query.as_str()));
    assert_eq!(plan.budget_items, 1);
    assert_eq!(
        plan.allowed_kinds,
        vec![
            DerivedMemoryKind::Reference,
            DerivedMemoryKind::Overview,
            DerivedMemoryKind::Fact,
        ]
    );
}

#[test]
fn workflow_task_query_seed_from_metadata_includes_non_execute_phase_and_non_task_scope() {
    let metadata = delegate_child_metadata_hint(
        WORKFLOW_PHASE_FAILED,
        Some(WORKFLOW_TASK_TEXT),
        Some(WORKFLOW_PHASE_FAILED),
        Some(WORKFLOW_OPERATION_KIND_APPROVAL),
        Some(WORKFLOW_OPERATION_SCOPE_SESSION),
    );

    let query =
        workflow_task_query_seed_from_metadata(&metadata).expect("workflow task query seed");

    let expected_query = expected_workflow_task_query(
        WORKFLOW_PHASE_FAILED,
        Some(WORKFLOW_OPERATION_KIND_APPROVAL),
        Some(WORKFLOW_OPERATION_SCOPE_SESSION),
    );
    assert_eq!(query, expected_query);
}

#[test]
fn structured_signal_query_seed_from_records_dedupes_terms_and_keeps_order() {
    let records = vec![
        crate::memory::CanonicalMemoryRecord {
            session_id: TEST_SECONDARY_SESSION_ID.to_owned(),
            scope: MemoryScope::Session,
            kind: CanonicalMemoryKind::ToolDecision,
            role: Some("assistant".to_owned()),
            content: String::new(),
            metadata: json!({
                "decision": {
                    "tool_name": SHELL_EXEC_TOOL_NAME
                }
            }),
        },
        crate::memory::CanonicalMemoryRecord {
            session_id: TEST_SECONDARY_SESSION_ID.to_owned(),
            scope: MemoryScope::Session,
            kind: CanonicalMemoryKind::ConversationEvent,
            role: Some("assistant".to_owned()),
            content: String::new(),
            metadata: json!({
                "event": DELEGATE_STARTED_EVENT
            }),
        },
        crate::memory::CanonicalMemoryRecord {
            session_id: TEST_SECONDARY_SESSION_ID.to_owned(),
            scope: MemoryScope::Session,
            kind: CanonicalMemoryKind::ToolOutcome,
            role: Some("assistant".to_owned()),
            content: String::new(),
            metadata: json!({
                "outcome": {
                    "tool_name": SHELL_EXEC_TOOL_NAME
                }
            }),
        },
    ];

    let query =
        structured_signal_query_seed_from_records(records.as_slice()).expect("structured seed");

    assert_eq!(query, expected_structured_signal_query());
}

#[test]
fn delegate_lineage_query_seed_from_metadata_uses_labels_when_present() {
    let metadata = delegate_child_metadata_hint("running", None, None, None, None);

    let query =
        delegate_lineage_query_seed_from_metadata(&metadata).expect("delegate lineage seed");

    assert_eq!(query, expected_delegate_lineage_query());
}

#[test]
fn workflow_task_policy_from_metadata_prefers_procedure_for_approval_kind() {
    let metadata = delegate_child_metadata_hint(
        WORKFLOW_PHASE_FAILED,
        Some(WORKFLOW_TASK_TEXT),
        Some(WORKFLOW_PHASE_FAILED),
        Some(WORKFLOW_OPERATION_KIND_APPROVAL),
        Some(WORKFLOW_OPERATION_SCOPE_SESSION),
    );

    let policy = workflow_task_policy_from_metadata(&metadata);

    assert_eq!(
        policy.allowed_kinds,
        vec![
            DerivedMemoryKind::Procedure,
            DerivedMemoryKind::Reference,
            DerivedMemoryKind::Fact,
        ]
    );
    assert_eq!(policy.budget_items, 1);
}

#[test]
fn workspace_recall_memory_system_metadata_is_stable() {
    let metadata = WorkspaceRecallMemorySystem.metadata();
    assert_eq!(metadata.id, WORKSPACE_RECALL_MEMORY_SYSTEM_ID);
    assert_eq!(
        metadata.runtime_fallback_kind,
        MemorySystemRuntimeFallbackKind::SystemBacked
    );
    assert_eq!(
        metadata.capability_names(),
        vec!["prompt_hydration", "retrieval_provenance"]
    );
    assert_eq!(
        metadata.supported_pre_assembly_stage_families,
        vec![
            MemoryStageFamily::Derive,
            MemoryStageFamily::Retrieve,
            MemoryStageFamily::Rank,
        ]
    );
    assert_eq!(
        metadata.supported_stage_families,
        vec![
            MemoryStageFamily::Derive,
            MemoryStageFamily::Retrieve,
            MemoryStageFamily::Rank,
        ]
    );
    assert_eq!(
        metadata.supported_recall_modes,
        vec![
            MemoryRecallMode::PromptAssembly,
            MemoryRecallMode::OperatorInspection
        ]
    );
}

#[test]
fn workspace_recall_retrieval_plan_result_matches_request_snapshot() {
    let result = assert_plan_result_snapshot_matches_request_for_workspace_recall(
        &WorkspaceRecallMemorySystem,
    );
    assert_workspace_recall_plan_result_snapshot_matches_request_and_budget(&result);
}

#[test]
fn workspace_recall_plan_result_helper_matches_default_plan_result() {
    assert_plan_result_helper_matches_default_for_workspace_recall(&WorkspaceRecallMemorySystem);
}

#[test]
fn workspace_recall_request_adapter_matches_plan_result_request() {
    assert_request_adapter_matches_plan_result_for_workspace_recall(&WorkspaceRecallMemorySystem);
    assert_workspace_recall_request_budget_items(&WorkspaceRecallMemorySystem, 4);
}

#[test]
fn workspace_recall_request_helper_matches_request_adapter() {
    assert_request_helper_matches_request_adapter_for_workspace_recall(
        &WorkspaceRecallMemorySystem,
    );
}

#[test]
fn boxed_workspace_recall_request_helper_matches_request_adapter() {
    assert_boxed_request_helper_matches_request_adapter_for_workspace_recall(
        boxed_workspace_recall_system(),
    );
}

#[test]
fn boxed_workspace_recall_request_adapter_matches_plan_result_request() {
    assert_boxed_request_adapter_matches_plan_result_for_workspace_recall(
        boxed_workspace_recall_system(),
    );
}

#[test]
fn boxed_workspace_recall_request_budget_items_match_expected() {
    assert_boxed_workspace_recall_request_budget_items(boxed_workspace_recall_system(), 4);
}

#[test]
fn boxed_workspace_recall_plan_result_helper_matches_default_plan_result() {
    assert_boxed_plan_result_helper_matches_default_for_workspace_recall(
        boxed_workspace_recall_system(),
    );
}

#[test]
fn boxed_workspace_recall_retrieval_plan_result_matches_request_snapshot() {
    let result = assert_boxed_plan_result_snapshot_matches_request_for_workspace_recall(
        boxed_workspace_recall_system(),
    );
    assert_workspace_recall_plan_result_snapshot_matches_request_and_budget(&result);
}

#[test]
fn supported_stage_families_replace_previous_explicit_stage_set() {
    let metadata = MemorySystemMetadata::new(
        "stage-reset",
        [MemorySystemCapability::PromptHydration],
        "Stage reset test system",
    )
    .with_supported_pre_assembly_stage_families([
        MemoryStageFamily::Derive,
        MemoryStageFamily::Retrieve,
    ])
    .with_supported_stage_families([MemoryStageFamily::Compact])
    .with_supported_stage_families([MemoryStageFamily::AfterTurn]);

    assert_eq!(
        metadata.supported_stage_families,
        vec![
            MemoryStageFamily::Derive,
            MemoryStageFamily::Retrieve,
            MemoryStageFamily::AfterTurn,
        ]
    );
}

#[test]
fn supported_stage_builders_auto_promote_runtime_fallback_to_system_backed() {
    let metadata = MemorySystemMetadata::new(
        "auto-promote",
        [MemorySystemCapability::PromptHydration],
        "Auto promote test system",
    )
    .with_supported_pre_assembly_stage_families([MemoryStageFamily::Retrieve]);

    assert_eq!(
        metadata.runtime_fallback_kind,
        MemorySystemRuntimeFallbackKind::SystemBacked
    );
}

#[test]
fn memory_system_runtime_snapshot_defaults_to_builtin() {
    let config = crate::config::LoongConfig::default();
    let snapshot = crate::memory::collect_memory_system_runtime_snapshot(&config)
        .expect("collect memory-system snapshot");
    assert_eq!(snapshot.selected.id, DEFAULT_MEMORY_SYSTEM_ID);
    assert_eq!(
        snapshot.selected.source,
        crate::memory::MemorySystemSelectionSource::Default
    );
    assert_eq!(snapshot.selected_metadata.id, DEFAULT_MEMORY_SYSTEM_ID);
}

#[test]
fn workspace_recall_rank_stage_keeps_summary_without_retrieved_entries() {
    let entries = vec![
        MemoryContextEntry {
            kind: MemoryContextKind::Profile,
            role: "system".to_owned(),
            content: "profile".to_owned(),
            provenance: Vec::new(),
        },
        MemoryContextEntry {
            kind: MemoryContextKind::Summary,
            role: "system".to_owned(),
            content: "summary".to_owned(),
            provenance: Vec::new(),
        },
        MemoryContextEntry {
            kind: MemoryContextKind::Turn,
            role: "user".to_owned(),
            content: "turn".to_owned(),
            provenance: Vec::new(),
        },
    ];

    let ranked_entries = WorkspaceRecallMemorySystem
        .run_rank_stage(entries, &MemoryRuntimeConfig::default())
        .expect("rank stage should succeed")
        .expect("workspace recall rank stage should return entries");

    let kinds = ranked_entries
        .into_iter()
        .map(|entry| entry.kind)
        .collect::<Vec<_>>();

    assert_eq!(
        kinds,
        vec![
            MemoryContextKind::Profile,
            MemoryContextKind::Summary,
            MemoryContextKind::Turn,
        ]
    );
}

#[test]
fn workspace_recall_rank_stage_drops_summary_when_retrieved_entries_exist() {
    let entries = vec![
        MemoryContextEntry {
            kind: MemoryContextKind::Summary,
            role: "system".to_owned(),
            content: "summary".to_owned(),
            provenance: Vec::new(),
        },
        MemoryContextEntry {
            kind: MemoryContextKind::RetrievedMemory,
            role: "system".to_owned(),
            content: "retrieved".to_owned(),
            provenance: Vec::new(),
        },
        MemoryContextEntry {
            kind: MemoryContextKind::Turn,
            role: "user".to_owned(),
            content: "turn".to_owned(),
            provenance: Vec::new(),
        },
    ];

    let ranked_entries = WorkspaceRecallMemorySystem
        .run_rank_stage(entries, &MemoryRuntimeConfig::default())
        .expect("rank stage should succeed")
        .expect("workspace recall rank stage should return entries");

    let kinds = ranked_entries
        .into_iter()
        .map(|entry| entry.kind)
        .collect::<Vec<_>>();

    assert_eq!(
        kinds,
        vec![MemoryContextKind::RetrievedMemory, MemoryContextKind::Turn]
    );
}

#[test]
fn recall_first_rank_stage_drops_summary_when_retrieved_entries_exist() {
    let entries = vec![
        MemoryContextEntry {
            kind: MemoryContextKind::Summary,
            role: "system".to_owned(),
            content: "summary".to_owned(),
            provenance: Vec::new(),
        },
        MemoryContextEntry {
            kind: MemoryContextKind::Profile,
            role: "system".to_owned(),
            content: "profile".to_owned(),
            provenance: Vec::new(),
        },
        MemoryContextEntry {
            kind: MemoryContextKind::RetrievedMemory,
            role: "system".to_owned(),
            content: "retrieved".to_owned(),
            provenance: Vec::new(),
        },
        MemoryContextEntry {
            kind: MemoryContextKind::Turn,
            role: "user".to_owned(),
            content: "turn".to_owned(),
            provenance: Vec::new(),
        },
    ];

    let runtime_config = MemoryRuntimeConfig::default();
    let ranked_entries_result = RecallFirstMemorySystem.run_rank_stage(entries, &runtime_config);
    let ranked_entries_option = ranked_entries_result.expect("rank stage should succeed");
    let ranked_entries =
        ranked_entries_option.expect("recall-first rank stage should return entries");

    let kinds = ranked_entries
        .into_iter()
        .map(|entry| entry.kind)
        .collect::<Vec<_>>();

    assert_eq!(
        kinds,
        vec![
            MemoryContextKind::Profile,
            MemoryContextKind::RetrievedMemory,
            MemoryContextKind::Turn,
        ]
    );
}

#[test]
fn filter_cross_session_recall_hits_respects_scopes_and_allowed_kinds() {
    let profile_hit = CanonicalMemorySearchHit {
        record: crate::memory::CanonicalMemoryRecord {
            session_id: "profile-session".to_owned(),
            scope: MemoryScope::Workspace,
            kind: CanonicalMemoryKind::ImportedProfile,
            role: None,
            content: "release checklist".to_owned(),
            metadata: json!({}),
        },
        session_turn_index: Some(1),
    };
    let turn_hit = CanonicalMemorySearchHit {
        record: crate::memory::CanonicalMemoryRecord {
            session_id: "turn-session".to_owned(),
            scope: MemoryScope::Session,
            kind: CanonicalMemoryKind::AssistantTurn,
            role: Some("assistant".to_owned()),
            content: "deployment cutoff is 17:00".to_owned(),
            metadata: json!({}),
        },
        session_turn_index: Some(2),
    };
    let request = MemoryRetrievalRequest {
        session_id: "active-session".to_owned(),
        memory_system_id: DEFAULT_MEMORY_SYSTEM_ID.to_owned(),
        strategy: MemoryRetrievalStrategy::RecentUserQuery,
        planning_notes: Vec::new(),
        query: Some("deployment release".to_owned()),
        recall_mode: MemoryRecallMode::PromptAssembly,
        scopes: vec![MemoryScope::Workspace],
        budget_items: 4,
        allowed_kinds: vec![DerivedMemoryKind::Profile],
    };

    let filtered_hits = filter_cross_session_recall_hits(&request, vec![profile_hit, turn_hit]);

    assert_eq!(filtered_hits.len(), 1);
    assert_eq!(filtered_hits[0].record.session_id, "profile-session");
    assert_eq!(
        filtered_hits[0].record.kind,
        CanonicalMemoryKind::ImportedProfile
    );
}

#[test]
fn build_cross_session_recall_entries_attach_canonical_record_provenance() {
    let hit = CanonicalMemorySearchHit {
        record: crate::memory::CanonicalMemoryRecord {
            session_id: "prior-session".to_owned(),
            scope: MemoryScope::Session,
            kind: CanonicalMemoryKind::AssistantTurn,
            role: Some("assistant".to_owned()),
            content: "deployment cutoff is 17:00 Beijing time".to_owned(),
            metadata: json!({}),
        },
        session_turn_index: Some(3),
    };

    let entries = build_cross_session_recall_entries(
        DEFAULT_MEMORY_SYSTEM_ID,
        MemoryRecallMode::PromptAssembly,
        &[hit],
    );

    assert_eq!(entries.len(), 1);
    assert!(
        entries[0]
            .content
            .contains("Cross-session source: prior-session")
    );
    assert_eq!(entries[0].provenance.len(), 1);
    assert_eq!(
        entries[0].provenance[0].source_kind,
        MemoryProvenanceSourceKind::CanonicalMemoryRecord
    );
    assert_eq!(entries[0].provenance[0].scope, Some(MemoryScope::Session));
    assert_eq!(
        entries[0].provenance[0].trust_level,
        Some(MemoryTrustLevel::Session)
    );
    assert_eq!(
        entries[0].provenance[0].record_status,
        Some(MemoryRecordStatus::Active)
    );
}

#[test]
fn builtin_memory_system_derives_session_local_overview_from_structured_turns() {
    let recent_window = vec![
        WindowTurn {
            role: "assistant".to_owned(),
            content: crate::memory::build_tool_decision_content(
                "turn-1",
                "call-1",
                json!({"tool": "memory_search"}),
            ),
            ts: Some(10),
        },
        WindowTurn {
            role: "assistant".to_owned(),
            content: crate::memory::build_conversation_event_content(
                "tool_discovery",
                json!({"state": "visible"}),
            ),
            ts: Some(20),
        },
    ];

    let derived_entries = BuiltinMemorySystem
        .run_derive_stage(
            "session-local-overview-session",
            &MemoryRuntimeConfig::default(),
            recent_window.as_slice(),
        )
        .expect("derive stage should succeed")
        .expect("derive stage should return entries");

    assert_eq!(derived_entries.len(), 1);
    assert_eq!(derived_entries[0].kind, MemoryContextKind::Derived);
    assert!(
        derived_entries[0]
            .content
            .contains("## Session Local Overview")
    );
    assert_eq!(
        derived_entries[0].provenance[0].source_kind,
        MemoryProvenanceSourceKind::DerivedSessionOverview
    );
    assert_eq!(
        derived_entries[0].provenance[0].derived_kind,
        Some(DerivedMemoryKind::Overview)
    );
}

#[test]
fn builtin_rank_stage_filters_inactive_workspace_entries_and_orders_advisory_blocks() {
    let inactive_entry = MemoryContextEntry {
        kind: MemoryContextKind::RetrievedMemory,
        role: "system".to_owned(),
        content: "inactive".to_owned(),
        provenance: vec![
            MemoryContextProvenance::new(
                DEFAULT_MEMORY_SYSTEM_ID,
                MemoryProvenanceSourceKind::WorkspaceDocument,
                Some("MEMORY.md".to_owned()),
                None,
                Some(MemoryScope::Workspace),
                MemoryRecallMode::PromptAssembly,
            )
            .with_trust_level(MemoryTrustLevel::WorkspaceCurated)
            .with_record_status(MemoryRecordStatus::Tombstoned),
        ],
    };
    let summary_entry = MemoryContextEntry {
        kind: MemoryContextKind::Summary,
        role: "system".to_owned(),
        content: "summary".to_owned(),
        provenance: vec![
            MemoryContextProvenance::new(
                DEFAULT_MEMORY_SYSTEM_ID,
                MemoryProvenanceSourceKind::SummaryCheckpoint,
                Some("summary_checkpoint".to_owned()),
                None,
                Some(MemoryScope::Session),
                MemoryRecallMode::PromptAssembly,
            )
            .with_trust_level(MemoryTrustLevel::Derived)
            .with_record_status(MemoryRecordStatus::Active),
        ],
    };
    let derived_entry = MemoryContextEntry {
        kind: MemoryContextKind::Derived,
        role: "system".to_owned(),
        content: "derived".to_owned(),
        provenance: vec![
            MemoryContextProvenance::new(
                DEFAULT_MEMORY_SYSTEM_ID,
                MemoryProvenanceSourceKind::DerivedSessionOverview,
                Some("session_local_overview".to_owned()),
                None,
                Some(MemoryScope::Session),
                MemoryRecallMode::PromptAssembly,
            )
            .with_trust_level(MemoryTrustLevel::Derived)
            .with_record_status(MemoryRecordStatus::Active),
        ],
    };
    let turn_entry = MemoryContextEntry {
        kind: MemoryContextKind::Turn,
        role: "user".to_owned(),
        content: "turn".to_owned(),
        provenance: Vec::new(),
    };

    let ranked_entries = BuiltinMemorySystem
        .run_rank_stage(
            vec![turn_entry, inactive_entry, derived_entry, summary_entry],
            &MemoryRuntimeConfig::default(),
        )
        .expect("rank stage should succeed")
        .expect("rank stage should return entries");

    let kinds = ranked_entries
        .into_iter()
        .map(|entry| entry.kind)
        .collect::<Vec<_>>();

    assert_eq!(
        kinds,
        vec![
            MemoryContextKind::Summary,
            MemoryContextKind::Derived,
            MemoryContextKind::Turn,
        ]
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn builtin_retrieve_stage_keeps_allowed_hits_when_top_match_is_filtered_out() {
    let (_temp_dir, _workspace_root, config, _conn) = sqlite_test_workspace();
    let allowed_payload = json!({
        "type": crate::memory::CANONICAL_MEMORY_RECORD_TYPE,
        "_loong_internal": true,
        "scope": "workspace",
        "kind": "imported_profile",
        "content": "release checklist",
        "metadata": {
            "source": "workspace-import"
        },
    })
    .to_string();
    let recent_window = Vec::new();
    let request = MemoryRetrievalRequest {
        session_id: "active-session".to_owned(),
        memory_system_id: DEFAULT_MEMORY_SYSTEM_ID.to_owned(),
        strategy: MemoryRetrievalStrategy::RecentUserQuery,
        planning_notes: Vec::new(),
        query: Some("release checklist".to_owned()),
        recall_mode: MemoryRecallMode::PromptAssembly,
        scopes: vec![MemoryScope::Workspace],
        budget_items: 1,
        allowed_kinds: vec![DerivedMemoryKind::Profile],
    };

    crate::memory::append_turn_direct(
        "workspace-session",
        "assistant",
        allowed_payload.as_str(),
        &config,
    )
    .expect("append allowed canonical payload");
    crate::memory::append_turn_direct("session-session", "assistant", "release checklist", &config)
        .expect("append disallowed session hit");

    let entries = BuiltinMemorySystem
        .run_retrieve_stage(&request, None, &config, recent_window.as_slice())
        .expect("retrieve stage should succeed")
        .expect("retrieve stage should return entries");

    assert_eq!(entries.len(), 1);
    assert!(entries[0].content.contains("workspace-session"));
    assert!(!entries[0].content.contains("session-session"));
    assert_eq!(entries[0].provenance.len(), 1);
    assert_eq!(entries[0].provenance[0].scope, Some(MemoryScope::Workspace));
}
