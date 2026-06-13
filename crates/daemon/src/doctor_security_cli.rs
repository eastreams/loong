#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use loong_app as mvp;
use loong_spec::CliResult;
use serde::Serialize;
use serde_json::json;

use crate::doctor_cli::durable_audit_target_issue;

const DOCTOR_SECURITY_CLI_JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorSecurityCliJsonSchema {
    pub version: u32,
    pub surface: &'static str,
    pub purpose: &'static str,
}

#[derive(Debug, Clone)]
pub struct DoctorSecurityCommandOptions {
    pub config: Option<String>,
    pub json: bool,
    pub fix: bool,
    pub skip_model_probe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityFindingStatus {
    Covered,
    Partial,
    Exposed,
    Unknown,
}

impl SecurityFindingStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Covered => "covered",
            Self::Partial => "partial",
            Self::Exposed => "exposed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityFindingSeverity {
    Info,
    Warn,
    Critical,
}

impl SecurityFindingSeverity {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecurityFinding {
    pub id: String,
    pub title: String,
    pub status: SecurityFindingStatus,
    pub severity: SecurityFindingSeverity,
    pub summary: String,
    pub evidence: Vec<String>,
    pub next_steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecurityAuditSummary {
    pub covered: usize,
    pub partial: usize,
    pub exposed: usize,
    pub unknown: usize,
    pub info: usize,
    pub warn: usize,
    pub critical: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorSecurityAuditExecution {
    pub resolved_config_path: String,
    pub ok: bool,
    pub summary: SecurityAuditSummary,
    pub findings: Vec<SecurityFinding>,
}

pub async fn run_doctor_security_cli(options: DoctorSecurityCommandOptions) -> CliResult<()> {
    if options.fix {
        return Err("doctor security does not support --fix".to_owned());
    }

    if options.skip_model_probe {
        return Err("doctor security does not support --skip-model-probe".to_owned());
    }

    let config_path = options.config.as_deref();
    let execution = execute_doctor_security_command(config_path).await?;

    if options.json {
        let payload = doctor_security_cli_json(&execution);
        let encoded = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize doctor security output failed: {error}"))?;
        println!("{encoded}");
    } else {
        let rendered = render_doctor_security_cli_text(&execution);
        println!("{rendered}");
    }

    if !execution.ok {
        return Err("doctor security detected exposed surfaces".to_owned());
    }

    Ok(())
}

pub async fn execute_doctor_security_command(
    config: Option<&str>,
) -> CliResult<DoctorSecurityAuditExecution> {
    let (config_path, config) = mvp::config::load(config)?;
    let execution = build_doctor_security_execution(&config_path, &config).await?;
    Ok(execution)
}

pub fn doctor_security_cli_json(execution: &DoctorSecurityAuditExecution) -> serde_json::Value {
    json!({
        "schema": doctor_security_cli_schema(),
        "command": "security",
        "config": execution.resolved_config_path,
        "ok": execution.ok,
        "summary": execution.summary,
        "findings": execution.findings,
    })
}

fn doctor_security_cli_schema() -> DoctorSecurityCliJsonSchema {
    DoctorSecurityCliJsonSchema {
        version: DOCTOR_SECURITY_CLI_JSON_SCHEMA_VERSION,
        surface: "doctor_security",
        purpose: "operator_security_posture",
    }
}

pub fn render_doctor_security_cli_text(execution: &DoctorSecurityAuditExecution) -> String {
    let mut lines = Vec::new();
    let config_line = format!("doctor security config={}", execution.resolved_config_path);
    lines.push(config_line);

    let summary = &execution.summary;
    let summary_line = format!(
        "security summary: covered={} partial={} exposed={} unknown={} info={} warn={} critical={} ok={}",
        summary.covered,
        summary.partial,
        summary.exposed,
        summary.unknown,
        summary.info,
        summary.warn,
        summary.critical,
        execution.ok
    );
    lines.push(summary_line);

    for finding in &execution.findings {
        let finding_line = format!(
            "- {} [{} / {}] {}",
            finding.title,
            finding.status.as_str(),
            finding.severity.as_str(),
            finding.summary
        );
        lines.push(finding_line);

        for evidence in &finding.evidence {
            let evidence_line = format!("  evidence: {evidence}");
            lines.push(evidence_line);
        }

        for next_step in &finding.next_steps {
            let next_step_line = format!("  next: {next_step}");
            lines.push(next_step_line);
        }
    }

    lines.join("\n")
}

async fn build_doctor_security_execution(
    config_path: &Path,
    config: &mvp::config::LoongConfig,
) -> CliResult<DoctorSecurityAuditExecution> {
    let runtime =
        mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, Some(config_path));

    let mut findings = Vec::new();

    let audit_finding = assess_audit_retention(config);
    findings.push(audit_finding);

    let shell_finding = assess_shell_execution(config, &runtime);
    findings.push(shell_finding);

    let file_root_finding = assess_tool_file_root(config);
    findings.push(file_root_finding);

    let web_fetch_finding = assess_web_fetch(runtime.web_fetch.clone());
    findings.push(web_fetch_finding);

    let skills_finding = match crate::skills_policy_probe::resolve_effective_skills_policy(&runtime)
    {
        Ok(policy_probe) => assess_skills(policy_probe),
        Err(error) => assess_skills_probe_failure(runtime.skills.clone(), error),
    };
    findings.push(skills_finding);

    let secret_hygiene_finding = assess_secret_hygiene(config_path, config)?;
    findings.push(secret_hygiene_finding);

    let browser_finding = assess_browser_surfaces(&runtime);
    findings.push(browser_finding);

    let summary = summarize_findings(&findings);
    let ok = summary.exposed == 0;
    let resolved_config_path = config_path.display().to_string();

    Ok(DoctorSecurityAuditExecution {
        resolved_config_path,
        ok,
        summary,
        findings,
    })
}

fn assess_audit_retention(config: &mvp::config::LoongConfig) -> SecurityFinding {
    let audit_mode = config.audit.mode;
    let audit_mode_name = audit_mode.as_str();
    let journal_path = config.audit.resolved_path();
    let journal_path_string = journal_path.display().to_string();
    let mut evidence = Vec::new();
    let mode_evidence = format!("audit.mode={audit_mode_name}");
    evidence.push(mode_evidence);
    let path_evidence = format!("audit.journal={journal_path_string}");
    evidence.push(path_evidence);

    if matches!(audit_mode, mvp::config::AuditMode::InMemory) {
        let summary =
            "Audit evidence is kept in memory only and will be lost on restart.".to_owned();
        let next_steps = vec![
            "Switch to audit.mode = \"fanout\" or audit.mode = \"jsonl\".".to_owned(),
            "Re-run doctor security after enabling durable audit retention.".to_owned(),
        ];
        return build_finding(
            "audit_retention",
            "Audit Retention",
            SecurityFindingStatus::Exposed,
            SecurityFindingSeverity::Critical,
            summary,
            evidence,
            next_steps,
        );
    }

    let runtime_issue = durable_audit_target_issue(&journal_path);
    if let Some(runtime_issue) = runtime_issue {
        let issue_evidence = format!("runtime_probe={runtime_issue}");
        evidence.push(issue_evidence);
        let summary =
            "Durable audit retention is configured, but the journal target is not runtime-ready."
                .to_owned();
        let next_steps = vec![
            "Repair the audit journal path or parent directory permissions.".to_owned(),
            format!(
                "Run {} doctor to confirm the journal path opens cleanly.",
                mvp::config::active_cli_command_name()
            ),
        ];
        return build_finding(
            "audit_retention",
            "Audit Retention",
            SecurityFindingStatus::Exposed,
            SecurityFindingSeverity::Critical,
            summary,
            evidence,
            next_steps,
        );
    }

    let summary =
        "Durable audit retention is active and the journal target passed the runtime probe."
            .to_owned();
    let next_steps = Vec::new();
    build_finding(
        "audit_retention",
        "Audit Retention",
        SecurityFindingStatus::Covered,
        SecurityFindingSeverity::Info,
        summary,
        evidence,
        next_steps,
    )
}

fn assess_shell_execution(
    config: &mvp::config::LoongConfig,
    runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> SecurityFinding {
    let posture = mvp::tools::shell_execution_security_posture(config, runtime);
    let approval_mode = render_tool_approval_mode(posture.approval_mode);
    let autonomy_profile = posture.autonomy_profile.as_str();

    let mut evidence = Vec::new();
    let default_mode_evidence = format!(
        "tools.shell_default_mode={}",
        render_shell_default_mode(posture.default_mode)
    );
    evidence.push(default_mode_evidence);
    let allow_count_evidence = format!("tools.shell_allow.count={}", posture.allow_count);
    evidence.push(allow_count_evidence);
    let deny_count_evidence = format!("tools.shell_deny.count={}", posture.deny_count);
    evidence.push(deny_count_evidence);
    let approval_mode_evidence = format!("tools.approval.mode={approval_mode}");
    evidence.push(approval_mode_evidence);
    let autonomy_profile_evidence = format!("tools.autonomy_profile={autonomy_profile}");
    evidence.push(autonomy_profile_evidence);

    if matches!(
        posture.default_mode,
        mvp::tools::shell_policy_ext::ShellPolicyDefault::Allow
    ) {
        let summary =
            "Shell execution allows unknown commands by default, which leaves the runtime open-ended."
                .to_owned();
        let next_steps = vec![
            "Set tools.shell_default_mode = \"deny\".".to_owned(),
            "Keep tools.shell_allow to the smallest practical command set.".to_owned(),
        ];
        return build_finding(
            "shell_execution",
            "Shell Execution",
            SecurityFindingStatus::Exposed,
            SecurityFindingSeverity::Critical,
            summary,
            evidence,
            next_steps,
        );
    }

    if posture.allow_count == 0 {
        let summary =
            "Shell execution is effectively disabled by default-deny with an empty allowlist."
                .to_owned();
        let next_steps = Vec::new();
        return build_finding(
            "shell_execution",
            "Shell Execution",
            SecurityFindingStatus::Covered,
            SecurityFindingSeverity::Info,
            summary,
            evidence,
            next_steps,
        );
    }

    let summary =
        "Shell execution is default-deny, but allowlisted commands remain available without OS-level isolation."
            .to_owned();
    let next_steps = vec![
        "Review whether every command in tools.shell_allow still needs to be present.".to_owned(),
        "Prefer approval gating for risky shell workflows when commands must remain available."
            .to_owned(),
    ];
    build_finding(
        "shell_execution",
        "Shell Execution",
        SecurityFindingStatus::Partial,
        SecurityFindingSeverity::Warn,
        summary,
        evidence,
        next_steps,
    )
}

fn assess_tool_file_root(config: &mvp::config::LoongConfig) -> SecurityFinding {
    let posture = mvp::tools::tool_file_root_security_posture(config);

    let mut evidence = Vec::new();
    let explicit_root_value = posture
        .explicit_root
        .as_deref()
        .unwrap_or("(current working directory)");
    let explicit_root_evidence = format!("tools.file_root={explicit_root_value}");
    evidence.push(explicit_root_evidence);
    let effective_root_evidence = format!("effective_tool_root={}", posture.effective_root);
    evidence.push(effective_root_evidence);
    let root_exists_evidence = format!("effective_tool_root.exists={}", posture.root_exists);
    evidence.push(root_exists_evidence);

    if posture.uses_current_working_directory_fallback {
        let summary =
            "File tools still fall back to the current working directory because tools.file_root is unset."
                .to_owned();
        let next_steps = vec![
            "Set tools.file_root to a dedicated workspace path.".to_owned(),
            format!(
                "Run {} doctor --fix if you want the workspace directory created automatically.",
                mvp::config::active_cli_command_name()
            ),
        ];
        return build_finding(
            "tool_file_root",
            "Tool File Root",
            SecurityFindingStatus::Exposed,
            SecurityFindingSeverity::Critical,
            summary,
            evidence,
            next_steps,
        );
    }

    let summary =
        "File tools are rooted to an explicit workspace path, but confinement remains tool-layer rather than OS-enforced."
            .to_owned();
    let next_steps = vec![
        "Keep tools.file_root on a narrow workspace path instead of a broad home-directory root."
            .to_owned(),
    ];
    build_finding(
        "tool_file_root",
        "Tool File Root",
        SecurityFindingStatus::Partial,
        SecurityFindingSeverity::Warn,
        summary,
        evidence,
        next_steps,
    )
}

fn assess_web_fetch(policy: mvp::tools::runtime_config::WebFetchRuntimePolicy) -> SecurityFinding {
    let posture = mvp::tools::web_fetch_security_posture(&policy);
    let mut evidence = Vec::new();
    let enabled_evidence = format!("tools.web.enabled={}", posture.enabled);
    evidence.push(enabled_evidence);
    let private_hosts_evidence = format!(
        "tools.web.allow_private_hosts={}",
        posture.allow_private_hosts
    );
    evidence.push(private_hosts_evidence);
    let allowed_domain_count = posture.allowed_domain_count;
    let allowed_domain_count_evidence =
        format!("tools.web.allowed_domains.count={allowed_domain_count}");
    evidence.push(allowed_domain_count_evidence);
    let blocked_domain_count = posture.blocked_domain_count;
    let blocked_domain_count_evidence =
        format!("tools.web.blocked_domains.count={blocked_domain_count}");
    evidence.push(blocked_domain_count_evidence);

    if !posture.enabled {
        let summary = "Web fetch is disabled for the local runtime.".to_owned();
        let next_steps = Vec::new();
        return build_finding(
            "web_fetch",
            "Web Fetch Egress",
            SecurityFindingStatus::Covered,
            SecurityFindingSeverity::Info,
            summary,
            evidence,
            next_steps,
        );
    }

    if posture.allow_private_hosts {
        let summary =
            "Web fetch allows private hosts, which weakens the default SSRF boundary for operator workloads."
                .to_owned();
        let next_steps = vec![
            "Set tools.web.allow_private_hosts = false unless private-network fetch is required."
                .to_owned(),
        ];
        return build_finding(
            "web_fetch",
            "Web Fetch Egress",
            SecurityFindingStatus::Exposed,
            SecurityFindingSeverity::Critical,
            summary,
            evidence,
            next_steps,
        );
    }

    if posture.enforce_allowed_domains {
        let summary =
            "Web fetch denies private hosts and is constrained to an explicit domain allowlist."
                .to_owned();
        let next_steps = Vec::new();
        return build_finding(
            "web_fetch",
            "Web Fetch Egress",
            SecurityFindingStatus::Covered,
            SecurityFindingSeverity::Info,
            summary,
            evidence,
            next_steps,
        );
    }

    let summary =
        "Web fetch denies private hosts, but public-domain access is still open-ended because no allowlist is configured."
            .to_owned();
    let next_steps = vec![
        "Add tools.web.allowed_domains when the runtime only needs a known destination set."
            .to_owned(),
    ];
    build_finding(
        "web_fetch",
        "Web Fetch Egress",
        SecurityFindingStatus::Partial,
        SecurityFindingSeverity::Warn,
        summary,
        evidence,
        next_steps,
    )
}

fn assess_skills(
    policy_probe: crate::skills_policy_probe::EffectiveSkillsPolicyProbe,
) -> SecurityFinding {
    let posture =
        mvp::tools::skills_security_posture(&policy_probe.policy, policy_probe.override_active);
    let mut evidence = Vec::new();
    let enabled_evidence = format!("skills.enabled={}", posture.enabled);
    evidence.push(enabled_evidence);
    let override_active_evidence = format!("skills.override_active={}", posture.override_active);
    evidence.push(override_active_evidence);
    let approval_evidence = format!(
        "skills.require_download_approval={}",
        posture.require_download_approval
    );
    evidence.push(approval_evidence);
    let allow_count = posture.allowed_domain_count;
    let allow_count_evidence = format!("skills.allowed_domains.count={allow_count}");
    evidence.push(allow_count_evidence);
    let block_count = posture.blocked_domain_count;
    let block_count_evidence = format!("skills.blocked_domains.count={block_count}");
    evidence.push(block_count_evidence);
    let auto_expose_evidence = format!(
        "skills.auto_expose_installed={}",
        posture.auto_expose_installed
    );
    evidence.push(auto_expose_evidence);

    if !posture.enabled {
        let summary = "External skills are disabled for this runtime.".to_owned();
        let next_steps = Vec::new();
        return build_finding(
            "skills",
            "External Skills",
            SecurityFindingStatus::Covered,
            SecurityFindingSeverity::Info,
            summary,
            evidence,
            next_steps,
        );
    }

    if posture.auto_expose_installed || !posture.require_download_approval {
        let summary =
            "External skills are enabled with a posture that can auto-expose or download without explicit approval."
                .to_owned();
        let next_steps = vec![
            "Keep skills.require_download_approval = true.".to_owned(),
            "Keep skills.auto_expose_installed = false until a review step completes.".to_owned(),
        ];
        return build_finding(
            "skills",
            "External Skills",
            SecurityFindingStatus::Exposed,
            SecurityFindingSeverity::Critical,
            summary,
            evidence,
            next_steps,
        );
    }

    let summary =
        "External skills are approval-gated, but the current runtime still lacks provenance scanning and isolated execution."
            .to_owned();
    let mut next_steps = Vec::new();
    if posture.allowed_domain_count == 0 {
        next_steps.push("Pin skills.allowed_domains to the smallest trusted host set.".to_owned());
    }
    next_steps.push("Keep installed skills dark until operator review completes.".to_owned());
    build_finding(
        "skills",
        "External Skills",
        SecurityFindingStatus::Partial,
        SecurityFindingSeverity::Warn,
        summary,
        evidence,
        next_steps,
    )
}

fn assess_skills_probe_failure(
    config_projection: mvp::tools::runtime_config::SkillsRuntimePolicy,
    error: String,
) -> SecurityFinding {
    let posture = mvp::tools::skills_security_posture_probe_failure(&config_projection, error);
    let mut evidence = Vec::new();
    let error_evidence = format!("effective_policy_probe.error={}", posture.error);
    evidence.push(error_evidence);
    let enabled_evidence = format!("config_projection.skills.enabled={}", posture.enabled);
    evidence.push(enabled_evidence);
    let approval_evidence = format!(
        "config_projection.skills.require_download_approval={}",
        posture.require_download_approval
    );
    evidence.push(approval_evidence);
    let auto_expose_evidence = format!(
        "config_projection.skills.auto_expose_installed={}",
        posture.auto_expose_installed
    );
    evidence.push(auto_expose_evidence);

    let summary =
        "The effective skills runtime policy could not be resolved through the policy surface, so the live posture is unknown."
            .to_owned();
    let cli = mvp::config::active_cli_command_name();
    let next_steps = vec![
        format!("Run `{cli} skills policy show --json` to confirm the effective runtime policy."),
        "Repair the skills.policy tool path before relying on this audit result.".to_owned(),
    ];
    build_finding(
        "skills",
        "External Skills",
        SecurityFindingStatus::Unknown,
        SecurityFindingSeverity::Warn,
        summary,
        evidence,
        next_steps,
    )
}

fn assess_secret_hygiene(
    config_path: &Path,
    config: &mvp::config::LoongConfig,
) -> CliResult<SecurityFinding> {
    let mut observations = mvp::config::collect_secret_observations(config);
    observations.sort_by(|left, right| left.field_path.cmp(&right.field_path));

    let counts = mvp::config::summarize_secret_observations(&observations);
    let env_pointer_diagnostics = mvp::config::collect_env_pointer_diagnostics(config);
    let inline_paths = mvp::config::observation_paths_for_kind(
        &observations,
        mvp::config::SecretReferenceKind::InlineLiteral,
    );
    let exec_paths = mvp::config::observation_paths_for_kind(
        &observations,
        mvp::config::SecretReferenceKind::Exec,
    );

    let mut evidence = Vec::new();
    let counts_evidence = format!(
        "secret_refs env={} file={} exec={} inline_literal={}",
        counts.env, counts.file, counts.exec, counts.inline_literal
    );
    evidence.push(counts_evidence);

    if !inline_paths.is_empty() {
        let inline_evidence = format!("inline_literal_paths={}", inline_paths.join(", "));
        evidence.push(inline_evidence);
    }

    if !exec_paths.is_empty() {
        let exec_evidence = format!("exec_paths={}", exec_paths.join(", "));
        evidence.push(exec_evidence);
    }

    let env_pointer_count = env_pointer_diagnostics.len();
    let env_pointer_count_evidence =
        format!("config.env_pointer_diagnostics.count={env_pointer_count}");
    evidence.push(env_pointer_count_evidence);

    for diagnostic in env_pointer_diagnostics {
        let diagnostic_evidence = format!(
            "diagnostic {} field={} severity={}",
            diagnostic.code, diagnostic.field_path, diagnostic.severity
        );
        evidence.push(diagnostic_evidence);
    }

    let config_mode = config_file_mode(config_path)?;
    if let Some(config_mode) = config_mode {
        let mode_evidence = format!("config.permissions={config_mode}");
        evidence.push(mode_evidence);
    }

    if counts.inline_literal > 0 {
        let permission_issue = config_file_permission_issue(config_path)?;
        if let Some(permission_issue) = permission_issue {
            evidence.push(permission_issue);
        }

        let summary =
            "Inline secret literals are present in the config, so credential material is stored directly on disk."
                .to_owned();
        let mut next_steps = Vec::new();
        next_steps.push(
            "Move inline secrets to env or file secret refs and re-run doctor security.".to_owned(),
        );
        if cfg!(unix) {
            next_steps.push(
                "Restrict the config file to chmod 600 when secrets must stay on disk.".to_owned(),
            );
        }
        return Ok(build_finding(
            "secret_hygiene",
            "Secret Hygiene",
            SecurityFindingStatus::Exposed,
            SecurityFindingSeverity::Critical,
            summary,
            evidence,
            next_steps,
        ));
    }

    if counts.exec > 0 || !mvp::config::collect_env_pointer_diagnostics(config).is_empty() {
        let summary =
            "Secret references avoid inline literals, but some entries still rely on host exec or env-pointer cleanup."
                .to_owned();
        let mut next_steps = Vec::new();
        if counts.exec > 0 {
            next_steps.push(
                "Prefer env or file secret refs when exec-based secret resolution is not required."
                    .to_owned(),
            );
        }
        if !mvp::config::collect_env_pointer_diagnostics(config).is_empty() {
            next_steps.push(
                "Normalize env-pointer fields so the config stays on the canonical secret-ref path."
                    .to_owned(),
            );
        }
        return Ok(build_finding(
            "secret_hygiene",
            "Secret Hygiene",
            SecurityFindingStatus::Partial,
            SecurityFindingSeverity::Warn,
            summary,
            evidence,
            next_steps,
        ));
    }

    let summary =
        "Configured secrets use env/file references without inline literals or exec-based secret resolution."
            .to_owned();
    let next_steps = Vec::new();
    Ok(build_finding(
        "secret_hygiene",
        "Secret Hygiene",
        SecurityFindingStatus::Covered,
        SecurityFindingSeverity::Info,
        summary,
        evidence,
        next_steps,
    ))
}

fn assess_browser_surfaces(
    runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> SecurityFinding {
    let posture = mvp::tools::browser_surface_security_posture(runtime);

    let mut evidence = Vec::new();
    let browser_enabled_evidence = format!("tools.browser.enabled={}", posture.enabled);
    evidence.push(browser_enabled_evidence);
    let browser_tier_evidence =
        format!("browser.execution_tier={}", posture.execution_tier.as_str());
    evidence.push(browser_tier_evidence);
    let summary = if posture.enabled {
        "Built-in browse stays on the restricted lane, and richer browser automation is expected to run through an external skill or plugin."
            .to_owned()
    } else {
        "Browser automation surfaces are disabled.".to_owned()
    };
    let next_steps = Vec::new();
    build_finding(
        "browser_surfaces",
        "Browser Surfaces",
        SecurityFindingStatus::Covered,
        SecurityFindingSeverity::Info,
        summary,
        evidence,
        next_steps,
    )
}

fn build_finding(
    id: &str,
    title: &str,
    status: SecurityFindingStatus,
    severity: SecurityFindingSeverity,
    summary: String,
    evidence: Vec<String>,
    next_steps: Vec<String>,
) -> SecurityFinding {
    SecurityFinding {
        id: id.to_owned(),
        title: title.to_owned(),
        status,
        severity,
        summary,
        evidence,
        next_steps,
    }
}

fn summarize_findings(findings: &[SecurityFinding]) -> SecurityAuditSummary {
    let mut summary = SecurityAuditSummary {
        covered: 0,
        partial: 0,
        exposed: 0,
        unknown: 0,
        info: 0,
        warn: 0,
        critical: 0,
    };

    for finding in findings {
        match finding.status {
            SecurityFindingStatus::Covered => summary.covered += 1,
            SecurityFindingStatus::Partial => summary.partial += 1,
            SecurityFindingStatus::Exposed => summary.exposed += 1,
            SecurityFindingStatus::Unknown => summary.unknown += 1,
        }

        match finding.severity {
            SecurityFindingSeverity::Info => summary.info += 1,
            SecurityFindingSeverity::Warn => summary.warn += 1,
            SecurityFindingSeverity::Critical => summary.critical += 1,
        }
    }

    summary
}

fn render_shell_default_mode(
    mode: mvp::tools::shell_policy_ext::ShellPolicyDefault,
) -> &'static str {
    match mode {
        mvp::tools::shell_policy_ext::ShellPolicyDefault::Deny => "deny",
        mvp::tools::shell_policy_ext::ShellPolicyDefault::Allow => "allow",
    }
}

fn render_tool_approval_mode(mode: mvp::config::GovernedToolApprovalMode) -> &'static str {
    match mode {
        mvp::config::GovernedToolApprovalMode::Disabled => "disabled",
        mvp::config::GovernedToolApprovalMode::MediumBalanced => "medium_balanced",
        mvp::config::GovernedToolApprovalMode::Strict => "strict",
    }
}

fn config_file_mode(config_path: &Path) -> CliResult<Option<String>> {
    #[cfg(unix)]
    {
        let metadata = fs::metadata(config_path).map_err(|error| {
            format!(
                "inspect config permissions for {} failed: {error}",
                config_path.display()
            )
        })?;
        let mode = metadata.permissions().mode() & 0o777;
        let rendered_mode = format!("0o{mode:03o}");
        Ok(Some(rendered_mode))
    }

    #[cfg(not(unix))]
    {
        let _ = config_path;
        Ok(None)
    }
}

fn config_file_permission_issue(config_path: &Path) -> CliResult<Option<String>> {
    #[cfg(unix)]
    {
        let metadata = fs::metadata(config_path).map_err(|error| {
            format!(
                "inspect config permissions for {} failed: {error}",
                config_path.display()
            )
        })?;
        let mode = metadata.permissions().mode() & 0o777;
        let group_or_world_bits = mode & 0o077;
        if group_or_world_bits == 0 {
            return Ok(None);
        }

        let rendered_mode = format!(
            "config file mode is 0o{mode:03o}; inline secrets should be stored under 0o600 or stricter"
        );
        Ok(Some(rendered_mode))
    }

    #[cfg(not(unix))]
    {
        let _ = config_path;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use loong_contracts::SecretRef;
    use std::path::PathBuf;
    use std::sync::MutexGuard;

    fn temp_config_path(label: &str) -> PathBuf {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let file_name = format!("loong-doctor-security-{label}-{epoch}.toml");
        std::env::temp_dir().join(file_name)
    }

    fn write_placeholder_config(path: &Path) {
        fs::write(path, "active_provider = \"openai\"\n").expect("write placeholder config");
    }

    fn finding_by_id<'a>(findings: &'a [SecurityFinding], id: &str) -> &'a SecurityFinding {
        findings
            .iter()
            .find(|finding| finding.id == id)
            .unwrap_or_else(|| panic!("missing finding `{id}`"))
    }

