use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use loong_contracts::ToolSchedulingClass;
use serde_json::json;

use super::{ToolDecisionTelemetry, ToolOutcomeTelemetry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolBatchExecutionMode {
    Sequential,
    Parallel,
}

impl ToolBatchExecutionMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Parallel => "parallel",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolBatchExecutionSegmentTrace {
    pub segment_index: usize,
    pub scheduling_class: ToolSchedulingClass,
    pub execution_mode: ToolBatchExecutionMode,
    pub intent_count: usize,
    pub observed_peak_in_flight: Option<usize>,
    pub observed_wall_time_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolBatchExecutionIntentStatus {
    Completed,
    NeedsApproval,
    Denied,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolBatchExecutionIntentTrace {
    pub tool_call_id: String,
    pub tool_name: String,
    pub status: ToolBatchExecutionIntentStatus,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolDecisionTraceRecord {
    pub turn_id: String,
    pub tool_call_id: String,
    pub decision: ToolDecisionTelemetry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolOutcomeTraceRecord {
    pub turn_id: String,
    pub tool_call_id: String,
    pub outcome: ToolOutcomeTelemetry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolBatchExecutionTrace {
    pub total_intents: usize,
    pub parallel_execution_enabled: bool,
    pub parallel_execution_max_in_flight: usize,
    pub observed_peak_in_flight: usize,
    pub observed_wall_time_ms: u64,
    pub segments: Vec<ToolBatchExecutionSegmentTrace>,
    pub decision_records: Vec<ToolDecisionTraceRecord>,
    pub outcome_records: Vec<ToolOutcomeTraceRecord>,
    pub intent_outcomes: Vec<ToolBatchExecutionIntentTrace>,
}

impl ToolBatchExecutionSegmentTrace {
    pub(crate) fn record_observation(
        &mut self,
        observed_peak_in_flight: usize,
        observed_wall_time_ms: u64,
    ) {
        self.observed_peak_in_flight = Some(observed_peak_in_flight);
        self.observed_wall_time_ms = Some(observed_wall_time_ms);
    }
}

impl ToolBatchExecutionTrace {
    pub(crate) fn has_execution_segments(&self) -> bool {
        !self.segments.is_empty()
    }

    pub(crate) fn finish_observation(&mut self, observed_wall_time_ms: u64) {
        self.observed_wall_time_ms = observed_wall_time_ms;
        self.observed_peak_in_flight = self
            .segments
            .iter()
            .filter_map(|segment| segment.observed_peak_in_flight)
            .max()
            .unwrap_or_default();
    }

    pub(crate) fn as_event_payload(&self) -> serde_json::Value {
        let parallel_safe_intents = self
            .segments
            .iter()
            .filter(|segment| segment.scheduling_class == ToolSchedulingClass::ParallelSafe)
            .map(|segment| segment.intent_count)
            .sum::<usize>();
        let serial_only_intents = self
            .segments
            .iter()
            .filter(|segment| segment.scheduling_class == ToolSchedulingClass::SerialOnly)
            .map(|segment| segment.intent_count)
            .sum::<usize>();
        let parallel_segments = self
            .segments
            .iter()
            .filter(|segment| segment.execution_mode == ToolBatchExecutionMode::Parallel)
            .count();
        let sequential_segments = self
            .segments
            .iter()
            .filter(|segment| segment.execution_mode == ToolBatchExecutionMode::Sequential)
            .count();

        json!({
            "schema_version": 2,
            "total_intents": self.total_intents,
            "parallel_execution_enabled": self.parallel_execution_enabled,
            "parallel_execution_max_in_flight": self.parallel_execution_max_in_flight,
            "observed_peak_in_flight": self.observed_peak_in_flight,
            "observed_wall_time_ms": self.observed_wall_time_ms,
            "parallel_safe_intents": parallel_safe_intents,
            "serial_only_intents": serial_only_intents,
            "parallel_segments": parallel_segments,
            "sequential_segments": sequential_segments,
            "segments": self
                .segments
                .iter()
                .map(|segment| {
                    json!({
                        "segment_index": segment.segment_index,
                        "scheduling_class": segment.scheduling_class.as_str(),
                        "execution_mode": segment.execution_mode.as_str(),
                        "intent_count": segment.intent_count,
                        "observed_peak_in_flight": segment.observed_peak_in_flight,
                        "observed_wall_time_ms": segment.observed_wall_time_ms,
                    })
                })
                .collect::<Vec<_>>(),
        })
    }
}

pub(crate) fn elapsed_ms_u64(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

pub(crate) fn observe_peak_in_flight(peak: &AtomicUsize, current: usize) {
    let mut observed = peak.load(Ordering::Relaxed);
    while current > observed {
        match peak.compare_exchange_weak(observed, current, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(next) => observed = next,
        }
    }
}
