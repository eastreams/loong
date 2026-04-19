use std::time::Duration;

use tokio::time::Instant as TokioInstant;

use super::ConnectorCircuitBreakerPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorCircuitPhase {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone)]
pub struct ConnectorCircuitRuntimeState {
    pub phase: ConnectorCircuitPhase,
    pub consecutive_failures: usize,
    pub open_until: Option<TokioInstant>,
    pub half_open_remaining_calls: usize,
    pub half_open_successes: usize,
}

impl Default for ConnectorCircuitRuntimeState {
    fn default() -> Self {
        Self {
            phase: ConnectorCircuitPhase::Closed,
            consecutive_failures: 0,
            open_until: None,
            half_open_remaining_calls: 0,
            half_open_successes: 0,
        }
    }
}

pub type ProgrammaticCircuitPhase = ConnectorCircuitPhase;
pub type ProgrammaticCircuitRuntimeState = ConnectorCircuitRuntimeState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorCircuitAcquireError {
    Open { remaining_cooldown_ms: u64 },
    HalfOpenReopened,
}

pub fn validate_connector_circuit_breaker_policy(
    policy: &ConnectorCircuitBreakerPolicy,
    context: &str,
) -> Result<(), String> {
    if !policy.enabled {
        return Ok(());
    }
    if policy.failure_threshold == 0 {
        return Err(format!(
            "{context} failure_threshold must be greater than 0"
        ));
    }
    if policy.cooldown_ms == 0 {
        return Err(format!("{context} cooldown_ms must be greater than 0"));
    }
    if policy.half_open_max_calls == 0 {
        return Err(format!(
            "{context} half_open_max_calls must be greater than 0"
        ));
    }
    if policy.success_threshold == 0 {
        return Err(format!(
            "{context} success_threshold must be greater than 0"
        ));
    }
    if policy.success_threshold > policy.half_open_max_calls {
        return Err(format!(
            "{context} success_threshold must be <= half_open_max_calls"
        ));
    }
    Ok(())
}

pub const fn connector_circuit_phase_label(phase: ConnectorCircuitPhase) -> &'static str {
    match phase {
        ConnectorCircuitPhase::Closed => "closed",
        ConnectorCircuitPhase::Open => "open",
        ConnectorCircuitPhase::HalfOpen => "half_open",
    }
}

pub fn connector_circuit_remaining_cooldown_ms(
    state: &ConnectorCircuitRuntimeState,
    now: TokioInstant,
) -> Option<u64> {
    let open_until = state.open_until?;
    if open_until <= now {
        return Some(0);
    }

    let remaining_duration = open_until.duration_since(now);
    let remaining_ms = remaining_duration.as_millis();
    let clamped_ms = remaining_ms.min(u128::from(u64::MAX));

    Some(clamped_ms as u64)
}

pub fn acquire_connector_circuit_slot_for_state(
    policy: &ConnectorCircuitBreakerPolicy,
    state: &mut ConnectorCircuitRuntimeState,
    now: TokioInstant,
) -> Result<&'static str, ConnectorCircuitAcquireError> {
    if !policy.enabled {
        return Ok("disabled");
    }

    if state.phase == ConnectorCircuitPhase::Open {
        let remaining_cooldown_ms = connector_circuit_remaining_cooldown_ms(state, now);
        if let Some(remaining_cooldown_ms) = remaining_cooldown_ms
            && remaining_cooldown_ms > 0
        {
            return Err(ConnectorCircuitAcquireError::Open {
                remaining_cooldown_ms,
            });
        }

        state.phase = ConnectorCircuitPhase::HalfOpen;
        state.open_until = None;
        state.half_open_remaining_calls = policy.half_open_max_calls;
        state.half_open_successes = 0;
    }

    if state.phase == ConnectorCircuitPhase::HalfOpen {
        if state.half_open_remaining_calls == 0 {
            let reopen_deadline = now + Duration::from_millis(policy.cooldown_ms);

            state.phase = ConnectorCircuitPhase::Open;
            state.open_until = Some(reopen_deadline);
            return Err(ConnectorCircuitAcquireError::HalfOpenReopened);
        }

        state.half_open_remaining_calls = state.half_open_remaining_calls.saturating_sub(1);
        return Ok("half_open");
    }

    Ok("closed")
}