    struct SkillsPolicyResetGuard {
        _lock: MutexGuard<'static, ()>,
        runtime_config: mvp::tools::runtime_config::ToolRuntimeConfig,
    }

    impl SkillsPolicyResetGuard {
        fn new(runtime_config: &mvp::tools::runtime_config::ToolRuntimeConfig) -> Self {
            let lock = crate::test_support::lock_daemon_test_environment();
            Self {
                _lock: lock,
                runtime_config: runtime_config.clone(),
            }
        }
    }

    impl Drop for SkillsPolicyResetGuard {
        fn drop(&mut self) {
            let _ = mvp::tools::skills_policy_reset_with_config(true, &self.runtime_config);
        }
    }

    #[tokio::test]
    async fn default_shell_execution_is_exposed_in_yolo_mode() {
        let path = temp_config_path("shell-covered");
        write_placeholder_config(&path);

        let config = mvp::config::LoongConfig::default();
        let execution = build_doctor_security_execution(&path, &config)
            .await
            .expect("build security execution");
        let finding = finding_by_id(&execution.findings, "shell_execution");

        assert_eq!(finding.status, SecurityFindingStatus::Exposed);
        assert_eq!(finding.severity, SecurityFindingSeverity::Critical);
        assert!(
            finding
                .summary
                .contains("allows unknown commands by default"),
            "unexpected summary: {}",
            finding.summary
        );
    }

