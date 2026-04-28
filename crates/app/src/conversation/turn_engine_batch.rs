use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use futures_util::stream::{self, StreamExt};

use super::prepare::PreparedToolIntent;
use super::{
    AppToolDispatcher, ConversationRuntimeBinding, ConversationTurnObserverHandle, SessionContext,
    ToolBatchExecutionIntentTrace, ToolBatchExecutionMode, ToolBatchExecutionSegmentTrace,
    ToolBatchExecutionTrace, ToolOutcomeTraceRecord, TurnEngine, TurnResult,
    build_failure_tool_outcome_trace_record, build_success_tool_outcome_trace_record,
    build_tool_intent_completed_trace, build_tool_intent_failure_trace, elapsed_ms_u64,
    format_tool_result_line_with_limit, observe_peak_in_flight,
};
use crate::tools::ToolSchedulingClass;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PreparedBatchSegment {
    pub(super) len: usize,
    pub(super) scheduling_class: ToolSchedulingClass,
    pub(super) execution_mode: ToolBatchExecutionMode,
}

#[derive(Clone, Copy)]
pub(super) struct ToolBatchHarness<'a> {
    engine: &'a TurnEngine,
}

impl<'a> ToolBatchHarness<'a> {
    pub(super) fn new(engine: &'a TurnEngine) -> Self {
        Self { engine }
    }

    pub(super) fn trace_empty_batch(self, total_intents: usize) -> ToolBatchExecutionTrace {
        ToolBatchExecutionTrace {
            total_intents,
            parallel_execution_enabled: self.engine.parallel_tool_execution_enabled,
            parallel_execution_max_in_flight: self.engine.parallel_tool_execution_max_in_flight,
            observed_peak_in_flight: 0,
            observed_wall_time_ms: 0,
            segments: Vec::new(),
            decision_records: Vec::new(),
            outcome_records: Vec::new(),
            intent_outcomes: Vec::new(),
        }
    }

    pub(super) fn populate_trace_segments(
        self,
        trace: &mut ToolBatchExecutionTrace,
        batch_segments: &[PreparedBatchSegment],
    ) {
        trace.parallel_execution_enabled = self.engine.parallel_tool_execution_enabled;
        trace.parallel_execution_max_in_flight = self.engine.parallel_tool_execution_max_in_flight;
        trace.segments = batch_segments
            .iter()
            .enumerate()
            .map(|(segment_index, segment)| ToolBatchExecutionSegmentTrace {
                segment_index,
                scheduling_class: segment.scheduling_class,
                execution_mode: segment.execution_mode,
                intent_count: segment.len,
                observed_peak_in_flight: None,
                observed_wall_time_ms: None,
            })
            .collect();
    }

    pub(super) fn prepared_batch_segments(
        self,
        prepared: &[PreparedToolIntent],
    ) -> Vec<PreparedBatchSegment> {
        let mut segments = Vec::new();
        let mut remaining = prepared;

        while let Some((first, _)) = remaining.split_first() {
            let scheduling_class = first.scheduling_class;
            let len = remaining
                .iter()
                .take_while(|prepared_intent| prepared_intent.scheduling_class == scheduling_class)
                .count();
            let execution_mode = self.segment_execution_mode(scheduling_class, len);

            segments.push(PreparedBatchSegment {
                len,
                scheduling_class,
                execution_mode,
            });

            let (_, rest) = remaining.split_at(len);
            remaining = rest;
        }

        segments
    }

    fn segment_execution_mode(
        self,
        scheduling_class: ToolSchedulingClass,
        segment_len: usize,
    ) -> ToolBatchExecutionMode {
        let parallel_enabled = self.engine.parallel_tool_execution_enabled;
        let max_in_flight = self.engine.parallel_tool_execution_max_in_flight;
        let is_parallel_safe = scheduling_class == ToolSchedulingClass::ParallelSafe;
        let has_multiple_intents = segment_len > 1;

        if parallel_enabled && max_in_flight > 1 && is_parallel_safe && has_multiple_intents {
            return ToolBatchExecutionMode::Parallel;
        }

        ToolBatchExecutionMode::Sequential
    }

