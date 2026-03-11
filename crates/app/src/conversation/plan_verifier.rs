use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanVerificationPolicy {
    pub require_non_empty: bool,
    pub min_output_chars: usize,
    pub require_status_prefix: bool,
    pub deny_markers: BTreeSet<String>,
}

impl Default for PlanVerificationPolicy {
    fn default() -> Self {
        Self {
            require_non_empty: true,
            min_output_chars: 8,
            require_status_prefix: true,
            deny_markers: default_deny_markers().into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanVerificationContext {
    pub expected_result_lines: usize,
    pub semantic_anchors: BTreeSet<String>,
    pub min_anchor_matches: usize,
}

impl Default for PlanVerificationContext {
    fn default() -> Self {
        Self {
            expected_result_lines: 1,
            semantic_anchors: BTreeSet::new(),
            min_anchor_matches: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanVerificationFailureCode {
    EmptyOutput,
    OutputTooShort,
    DenyMarkerDetected,
    InsufficientResultLines,
    MissingStatusPrefix,
    FailureStatusDetected,
    MissingSemanticAnchors,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanVerificationReport {
    pub passed: bool,
    pub failure_reasons: Vec<String>,
    pub failure_codes: Vec<PlanVerificationFailureCode>,
    pub observed_statuses: Vec<String>,
    pub matched_anchors: Vec<String>,
}

pub fn verify_output(
    output: &str,
    context: &PlanVerificationContext,
    policy: &PlanVerificationPolicy,
) -> PlanVerificationReport {
    let trimmed = output.trim();
    let mut failure_reasons = Vec::new();
    let mut failure_codes = Vec::new();

    if trimmed.is_empty() {
        if policy.require_non_empty {
            failure_codes.push(PlanVerificationFailureCode::EmptyOutput);
            failure_reasons.push("empty_output".to_owned());
        }
        return PlanVerificationReport {
            passed: failure_reasons.is_empty(),
            failure_reasons,
            failure_codes,
            observed_statuses: Vec::new(),
            matched_anchors: Vec::new(),
        };
    }

    let char_count = trimmed.chars().count();
    if char_count < policy.min_output_chars {
        failure_codes.push(PlanVerificationFailureCode::OutputTooShort);
        failure_reasons.push(format!(
            "output_too_short chars={char_count} min={}",
            policy.min_output_chars
        ));
    }

    let normalized = trimmed.to_ascii_lowercase();
    for marker in &policy.deny_markers {
        if marker.is_empty() {
            continue;
        }
        if normalized.contains(marker.as_str()) {
            failure_codes.push(PlanVerificationFailureCode::DenyMarkerDetected);
            failure_reasons.push(format!("deny_marker_detected marker={marker}"));
        }
    }

    let lines = trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let expected_result_lines = context.expected_result_lines.max(1);
    if lines.len() < expected_result_lines {
        failure_codes.push(PlanVerificationFailureCode::InsufficientResultLines);
        failure_reasons.push(format!(
            "insufficient_result_lines actual={} expected>={expected_result_lines}",
            lines.len()
        ));
    }

    let mut observed_statuses = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        match parse_status_prefix(line) {
            Some(status) => {
                let normalized_status = status.to_ascii_lowercase();
                observed_statuses.push(normalized_status.clone());
                if looks_like_failure_status(normalized_status.as_str()) {
                    failure_codes.push(PlanVerificationFailureCode::FailureStatusDetected);
                    failure_reasons.push(format!(
                        "failure_status_detected line_index={} status={normalized_status}",
                        index + 1
                    ));
                }
            }
            None if policy.require_status_prefix => {
                failure_codes.push(PlanVerificationFailureCode::MissingStatusPrefix);
                failure_reasons.push(format!(
                    "missing_status_prefix line_index={} content={}",
                    index + 1,
                    line
                ));
                break;
            }
            None => {}
        }
    }

    let semantic_anchors = context
        .semantic_anchors
        .iter()
        .map(|anchor| anchor.trim().to_ascii_lowercase())
        .filter(|anchor| anchor.len() >= 3)
        .collect::<BTreeSet<_>>();
    let matched_anchors = semantic_anchors
        .iter()
        .filter(|anchor| normalized.contains(anchor.as_str()))
        .cloned()
        .collect::<Vec<_>>();

    if !semantic_anchors.is_empty() && context.min_anchor_matches > 0 {
        let required_matches = context
            .min_anchor_matches
            .min(semantic_anchors.len())
            .max(1);
        if matched_anchors.len() < required_matches {
            failure_codes.push(PlanVerificationFailureCode::MissingSemanticAnchors);
            failure_reasons.push(format!(
                "semantic_anchor_miss matched={} required>={required_matches}",
                matched_anchors.len()
            ));
        }
    }

    PlanVerificationReport {
        passed: failure_reasons.is_empty(),
        failure_reasons,
        failure_codes,
        observed_statuses,
        matched_anchors,
    }
}

fn parse_status_prefix(line: &str) -> Option<&str> {
    let remainder = line.strip_prefix('[')?;
    let bracket_index = remainder.find(']')?;
    let status = &remainder[..bracket_index];
    if status.trim().is_empty() {
        return None;
    }
    Some(status.trim())
}

fn looks_like_failure_status(status: &str) -> bool {
    matches!(
        status,
        "error" | "denied" | "fail" | "failed" | "forbidden" | "timeout"
    )
}

fn default_deny_markers() -> Vec<String> {
    vec![
        "tool_failure".to_owned(),
        "provider_error".to_owned(),
        "no_kernel_context".to_owned(),
        "tool_not_found".to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_accepts_well_formed_tool_output() {
        let policy = PlanVerificationPolicy::default();
        let context = PlanVerificationContext {
            expected_result_lines: 2,
            ..PlanVerificationContext::default()
        };
        let output = "[ok] {\"path\":\"note.md\"}\n[ok] {\"path\":\"checklist.md\"}";
        let report = verify_output(output, &context, &policy);
        assert!(report.passed, "report={report:?}");
    }

    #[test]
    fn verifier_rejects_short_output() {
        let mut policy = PlanVerificationPolicy::default();
        policy.min_output_chars = 20;
        let report = verify_output("[ok] {}", &PlanVerificationContext::default(), &policy);
        assert!(!report.passed);
        assert!(
            report
                .failure_reasons
                .iter()
                .any(|reason| reason.contains("output_too_short")),
            "report={report:?}"
        );
    }

    #[test]
    fn verifier_rejects_output_with_deny_markers() {
        let policy = PlanVerificationPolicy::default();
        let report = verify_output(
            "[ok] no_kernel_context",
            &PlanVerificationContext::default(),
            &policy,
        );
        assert!(!report.passed);
        assert!(
            report
                .failure_reasons
                .iter()
                .any(|reason| reason.contains("deny_marker_detected")),
            "report={report:?}"
        );
    }

    #[test]
    fn verifier_rejects_missing_status_prefix() {
        let policy = PlanVerificationPolicy::default();
        let report = verify_output("plain output", &PlanVerificationContext::default(), &policy);
        assert!(!report.passed);
        assert!(
            report
                .failure_reasons
                .iter()
                .any(|reason| reason.contains("missing_status_prefix")),
            "report={report:?}"
        );
    }

    #[test]
    fn verifier_rejects_failure_status_lines() {
        let policy = PlanVerificationPolicy::default();
        let report = verify_output(
            "[error] {\"msg\":\"failed\"}",
            &PlanVerificationContext::default(),
            &policy,
        );
        assert!(!report.passed);
        assert!(
            report
                .failure_codes
                .contains(&PlanVerificationFailureCode::FailureStatusDetected),
            "report={report:?}"
        );
    }

    #[test]
    fn verifier_rejects_when_semantic_anchors_are_missing() {
        let policy = PlanVerificationPolicy::default();
        let context = PlanVerificationContext {
            expected_result_lines: 1,
            semantic_anchors: BTreeSet::from(["critical-note.md".to_owned()]),
            min_anchor_matches: 1,
        };
        let report = verify_output("[ok] {\"path\":\"other.md\"}", &context, &policy);
        assert!(!report.passed);
        assert!(
            report
                .failure_codes
                .contains(&PlanVerificationFailureCode::MissingSemanticAnchors),
            "report={report:?}"
        );
    }
}