    #[test]
    fn tool_file_root_finding_uses_explicit_and_effective_resolution_truth() {
        let config = mvp::config::LoongConfig::default();
        let finding = assess_tool_file_root(&config);
        let rendered_evidence = finding.evidence.join("\n");
        let effective_root = config.tools.resolved_file_root();
        let effective_root_text = effective_root.display().to_string();

        assert_eq!(finding.status, SecurityFindingStatus::Exposed);
        assert!(rendered_evidence.contains("tools.file_root=(current working directory)"));
        assert!(rendered_evidence.contains(effective_root_text.as_str()));
    }

    #[tokio::test]
    async fn secret_hygiene_exposes_inline_literals() {
        let path = temp_config_path("inline-secret");
        write_placeholder_config(&path);

        #[cfg(unix)]
        {
            let metadata = fs::metadata(&path).expect("config metadata");
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o644);
            fs::set_permissions(&path, permissions).expect("set config permissions");
        }

        let mut config = mvp::config::LoongConfig::default();
        config.provider.api_key = Some(SecretRef::Inline("inline-secret".to_owned()));

        let execution = build_doctor_security_execution(&path, &config)
            .await
            .expect("build security execution");
        let finding = finding_by_id(&execution.findings, "secret_hygiene");

        assert_eq!(finding.status, SecurityFindingStatus::Exposed);
        assert_eq!(finding.severity, SecurityFindingSeverity::Critical);
        assert!(
            finding
                .evidence
                .iter()
                .any(|line| line.contains("provider.api_key")),
            "expected provider.api_key evidence: {:?}",
            finding.evidence
        );
    }