    pub(super) async fn execute_prepared_batch<D: AppToolDispatcher + ?Sized>(
        self,
        prepared: &[PreparedToolIntent],
        batch_segments: &[PreparedBatchSegment],
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        trace: &mut ToolBatchExecutionTrace,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> Result<Vec<String>, TurnResult> {
        let started_at = Instant::now();
        let result = async {
            let mut outputs = Vec::with_capacity(prepared.len());
            let mut remaining = prepared;

            debug_assert_eq!(trace.segments.len(), batch_segments.len());

            for (segment, trace_segment) in batch_segments
                .iter()
                .copied()
                .zip(trace.segments.iter_mut())
            {
                let (prepared_segment, rest) = remaining.split_at(segment.len);
                let mut segment_outputs = match segment.execution_mode {
                    ToolBatchExecutionMode::Parallel => {
                        self.execute_prepared_batch_in_parallel(
                            prepared_segment,
                            session_context,
                            app_dispatcher,
                            binding,
                            &mut trace.intent_outcomes,
                            &mut trace.outcome_records,
                            trace_segment,
                            observer,
                        )
                        .await?
                    }
                    ToolBatchExecutionMode::Sequential => {
                        self.execute_prepared_batch_sequential(
                            prepared_segment,
                            session_context,
                            app_dispatcher,
                            binding,
                            &mut trace.intent_outcomes,
                            &mut trace.outcome_records,
                            trace_segment,
                            observer,
                        )
                        .await?
                    }
                };

                outputs.append(&mut segment_outputs);
                remaining = rest;
            }

            Ok(outputs)
        }
        .await;

        trace.finish_observation(elapsed_ms_u64(started_at));

        result
    }

    async fn execute_prepared_batch_sequential<D: AppToolDispatcher + ?Sized>(
        self,
        prepared: &[PreparedToolIntent],
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        intent_outcomes: &mut Vec<ToolBatchExecutionIntentTrace>,
        outcome_records: &mut Vec<ToolOutcomeTraceRecord>,
        trace_segment: &mut ToolBatchExecutionSegmentTrace,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> Result<Vec<String>, TurnResult> {
        let started_at = Instant::now();
        let result = async {
            let mut outputs = Vec::with_capacity(prepared.len());

            for prepared_intent in prepared {
                let outcome = match self
                    .engine
                    .execute_prepared_tool_intent(
                        prepared_intent,
                        session_context,
                        app_dispatcher,
                        binding,
                        observer,
                    )
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(turn_result) => {
                        let outcome_record = build_failure_tool_outcome_trace_record(
                            &prepared_intent.intent,
                            &turn_result,
                        );

                        if let Some(outcome_record) = outcome_record {
                            outcome_records.push(outcome_record);
                        }

                        let intent_outcome =
                            build_tool_intent_failure_trace(&prepared_intent.intent, &turn_result);

                        if let Some(intent_outcome) = intent_outcome {
                            intent_outcomes.push(intent_outcome);
                        }

                        return Err(turn_result);
                    }
                };

                app_dispatcher
                    .after_tool_execution(
                        session_context,
                        &prepared_intent.intent,
                        prepared_intent.intent_sequence,
                        &prepared_intent.request,
                        &outcome,
                        binding,
                    )
                    .await;

                let outcome_record =
                    build_success_tool_outcome_trace_record(&prepared_intent.intent, &outcome);
                outcome_records.push(outcome_record);

                let intent_outcome =
                    build_tool_intent_completed_trace(&prepared_intent.intent, &outcome);
                intent_outcomes.push(intent_outcome);

                let payload_summary_limit_chars =
                    self.engine.tool_result_payload_summary_limit_chars;
                let output = format_tool_result_line_with_limit(
                    &prepared_intent.intent,
                    &outcome,
                    payload_summary_limit_chars,
                );
                outputs.push(output);
            }

            Ok(outputs)
        }
        .await;

        let observed_peak_in_flight = if prepared.is_empty() { 0 } else { 1 };
        let observed_wall_time_ms = elapsed_ms_u64(started_at);
        trace_segment.record_observation(observed_peak_in_flight, observed_wall_time_ms);

        result
    }

    async fn execute_prepared_batch_in_parallel<D: AppToolDispatcher + ?Sized>(
        self,
        prepared: &[PreparedToolIntent],
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        intent_outcomes: &mut Vec<ToolBatchExecutionIntentTrace>,
        outcome_records: &mut Vec<ToolOutcomeTraceRecord>,
        trace_segment: &mut ToolBatchExecutionSegmentTrace,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> Result<Vec<String>, TurnResult> {
        let started_at = Instant::now();
        let payload_summary_limit_chars = self.engine.tool_result_payload_summary_limit_chars;
        let in_flight = Arc::new(AtomicUsize::new(0));
        let observed_peak = Arc::new(AtomicUsize::new(0));
        let mut indexed_intent_outcomes = Vec::with_capacity(prepared.len());
        let mut indexed_outcome_records = Vec::with_capacity(prepared.len());
        let mut results = Vec::with_capacity(prepared.len());
        let max_in_flight = self.engine.parallel_tool_execution_max_in_flight;
        let mut executions = stream::iter(prepared.iter().cloned().enumerate().map(
            |(index, prepared_intent)| {
                let in_flight = Arc::clone(&in_flight);
                let observed_peak = Arc::clone(&observed_peak);

                async move {
                    let current_in_flight = in_flight.fetch_add(1, Ordering::Relaxed) + 1;
                    observe_peak_in_flight(observed_peak.as_ref(), current_in_flight);

                    let result = match self
                        .engine
                        .execute_prepared_tool_intent(
                            &prepared_intent,
                            session_context,
                            app_dispatcher,
                            binding,
                            observer,
                        )
                        .await
                    {
                        Ok(outcome) => {
                            app_dispatcher
                                .after_tool_execution(
                                    session_context,
                                    &prepared_intent.intent,
                                    prepared_intent.intent_sequence,
                                    &prepared_intent.request,
                                    &outcome,
                                    binding,
                                )
                                .await;

                            let output = format_tool_result_line_with_limit(
                                &prepared_intent.intent,
                                &outcome,
                                payload_summary_limit_chars,
                            );
                            let outcome_record = build_success_tool_outcome_trace_record(
                                &prepared_intent.intent,
                                &outcome,
                            );
                            let intent_outcome = build_tool_intent_completed_trace(
                                &prepared_intent.intent,
                                &outcome,
                            );

                            Ok((output, intent_outcome, outcome_record))
                        }
                        Err(turn_result) => {
                            let intent_outcome = build_tool_intent_failure_trace(
                                &prepared_intent.intent,
                                &turn_result,
                            );
                            let outcome_record = build_failure_tool_outcome_trace_record(
                                &prepared_intent.intent,
                                &turn_result,
                            );

                            Err((turn_result, intent_outcome, outcome_record))
                        }
                    };

                    in_flight.fetch_sub(1, Ordering::Relaxed);

                    (index, result)
                }
            },
        ))
        .buffer_unordered(max_in_flight);

        let mut batch_failure = None;
        while let Some((index, result)) = executions.next().await {
            match result {
                Ok((output, intent_outcome, outcome_record)) => {
                    indexed_intent_outcomes.push((index, intent_outcome));
                    indexed_outcome_records.push((index, outcome_record));
                    results.push((index, output));
                }
                Err((turn_result, intent_outcome, outcome_record)) => {
                    if let Some(intent_outcome) = intent_outcome {
                        indexed_intent_outcomes.push((index, intent_outcome));
                    }

                    if let Some(outcome_record) = outcome_record {
                        indexed_outcome_records.push((index, outcome_record));
                    }

                    batch_failure = Some(turn_result);
                    break;
                }
            }
        }

        let observed_peak_in_flight = observed_peak.load(Ordering::Relaxed);
        let observed_wall_time_ms = elapsed_ms_u64(started_at);
        trace_segment.record_observation(observed_peak_in_flight, observed_wall_time_ms);
        results.sort_by_key(|(index, _)| *index);
        indexed_intent_outcomes.sort_by_key(|(index, _)| *index);
        indexed_outcome_records.sort_by_key(|(index, _)| *index);
        intent_outcomes.extend(
            indexed_intent_outcomes
                .into_iter()
                .map(|(_, intent_outcome)| intent_outcome),
        );
        outcome_records.extend(
            indexed_outcome_records
                .into_iter()
                .map(|(_, outcome_record)| outcome_record),
        );

        if let Some(turn_result) = batch_failure {
            return Err(turn_result);
        }

        Ok(results.into_iter().map(|(_, output)| output).collect())
    }
}
