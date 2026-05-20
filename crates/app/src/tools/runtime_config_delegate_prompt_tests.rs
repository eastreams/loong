use super::*;
use crate::conversation::{
    ConstrainedSubagentContractView, ConstrainedSubagentExecution, ConstrainedSubagentIdentity,
    ConstrainedSubagentProfile,
};

#[test]
fn delegate_child_prompt_summary_returns_none_when_narrowing_is_empty() {
    assert_eq!(
        ToolRuntimeConfig::default().delegate_child_prompt_summary(None),
        None
    );
}

#[test]
fn delegate_child_prompt_summary_is_effective_stable_and_sparse() {
    let base = ToolRuntimeConfig {
        browser: BrowserRuntimePolicy {
            enabled: true,
            max_sessions: 1,
            max_links: 4,
            max_text_chars: 1_024,
        },
        web_fetch: WebFetchRuntimePolicy {
            enabled: true,
            allow_private_hosts: false,
            enforce_allowed_domains: true,
            allowed_domains: BTreeSet::from(["api.example.com".to_owned()]),
            blocked_domains: BTreeSet::from(["base-block.example.com".to_owned()]),
            timeout_seconds: 3,
            max_bytes: 2_048,
            max_redirects: 1,
        },
        ..ToolRuntimeConfig::default()
    };
    let narrowing = ToolRuntimeNarrowing {
        browser: BrowserRuntimeNarrowing {
            max_sessions: Some(8),
            max_links: Some(8),
            max_text_chars: Some(512),
        },
        web_fetch: WebFetchRuntimeNarrowing {
            allow_private_hosts: Some(true),
            enforce_allowed_domains: false,
            allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
            blocked_domains: BTreeSet::from(["deny.example.com".to_owned()]),
            timeout_seconds: Some(5),
            max_bytes: Some(4_096),
            max_redirects: Some(2),
        },
    };
    let execution = ConstrainedSubagentExecution {
        mode: crate::conversation::ConstrainedSubagentMode::Async,
        isolation: crate::conversation::ConstrainedSubagentIsolation::Shared,
        owner_kind: None,
        depth: 1,
        max_depth: 2,
        active_children: 0,
        max_active_children: 3,
        timeout_seconds: 60,
        allow_shell_in_child: false,
        child_tool_allowlist: vec!["web.fetch".to_owned()],
        workspace_root: None,
        runtime_narrowing: narrowing,
        kernel_bound: false,
        identity: Some(ConstrainedSubagentIdentity {
            nickname: Some("child-research".to_owned()),
            specialization: Some("researcher".to_owned()),
        }),
        profile: Some(ConstrainedSubagentProfile::for_child_depth(1, 2)),
    };

    let contract = execution.contract_view();
    let summary = base
        .delegate_child_prompt_summary(Some(&contract))
        .expect("delegate child prompt summary");

    assert_eq!(
        summary,
        "[delegate_child_runtime_contract]\n\
Plan within these child-session runtime limits:\n\
- subagent mode: async\n\
- subagent nickname: child-research\n\
- subagent specialization: researcher\n\
- subagent depth budget: 1/2\n\
- subagent active-child budget snapshot: 0/3\n\
- child timeout seconds: 60\n\
- child bash: denied\n\
- child tool allowlist: web\n\
- child runtime binding: direct\n\
- subagent role: orchestrator\n\
- subagent control scope: children\n\
- web network private hosts: denied\n\
- web network allowed domains: none (effective intersection is empty)\n\
- web network blocked domains: base-block.example.com, deny.example.com\n\
- web network timeout seconds: 3\n\
- web network max bytes: 2048\n\
- web network max redirects: 1\n\
- browser max sessions: 1\n\
- browser max links: 4\n\
- browser max text chars: 512\n\
Treat these as enforced limits for this child session."
    );
}