    #[tokio::test]
    async fn secret_hygiene_scans_legacy_provider_fields_and_auth_headers_with_profiles_present() {
        let path = temp_config_path("provider-secret-headers");
        write_placeholder_config(&path);

        let mut config = mvp::config::LoongConfig::default();
        config.provider.api_key = Some(SecretRef::Inline("legacy-inline-secret".to_owned()));
        config
            .provider
            .headers
            .insert("X-API-Key".to_owned(), "top-level-header-secret".to_owned());

        let mut profile = mvp::config::ProviderProfileConfig::default();
        profile.provider.headers.insert(
            "Authorization".to_owned(),
            "Bearer profile-secret".to_owned(),
        );
        config.providers.insert("openai".to_owned(), profile);

        let execution = build_doctor_security_execution(&path, &config)
            .await
            .expect("build security execution");
        let finding = finding_by_id(&execution.findings, "secret_hygiene");
        let rendered_evidence = finding.evidence.join("\n");

        assert_eq!(finding.status, SecurityFindingStatus::Exposed);
        assert!(rendered_evidence.contains("provider.api_key"));
        assert!(rendered_evidence.contains("provider.headers.X-API-Key"));
        assert!(rendered_evidence.contains("providers.openai.headers.Authorization"));
    }

    #[tokio::test]
    async fn secret_hygiene_scans_firecrawl_web_search_credentials() {
        let path = temp_config_path("firecrawl-web-search-secret");
        write_placeholder_config(&path);

        let mut config = mvp::config::LoongConfig::default();
        config.tools.web_search.firecrawl_api_key = Some("firecrawl-inline-secret".to_owned());

        let execution = build_doctor_security_execution(&path, &config)
            .await
            .expect("build security execution");
        let finding = finding_by_id(&execution.findings, "secret_hygiene");
        let rendered_evidence = finding.evidence.join("\n");

        assert_eq!(finding.status, SecurityFindingStatus::Exposed);
        assert!(rendered_evidence.contains("tools.web_search.firecrawl_api_key"));
    }

