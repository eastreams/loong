use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanNodeKind {
    Tool,
    Transform,
    Verify,
    Respond,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanNode {
    pub id: String,
    pub kind: PlanNodeKind,
    pub label: String,
    pub tool_name: Option<String>,
    pub timeout_ms: u64,
    pub max_attempts: u8,
    pub risk_tier: RiskTier,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanBudget {
    pub max_nodes: usize,
    pub max_total_attempts: usize,
    pub max_wall_time_ms: u64,
}

impl Default for PlanBudget {
    fn default() -> Self {
        Self {
            max_nodes: 16,
            max_total_attempts: 32,
            max_wall_time_ms: 60_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanGraph {
    pub version: String,
    pub nodes: Vec<PlanNode>,
    pub edges: Vec<PlanEdge>,
    #[serde(default)]
    pub budget: PlanBudget,
}

impl PlanGraph {
    pub fn validate(&self) -> Result<(), String> {
        if self.nodes.is_empty() {
            return Err("plan graph must include at least one node".to_owned());
        }
        if self.nodes.len() > self.budget.max_nodes {
            return Err(format!(
                "plan graph node budget exceeded actual={} limit={}",
                self.nodes.len(),
                self.budget.max_nodes
            ));
        }

        let mut node_index = BTreeMap::new();
        let mut total_attempts = 0usize;
        for node in &self.nodes {
            if node.id.trim().is_empty() {
                return Err("plan node id must not be empty".to_owned());
            }
            if node.timeout_ms == 0 {
                return Err(format!("plan node `{}` has invalid timeout_ms=0", node.id));
            }
            if node.max_attempts == 0 {
                return Err(format!(
                    "plan node `{}` has invalid max_attempts=0",
                    node.id
                ));
            }
            total_attempts = total_attempts.saturating_add(node.max_attempts as usize);
            if node_index
                .insert(node.id.clone(), node_index.len())
                .is_some()
            {
                return Err(format!("duplicate plan node id `{}`", node.id));
            }
        }
        if total_attempts > self.budget.max_total_attempts {
            return Err(format!(
                "plan attempt budget exceeded actual={} limit={}",
                total_attempts, self.budget.max_total_attempts
            ));
        }

        let mut indegree = vec![0usize; self.nodes.len()];
        let mut outdegree = vec![0usize; self.nodes.len()];
        let mut adjacency = vec![Vec::<usize>::new(); self.nodes.len()];
        for edge in &self.edges {
            let from_index = match node_index.get(&edge.from) {
                Some(index) => *index,
                None => {
                    return Err(format!(
                        "plan edge references unknown `from` node `{}`",
                        edge.from
                    ));
                }
            };
            let to_index = match node_index.get(&edge.to) {
                Some(index) => *index,
                None => {
                    return Err(format!(
                        "plan edge references unknown `to` node `{}`",
                        edge.to
                    ))
                }
            };
            if from_index == to_index {
                return Err(format!("plan edge has self-loop at `{}`", edge.from));
            }
            indegree[to_index] = indegree[to_index].saturating_add(1);
            outdegree[from_index] = outdegree[from_index].saturating_add(1);
            adjacency[from_index].push(to_index);
        }

        let entry_nodes = indegree.iter().filter(|degree| **degree == 0).count();
        if entry_nodes == 0 {
            return Err("plan graph must include at least one entry node".to_owned());
        }
        let terminal_nodes = outdegree.iter().filter(|degree| **degree == 0).count();
        if terminal_nodes == 0 {
            return Err("plan graph must include at least one terminal node".to_owned());
        }

        let mut queue = VecDeque::new();
        let mut remaining_indegree = indegree.clone();
        for (index, degree) in remaining_indegree.iter().enumerate() {
            if *degree == 0 {
                queue.push_back(index);
            }
        }
        let mut visited = 0usize;
        while let Some(index) = queue.pop_front() {
            visited = visited.saturating_add(1);
            for next in &adjacency[index] {
                remaining_indegree[*next] = remaining_indegree[*next].saturating_sub(1);
                if remaining_indegree[*next] == 0 {
                    queue.push_back(*next);
                }
            }
        }
        if visited != self.nodes.len() {
            return Err("plan graph must be acyclic".to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_graph() -> PlanGraph {
        PlanGraph {
            version: "hvgr.v1".to_owned(),
            nodes: vec![
                PlanNode {
                    id: "n1".to_owned(),
                    kind: PlanNodeKind::Tool,
                    label: "read context".to_owned(),
                    tool_name: Some("file.read".to_owned()),
                    timeout_ms: 3_000,
                    max_attempts: 2,
                    risk_tier: RiskTier::Low,
                },
                PlanNode {
                    id: "n2".to_owned(),
                    kind: PlanNodeKind::Verify,
                    label: "validate output".to_owned(),
                    tool_name: None,
                    timeout_ms: 1_000,
                    max_attempts: 1,
                    risk_tier: RiskTier::Low,
                },
                PlanNode {
                    id: "n3".to_owned(),
                    kind: PlanNodeKind::Respond,
                    label: "compose answer".to_owned(),
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
            budget: PlanBudget::default(),
        }
    }

    #[test]
    fn valid_plan_graph_passes() {
        let graph = sample_graph();
        assert!(graph.validate().is_ok());
    }

    #[test]
    fn duplicate_node_id_is_rejected() {
        let mut graph = sample_graph();
        graph.nodes.push(graph.nodes[0].clone());
        let error = graph.validate().expect_err("duplicate node id should fail");
        assert!(error.contains("duplicate plan node id"), "error: {error}");
    }

    #[test]
    fn unknown_edge_reference_is_rejected() {
        let mut graph = sample_graph();
        graph.edges.push(PlanEdge {
            from: "n3".to_owned(),
            to: "n404".to_owned(),
        });
        let error = graph
            .validate()
            .expect_err("unknown edge node reference should fail");
        assert!(error.contains("unknown `to` node"), "error: {error}");
    }

    #[test]
    fn cyclic_graph_is_rejected() {
        let mut graph = sample_graph();
        graph.nodes.push(PlanNode {
            id: "n4".to_owned(),
            kind: PlanNodeKind::Respond,
            label: "side terminal".to_owned(),
            tool_name: None,
            timeout_ms: 500,
            max_attempts: 1,
            risk_tier: RiskTier::Low,
        });
        graph.edges.push(PlanEdge {
            from: "n3".to_owned(),
            to: "n2".to_owned(),
        });
        let error = graph.validate().expect_err("cycle should fail");
        assert!(error.contains("acyclic"), "error: {error}");
    }

    #[test]
    fn attempt_budget_exceeded_is_rejected() {
        let mut graph = sample_graph();
        graph.budget.max_total_attempts = 2;
        let error = graph
            .validate()
            .expect_err("attempt budget overflow should fail");
        assert!(error.contains("attempt budget exceeded"), "error: {error}");
    }

    #[test]
    fn zero_timeout_node_is_rejected() {
        let mut graph = sample_graph();
        graph.nodes[0].timeout_ms = 0;
        let error = graph.validate().expect_err("zero timeout should fail");
        assert!(error.contains("invalid timeout_ms=0"), "error: {error}");
    }
}