#[test]
fn delegate_child_prompt_summary_omits_disabled_web_fetch() {
    let base = ToolRuntimeConfig {
        web_fetch: WebFetchRuntimePolicy {
            enabled: false,
            allow_private_hosts: true,
            enforce_allowed_domains: false,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
            timeout_seconds: 30,
            max_bytes: 1_048_576,
            max_redirects: 5,
        },
        browser: BrowserRuntimePolicy {
            enabled: true,
            max_sessions: 4,
            max_links: 16,
            max_text_chars: 8_192,
        },
        ..ToolRuntimeConfig::default()
    };
    let narrowing = ToolRuntimeNarrowing {
        web_fetch: WebFetchRuntimeNarrowing {
            allow_private_hosts: Some(false),
            timeout_seconds: Some(5),
            ..WebFetchRuntimeNarrowing::default()
        },
        browser: BrowserRuntimeNarrowing {
            max_sessions: Some(2),
            ..BrowserRuntimeNarrowing::default()
        },
    };

    let contract = ConstrainedSubagentContractView::from_runtime_narrowing(narrowing);
    let summary = base
        .delegate_child_prompt_summary(Some(&contract))
        .expect("should still render browser section");

    assert!(
        !summary.contains("web.fetch"),
        "disabled web_fetch fields should not appear in prompt summary: {summary}"
    );
    assert!(
        summary.contains("- browser max sessions: 2"),
        "enabled browser fields should still appear: {summary}"
    );
}

#[test]
fn delegate_child_prompt_summary_omits_disabled_browser() {
    let base = ToolRuntimeConfig {
        web_fetch: WebFetchRuntimePolicy {
            enabled: true,
            allow_private_hosts: true,
            enforce_allowed_domains: false,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
            timeout_seconds: 30,
            max_bytes: 1_048_576,
            max_redirects: 5,
        },
        browser: BrowserRuntimePolicy {
            enabled: false,
            max_sessions: 4,
            max_links: 16,
            max_text_chars: 8_192,
        },
        ..ToolRuntimeConfig::default()
    };
    let narrowing = ToolRuntimeNarrowing {
        web_fetch: WebFetchRuntimeNarrowing {
            timeout_seconds: Some(5),
            ..WebFetchRuntimeNarrowing::default()
        },
        browser: BrowserRuntimeNarrowing {
            max_sessions: Some(2),
            max_links: Some(8),
            ..BrowserRuntimeNarrowing::default()
        },
    };

    let contract = ConstrainedSubagentContractView::from_runtime_narrowing(narrowing);
    let summary = base
        .delegate_child_prompt_summary(Some(&contract))
        .expect("should still render web_fetch section");

    assert!(
        !summary.contains("browser"),
        "disabled browser fields should not appear in prompt summary: {summary}"
    );
    assert!(
        summary.contains("- web network timeout seconds: 5"),
        "enabled web_fetch fields should still appear: {summary}"
    );
}

#[test]
fn delegate_child_prompt_summary_returns_none_when_all_tools_disabled() {
    let base = ToolRuntimeConfig {
        web_fetch: WebFetchRuntimePolicy {
            enabled: false,
            ..WebFetchRuntimePolicy::default()
        },
        browser: BrowserRuntimePolicy {
            enabled: false,
            ..BrowserRuntimePolicy::default()
        },
        ..ToolRuntimeConfig::default()
    };
    let narrowing = ToolRuntimeNarrowing {
        web_fetch: WebFetchRuntimeNarrowing {
            timeout_seconds: Some(5),
            ..WebFetchRuntimeNarrowing::default()
        },
        browser: BrowserRuntimeNarrowing {
            max_sessions: Some(2),
            ..BrowserRuntimeNarrowing::default()
        },
    };
    let contract = ConstrainedSubagentContractView::from_runtime_narrowing(narrowing);

    assert_eq!(
        base.delegate_child_prompt_summary(Some(&contract)),
        None,
        "should return None when all narrowed tools are disabled"
    );
}

#[test]
fn delegate_child_prompt_summary_renders_profile_even_when_narrowing_is_empty() {
    let contract = ConstrainedSubagentContractView::from_profile(
        ConstrainedSubagentProfile::for_child_depth(1, 1),
    );
    let summary = ToolRuntimeConfig::default()
        .delegate_child_prompt_summary(Some(&contract))
        .expect("profile-only child prompt summary");

    assert_eq!(
        summary,
        "[delegate_child_runtime_contract]\n\
Plan within these child-session runtime limits:\n\
- subagent role: leaf\n\
- subagent control scope: none\n\
Treat these as enforced limits for this child session."
    );
}