    #[tokio::test]
    async fn skills_expose_when_auto_expose_or_approval_is_open() {
        let path = temp_config_path("external-skills");
        write_placeholder_config(&path);

        let mut config = mvp::config::LoongConfig::default();
        config.skills.enabled = true;
        config.skills.require_download_approval = false;
        config.skills.auto_expose_installed = true;

        let execution = build_doctor_security_execution(&path, &config)
            .await
            .expect("build security execution");
        let finding = finding_by_id(&execution.findings, "skills");

        assert_eq!(finding.status, SecurityFindingStatus::Exposed);
        assert_eq!(finding.severity, SecurityFindingSeverity::Critical);
    }

    #[tokio::test]
    async fn skills_audit_uses_effective_policy_override() {
        let path = temp_config_path("external-skills-override");
        write_placeholder_config(&path);

        let config = mvp::config::LoongConfig::default();
        let runtime_config =
            mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(&config, Some(&path));
        let _reset_guard = SkillsPolicyResetGuard::new(&runtime_config);

        mvp::tools::skills_policy_set_with_config(
            Some(true),
            Some(false),
            Some(std::collections::BTreeSet::from([
                "override.example".to_owned()
            ])),
            Some(std::collections::BTreeSet::from([
                "blocked.example".to_owned()
            ])),
            true,
            &runtime_config,
        )
        .expect("override skills policy");

        let execution = build_doctor_security_execution(&path, &config)
            .await
            .expect("build security execution");
        let finding = finding_by_id(&execution.findings, "skills");
        let rendered_evidence = finding.evidence.join("\n");

        assert_eq!(finding.status, SecurityFindingStatus::Exposed);
        assert!(rendered_evidence.contains("skills.override_active=true"));
        assert!(rendered_evidence.contains("skills.enabled=true"));
        assert!(rendered_evidence.contains("skills.allowed_domains.count=1"));
    }

