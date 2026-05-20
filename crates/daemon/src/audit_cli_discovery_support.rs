use super::*;

fn serialize_json_object_or_empty<T>(value: &T) -> Map<String, Value>
where
    T: serde::Serialize,
{
    let serialized_result = serde_json::to_value(value);
    let Ok(serialized_value) = serialized_result else {
        return Map::new();
    };

    let Value::Object(payload) = serialized_value else {
        return Map::new();
    };

    payload
}

pub(super) fn audit_discovery_groups_json(
    execution: &AuditCommandExecution,
    limit: usize,
    group_by: Option<&str>,
    groups: &[AuditDiscoveryGroup],
) -> Value {
    Value::Array(
        groups
            .iter()
            .map(|group| {
                let mut payload = serialize_json_object_or_empty(group);
                payload.insert(
                    "drill_down_command".to_owned(),
                    json!(discovery_group_drill_down_command(
                        execution, limit, group_by, group
                    )),
                );
                payload.insert(
                    "correlated_summary_command".to_owned(),
                    json!(discovery_group_correlated_summary_command(
                        execution, limit, group_by, group
                    )),
                );
                payload.insert(
                    "correlated_remediation_command".to_owned(),
                    json!(discovery_group_correlated_remediation_command(
                        execution, limit, group_by, group
                    )),
                );
                Value::Object(payload)
            })
            .collect(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CorrelatedRemediationCommandTarget {
    RecentTriage { triage_label: String },
    SummaryByToken { triage_label: String },
    RecentKind { kind: String },
    SummaryScope,
}

pub(super) fn discovery_group_drill_down_command(
    execution: &AuditCommandExecution,
    limit: usize,
    group_by: Option<&str>,
    group: &AuditDiscoveryGroup,
) -> Option<String> {
    let (pack_id_filter, agent_id_filter) =
        discovery_group_identity_filters(execution, group_by, group)?;
    let pack_id_filter = pack_id_filter.as_deref();
    let agent_id_filter = agent_id_filter.as_deref();
    let mut parts = discovery_group_scoped_command_parts(
        "audit recent",
        execution,
        limit,
        pack_id_filter,
        agent_id_filter,
    );
    push_optional_shell_flag(
        &mut parts,
        "--event-id",
        execution.event_id_filter.as_deref(),
    );
    push_optional_shell_flag(
        &mut parts,
        "--token-id",
        execution.token_id_filter.as_deref(),
    );
    push_optional_shell_flag(&mut parts, "--kind", execution.kind_filter.as_deref());
    push_optional_shell_flag(
        &mut parts,
        "--triage-label",
        execution.triage_label_filter.as_deref(),
    );
    push_optional_shell_flag(
        &mut parts,
        "--query-contains",
        execution.query_contains_filter.as_deref(),
    );
    push_optional_shell_flag(
        &mut parts,
        "--trust-tier",
        execution.trust_tier_filter.as_deref(),
    );

    Some(parts.join(" "))
}

pub(super) fn discovery_group_correlated_summary_command(
    execution: &AuditCommandExecution,
    limit: usize,
    group_by: Option<&str>,
    group: &AuditDiscoveryGroup,
) -> Option<String> {
    let (pack_id_filter, agent_id_filter) =
        discovery_group_identity_filters(execution, group_by, group)?;
    let pack_id_filter = pack_id_filter.as_deref();
    let agent_id_filter = agent_id_filter.as_deref();
    let parts = discovery_group_scoped_command_parts(
        "audit summary",
        execution,
        limit,
        pack_id_filter,
        agent_id_filter,
    );

    Some(parts.join(" "))
}

pub(super) fn discovery_group_correlated_remediation_command(
    execution: &AuditCommandExecution,
    limit: usize,
    group_by: Option<&str>,
    group: &AuditDiscoveryGroup,
) -> Option<String> {
    let target = correlated_remediation_command_target(
        group.correlated_additional_events,
        &group.correlated_non_discovery_event_kind_counts,
        &group.correlated_non_discovery_triage_counts,
    )?;
    let (pack_id_filter, agent_id_filter) =
        discovery_group_identity_filters(execution, group_by, group)?;
    let pack_id_filter = pack_id_filter.as_deref();
    let agent_id_filter = agent_id_filter.as_deref();
    let subcommand = remediation_command_subcommand(&target);
    let mut parts = discovery_group_scoped_command_parts(
        subcommand,
        execution,
        limit,
        pack_id_filter,
        agent_id_filter,
    );

    match target {
        CorrelatedRemediationCommandTarget::RecentTriage { triage_label } => {
            push_shell_argument_flag(&mut parts, "--triage-label", &triage_label);
        }
        CorrelatedRemediationCommandTarget::SummaryByToken { triage_label } => {
            push_shell_argument_flag(&mut parts, "--triage-label", &triage_label);
            push_shell_argument_flag(&mut parts, "--group-by", "token");
        }
        CorrelatedRemediationCommandTarget::RecentKind { kind } => {
            push_shell_argument_flag(&mut parts, "--kind", &kind);
        }
        CorrelatedRemediationCommandTarget::SummaryScope => {}
    }

    Some(parts.join(" "))
}

fn remediation_command_subcommand(target: &CorrelatedRemediationCommandTarget) -> &'static str {
    match target {
        CorrelatedRemediationCommandTarget::RecentTriage { .. } => "audit recent",
        CorrelatedRemediationCommandTarget::SummaryByToken { .. } => "audit summary",
        CorrelatedRemediationCommandTarget::RecentKind { .. } => "audit recent",
        CorrelatedRemediationCommandTarget::SummaryScope => "audit summary",
    }
}

fn correlated_remediation_command_target(
    additional_events: usize,
    non_discovery_event_kind_counts: &BTreeMap<String, usize>,
    non_discovery_triage_counts: &BTreeMap<String, usize>,
) -> Option<CorrelatedRemediationCommandTarget> {
    let top_triage_label = top_rollup_label(non_discovery_triage_counts);
    if let Some(top_triage_label) = top_triage_label {
        let triage_label = top_triage_label.to_owned();
        if top_triage_label == "authorization_denied" {
            return Some(CorrelatedRemediationCommandTarget::SummaryByToken { triage_label });
        }
        return Some(CorrelatedRemediationCommandTarget::RecentTriage { triage_label });
    }

    let top_event_kind = top_rollup_label(non_discovery_event_kind_counts);
    if let Some(top_event_kind) = top_event_kind {
        let kind = top_event_kind.to_owned();
        return Some(CorrelatedRemediationCommandTarget::RecentKind { kind });
    }

    if additional_events > 0 {
        return Some(CorrelatedRemediationCommandTarget::SummaryScope);
    }

    None
}

fn discovery_group_scoped_command_parts(
    subcommand: &str,
    execution: &AuditCommandExecution,
    limit: usize,
    pack_id_filter: Option<&str>,
    agent_id_filter: Option<&str>,
) -> Vec<String> {
    let mut parts = Vec::new();
    let base_command = crate::cli_handoff::format_subcommand_with_config(
        subcommand,
        &execution.resolved_config_path,
    );
    parts.push(base_command);
    parts.push("--limit".to_owned());
    parts.push(limit.to_string());
    push_optional_numeric_flag(
        &mut parts,
        "--since-epoch-s",
        execution.since_epoch_s_filter,
    );
    push_optional_numeric_flag(
        &mut parts,
        "--until-epoch-s",
        execution.until_epoch_s_filter,
    );
    push_optional_shell_flag(&mut parts, "--pack-id", pack_id_filter);
    push_optional_shell_flag(&mut parts, "--agent-id", agent_id_filter);
    parts
}

fn discovery_group_identity_filters(
    execution: &AuditCommandExecution,
    group_by: Option<&str>,
    group: &AuditDiscoveryGroup,
) -> Option<(Option<String>, Option<String>)> {
    let group_by = group_by?;

    let mut pack_id_filter = execution.pack_id_filter.clone();
    let mut agent_id_filter = execution.agent_id_filter.clone();

    match group_by {
        "pack" => merge_discovery_group_filter(&mut pack_id_filter, group.group_value.as_ref())?,
        "agent" => {
            merge_discovery_group_filter(&mut agent_id_filter, group.group_value.as_ref())?
        }
        _ => return None,
    }

    Some((pack_id_filter, agent_id_filter))
}

fn merge_discovery_group_filter(
    filter: &mut Option<String>,
    group_value: Option<&String>,
) -> Option<()> {
    let group_value = group_value?;
    match filter.as_deref() {
        Some(existing) if existing != group_value => None,
        None => {
            *filter = Some(group_value.clone());
            Some(())
        }
        _ => Some(()),
    }
}

fn push_optional_numeric_flag(parts: &mut Vec<String>, flag: &str, value: Option<u64>) {
    if let Some(value) = value {
        parts.push(flag.to_owned());
        parts.push(value.to_string());
    }
}

fn push_optional_shell_flag(parts: &mut Vec<String>, flag: &str, value: Option<&str>) {
    if let Some(value) = value {
        push_shell_argument_flag(parts, flag, value);
    }
}

fn push_shell_argument_flag(parts: &mut Vec<String>, flag: &str, value: &str) {
    parts.push(flag.to_owned());
    parts.push(crate::cli_handoff::shell_quote_argument(value));
}
