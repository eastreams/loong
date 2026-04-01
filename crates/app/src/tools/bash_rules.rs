use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

use starlark::environment::{Globals, GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrefixRuleDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledPrefixRule {
    pub source: String,
    pub prefix: Vec<String>,
    pub decision: PrefixRuleDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixRuleSpec {
    pub source: String,
    pub pattern: Vec<String>,
    pub decision: PrefixRuleDecision,
}

#[derive(Debug, Default)]
struct RuleFileCollection {
    source_name: String,
    specs: Vec<PrefixRuleSpec>,
}

thread_local! {
    static RULE_FILE_COLLECTION: RefCell<Option<RuleFileCollection>> = const { RefCell::new(None) };
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn compile_compatibility_rules<I, S>(
    source: &str,
    decision: PrefixRuleDecision,
    commands: I,
) -> Vec<CompiledPrefixRule>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    compile_rules(commands.into_iter().filter_map(|command| {
        let normalized = command.as_ref().trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return None;
        }

        Some(PrefixRuleSpec {
            source: format!("{source}:{normalized}"),
            pattern: vec![normalized],
            decision,
        })
    }))
}

pub fn load_rules_from_dir(rules_dir: &Path) -> Result<Vec<CompiledPrefixRule>, String> {
    match rules_dir.try_exists() {
        Ok(false) => return Ok(Vec::new()),
        Ok(true) => {}
        Err(error) => {
            return Err(format!(
                "failed to inspect bash rules dir `{}`: {error}",
                rules_dir.display()
            ));
        }
    }

    if !rules_dir.is_dir() {
        return Err(format!(
            "bash rules path `{}` is not a directory",
            rules_dir.display()
        ));
    }

    let mut rule_files = Vec::new();
    let entries = fs::read_dir(rules_dir).map_err(|error| {
        format!(
            "failed to read bash rules dir `{}`: {error}",
            rules_dir.display()
        )
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to enumerate bash rules dir `{}`: {error}",
                rules_dir.display()
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "failed to inspect bash rule entry `{}`: {error}",
                entry.path().display()
            )
        })?;
        let path = entry.path();
        if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "rules")
        {
            rule_files.push(path);
        }
    }

    rule_files.sort_by_key(|path| path.file_name().map(|name| name.to_os_string()));

    let mut specs = Vec::new();
    for rule_file in rule_files {
        let content = fs::read_to_string(&rule_file).map_err(|error| {
            format!(
                "failed to read bash rule file `{}`: {error}",
                rule_file.display()
            )
        })?;
        specs.extend(load_rule_specs_from_file(&rule_file, content)?);
    }

    Ok(compile_rules(specs))
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn merge_rule_sources<I, J>(left: I, right: J) -> Vec<CompiledPrefixRule>
where
    I: IntoIterator<Item = CompiledPrefixRule>,
    J: IntoIterator<Item = CompiledPrefixRule>,
{
    normalize_compiled_rules(left.into_iter().chain(right))
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn evaluate_prefix_rules<I, S>(
    rules: &[CompiledPrefixRule],
    command: I,
) -> Option<PrefixRuleDecision>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let command: Vec<String> = command
        .into_iter()
        .map(|token| token.as_ref().trim().to_owned())
        .collect();
    let lowered_command: Vec<String> = command
        .iter()
        .map(|token| token.to_ascii_lowercase())
        .collect();

    if rules.iter().any(|rule| {
        rule.decision == PrefixRuleDecision::Deny
            && prefix_matches(
                command_tokens_for_rule(rule, &command, &lowered_command),
                &rule.prefix,
            )
    }) {
        return Some(PrefixRuleDecision::Deny);
    }

    rules
        .iter()
        .any(|rule| {
            rule.decision == PrefixRuleDecision::Allow
                && prefix_matches(
                    command_tokens_for_rule(rule, &command, &lowered_command),
                    &rule.prefix,
                )
        })
        .then_some(PrefixRuleDecision::Allow)
}

fn command_tokens_for_rule<'a>(
    rule: &CompiledPrefixRule,
    command: &'a [String],
    lowered_command: &'a [String],
) -> &'a [String] {
    if is_legacy_shell_rule(rule) {
        lowered_command
    } else {
        command
    }
}

fn is_legacy_shell_rule(rule: &CompiledPrefixRule) -> bool {
    rule.source.starts_with("shell_allow:") || rule.source.starts_with("shell_deny:")
}

fn compile_rules<I>(specs: I) -> Vec<CompiledPrefixRule>
where
    I: IntoIterator<Item = PrefixRuleSpec>,
{
    normalize_compiled_rules(specs.into_iter().map(|spec| CompiledPrefixRule {
        source: spec.source,
        prefix: spec.pattern,
        decision: spec.decision,
    }))
}

fn normalize_compiled_rules<I>(rules: I) -> Vec<CompiledPrefixRule>
where
    I: IntoIterator<Item = CompiledPrefixRule>,
{
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();

    for rule in rules {
        if seen.insert((rule.decision, rule.prefix.clone())) {
            normalized.push(rule);
        }
    }

    normalized
}