    #[tokio::test]
    async fn browser_surfaces_report_built_in_browse_lane() {
        let path = temp_config_path("browser-companion");
        write_placeholder_config(&path);

        let mut config = mvp::config::LoongConfig::default();
        config.tools.browser.enabled = true;

        let execution = build_doctor_security_execution(&path, &config)
            .await
            .expect("build security execution");
        let finding = finding_by_id(&execution.findings, "browser_surfaces");

        assert_eq!(finding.status, SecurityFindingStatus::Covered);
        assert_eq!(finding.severity, SecurityFindingSeverity::Info);
    }

    #[test]
    fn json_payload_uses_security_command_name() {
        let execution = DoctorSecurityAuditExecution {
            resolved_config_path: "/tmp/config.toml".to_owned(),
            ok: true,
            summary: SecurityAuditSummary {
                covered: 1,
                partial: 0,
                exposed: 0,
                unknown: 0,
                info: 1,
                warn: 0,
                critical: 0,
            },
            findings: vec![build_finding(
                "audit_retention",
                "Audit Retention",
                SecurityFindingStatus::Covered,
                SecurityFindingSeverity::Info,
                "durable".to_owned(),
                Vec::new(),
                Vec::new(),
            )],
        };

        let payload = doctor_security_cli_json(&execution);

        assert_eq!(payload["schema"]["version"], 1);
        assert_eq!(payload["schema"]["surface"], "doctor_security");
        assert_eq!(payload["schema"]["purpose"], "operator_security_posture");
        assert_eq!(payload["command"], "security");
        assert_eq!(payload["summary"]["covered"], 1);
        assert_eq!(payload["findings"][0]["id"], "audit_retention");
    }

