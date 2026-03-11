use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use async_trait::async_trait;
use tokio::time::{timeout, Duration};

use super::plan_ir::{PlanGraph, PlanNode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRunStatus {
    Succeeded,
    Failed(PlanRunFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRunFailure {
    ValidationFailed(String),
    TopologyResolutionFailed,
    BudgetExceeded {
        attempts_used: usize,
        limit: usize,
    },
    WallTimeExceeded {
        elapsed_ms: u128,
        limit_ms: u64,
    },
    NodeFailed {
        node_id: String,
        attempts_used: u8,
        last_error_kind: PlanNodeErrorKind,
        last_error: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanNodeErrorKind {
    Retryable,
    PolicyDenied,
    NonRetryable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanNodeError {
    pub kind: PlanNodeErrorKind,
    pub message: String,
}

impl PlanNodeError {
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            kind: PlanNodeErrorKind::Retryable,
            message: message.into(),
        }
    }

    pub fn policy_denied(message: impl Into<String>) -> Self {
        Self {
            kind: PlanNodeErrorKind::PolicyDenied,
            message: message.into(),
        }
    }

    pub fn non_retryable(message: impl Into<String>) -> Self {
        Self {
            kind: PlanNodeErrorKind::NonRetryable,
            message: message.into(),
        }
    }
}

impl From<String> for PlanNodeError {
    fn from(value: String) -> Self {
        Self::non_retryable(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanNodeAttemptEvent {
    pub node_id: String,
    pub attempt: u8,
    pub success: bool,
    pub error_kind: Option<PlanNodeErrorKind>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRunReport {
    pub status: PlanRunStatus,
    pub ordered_nodes: Vec<String>,
    pub attempts_used: usize,
    pub attempt_events: Vec<PlanNodeAttemptEvent>,
    pub elapsed_ms: u128,
}

#[async_trait]
pub trait PlanNodeExecutor: Send + Sync {
    async fn execute(&self, node: &PlanNode, attempt: u8) -> Result<(), PlanNodeError>;
}

pub struct PlanExecutor;

impl PlanExecutor {
    pub async fn execute<E: PlanNodeExecutor>(plan: &PlanGraph, executor: &E) -> PlanRunReport {
        if let Err(error) = plan.validate() {
            return PlanRunReport {
                status: PlanRunStatus::Failed(PlanRunFailure::ValidationFailed(error)),
                ordered_nodes: Vec::new(),
                attempts_used: 0,
                attempt_events: Vec::new(),
                elapsed_ms: 0,
            };
        }

        let ordered_indexes = match topological_order(plan) {
            Some(order) => order,
            None => {
                return PlanRunReport {
                    status: PlanRunStatus::Failed(PlanRunFailure::TopologyResolutionFailed),
                    ordered_nodes: Vec::new(),
                    attempts_used: 0,
                    attempt_events: Vec::new(),
                    elapsed_ms: 0,
                };
            }
        };

        let mut attempts_used = 0usize;
        let mut events = Vec::new();
        let started_at = Instant::now();
        let Some(ordered_nodes) = ordered_indexes
            .iter()
            .map(|index| plan.nodes.get(*index).map(|node| node.id.clone()))
            .collect::<Option<Vec<_>>>()
        else {
            return PlanRunReport {
                status: PlanRunStatus::Failed(PlanRunFailure::TopologyResolutionFailed),
                ordered_nodes: Vec::new(),
                attempts_used: 0,
                attempt_events: Vec::new(),
                elapsed_ms: 0,
            };
        };

        for node_index in ordered_indexes {
            let Some(node) = plan.nodes.get(node_index) else {
                return PlanRunReport {
                    status: PlanRunStatus::Failed(PlanRunFailure::TopologyResolutionFailed),
                    ordered_nodes,
                    attempts_used,
                    attempt_events: events,
                    elapsed_ms: started_at.elapsed().as_millis(),
                };
            };

            for attempt in 1..=node.max_attempts {
                let elapsed_ms = started_at.elapsed().as_millis();
                if elapsed_ms > plan.budget.max_wall_time_ms as u128 {
                    return PlanRunReport {
                        status: PlanRunStatus::Failed(PlanRunFailure::WallTimeExceeded {
                            elapsed_ms,
                            limit_ms: plan.budget.max_wall_time_ms,
                        }),
                        ordered_nodes,
                        attempts_used,
                        attempt_events: events,
                        elapsed_ms,
                    };
                }

                if attempts_used >= plan.budget.max_total_attempts {
                    return PlanRunReport {
                        status: PlanRunStatus::Failed(PlanRunFailure::BudgetExceeded {
                            attempts_used,
                            limit: plan.budget.max_total_attempts,
                        }),
                        ordered_nodes,
                        attempts_used,
                        attempt_events: events,
                        elapsed_ms,
                    };
                }
                attempts_used = attempts_used.saturating_add(1);

                let attempt_outcome = timeout(
                    Duration::from_millis(node.timeout_ms.max(1)),
                    executor.execute(node, attempt),
                )
                .await;
                match attempt_outcome {
                    Err(_) => {
                        let timeout_error =
                            normalize_node_error(PlanNodeError::retryable(format!(
                                "node_timeout node={} timeout_ms={}",
                                node.id, node.timeout_ms
                            )));
                        events.push(PlanNodeAttemptEvent {
                            node_id: node.id.clone(),
                            attempt,
                            success: false,
                            error_kind: Some(timeout_error.kind),
                            error: Some(timeout_error.message.clone()),
                        });
                        if attempt == node.max_attempts {
                            let elapsed_ms = started_at.elapsed().as_millis();
                            return PlanRunReport {
                                status: PlanRunStatus::Failed(PlanRunFailure::NodeFailed {
                                    node_id: node.id.clone(),
                                    attempts_used: attempt,
                                    last_error_kind: timeout_error.kind,
                                    last_error: timeout_error.message,
                                }),
                                ordered_nodes,
                                attempts_used,
                                attempt_events: events,
                                elapsed_ms,
                            };
                        }
                    }
                    Ok(outcome) => match outcome {
                        Ok(()) => {
                            events.push(PlanNodeAttemptEvent {
                                node_id: node.id.clone(),
                                attempt,
                                success: true,
                                error_kind: None,
                                error: None,
                            });
                            break;
                        }
                        Err(error) => {
                            let normalized = normalize_node_error(error);
                            events.push(PlanNodeAttemptEvent {
                                node_id: node.id.clone(),
                                attempt,
                                success: false,
                                error_kind: Some(normalized.kind),
                                error: Some(normalized.message.clone()),
                            });
                            if attempt == node.max_attempts {
                                let elapsed_ms = started_at.elapsed().as_millis();
                                return PlanRunReport {
                                    status: PlanRunStatus::Failed(PlanRunFailure::NodeFailed {
                                        node_id: node.id.clone(),
                                        attempts_used: attempt,
                                        last_error_kind: normalized.kind,
                                        last_error: normalized.message,
                                    }),
                                    ordered_nodes,
                                    attempts_used,
                                    attempt_events: events,
                                    elapsed_ms,
                                };
                            }
                        }
                    },
                }
            }
        }

        PlanRunReport {
            status: PlanRunStatus::Succeeded,
            ordered_nodes,
            attempts_used,
            attempt_events: events,
            elapsed_ms: started_at.elapsed().as_millis(),
        }
    }
}

fn normalize_node_error(error: PlanNodeError) -> PlanNodeError {
    let trimmed = error.message.trim();
    if trimmed.is_empty() {
        return PlanNodeError {
            kind: error.kind,
            message: "node execution failed with empty reason".to_owned(),
        };
    }
    PlanNodeError {
        kind: error.kind,
        message: trimmed.to_owned(),
    }
}

fn topological_order(plan: &PlanGraph) -> Option<Vec<usize>> {
    let mut node_index = BTreeMap::new();
    for (index, node) in plan.nodes.iter().enumerate() {
        node_index.insert(node.id.as_str(), index);
    }

    let mut indegree = vec![0usize; plan.nodes.len()];
    let mut adjacency = vec![Vec::<usize>::new(); plan.nodes.len()];
    for edge in &plan.edges {
        let from = *node_index.get(edge.from.as_str())?;
        let to = *node_index.get(edge.to.as_str())?;
        let degree = indegree.get_mut(to)?;
        *degree = degree.saturating_add(1);
        adjacency.get_mut(from)?.push(to);
    }

    let mut queue = VecDeque::new();
    for (index, degree) in indegree.iter().enumerate() {
        if *degree == 0 {
            queue.push_back(index);
        }
    }

    let mut order = Vec::new();
    while let Some(index) = queue.pop_front() {
        order.push(index);
        for next in adjacency.get(index)? {
            let next_degree = indegree.get_mut(*next)?;
            *next_degree = next_degree.saturating_sub(1);
            if *next_degree == 0 {
                queue.push_back(*next);
            }
        }
    }
    if order.len() == plan.nodes.len() {
        Some(order)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::conversation::plan_ir::{
        PlanBudget, PlanEdge, PlanGraph, PlanNode, PlanNodeKind, RiskTier, PLAN_GRAPH_VERSION,
    };

    fn sample_graph() -> PlanGraph {
        PlanGraph {
            version: PLAN_GRAPH_VERSION.to_owned(),
            nodes: vec![
                PlanNode {
                    id: "n1".to_owned(),
                    kind: PlanNodeKind::Tool,
                    label: "collect".to_owned(),
                    tool_name: Some("file.read".to_owned()),
                    timeout_ms: 1_000,
                    max_attempts: 1,
                    risk_tier: RiskTier::Low,
                },
                PlanNode {
                    id: "n2".to_owned(),
                    kind: PlanNodeKind::Verify,
                    label: "verify".to_owned(),
                    tool_name: None,
                    timeout_ms: 1_000,
                    max_attempts: 2,
                    risk_tier: RiskTier::Medium,
                },
                PlanNode {
                    id: "n3".to_owned(),
                    kind: PlanNodeKind::Respond,
                    label: "reply".to_owned(),
                    tool_name: None,
                    timeout_ms: 1_000,
                    max_attempts: 1,
                    risk_tier: RiskTier::Low,
                },
            ],
            edges: vec![
                PlanEdge {
                    from: "n1".to_owned(),
                    to: "n2".to_owned(),
                },
                PlanEdge {
                    from: "n2".to_owned(),
                    to: "n3".to_owned(),
                },
            ],
            budget: PlanBudget {
                max_nodes: 16,
                max_total_attempts: 8,
                max_wall_time_ms: 5_000,
            },
        }
    }

    struct RecordingExecutor {
        calls: Mutex<Vec<String>>,
        fail_once_nodes: Vec<String>,
        seen_attempts: Mutex<BTreeMap<String, u8>>,
        always_fail_node: Option<String>,
    }

    impl RecordingExecutor {
        fn succeed_all() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_once_nodes: Vec::new(),
                seen_attempts: Mutex::new(BTreeMap::new()),
                always_fail_node: None,
            }
        }

        fn fail_once(node_id: &str) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_once_nodes: vec![node_id.to_owned()],
                seen_attempts: Mutex::new(BTreeMap::new()),
                always_fail_node: None,
            }
        }

        fn always_fail(node_id: &str) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_once_nodes: Vec::new(),
                seen_attempts: Mutex::new(BTreeMap::new()),
                always_fail_node: Some(node_id.to_owned()),
            }
        }
    }

    struct TimeoutOnFirstAttemptExecutor {
        timeout_node: String,
    }

    impl TimeoutOnFirstAttemptExecutor {
        fn new(node_id: &str) -> Self {
            Self {
                timeout_node: node_id.to_owned(),
            }
        }
    }

    #[async_trait]
    impl PlanNodeExecutor for TimeoutOnFirstAttemptExecutor {
        async fn execute(&self, node: &PlanNode, attempt: u8) -> Result<(), PlanNodeError> {
            if node.id == self.timeout_node && attempt == 1 {
                tokio::time::sleep(Duration::from_millis(node.timeout_ms.saturating_add(50))).await;
            }
            Ok(())
        }
    }

    struct TimeoutAlwaysExecutor {
        timeout_node: String,
    }

    impl TimeoutAlwaysExecutor {
        fn new(node_id: &str) -> Self {
            Self {
                timeout_node: node_id.to_owned(),
            }
        }
    }

    #[async_trait]
    impl PlanNodeExecutor for TimeoutAlwaysExecutor {
        async fn execute(&self, node: &PlanNode, _attempt: u8) -> Result<(), PlanNodeError> {
            if node.id == self.timeout_node {
                tokio::time::sleep(Duration::from_millis(node.timeout_ms.saturating_add(50))).await;
            }
            Ok(())
        }
    }

    #[async_trait]
    impl PlanNodeExecutor for RecordingExecutor {
        async fn execute(&self, node: &PlanNode, attempt: u8) -> Result<(), PlanNodeError> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(format!("{}#{attempt}", node.id));
            self.seen_attempts
                .lock()
                .expect("attempt lock")
                .insert(node.id.clone(), attempt);

            if self
                .always_fail_node
                .as_ref()
                .map(|target| target == &node.id)
                .unwrap_or(false)
            {
                return Err(PlanNodeError::non_retryable(format!(
                    "forced failure for {}",
                    node.id
                )));
            }

            if self.fail_once_nodes.iter().any(|target| target == &node.id) && attempt == 1 {
                return Err(PlanNodeError::retryable(format!(
                    "transient failure for {}",
                    node.id
                )));
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn executor_runs_nodes_in_topological_order() {
        let graph = sample_graph();
        let executor = RecordingExecutor::succeed_all();
        let report = PlanExecutor::execute(&graph, &executor).await;

        assert_eq!(report.status, PlanRunStatus::Succeeded);
        assert_eq!(report.ordered_nodes, vec!["n1", "n2", "n3"]);
        assert_eq!(report.attempts_used, 3);
    }

    #[tokio::test]
    async fn executor_retries_and_recovers_before_node_budget_exhaustion() {
        let graph = sample_graph();
        let executor = RecordingExecutor::fail_once("n2");
        let report = PlanExecutor::execute(&graph, &executor).await;

        assert_eq!(report.status, PlanRunStatus::Succeeded);
        assert_eq!(report.attempts_used, 4);
        assert!(report.attempt_events.iter().any(|event| {
            event.node_id == "n2"
                && event.attempt == 1
                && !event.success
                && event.error_kind == Some(PlanNodeErrorKind::Retryable)
        }));
        assert!(report
            .attempt_events
            .iter()
            .any(|event| event.node_id == "n2" && event.attempt == 2 && event.success));
    }

    #[tokio::test]
    async fn executor_fails_when_node_retries_are_exhausted() {
        let graph = sample_graph();
        let executor = RecordingExecutor::always_fail("n2");
        let report = PlanExecutor::execute(&graph, &executor).await;

        match report.status {
            PlanRunStatus::Failed(PlanRunFailure::NodeFailed {
                node_id,
                attempts_used,
                last_error_kind,
                ..
            }) => {
                assert_eq!(node_id, "n2");
                assert_eq!(attempts_used, 2);
                assert_eq!(last_error_kind, PlanNodeErrorKind::NonRetryable);
            }
            other => panic!("expected node failure, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn executor_retries_when_node_attempt_times_out() {
        let mut graph = sample_graph();
        graph.nodes[1].timeout_ms = 10;
        graph.nodes[1].max_attempts = 2;
        graph.budget.max_total_attempts = 8;

        let executor = TimeoutOnFirstAttemptExecutor::new("n2");
        let report = PlanExecutor::execute(&graph, &executor).await;

        assert_eq!(report.status, PlanRunStatus::Succeeded);
        assert_eq!(report.attempts_used, 4);
        assert!(report.attempt_events.iter().any(|event| {
            event.node_id == "n2"
                && event.attempt == 1
                && !event.success
                && event.error_kind == Some(PlanNodeErrorKind::Retryable)
                && event
                    .error
                    .as_ref()
                    .map(|reason| reason.contains("node_timeout"))
                    .unwrap_or(false)
        }));
        assert!(report
            .attempt_events
            .iter()
            .any(|event| event.node_id == "n2" && event.attempt == 2 && event.success));
    }

    #[tokio::test]
    async fn executor_fails_when_node_timeout_retries_are_exhausted() {
        let mut graph = sample_graph();
        graph.nodes[1].timeout_ms = 10;
        graph.nodes[1].max_attempts = 1;
        graph.budget.max_total_attempts = 8;

        let executor = TimeoutAlwaysExecutor::new("n2");
        let report = PlanExecutor::execute(&graph, &executor).await;

        match report.status {
            PlanRunStatus::Failed(PlanRunFailure::NodeFailed {
                node_id,
                attempts_used,
                last_error_kind,
                last_error,
            }) => {
                assert_eq!(node_id, "n2");
                assert_eq!(attempts_used, 1);
                assert_eq!(last_error_kind, PlanNodeErrorKind::Retryable);
                assert!(
                    last_error.contains("node_timeout"),
                    "expected timeout reason, got: {last_error}"
                );
            }
            other => panic!("expected timeout node failure, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn executor_rejects_plan_when_attempt_budget_is_below_theoretical_max() {
        let mut graph = sample_graph();
        graph.budget.max_total_attempts = 2;
        let executor = RecordingExecutor::succeed_all();
        let report = PlanExecutor::execute(&graph, &executor).await;

        match report.status {
            PlanRunStatus::Failed(PlanRunFailure::ValidationFailed(error)) => {
                assert!(error.contains("attempt budget exceeded"), "error={error}");
            }
            other => panic!("expected validation failure, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn executor_fails_fast_for_invalid_graph() {
        let mut graph = sample_graph();
        graph.edges.push(PlanEdge {
            from: "n1".to_owned(),
            to: "n404".to_owned(),
        });

        let executor = RecordingExecutor::succeed_all();
        let report = PlanExecutor::execute(&graph, &executor).await;
        match report.status {
            PlanRunStatus::Failed(PlanRunFailure::ValidationFailed(error)) => {
                assert!(error.contains("unknown `to` node"), "error={error}");
            }
            other => panic!("expected validation failure, got: {other:?}"),
        }
    }
}