fn load_rule_specs_from_file(path: &Path, content: String) -> Result<Vec<PrefixRuleSpec>, String> {
    let ast = AstModule::parse(&path.display().to_string(), content, &rule_file_dialect())
        .map_err(|error| {
            format!(
                "failed to parse bash rule file `{}`: {error:#}",
                path.display()
            )
        })?;

    let module = Module::new();
    let globals = rule_file_globals();
    let mut evaluator = Evaluator::new(&module);

    RULE_FILE_COLLECTION.with(|collection| {
        let mut slot = collection.borrow_mut();
        if slot.is_some() {
            return Err(format!(
                "internal error: attempted to re-enter bash rule collection for `{}`",
                path.display()
            ));
        }
        *slot = Some(RuleFileCollection {
            source_name: path.display().to_string(),
            specs: Vec::new(),
        });
        drop(slot);

        let eval_result = evaluator.eval_module(ast, &globals);
        let specs = collection
            .borrow_mut()
            .take()
            .map(|collected| collected.specs)
            .unwrap_or_default();

        eval_result.map(|_| specs).map_err(|error| {
            format!(
                "failed to evaluate bash rule file `{}`: {error:#}",
                path.display()
            )
        })
    })
}

fn rule_file_globals() -> Globals {
    GlobalsBuilder::new().with(register_rule_globals).build()
}

fn rule_file_dialect() -> Dialect {
    Dialect {
        enable_def: false,
        enable_lambda: false,
        enable_load: false,
        enable_top_level_stmt: true,
        ..Dialect::Standard
    }
}

fn parse_prefix_rule_decision(raw: &str) -> Result<PrefixRuleDecision, String> {
    match raw {
        "allow" => Ok(PrefixRuleDecision::Allow),
        "deny" => Ok(PrefixRuleDecision::Deny),
        _ => Err(format!(
            "prefix_rule decision must be \"allow\" or \"deny\", got `{raw}`"
        )),
    }
}

fn normalize_rule_pattern<I, S>(pattern: I) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let normalized: Vec<String> = pattern
        .into_iter()
        .map(|token| token.as_ref().trim().to_owned())
        .collect();

    if normalized.is_empty() {
        return Err("prefix_rule pattern must be a non-empty string list".to_owned());
    }
    if normalized.iter().any(|token| token.is_empty()) {
        return Err("prefix_rule pattern entries must be non-empty strings".to_owned());
    }

    Ok(normalized)
}

fn record_loaded_rule(pattern: Vec<String>, decision: PrefixRuleDecision) -> starlark::Result<()> {
    RULE_FILE_COLLECTION.with(|collection| {
        let mut slot = collection.borrow_mut();
        let collected = slot.as_mut().ok_or_else(|| {
            starlark_error("prefix_rule called outside of bash rule file evaluation")
        })?;
        let ordinal = collected.specs.len() + 1;
        collected.specs.push(PrefixRuleSpec {
            source: format!("{}#{ordinal}", collected.source_name),
            pattern,
            decision,
        });
        Ok(())
    })
}

fn starlark_error(message: impl Into<String>) -> starlark::Error {
    starlark::Error::new_other(io::Error::other(message.into()))
}

#[cfg_attr(not(test), allow(dead_code))]
fn prefix_matches(command: &[String], prefix: &[String]) -> bool {
    command.len() >= prefix.len()
        && command
            .iter()
            .zip(prefix.iter())
            .all(|(command_token, prefix_token)| command_token == prefix_token)
}