    #[tokio::test]
    async fn run_doctor_security_cli_rejects_unsupported_parent_flags() {
        let fix_error = run_doctor_security_cli(DoctorSecurityCommandOptions {
            config: None,
            json: false,
            fix: true,
            skip_model_probe: false,
        })
        .await
        .expect_err("doctor security should reject --fix");

        let probe_error = run_doctor_security_cli(DoctorSecurityCommandOptions {
            config: None,
            json: false,
            fix: false,
            skip_model_probe: true,
        })
        .await
        .expect_err("doctor security should reject --skip-model-probe");

        assert!(fix_error.contains("--fix"));
        assert!(probe_error.contains("--skip-model-probe"));
    }

    #[tokio::test]
    async fn run_doctor_security_cli_json_fails_when_exposed_findings_exist() {
        let path = temp_config_path("json-exposed");
        let path_string = path.display().to_string();

        let mut config = mvp::config::LoongConfig::default();
        config.audit.mode = mvp::config::AuditMode::InMemory;
        mvp::config::write(Some(path_string.as_str()), &config, true).expect("write config");

        let error = run_doctor_security_cli(DoctorSecurityCommandOptions {
            config: Some(path_string),
            json: true,
            fix: false,
            skip_model_probe: false,
        })
        .await
        .expect_err("json mode should fail when exposed findings exist");

        assert!(error.contains("exposed surfaces"));
    }
}
