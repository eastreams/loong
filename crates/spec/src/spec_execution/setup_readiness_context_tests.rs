use std::collections::BTreeSet;

use super::*;
use crate::PluginSetupReadinessSpec;

#[test]
fn collect_verified_env_var_names_ignores_blank_names_and_values() {
    let env_vars = vec![
        (OsString::from("TAVILY_API_KEY"), OsString::from("secret")),
        (OsString::from("EMPTY_VALUE"), OsString::from("   ")),
        (OsString::from("   "), OsString::from("ignored")),
    ];

    let verified_env_vars = collect_verified_env_var_names(env_vars);

    assert_eq!(
        verified_env_vars,
        BTreeSet::from(["TAVILY_API_KEY".to_owned()])
    );
}

#[test]
fn collect_verified_env_var_names_preserves_non_blank_name_spelling() {
    let env_vars = vec![
        (OsString::from(" TAVILY_API_KEY"), OsString::from("secret")),
        (OsString::from("TAVILY_API_KEY "), OsString::from("secret")),
    ];

    let verified_env_vars = collect_verified_env_var_names(env_vars);

    assert_eq!(
        verified_env_vars,
        BTreeSet::from([" TAVILY_API_KEY".to_owned(), "TAVILY_API_KEY ".to_owned(),])
    );
}

#[test]
fn resolve_plugin_setup_readiness_context_falls_back_to_process_env_when_unspecified() {
    let env_vars = vec![
        (OsString::from("TAVILY_API_KEY"), OsString::from("secret")),
        (OsString::from("EMPTY_VALUE"), OsString::from("   ")),
    ];

    let context = resolve_plugin_setup_readiness_context(None, env_vars);

    assert_eq!(
        context.verified_env_vars,
        BTreeSet::from(["TAVILY_API_KEY".to_owned()])
    );
    assert!(context.verified_config_keys.is_empty());
}

#[test]
fn resolve_plugin_setup_readiness_context_uses_explicit_values_without_env_inheritance() {
    let readiness_spec = PluginSetupReadinessSpec {
        inherit_process_env: false,
        verified_env_vars: vec![" TAVILY_API_KEY ".to_owned(), "".to_owned()],
        verified_config_keys: vec![
            " tools.web_search.default_provider ".to_owned(),
            "   ".to_owned(),
        ],
    };
    let env_vars = vec![(OsString::from("SHOULD_NOT_BE_USED"), OsString::from("set"))];

    let context = resolve_plugin_setup_readiness_context(Some(&readiness_spec), env_vars);

    assert_eq!(
        context.verified_env_vars,
        BTreeSet::from(["TAVILY_API_KEY".to_owned()])
    );
    assert_eq!(
        context.verified_config_keys,
        BTreeSet::from(["tools.web_search.default_provider".to_owned()])
    );
}

#[test]
fn resolve_plugin_setup_readiness_context_merges_process_env_when_requested() {
    let readiness_spec = PluginSetupReadinessSpec {
        inherit_process_env: true,
        verified_env_vars: vec!["TAVILY_API_KEY".to_owned()],
        verified_config_keys: vec!["tools.web_search.default_provider".to_owned()],
    };
    let env_vars = vec![(OsString::from("OPENAI_API_KEY"), OsString::from("set"))];

    let context = resolve_plugin_setup_readiness_context(Some(&readiness_spec), env_vars);

    assert_eq!(
        context.verified_env_vars,
        BTreeSet::from(["OPENAI_API_KEY".to_owned(), "TAVILY_API_KEY".to_owned(),])
    );
    assert_eq!(
        context.verified_config_keys,
        BTreeSet::from(["tools.web_search.default_provider".to_owned()])
    );
}
