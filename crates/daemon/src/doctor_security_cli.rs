use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;

use loongclaw_app as mvp;
use loongclaw_contracts::SecretRef;
use loongclaw_spec::CliResult;
use serde::Serialize;
use serde_json::json;

use crate::doctor_cli::durable_audit_target_issue;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecretReferenceKind {
    Env,
    File,
    Exec,
    InlineLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SecretObservation {
    field_path: String,
    kind: SecretReferenceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct SecretObservationCounts {
    env: usize,
    file: usize,
    exec: usize,
    inline_literal: usize,
}

impl SecretObservationCounts {
    fn record(&mut self, kind: SecretReferenceKind) {
        match kind {
            SecretReferenceKind::Env => self.env += 1,
            SecretReferenceKind::File => self.file += 1,
            SecretReferenceKind::Exec => self.exec += 1,
            SecretReferenceKind::InlineLiteral => self.inline_literal += 1,
        }
    }
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
        return Ok(());
    }

    let rendered = render_doctor_security_cli_text(&execution);
    println!("{rendered}");

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
        "command": "security",
        "config": execution.resolved_config_path,
        "ok": execution.ok,
        "summary": execution.summary,
        "findings": execution.findings,
    })
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
    config: &mvp::config::LoongClawConfig,
) -> CliResult<DoctorSecurityAuditExecution> {
    let runtime = mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        config,
        Some(config_path),
    );
    let browser_companion_diagnostics =
        crate::browser_companion_diagnostics::collect_browser_companion_diagnostics(config).await;

    let mut findings = Vec::new();

    let audit_finding = assess_audit_retention(config);
    findings.push(audit_finding);

    let shell_finding = assess_shell_execution(config, &runtime);
    findings.push(shell_finding);

    let file_root_finding = assess_tool_file_root(config, &runtime);
    findings.push(file_root_finding);

    let web_fetch_finding = assess_web_fetch(runtime.web_fetch.clone());
    findings.push(web_fetch_finding);

    let external_skills_finding = assess_external_skills(runtime.external_skills.clone());
    findings.push(external_skills_finding);

    let secret_hygiene_finding = assess_secret_hygiene(config_path, config)?;
    findings.push(secret_hygiene_finding);

    let browser_finding = assess_browser_surfaces(&runtime, browser_companion_diagnostics.as_ref());
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