#[starlark_module]
fn register_rule_globals(builder: &mut GlobalsBuilder) {
    fn prefix_rule(
        #[starlark(require = named)] pattern: UnpackList<String>,
        #[starlark(require = named)] decision: String,
    ) -> starlark::Result<NoneType> {
        let pattern = normalize_rule_pattern(pattern).map_err(starlark_error)?;
        let decision = parse_prefix_rule_decision(&decision).map_err(starlark_error)?;
        record_loaded_rule(pattern, decision)?;
        Ok(NoneType)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_allow_entries_translate_to_single_token_allow_prefix_rules() {
        let rules =
            compile_compatibility_rules("shell_allow", PrefixRuleDecision::Allow, ["cargo", "git"]);

        assert_eq!(
            rules,
            vec![
                CompiledPrefixRule {
                    source: "shell_allow:cargo".to_owned(),
                    prefix: vec!["cargo".to_owned()],
                    decision: PrefixRuleDecision::Allow,
                },
                CompiledPrefixRule {
                    source: "shell_allow:git".to_owned(),
                    prefix: vec!["git".to_owned()],
                    decision: PrefixRuleDecision::Allow,
                },
            ]
        );
    }

    #[test]
    fn shell_deny_entries_translate_to_single_token_deny_prefix_rules() {
        let rules =
            compile_compatibility_rules("shell_deny", PrefixRuleDecision::Deny, ["cargo", "git"]);

        assert_eq!(
            rules,
            vec![
                CompiledPrefixRule {
                    source: "shell_deny:cargo".to_owned(),
                    prefix: vec!["cargo".to_owned()],
                    decision: PrefixRuleDecision::Deny,
                },
                CompiledPrefixRule {
                    source: "shell_deny:git".to_owned(),
                    prefix: vec!["git".to_owned()],
                    decision: PrefixRuleDecision::Deny,
                },
            ]
        );
    }

    #[test]
    fn compatibility_rules_match_mixed_case_command_tokens() {
        let rules =
            compile_compatibility_rules("shell_allow", PrefixRuleDecision::Allow, ["Cargo"]);

        assert_eq!(
            evaluate_prefix_rules(&rules, ["Cargo", "publish"]),
            Some(PrefixRuleDecision::Allow)
        );
    }

    #[test]
    fn starlark_rules_keep_case_sensitive_matching() {
        let rules = compile_rules([PrefixRuleSpec {
            source: "rules:custom".to_owned(),
            pattern: vec!["Cargo".to_owned()],
            decision: PrefixRuleDecision::Allow,
        }]);

        assert_eq!(
            evaluate_prefix_rules(&rules, ["Cargo", "publish"]),
            Some(PrefixRuleDecision::Allow)
        );
        assert_eq!(evaluate_prefix_rules(&rules, ["cargo", "publish"]), None);
    }

    #[test]
    fn rules_dir_loads_rule_files_in_stable_lexical_order() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let rules_dir = tempdir.path().join(".loongclaw").join("rules");
        fs::create_dir_all(&rules_dir).expect("create rules dir");
        fs::write(
            rules_dir.join("10-second.rules"),
            "prefix_rule(pattern=[\"cargo\", \"test\"], decision=\"allow\")\n",
        )
        .expect("write second rule file");
        fs::write(
            rules_dir.join("01-first.rules"),
            "prefix_rule(pattern=[\"cargo\", \"publish\"], decision=\"deny\")\n",
        )
        .expect("write first rule file");

        let rules = load_rules_from_dir(&rules_dir).expect("load rules");

        assert_eq!(
            rules
                .iter()
                .map(|rule| rule.prefix.clone())
                .collect::<Vec<_>>(),
            vec![
                vec!["cargo".to_owned(), "publish".to_owned()],
                vec!["cargo".to_owned(), "test".to_owned()],
            ]
        );
    }

    #[test]
    fn rules_dir_ignores_non_rules_files() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let rules_dir = tempdir.path().join(".loongclaw").join("rules");
        fs::create_dir_all(&rules_dir).expect("create rules dir");
        fs::write(
            rules_dir.join("01-first.rules"),
            "prefix_rule(pattern=[\"cargo\", \"publish\"], decision=\"deny\")\n",
        )
        .expect("write first rule file");
        fs::write(
            rules_dir.join("README.txt"),
            "prefix_rule(pattern=[\"ignored\"], decision=\"allow\")\n",
        )
        .expect("write non-rules file");
        fs::write(
            rules_dir.join("10-second.rules"),
            "prefix_rule(pattern=[\"cargo\", \"test\"], decision=\"allow\")\n",
        )
        .expect("write second rule file");

        let rules = load_rules_from_dir(&rules_dir).expect("load rules");

        assert_eq!(
            rules
                .iter()
                .map(|rule| rule.prefix.clone())
                .collect::<Vec<_>>(),
            vec![
                vec!["cargo".to_owned(), "publish".to_owned()],
                vec!["cargo".to_owned(), "test".to_owned()],
            ]
        );
    }

    #[test]
    fn same_decision_duplicate_rules_can_be_normalized_away() {
        let compiled = compile_rules([
            PrefixRuleSpec {
                source: "first".to_owned(),
                pattern: vec!["cargo".to_owned(), "test".to_owned()],
                decision: PrefixRuleDecision::Allow,
            },
            PrefixRuleSpec {
                source: "second".to_owned(),
                pattern: vec!["cargo".to_owned(), "test".to_owned()],
                decision: PrefixRuleDecision::Allow,
            },
        ]);

        assert_eq!(compiled.len(), 1);
    }

    #[test]
    fn deny_precedence_holds_even_when_allow_and_deny_come_from_different_sources() {
        let merged = merge_rule_sources(
            compile_rules([PrefixRuleSpec {
                source: "explicit".to_owned(),
                pattern: vec!["cargo".to_owned(), "publish".to_owned()],
                decision: PrefixRuleDecision::Allow,
            }]),
            compile_compatibility_rules("shell_deny", PrefixRuleDecision::Deny, ["cargo"]),
        );

        assert_eq!(
            evaluate_prefix_rules(&merged, ["cargo", "publish"]),
            Some(PrefixRuleDecision::Deny)
        );
    }
}
