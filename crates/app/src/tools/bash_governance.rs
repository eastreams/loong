use super::bash_ast::{
    BashCommandAnalysis, MinimalCommandUnit, UnitClassification, UnitOperator,
    UnsupportedStructureKind, analyze_bash_command,
};
use super::bash_rules::{CompiledPrefixRule, PrefixRuleDecision};
use super::shell_policy_ext::ShellPolicyDefault;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UnitDecisionSource {
    ExplicitAllow { rule_source: String },
    ExplicitDeny { rule_source: String },
    DefaultUnmatchedPrefix,
    DefaultUnsupportedStructure(UnsupportedStructureKind),
    DefaultParseUnreliable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnitGovernanceDecision {
    Allow,
    Deny,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnitGovernanceOutcome {
    pub preceding_operator: Option<UnitOperator>,
    pub classification: UnitClassification,
    pub decision: UnitGovernanceDecision,
    pub decision_source: UnitDecisionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinalGovernanceDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FinalDecisionSource {
    AllUnitsExplicitlyAllowed,
    ParseUnreliableDefault,
    DefaultFallbackNoUnits,
    Unit {
        index: usize,
        source: UnitDecisionSource,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BashGovernanceOutcome {
    pub command: String,
    pub parse_unreliable: bool,
    pub unit_outcomes: Vec<UnitGovernanceOutcome>,
    pub default_mode: ShellPolicyDefault,
    pub final_decision: FinalGovernanceDecision,
    pub final_source: FinalDecisionSource,
}

impl BashGovernanceOutcome {
    pub(crate) fn denial_reason(&self) -> Option<String> {
        if self.final_decision != FinalGovernanceDecision::Deny {
            return None;
        }

        Some(match &self.final_source {
            FinalDecisionSource::AllUnitsExplicitlyAllowed => {
                "bash governance denied command unexpectedly".to_owned()
            }
            FinalDecisionSource::ParseUnreliableDefault => {
                format!(
                    "bash command could not be parsed reliably and default-{} policy denied it",
                    default_mode_label(self.default_mode)
                )
            }
            FinalDecisionSource::DefaultFallbackNoUnits => format!(
                "bash command produced no governable units and default-{} policy denied it",
                default_mode_label(self.default_mode)
            ),
            FinalDecisionSource::Unit { index, source } => {
                let unit_display = self
                    .unit_outcomes
                    .get(*index)
                    .map(unit_display)
                    .unwrap_or_else(|| "unknown unit".to_owned());
                match source {
                    UnitDecisionSource::ExplicitDeny { rule_source } => format!(
                        "bash command unit `{unit_display}` matched deny rule `{rule_source}`"
                    ),
                    UnitDecisionSource::ExplicitAllow { rule_source } => format!(
                        "bash command unit `{unit_display}` matched allow rule `{rule_source}` but overall evaluation still denied"
                    ),
                    UnitDecisionSource::DefaultUnmatchedPrefix => format!(
                        "bash command unit `{unit_display}` matched no allow rule and default-{} policy denied it",
                        default_mode_label(self.default_mode)
                    ),
                    UnitDecisionSource::DefaultUnsupportedStructure(kind) => format!(
                        "bash command unit `{unit_display}` uses unsupported {} structure and default-{} policy denied it",
                        unsupported_structure_label(*kind),
                        default_mode_label(self.default_mode)
                    ),
                    UnitDecisionSource::DefaultParseUnreliable => format!(
                        "bash command could not be parsed reliably and default-{} policy denied it",
                        default_mode_label(self.default_mode)
                    ),
                }
            }
        })
    }
}

pub(crate) fn evaluate_bash_command(
    command: &str,
    rules: &[CompiledPrefixRule],
    default_mode: ShellPolicyDefault,
) -> BashGovernanceOutcome {
    let analysis = analyze_bash_command(command);
    evaluate_bash_analysis(command, &analysis, rules, default_mode)
}

fn evaluate_bash_analysis(
    command: &str,
    analysis: &BashCommandAnalysis,
    rules: &[CompiledPrefixRule],
    default_mode: ShellPolicyDefault,
) -> BashGovernanceOutcome {
    if analysis.parse_unreliable {
        let unit_outcomes = analysis
            .units
            .iter()
            .map(parse_unreliable_unit_outcome)
            .collect::<Vec<_>>();
        return BashGovernanceOutcome {
            command: command.to_owned(),
            parse_unreliable: true,
            unit_outcomes,
            default_mode,
            final_decision: final_default_decision(default_mode),
            final_source: FinalDecisionSource::ParseUnreliableDefault,
        };
    }

    let unit_outcomes = analysis
        .units
        .iter()
        .map(|unit| evaluate_unit(unit, rules))
        .collect::<Vec<_>>();

    if let Some((index, source)) = unit_outcomes
        .iter()
        .enumerate()
        .find(|(_, outcome)| outcome.decision == UnitGovernanceDecision::Deny)
        .map(|(index, outcome)| (index, outcome.decision_source.clone()))
    {
        return BashGovernanceOutcome {
            command: command.to_owned(),
            parse_unreliable: false,
            unit_outcomes,
            default_mode,
            final_decision: FinalGovernanceDecision::Deny,
            final_source: FinalDecisionSource::Unit { index, source },
        };
    }

    if !unit_outcomes.is_empty()
        && unit_outcomes
            .iter()
            .all(|outcome| outcome.decision == UnitGovernanceDecision::Allow)
    {
        return BashGovernanceOutcome {
            command: command.to_owned(),
            parse_unreliable: false,
            unit_outcomes,
            default_mode,
            final_decision: FinalGovernanceDecision::Allow,
            final_source: FinalDecisionSource::AllUnitsExplicitlyAllowed,
        };
    }

    if let Some((index, source)) = unit_outcomes
        .iter()
        .enumerate()
        .find(|(_, outcome)| outcome.decision == UnitGovernanceDecision::Default)
        .map(|(index, outcome)| (index, outcome.decision_source.clone()))
    {
        return BashGovernanceOutcome {
            command: command.to_owned(),
            parse_unreliable: false,
            unit_outcomes,
            default_mode,
            final_decision: final_default_decision(default_mode),
            final_source: FinalDecisionSource::Unit { index, source },
        };
    }

    BashGovernanceOutcome {
        command: command.to_owned(),
        parse_unreliable: false,
        unit_outcomes,
        default_mode,
        final_decision: final_default_decision(default_mode),
        final_source: FinalDecisionSource::DefaultFallbackNoUnits,
    }
}

fn parse_unreliable_unit_outcome(unit: &MinimalCommandUnit) -> UnitGovernanceOutcome {
    UnitGovernanceOutcome {
        preceding_operator: unit.preceding_operator,
        classification: unit.classification.clone(),
        decision: UnitGovernanceDecision::Default,
        decision_source: UnitDecisionSource::DefaultParseUnreliable,
    }
}

fn evaluate_unit(unit: &MinimalCommandUnit, rules: &[CompiledPrefixRule]) -> UnitGovernanceOutcome {
    let (decision, decision_source) = match &unit.classification {
        UnitClassification::GovernablePlainCommand { argv } => {
            match matching_rule_source(rules, argv, PrefixRuleDecision::Deny) {
                Some(rule_source) => (
                    UnitGovernanceDecision::Deny,
                    UnitDecisionSource::ExplicitDeny { rule_source },
                ),
                None => match matching_rule_source(rules, argv, PrefixRuleDecision::Allow) {
                    Some(rule_source) => (
                        UnitGovernanceDecision::Allow,
                        UnitDecisionSource::ExplicitAllow { rule_source },
                    ),
                    None => (
                        UnitGovernanceDecision::Default,
                        UnitDecisionSource::DefaultUnmatchedPrefix,
                    ),
                },
            }
        }
        UnitClassification::Unsupported(kind) => (
            UnitGovernanceDecision::Default,
            UnitDecisionSource::DefaultUnsupportedStructure(*kind),
        ),
    };

    UnitGovernanceOutcome {
        preceding_operator: unit.preceding_operator,
        classification: unit.classification.clone(),
        decision,
        decision_source,
    }
}

fn matching_rule_source(
    rules: &[CompiledPrefixRule],
    argv: &[String],
    decision: PrefixRuleDecision,
) -> Option<String> {
    rules
        .iter()
        .find(|rule| rule.decision == decision && prefix_matches(argv, &rule.prefix))
        .map(|rule| rule.source.clone())
}

fn prefix_matches(command: &[String], prefix: &[String]) -> bool {
    command.len() >= prefix.len()
        && command
            .iter()
            .zip(prefix.iter())
            .all(|(command_token, prefix_token)| command_token == prefix_token)
}

fn final_default_decision(default_mode: ShellPolicyDefault) -> FinalGovernanceDecision {
    match default_mode {
        ShellPolicyDefault::Allow => FinalGovernanceDecision::Allow,
        ShellPolicyDefault::Deny => FinalGovernanceDecision::Deny,
    }
}

fn unit_display(unit: &UnitGovernanceOutcome) -> String {
    match &unit.classification {
        UnitClassification::GovernablePlainCommand { argv } => argv.join(" "),
        UnitClassification::Unsupported(kind) => {
            format!("unsupported {}", unsupported_structure_label(*kind))
        }
    }
}

fn unsupported_structure_label(kind: UnsupportedStructureKind) -> &'static str {
    match kind {
        UnsupportedStructureKind::BackgroundOperator => "background operator",
        UnsupportedStructureKind::CommandSubstitution => "command substitution",
        UnsupportedStructureKind::CompoundCommand => "compound command",
        UnsupportedStructureKind::EnvPrefixAssignment => "environment-prefix assignment",
        UnsupportedStructureKind::Pipeline => "pipeline",
        UnsupportedStructureKind::ProcessSubstitution => "process substitution",
        UnsupportedStructureKind::Redirection => "redirection",
        UnsupportedStructureKind::Subshell => "subshell",
    }
}

fn default_mode_label(default_mode: ShellPolicyDefault) -> &'static str {
    match default_mode {
        ShellPolicyDefault::Allow => "allow",
        ShellPolicyDefault::Deny => "deny",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PrefixRuleFixture;

    impl PrefixRuleFixture {
        fn allow<const N: usize>(pattern: [&str; N]) -> CompiledPrefixRule {
            Self::rule(pattern, PrefixRuleDecision::Allow)
        }

        fn deny<const N: usize>(pattern: [&str; N]) -> CompiledPrefixRule {
            Self::rule(pattern, PrefixRuleDecision::Deny)
        }

        fn none() -> Vec<CompiledPrefixRule> {
            Vec::new()
        }

        fn rules<const N: usize>(rules: [CompiledPrefixRule; N]) -> Vec<CompiledPrefixRule> {
            rules.into_iter().collect()
        }

        fn rule<const N: usize>(
            pattern: [&str; N],
            decision: PrefixRuleDecision,
        ) -> CompiledPrefixRule {
            CompiledPrefixRule {
                source: format!(
                    "test:{}:{}",
                    match decision {
                        PrefixRuleDecision::Allow => "allow",
                        PrefixRuleDecision::Deny => "deny",
                    },
                    pattern.join(" ")
                ),
                prefix: pattern.into_iter().map(str::to_owned).collect(),
                decision,
            }
        }
    }

    fn evaluate_bash_governance_for_test(
        command: &str,
        rules: Vec<CompiledPrefixRule>,
        default_mode: ShellPolicyDefault,
    ) -> BashGovernanceOutcome {
        evaluate_bash_command(command, &rules, default_mode)
    }

    #[test]
    fn whole_command_allows_when_plain_command_matches_allow_prefix_rule() {
        let outcome = evaluate_bash_governance_for_test(
            "printf ok",
            PrefixRuleFixture::rules([PrefixRuleFixture::allow(["printf", "ok"])]),
            ShellPolicyDefault::Deny,
        );

        assert_eq!(outcome.final_decision, FinalGovernanceDecision::Allow);
    }

    #[test]
    fn whole_command_allows_when_every_unit_is_explicitly_allowed() {
        let outcome = evaluate_bash_governance_for_test(
            "printf ok && printf ready",
            PrefixRuleFixture::rules([
                PrefixRuleFixture::allow(["printf", "ok"]),
                PrefixRuleFixture::allow(["printf", "ready"]),
            ]),
            ShellPolicyDefault::Deny,
        );

        assert_eq!(outcome.final_decision, FinalGovernanceDecision::Allow);
        assert_eq!(
            outcome.final_source,
            FinalDecisionSource::AllUnitsExplicitlyAllowed
        );
        assert_eq!(outcome.unit_outcomes.len(), 2);
        assert_eq!(
            outcome.unit_outcomes[0].decision,
            UnitGovernanceDecision::Allow
        );
        assert_eq!(
            outcome.unit_outcomes[1].decision,
            UnitGovernanceDecision::Allow
        );
    }

    #[test]
    fn unmatched_plain_command_uses_default_mode() {
        let outcome = evaluate_bash_governance_for_test(
            "printf ok",
            PrefixRuleFixture::none(),
            ShellPolicyDefault::Deny,
        );

        assert_eq!(outcome.final_decision, FinalGovernanceDecision::Deny);
        assert_eq!(
            outcome.unit_outcomes[0].decision_source,
            UnitDecisionSource::DefaultUnmatchedPrefix
        );
    }

    #[test]
    fn whole_command_denies_when_any_unit_denies() {
        let outcome = evaluate_bash_governance_for_test(
            "cargo publish && cargo test",
            PrefixRuleFixture::rules([PrefixRuleFixture::deny(["cargo", "publish"])]),
            ShellPolicyDefault::Allow,
        );

        assert_eq!(outcome.final_decision, FinalGovernanceDecision::Deny);
        assert_eq!(
            outcome.unit_outcomes[0].decision_source,
            UnitDecisionSource::ExplicitDeny {
                rule_source: "test:deny:cargo publish".to_owned(),
            }
        );
    }

    #[test]
    fn deny_rule_matches_escaped_static_command_name_under_default_allow() {
        let outcome = evaluate_bash_governance_for_test(
            r"r\m --version",
            PrefixRuleFixture::rules([PrefixRuleFixture::deny(["rm"])]),
            ShellPolicyDefault::Allow,
        );

        assert_eq!(outcome.final_decision, FinalGovernanceDecision::Deny);
        assert_eq!(
            outcome.unit_outcomes[0].decision_source,
            UnitDecisionSource::ExplicitDeny {
                rule_source: "test:deny:rm".to_owned(),
            }
        );
    }

    #[test]
    fn mixed_allow_and_default_resolves_through_default_mode() {
        let outcome = evaluate_bash_governance_for_test(
            "git status && cargo test | tee out.txt",
            PrefixRuleFixture::rules([PrefixRuleFixture::allow(["git", "status"])]),
            ShellPolicyDefault::Deny,
        );

        assert_eq!(outcome.final_decision, FinalGovernanceDecision::Deny);
        assert_eq!(
            outcome.unit_outcomes[1].decision_source,
            UnitDecisionSource::DefaultUnsupportedStructure(UnsupportedStructureKind::Pipeline)
        );
    }

    #[test]
    fn or_list_denies_when_rhs_branch_matches_deny_rule() {
        let outcome = evaluate_bash_governance_for_test(
            "printf ok || printf blocked",
            PrefixRuleFixture::rules([
                PrefixRuleFixture::allow(["printf", "ok"]),
                PrefixRuleFixture::deny(["printf", "blocked"]),
            ]),
            ShellPolicyDefault::Allow,
        );

        assert_eq!(outcome.final_decision, FinalGovernanceDecision::Deny);
        assert_eq!(outcome.unit_outcomes.len(), 2);
        assert_eq!(
            outcome.unit_outcomes[1].preceding_operator,
            Some(UnitOperator::OrIf)
        );
    }

    #[test]
    fn parse_unreliable_outcome_uses_default_allow_when_configured() {
        let outcome = evaluate_bash_governance_for_test(
            "if then",
            PrefixRuleFixture::none(),
            ShellPolicyDefault::Allow,
        );

        assert_eq!(outcome.final_decision, FinalGovernanceDecision::Allow);
        assert!(outcome.parse_unreliable);
    }

    #[test]
    fn no_unit_default_deny_does_not_claim_parse_unreliability() {
        let analysis = BashCommandAnalysis {
            parse_unreliable: false,
            units: Vec::new(),
        };

        let outcome = evaluate_bash_analysis(
            " \n ",
            &analysis,
            &PrefixRuleFixture::none(),
            ShellPolicyDefault::Deny,
        );

        assert!(!outcome.parse_unreliable);
        assert_eq!(
            outcome.final_source,
            FinalDecisionSource::DefaultFallbackNoUnits
        );
        assert_eq!(
            outcome.denial_reason().as_deref(),
            Some("bash command produced no governable units and default-deny policy denied it")
        );
    }
}