fn assess_audit_retention(config: &mvp::config::LoongClawConfig) -> SecurityFinding {
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
            "Run loongclaw doctor to confirm the journal path opens cleanly.".to_owned(),
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
    config: &mvp::config::LoongClawConfig,
    runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> SecurityFinding {
    let default_mode = runtime.shell_default_mode;
    let allow_count = runtime.shell_allow.len();
    let deny_count = runtime.shell_deny.len();
    let approval_mode = render_tool_approval_mode(config.tools.approval.mode);
    let autonomy_profile = config.tools.autonomy_profile.as_str();

    let mut evidence = Vec::new();
    let default_mode_evidence = format!(
        "tools.shell_default_mode={}",
        render_shell_default_mode(default_mode)
    );
    evidence.push(default_mode_evidence);
    let allow_count_evidence = format!("tools.shell_allow.count={allow_count}");
    evidence.push(allow_count_evidence);
    let deny_count_evidence = format!("tools.shell_deny.count={deny_count}");
    evidence.push(deny_count_evidence);
    let approval_mode_evidence = format!("tools.approval.mode={approval_mode}");
    evidence.push(approval_mode_evidence);
    let autonomy_profile_evidence = format!("tools.autonomy_profile={autonomy_profile}");
    evidence.push(autonomy_profile_evidence);

    if matches!(
        default_mode,
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

    if allow_count == 0 {
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

fn assess_tool_file_root(
    config: &mvp::config::LoongClawConfig,
    runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> SecurityFinding {
    let explicit_root = config.tools.file_root.as_deref();
    let effective_root = runtime
        .file_root
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let effective_root_string = effective_root.display().to_string();
    let root_exists = effective_root.exists();

    let mut evidence = Vec::new();
    let explicit_root_value = explicit_root.unwrap_or("(current working directory)");
    let explicit_root_evidence = format!("tools.file_root={explicit_root_value}");
    evidence.push(explicit_root_evidence);
    let effective_root_evidence = format!("effective_tool_root={effective_root_string}");
    evidence.push(effective_root_evidence);
    let root_exists_evidence = format!("effective_tool_root.exists={root_exists}");
    evidence.push(root_exists_evidence);

    let explicit_root_missing = explicit_root
        .map(str::trim)
        .is_none_or(|value| value.is_empty());
    if explicit_root_missing {
        let summary =
            "File tools still fall back to the current working directory because tools.file_root is unset."
                .to_owned();
        let next_steps = vec![
            "Set tools.file_root to a dedicated workspace path.".to_owned(),
            "Run loongclaw doctor --fix if you want the workspace directory created automatically."
                .to_owned(),
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
    let mut evidence = Vec::new();
    let enabled_evidence = format!("tools.web.enabled={}", policy.enabled);
    evidence.push(enabled_evidence);
    let private_hosts_evidence = format!(
        "tools.web.allow_private_hosts={}",
        policy.allow_private_hosts
    );
    evidence.push(private_hosts_evidence);
    let allowed_domain_count = policy.allowed_domains.len();
    let allowed_domain_count_evidence =
        format!("tools.web.allowed_domains.count={allowed_domain_count}");
    evidence.push(allowed_domain_count_evidence);
    let blocked_domain_count = policy.blocked_domains.len();
    let blocked_domain_count_evidence =
        format!("tools.web.blocked_domains.count={blocked_domain_count}");
    evidence.push(blocked_domain_count_evidence);

    if !policy.enabled {
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

    if policy.allow_private_hosts {
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

    if policy.enforce_allowed_domains {
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

fn assess_external_skills(
    policy: mvp::tools::runtime_config::ExternalSkillsRuntimePolicy,
) -> SecurityFinding {
    let mut evidence = Vec::new();
    let enabled_evidence = format!("external_skills.enabled={}", policy.enabled);
    evidence.push(enabled_evidence);
    let approval_evidence = format!(
        "external_skills.require_download_approval={}",
        policy.require_download_approval
    );
    evidence.push(approval_evidence);
    let allow_count = policy.allowed_domains.len();
    let allow_count_evidence = format!("external_skills.allowed_domains.count={allow_count}");
    evidence.push(allow_count_evidence);
    let block_count = policy.blocked_domains.len();
    let block_count_evidence = format!("external_skills.blocked_domains.count={block_count}");
    evidence.push(block_count_evidence);
    let auto_expose_evidence = format!(
        "external_skills.auto_expose_installed={}",
        policy.auto_expose_installed
    );
    evidence.push(auto_expose_evidence);

    if !policy.enabled {
        let summary = "External skills are disabled for this runtime.".to_owned();
        let next_steps = Vec::new();
        return build_finding(
            "external_skills",
            "External Skills",
            SecurityFindingStatus::Covered,
            SecurityFindingSeverity::Info,
            summary,
            evidence,
            next_steps,
        );
    }

    if policy.auto_expose_installed || !policy.require_download_approval {
        let summary =
            "External skills are enabled with a posture that can auto-expose or download without explicit approval."
                .to_owned();
        let next_steps = vec![
            "Keep external_skills.require_download_approval = true.".to_owned(),
            "Keep external_skills.auto_expose_installed = false until a review step completes."
                .to_owned(),
        ];
        return build_finding(
            "external_skills",
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
    if policy.allowed_domains.is_empty() {
        next_steps.push(
            "Pin external_skills.allowed_domains to the smallest trusted host set.".to_owned(),
        );
    }
    next_steps.push("Keep installed skills dark until operator review completes.".to_owned());
    build_finding(
        "external_skills",
        "External Skills",
        SecurityFindingStatus::Partial,
        SecurityFindingSeverity::Warn,
        summary,
        evidence,
        next_steps,
    )
}

fn assess_secret_hygiene(
    config_path: &Path,
    config: &mvp::config::LoongClawConfig,
) -> CliResult<SecurityFinding> {
    let mut observations = collect_secret_observations(config);
    observations.sort_by(|left, right| left.field_path.cmp(&right.field_path));

    let counts = summarize_secret_observations(&observations);
    let env_pointer_diagnostics = collect_env_pointer_diagnostics(config);
    let inline_paths =
        observation_paths_for_kind(&observations, SecretReferenceKind::InlineLiteral);
    let exec_paths = observation_paths_for_kind(&observations, SecretReferenceKind::Exec);

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

    if counts.exec > 0 || !collect_env_pointer_diagnostics(config).is_empty() {
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
        if !collect_env_pointer_diagnostics(config).is_empty() {
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
    diagnostics: Option<&crate::browser_companion_diagnostics::BrowserCompanionDiagnostics>,
) -> SecurityFinding {
    let browser_enabled = runtime.browser.enabled;
    let browser_tier = runtime.browser_execution_security_tier();
    let companion_enabled = runtime.browser_companion.enabled;
    let companion_tier = runtime.browser_companion_execution_security_tier();

    let mut evidence = Vec::new();
    let browser_enabled_evidence = format!("tools.browser.enabled={browser_enabled}");
    evidence.push(browser_enabled_evidence);
    let browser_tier_evidence = format!("browser.execution_tier={}", browser_tier.as_str());
    evidence.push(browser_tier_evidence);
    let companion_enabled_evidence = format!("tools.browser_companion.enabled={companion_enabled}");
    evidence.push(companion_enabled_evidence);
    let companion_tier_evidence = format!(
        "browser_companion.execution_tier={}",
        companion_tier.as_str()
    );
    evidence.push(companion_tier_evidence);

    if !companion_enabled {
        let summary = if browser_enabled {
            "Browser automation stays on the built-in restricted lane because the managed browser companion is disabled."
                .to_owned()
        } else {
            "Browser automation surfaces are disabled.".to_owned()
        };
        let next_steps = Vec::new();
        return build_finding(
            "browser_surfaces",
            "Browser Surfaces",
            SecurityFindingStatus::Covered,
            SecurityFindingSeverity::Info,
            summary,
            evidence,
            next_steps,
        );
    }

    if let Some(diagnostics) = diagnostics {
        let install_evidence =
            format!("browser_companion.install={}", diagnostics.install_detail());
        evidence.push(install_evidence);
        if let Some(runtime_gate_detail) = diagnostics.runtime_gate_detail() {
            let gate_evidence = format!("browser_companion.runtime_gate={runtime_gate_detail}");
            evidence.push(gate_evidence);
        }

        if !diagnostics.install_ready() || !diagnostics.runtime_ready {
            let summary =
                "The browser companion lane is enabled, but install or runtime readiness is still incomplete."
                    .to_owned();
            let next_steps = vec![
                "Keep the built-in browser lane as the active path until the companion runtime is fully ready."
                    .to_owned(),
                "Run loongclaw doctor to repair the companion install/runtime gate.".to_owned(),
            ];
            return build_finding(
                "browser_surfaces",
                "Browser Surfaces",
                SecurityFindingStatus::Partial,
                SecurityFindingSeverity::Warn,
                summary,
                evidence,
                next_steps,
            );
        }
    }

    let summary =
        "The managed browser companion lane is active, but this command does not prove remote/browser auth equivalence beyond local runtime readiness."
            .to_owned();
    let next_steps = vec![
        "Keep browser companion deployment local-first unless you have separately reviewed its auth boundary."
            .to_owned(),
    ];
    build_finding(
        "browser_surfaces",
        "Browser Surfaces",
        SecurityFindingStatus::Unknown,
        SecurityFindingSeverity::Warn,
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

fn collect_env_pointer_diagnostics(
    config: &mvp::config::LoongClawConfig,
) -> Vec<mvp::config::ConfigValidationDiagnostic> {
    let diagnostics = config.validation_diagnostics();
    diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.code.starts_with("config.env_pointer."))
        .collect()
}

fn collect_secret_observations(config: &mvp::config::LoongClawConfig) -> Vec<SecretObservation> {
    let mut observations = Vec::new();

    collect_provider_secret_observations(config, &mut observations);
    collect_web_search_secret_observations(config, &mut observations);
    collect_channel_secret_observations(config, &mut observations);

    observations
}

fn collect_provider_secret_observations(
    config: &mvp::config::LoongClawConfig,
    observations: &mut Vec<SecretObservation>,
) {
    if config.providers.is_empty() {
        let api_key_path = "provider.api_key".to_owned();
        push_secret_ref_observation(observations, api_key_path, config.provider.api_key.as_ref());
        let oauth_path = "provider.oauth_access_token".to_owned();
        push_secret_ref_observation(
            observations,
            oauth_path,
            config.provider.oauth_access_token.as_ref(),
        );
        return;
    }

    for (profile_id, profile) in &config.providers {
        let api_key_path = format!("providers.{profile_id}.api_key");
        push_secret_ref_observation(
            observations,
            api_key_path,
            profile.provider.api_key.as_ref(),
        );
        let oauth_path = format!("providers.{profile_id}.oauth_access_token");
        push_secret_ref_observation(
            observations,
            oauth_path,
            profile.provider.oauth_access_token.as_ref(),
        );
    }
}

fn collect_web_search_secret_observations(
    config: &mvp::config::LoongClawConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let brave_path = "tools.web_search.brave_api_key".to_owned();
    push_string_secret_observation(
        observations,
        brave_path,
        config.tools.web_search.brave_api_key.as_deref(),
    );

    let tavily_path = "tools.web_search.tavily_api_key".to_owned();
    push_string_secret_observation(
        observations,
        tavily_path,
        config.tools.web_search.tavily_api_key.as_deref(),
    );

    let perplexity_path = "tools.web_search.perplexity_api_key".to_owned();
    push_string_secret_observation(
        observations,
        perplexity_path,
        config.tools.web_search.perplexity_api_key.as_deref(),
    );

    let exa_path = "tools.web_search.exa_api_key".to_owned();
    push_string_secret_observation(
        observations,
        exa_path,
        config.tools.web_search.exa_api_key.as_deref(),
    );

    let jina_path = "tools.web_search.jina_api_key".to_owned();
    push_string_secret_observation(
        observations,
        jina_path,
        config.tools.web_search.jina_api_key.as_deref(),
    );
}

fn collect_channel_secret_observations(
    config: &mvp::config::LoongClawConfig,
    observations: &mut Vec<SecretObservation>,
) {
    collect_telegram_secret_observations(&config.telegram, observations);
    collect_feishu_secret_observations(&config.feishu, observations);
    collect_matrix_secret_observations(&config.matrix, observations);
    collect_wecom_secret_observations(&config.wecom, observations);
    collect_discord_secret_observations(&config.discord, observations);
    collect_line_secret_observations(&config.line, observations);
    collect_dingtalk_secret_observations(&config.dingtalk, observations);
    collect_webhook_secret_observations(&config.webhook, observations);
    collect_email_secret_observations(&config.email, observations);
    collect_slack_secret_observations(&config.slack, observations);
    collect_google_chat_secret_observations(&config.google_chat, observations);
    collect_mattermost_secret_observations(&config.mattermost, observations);
    collect_nextcloud_talk_secret_observations(&config.nextcloud_talk, observations);
    collect_synology_chat_secret_observations(&config.synology_chat, observations);
    collect_teams_secret_observations(&config.teams, observations);
    collect_imessage_secret_observations(&config.imessage, observations);
    collect_whatsapp_secret_observations(&config.whatsapp, observations);
}

fn collect_telegram_secret_observations(
    config: &mvp::config::TelegramChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let bot_token_path = "telegram.bot_token".to_owned();
    push_secret_ref_observation(observations, bot_token_path, config.bot_token.as_ref());

    for (account_id, account) in &config.accounts {
        let bot_token_path = format!("telegram.accounts.{account_id}.bot_token");
        push_secret_ref_observation(observations, bot_token_path, account.bot_token.as_ref());
    }
}

fn collect_feishu_secret_observations(
    config: &mvp::config::FeishuChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let app_id_path = "feishu.app_id".to_owned();
    push_secret_ref_observation(observations, app_id_path, config.app_id.as_ref());
    let app_secret_path = "feishu.app_secret".to_owned();
    push_secret_ref_observation(observations, app_secret_path, config.app_secret.as_ref());
    let verification_path = "feishu.verification_token".to_owned();
    push_secret_ref_observation(
        observations,
        verification_path,
        config.verification_token.as_ref(),
    );
    let encrypt_key_path = "feishu.encrypt_key".to_owned();
    push_secret_ref_observation(observations, encrypt_key_path, config.encrypt_key.as_ref());

    for (account_id, account) in &config.accounts {
        let app_id_path = format!("feishu.accounts.{account_id}.app_id");
        push_secret_ref_observation(observations, app_id_path, account.app_id.as_ref());
        let app_secret_path = format!("feishu.accounts.{account_id}.app_secret");
        push_secret_ref_observation(observations, app_secret_path, account.app_secret.as_ref());
        let verification_path = format!("feishu.accounts.{account_id}.verification_token");
        push_secret_ref_observation(
            observations,
            verification_path,
            account.verification_token.as_ref(),
        );
        let encrypt_key_path = format!("feishu.accounts.{account_id}.encrypt_key");
        push_secret_ref_observation(observations, encrypt_key_path, account.encrypt_key.as_ref());
    }
}

fn collect_matrix_secret_observations(
    config: &mvp::config::MatrixChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let access_token_path = "matrix.access_token".to_owned();
    push_secret_ref_observation(
        observations,
        access_token_path,
        config.access_token.as_ref(),
    );

    for (account_id, account) in &config.accounts {
        let access_token_path = format!("matrix.accounts.{account_id}.access_token");
        push_secret_ref_observation(
            observations,
            access_token_path,
            account.access_token.as_ref(),
        );
    }
}

fn collect_wecom_secret_observations(
    config: &mvp::config::WecomChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let bot_id_path = "wecom.bot_id".to_owned();
    push_secret_ref_observation(observations, bot_id_path, config.bot_id.as_ref());
    let secret_path = "wecom.secret".to_owned();
    push_secret_ref_observation(observations, secret_path, config.secret.as_ref());

    for (account_id, account) in &config.accounts {
        let bot_id_path = format!("wecom.accounts.{account_id}.bot_id");
        push_secret_ref_observation(observations, bot_id_path, account.bot_id.as_ref());
        let secret_path = format!("wecom.accounts.{account_id}.secret");
        push_secret_ref_observation(observations, secret_path, account.secret.as_ref());
    }
}

fn collect_discord_secret_observations(
    config: &mvp::config::DiscordChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let bot_token_path = "discord.bot_token".to_owned();
    push_secret_ref_observation(observations, bot_token_path, config.bot_token.as_ref());

    for (account_id, account) in &config.accounts {
        let bot_token_path = format!("discord.accounts.{account_id}.bot_token");
        push_secret_ref_observation(observations, bot_token_path, account.bot_token.as_ref());
    }
}

fn collect_line_secret_observations(
    config: &mvp::config::LineChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let access_token_path = "line.channel_access_token".to_owned();
    push_secret_ref_observation(
        observations,
        access_token_path,
        config.channel_access_token.as_ref(),
    );
    let secret_path = "line.channel_secret".to_owned();
    push_secret_ref_observation(observations, secret_path, config.channel_secret.as_ref());

    for (account_id, account) in &config.accounts {
        let access_token_path = format!("line.accounts.{account_id}.channel_access_token");
        push_secret_ref_observation(
            observations,
            access_token_path,
            account.channel_access_token.as_ref(),
        );
        let secret_path = format!("line.accounts.{account_id}.channel_secret");
        push_secret_ref_observation(observations, secret_path, account.channel_secret.as_ref());
    }
}

fn collect_dingtalk_secret_observations(
    config: &mvp::config::DingtalkChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let webhook_path = "dingtalk.webhook_url".to_owned();
    push_secret_ref_observation(observations, webhook_path, config.webhook_url.as_ref());
    let secret_path = "dingtalk.secret".to_owned();
    push_secret_ref_observation(observations, secret_path, config.secret.as_ref());

    for (account_id, account) in &config.accounts {
        let webhook_path = format!("dingtalk.accounts.{account_id}.webhook_url");
        push_secret_ref_observation(observations, webhook_path, account.webhook_url.as_ref());
        let secret_path = format!("dingtalk.accounts.{account_id}.secret");
        push_secret_ref_observation(observations, secret_path, account.secret.as_ref());
    }
}

fn collect_webhook_secret_observations(
    config: &mvp::config::WebhookChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let endpoint_path = "webhook.endpoint_url".to_owned();
    push_secret_ref_observation(observations, endpoint_path, config.endpoint_url.as_ref());
    let auth_token_path = "webhook.auth_token".to_owned();
    push_secret_ref_observation(observations, auth_token_path, config.auth_token.as_ref());
    let signing_secret_path = "webhook.signing_secret".to_owned();
    push_secret_ref_observation(
        observations,
        signing_secret_path,
        config.signing_secret.as_ref(),
    );

    for (account_id, account) in &config.accounts {
        let endpoint_path = format!("webhook.accounts.{account_id}.endpoint_url");
        push_secret_ref_observation(observations, endpoint_path, account.endpoint_url.as_ref());
        let auth_token_path = format!("webhook.accounts.{account_id}.auth_token");
        push_secret_ref_observation(observations, auth_token_path, account.auth_token.as_ref());
        let signing_secret_path = format!("webhook.accounts.{account_id}.signing_secret");
        push_secret_ref_observation(
            observations,
            signing_secret_path,
            account.signing_secret.as_ref(),
        );
    }
}

fn collect_email_secret_observations(
    config: &mvp::config::EmailChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let smtp_username_path = "email.smtp_username".to_owned();
    push_secret_ref_observation(
        observations,
        smtp_username_path,
        config.smtp_username.as_ref(),
    );
    let smtp_password_path = "email.smtp_password".to_owned();
    push_secret_ref_observation(
        observations,
        smtp_password_path,
        config.smtp_password.as_ref(),
    );
    let imap_username_path = "email.imap_username".to_owned();
    push_secret_ref_observation(
        observations,
        imap_username_path,
        config.imap_username.as_ref(),
    );
    let imap_password_path = "email.imap_password".to_owned();
    push_secret_ref_observation(
        observations,
        imap_password_path,
        config.imap_password.as_ref(),
    );

    for (account_id, account) in &config.accounts {
        let smtp_username_path = format!("email.accounts.{account_id}.smtp_username");
        push_secret_ref_observation(
            observations,
            smtp_username_path,
            account.smtp_username.as_ref(),
        );
        let smtp_password_path = format!("email.accounts.{account_id}.smtp_password");
        push_secret_ref_observation(
            observations,
            smtp_password_path,
            account.smtp_password.as_ref(),
        );
        let imap_username_path = format!("email.accounts.{account_id}.imap_username");
        push_secret_ref_observation(
            observations,
            imap_username_path,
            account.imap_username.as_ref(),
        );
        let imap_password_path = format!("email.accounts.{account_id}.imap_password");
        push_secret_ref_observation(
            observations,
            imap_password_path,
            account.imap_password.as_ref(),
        );
    }
}

fn collect_slack_secret_observations(
    config: &mvp::config::SlackChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let bot_token_path = "slack.bot_token".to_owned();
    push_secret_ref_observation(observations, bot_token_path, config.bot_token.as_ref());

    for (account_id, account) in &config.accounts {
        let bot_token_path = format!("slack.accounts.{account_id}.bot_token");
        push_secret_ref_observation(observations, bot_token_path, account.bot_token.as_ref());
    }
}

fn collect_google_chat_secret_observations(
    config: &mvp::config::GoogleChatChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let webhook_path = "google_chat.webhook_url".to_owned();
    push_secret_ref_observation(observations, webhook_path, config.webhook_url.as_ref());

    for (account_id, account) in &config.accounts {
        let webhook_path = format!("google_chat.accounts.{account_id}.webhook_url");
        push_secret_ref_observation(observations, webhook_path, account.webhook_url.as_ref());
    }
}

fn collect_mattermost_secret_observations(
    config: &mvp::config::MattermostChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let bot_token_path = "mattermost.bot_token".to_owned();
    push_secret_ref_observation(observations, bot_token_path, config.bot_token.as_ref());

    for (account_id, account) in &config.accounts {
        let bot_token_path = format!("mattermost.accounts.{account_id}.bot_token");
        push_secret_ref_observation(observations, bot_token_path, account.bot_token.as_ref());
    }
}

fn collect_nextcloud_talk_secret_observations(
    config: &mvp::config::NextcloudTalkChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let shared_secret_path = "nextcloud_talk.shared_secret".to_owned();
    push_secret_ref_observation(
        observations,
        shared_secret_path,
        config.shared_secret.as_ref(),
    );

    for (account_id, account) in &config.accounts {
        let shared_secret_path = format!("nextcloud_talk.accounts.{account_id}.shared_secret");
        push_secret_ref_observation(
            observations,
            shared_secret_path,
            account.shared_secret.as_ref(),
        );
    }
}

fn collect_synology_chat_secret_observations(
    config: &mvp::config::SynologyChatChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let token_path = "synology_chat.token".to_owned();
    push_secret_ref_observation(observations, token_path, config.token.as_ref());
    let incoming_url_path = "synology_chat.incoming_url".to_owned();
    push_secret_ref_observation(
        observations,
        incoming_url_path,
        config.incoming_url.as_ref(),
    );

    for (account_id, account) in &config.accounts {
        let token_path = format!("synology_chat.accounts.{account_id}.token");
        push_secret_ref_observation(observations, token_path, account.token.as_ref());
        let incoming_url_path = format!("synology_chat.accounts.{account_id}.incoming_url");
        push_secret_ref_observation(
            observations,
            incoming_url_path,
            account.incoming_url.as_ref(),
        );
    }
}

fn collect_teams_secret_observations(
    config: &mvp::config::TeamsChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let webhook_path = "teams.webhook_url".to_owned();
    push_secret_ref_observation(observations, webhook_path, config.webhook_url.as_ref());
    let app_id_path = "teams.app_id".to_owned();
    push_secret_ref_observation(observations, app_id_path, config.app_id.as_ref());
    let app_password_path = "teams.app_password".to_owned();
    push_secret_ref_observation(
        observations,
        app_password_path,
        config.app_password.as_ref(),
    );

    for (account_id, account) in &config.accounts {
        let webhook_path = format!("teams.accounts.{account_id}.webhook_url");
        push_secret_ref_observation(observations, webhook_path, account.webhook_url.as_ref());
        let app_id_path = format!("teams.accounts.{account_id}.app_id");
        push_secret_ref_observation(observations, app_id_path, account.app_id.as_ref());
        let app_password_path = format!("teams.accounts.{account_id}.app_password");
        push_secret_ref_observation(
            observations,
            app_password_path,
            account.app_password.as_ref(),
        );
    }
}

fn collect_imessage_secret_observations(
    config: &mvp::config::ImessageChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let bridge_token_path = "imessage.bridge_token".to_owned();
    push_secret_ref_observation(
        observations,
        bridge_token_path,
        config.bridge_token.as_ref(),
    );

    for (account_id, account) in &config.accounts {
        let bridge_token_path = format!("imessage.accounts.{account_id}.bridge_token");
        push_secret_ref_observation(
            observations,
            bridge_token_path,
            account.bridge_token.as_ref(),
        );
    }
}

fn collect_whatsapp_secret_observations(
    config: &mvp::config::WhatsappChannelConfig,
    observations: &mut Vec<SecretObservation>,
) {
    let access_token_path = "whatsapp.access_token".to_owned();
    push_secret_ref_observation(
        observations,
        access_token_path,
        config.access_token.as_ref(),
    );
    let verify_token_path = "whatsapp.verify_token".to_owned();
    push_secret_ref_observation(
        observations,
        verify_token_path,
        config.verify_token.as_ref(),
    );
    let app_secret_path = "whatsapp.app_secret".to_owned();
    push_secret_ref_observation(observations, app_secret_path, config.app_secret.as_ref());

    for (account_id, account) in &config.accounts {
        let access_token_path = format!("whatsapp.accounts.{account_id}.access_token");
        push_secret_ref_observation(
            observations,
            access_token_path,
            account.access_token.as_ref(),
        );
        let verify_token_path = format!("whatsapp.accounts.{account_id}.verify_token");
        push_secret_ref_observation(
            observations,
            verify_token_path,
            account.verify_token.as_ref(),
        );
        let app_secret_path = format!("whatsapp.accounts.{account_id}.app_secret");
        push_secret_ref_observation(observations, app_secret_path, account.app_secret.as_ref());
    }
}

fn push_string_secret_observation(
    observations: &mut Vec<SecretObservation>,
    field_path: String,
    raw_value: Option<&str>,
) {
    let Some(raw_value) = raw_value else {
        return;
    };

    let trimmed_value = raw_value.trim();
    if trimmed_value.is_empty() {
        return;
    }

    let secret_ref = SecretRef::Inline(trimmed_value.to_owned());
    let kind = classify_secret_ref_kind(&secret_ref);
    let Some(kind) = kind else {
        return;
    };

    let observation = SecretObservation { field_path, kind };
    observations.push(observation);
}

fn push_secret_ref_observation(
    observations: &mut Vec<SecretObservation>,
    field_path: String,
    secret_ref: Option<&SecretRef>,
) {
    let Some(secret_ref) = secret_ref else {
        return;
    };

    let kind = classify_secret_ref_kind(secret_ref);
    let Some(kind) = kind else {
        return;
    };

    let observation = SecretObservation { field_path, kind };
    observations.push(observation);
}

fn classify_secret_ref_kind(secret_ref: &SecretRef) -> Option<SecretReferenceKind> {
    match secret_ref {
        SecretRef::Env { .. } => Some(SecretReferenceKind::Env),
        SecretRef::File { .. } => Some(SecretReferenceKind::File),
        SecretRef::Exec { .. } => Some(SecretReferenceKind::Exec),
        SecretRef::Inline(_) => {
            if secret_ref.inline_literal_value().is_some() {
                return Some(SecretReferenceKind::InlineLiteral);
            }

            if secret_ref.explicit_env_name().is_some() {
                return Some(SecretReferenceKind::Env);
            }

            None
        }
    }
}

fn summarize_secret_observations(observations: &[SecretObservation]) -> SecretObservationCounts {
    let mut counts = SecretObservationCounts::default();

    for observation in observations {
        counts.record(observation.kind);
    }

    counts
}

fn observation_paths_for_kind(
    observations: &[SecretObservation],
    kind: SecretReferenceKind,
) -> Vec<String> {
    observations
        .iter()
        .filter(|observation| observation.kind == kind)
        .map(|observation| observation.field_path.clone())
        .collect()
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

    use crate::test_support::ScopedEnv;
    use loongclaw_contracts::SecretRef;

    fn temp_config_path(label: &str) -> PathBuf {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let file_name = format!("loongclaw-doctor-security-{label}-{epoch}.toml");
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

    #[tokio::test]
    async fn default_shell_execution_is_covered_when_allowlist_is_empty() {
        let path = temp_config_path("shell-covered");
        write_placeholder_config(&path);

        let config = mvp::config::LoongClawConfig::default();
        let execution = build_doctor_security_execution(&path, &config)
            .await
            .expect("build security execution");
        let finding = finding_by_id(&execution.findings, "shell_execution");

        assert_eq!(finding.status, SecurityFindingStatus::Covered);
        assert_eq!(finding.severity, SecurityFindingSeverity::Info);
        assert!(
            finding.summary.contains("effectively disabled"),
            "unexpected summary: {}",
            finding.summary
        );
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

        let mut config = mvp::config::LoongClawConfig::default();
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
    async fn external_skills_expose_when_auto_expose_or_approval_is_open() {
        let path = temp_config_path("external-skills");
        write_placeholder_config(&path);

        let mut config = mvp::config::LoongClawConfig::default();
        config.external_skills.enabled = true;
        config.external_skills.require_download_approval = false;
        config.external_skills.auto_expose_installed = true;

        let execution = build_doctor_security_execution(&path, &config)
            .await
            .expect("build security execution");
        let finding = finding_by_id(&execution.findings, "external_skills");

        assert_eq!(finding.status, SecurityFindingStatus::Exposed);
        assert_eq!(finding.severity, SecurityFindingSeverity::Critical);
    }

    #[tokio::test]
    async fn browser_surfaces_become_unknown_when_companion_is_ready() {
        let path = temp_config_path("browser-companion");
        write_placeholder_config(&path);

        let temp_dir = std::env::temp_dir();
        let command_path = temp_dir.join("loongclaw-browser-companion-security-test.sh");
        let script = "#!/bin/sh\necho 1.2.3\n";
        fs::write(&command_path, script).expect("write browser companion script");

        #[cfg(unix)]
        {
            let metadata = fs::metadata(&command_path).expect("script metadata");
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&command_path, permissions).expect("chmod script");
        }

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(command_path.display().to_string());
        config.tools.browser_companion.expected_version = Some("1.2.3".to_owned());

        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_BROWSER_COMPANION_READY", "true");

        let execution = build_doctor_security_execution(&path, &config)
            .await
            .expect("build security execution");
        let finding = finding_by_id(&execution.findings, "browser_surfaces");

        let _ = fs::remove_file(&command_path);

        assert_eq!(finding.status, SecurityFindingStatus::Unknown);
        assert_eq!(finding.severity, SecurityFindingSeverity::Warn);
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
}