pub fn record_connector_circuit_outcome_for_state(
    policy: &ConnectorCircuitBreakerPolicy,
    state: &mut ConnectorCircuitRuntimeState,
    success: bool,
    now: TokioInstant,
) -> &'static str {
    if !policy.enabled {
        return "disabled";
    }

    match state.phase {
        ConnectorCircuitPhase::Closed => {
            if success {
                state.consecutive_failures = 0;
            } else {
                state.consecutive_failures = state.consecutive_failures.saturating_add(1);
                if state.consecutive_failures >= policy.failure_threshold {
                    let reopen_deadline = now + Duration::from_millis(policy.cooldown_ms);

                    state.phase = ConnectorCircuitPhase::Open;
                    state.open_until = Some(reopen_deadline);
                    state.half_open_remaining_calls = 0;
                    state.half_open_successes = 0;
                }
            }
        }
        ConnectorCircuitPhase::HalfOpen => {
            if success {
                state.half_open_successes = state.half_open_successes.saturating_add(1);
                if state.half_open_successes >= policy.success_threshold {
                    state.phase = ConnectorCircuitPhase::Closed;
                    state.consecutive_failures = 0;
                    state.open_until = None;
                    state.half_open_remaining_calls = 0;
                    state.half_open_successes = 0;
                } else if state.half_open_remaining_calls == 0 {
                    let reopen_deadline = now + Duration::from_millis(policy.cooldown_ms);

                    state.phase = ConnectorCircuitPhase::Open;
                    state.open_until = Some(reopen_deadline);
                    state.half_open_successes = 0;
                }
            } else {
                let reopen_deadline = now + Duration::from_millis(policy.cooldown_ms);

                state.phase = ConnectorCircuitPhase::Open;
                state.open_until = Some(reopen_deadline);
                state.half_open_remaining_calls = 0;
                state.half_open_successes = 0;
            }
        }
        ConnectorCircuitPhase::Open => {}
    }

    connector_circuit_phase_label(state.phase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec_runtime::ConnectorCircuitBreakerPolicy;

    #[test]
    fn validate_connector_circuit_breaker_policy_rejects_success_threshold_above_max_calls() {
        let policy = ConnectorCircuitBreakerPolicy {
            success_threshold: 3,
            half_open_max_calls: 2,
            ..ConnectorCircuitBreakerPolicy::default()
        };

        let error = validate_connector_circuit_breaker_policy(&policy, "connector")
            .expect_err("expected invalid policy");

        assert_eq!(
            error,
            "connector success_threshold must be <= half_open_max_calls"
        );
    }

    #[test]
    fn acquire_connector_circuit_slot_transitions_open_circuits_into_half_open_probes() {
        let now = TokioInstant::now();
        let policy = ConnectorCircuitBreakerPolicy {
            cooldown_ms: 50,
            half_open_max_calls: 2,
            ..ConnectorCircuitBreakerPolicy::default()
        };
        let mut state = ConnectorCircuitRuntimeState {
            phase: ConnectorCircuitPhase::Open,
            open_until: Some(now),
            ..ConnectorCircuitRuntimeState::default()
        };

        let phase = acquire_connector_circuit_slot_for_state(&policy, &mut state, now)
            .expect("elapsed cooldown should enter half-open");

        assert_eq!(phase, "half_open");
        assert_eq!(state.phase, ConnectorCircuitPhase::HalfOpen);
        assert_eq!(state.half_open_remaining_calls, 1);
        assert_eq!(state.half_open_successes, 0);
        assert_eq!(state.open_until, None);
    }

    #[test]
    fn record_connector_circuit_outcome_reopens_half_open_state_after_failed_probe() {
        let now = TokioInstant::now();
        let policy = ConnectorCircuitBreakerPolicy {
            cooldown_ms: 25,
            ..ConnectorCircuitBreakerPolicy::default()
        };
        let mut state = ConnectorCircuitRuntimeState {
            phase: ConnectorCircuitPhase::HalfOpen,
            half_open_remaining_calls: 1,
            half_open_successes: 1,
            ..ConnectorCircuitRuntimeState::default()
        };

        let phase = record_connector_circuit_outcome_for_state(&policy, &mut state, false, now);

        assert_eq!(phase, "open");
        assert_eq!(state.phase, ConnectorCircuitPhase::Open);
        assert_eq!(state.half_open_remaining_calls, 0);
        assert_eq!(state.half_open_successes, 0);
        assert!(state.open_until.is_some());
    }
}
