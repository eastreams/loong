use loong_contracts::{Capabilities, Capability, GovernedSessionMode};
use loong_runtime::runtime::Runtime;

use crate::RuntimeContextFactory;
use crate::config::LoongConfig;
use crate::tools::ToolView;
#[cfg(feature = "memory-sqlite")]
use crate::tools::runtime_config::ToolRuntimeNarrowing;

use super::Session;

// Durable repository projection is owned as one feature-gated unit; generic
// Session construction below remains independent of the SQLite implementation.
#[cfg(feature = "memory-sqlite")]
mod sqlite;

/// Canonical effective tool-policy projection produced beside Session materialization.
///
/// `base_tool_view` already includes runtime availability, delegate contracts,
/// ancestor ceilings, and active-skill blocks. Only the target Session's own
/// persisted policy separates it from `effective_tool_view`. Runtime narrowing
/// is cumulative across the same lineage, while `delegate_runtime_narrowing`
/// remains the target anchor's local value for status reporting.
#[cfg(feature = "memory-sqlite")]
pub(crate) struct SessionToolPolicyProjection {
    pub(crate) base_tool_view: ToolView,
    pub(crate) effective_tool_view: ToolView,
    pub(crate) session_tool_policy: Option<crate::session::repository::SessionToolPolicyRecord>,
    pub(crate) delegate_runtime_narrowing: Option<ToolRuntimeNarrowing>,
    pub(crate) effective_runtime_narrowing: Option<ToolRuntimeNarrowing>,
}

impl Session {
    /// Select the host-approved capability baseline for one Session mode.
    ///
    /// Executable Session construction and read-only policy projection use the
    /// same definition so projection validation cannot drift from execution.
    fn capability_baseline(session_mode: GovernedSessionMode) -> Capabilities {
        match session_mode {
            GovernedSessionMode::MutatingCapable => Capabilities::from([
                Capability::InvokeTool,
                Capability::NetworkEgress,
                Capability::MemoryRead,
                Capability::MemoryWrite,
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
            ]),
            GovernedSessionMode::AdvisoryOnly => Capabilities::from([
                Capability::MemoryRead,
                Capability::FilesystemRead,
                Capability::NetworkEgress,
            ]),
        }
    }

    /// Materialize one host-approved Session before any Context can borrow it.
    ///
    /// Repository state is opened first because runtime availability may depend
    /// on durable resources. Persisted policy and lineage can then narrow this
    /// configured baseline, but cannot create authority of their own.
    pub fn from_config(
        runtime: &Runtime<RuntimeContextFactory>,
        config: &LoongConfig,
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
        session_mode: GovernedSessionMode,
    ) -> Result<Self, String> {
        let session_id = session_id.into();
        let agent_id = agent_id.into();
        let baseline_capabilities = Self::capability_baseline(session_mode);
        let tool_runtime_config =
            crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
        let memory_runtime_config =
            crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config_without_env_overrides(
                &config.memory,
            );
        let visible_skill_roots =
            crate::tools::skills::model_visible_skill_roots_with_config(&tool_runtime_config);

        #[cfg(feature = "memory-sqlite")]
        let session = sqlite::materialize(
            runtime,
            config,
            session_id,
            agent_id,
            session_mode,
            baseline_capabilities,
            tool_runtime_config,
            memory_runtime_config,
        )?;

        #[cfg(not(feature = "memory-sqlite"))]
        let session = Session::root(
            runtime,
            agent_id,
            session_id,
            session_mode,
            baseline_capabilities,
            tool_runtime_config,
            memory_runtime_config,
            runtime_authority_tool_view(runtime, config),
            None,
            None,
        )?;

        Ok(session.with_visible_skill_roots(visible_skill_roots))
    }

    /// Project the policy state of one persisted Session through the same
    /// lineage materializer used to construct executable Sessions.
    #[cfg(feature = "memory-sqlite")]
    pub(crate) fn tool_policy_projection(
        runtime: &Runtime<RuntimeContextFactory>,
        config: &LoongConfig,
        session_id: &str,
    ) -> Result<SessionToolPolicyProjection, String> {
        sqlite::materialize_tool_policy_projection(runtime, config, session_id)
    }

    /// Materialize a persisted direct child under this live Session authority.
    ///
    /// Continuation is recursive execution, not a fresh host bootstrap. Building
    /// the child from its live parent preserves backend identity and prevents a
    /// changed host config from replacing authority mid-execution.
    #[cfg(feature = "memory-sqlite")]
    pub(crate) fn materialize_child(
        &self,
        runtime: &Runtime<RuntimeContextFactory>,
        config: &LoongConfig,
        child_session_id: &str,
    ) -> Result<Self, String> {
        let requested_tool_runtime_config =
            crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
        let visible_skill_roots = crate::tools::skills::model_visible_skill_roots_with_config(
            &requested_tool_runtime_config,
        )
        .into_iter()
        .map(|path| super::absolute_lexical(path, &self.fs_resolution_root))
        .filter(|path| {
            self.visible_skill_roots
                .iter()
                .any(|authority_root| path.starts_with(authority_root))
        })
        .collect();
        let session = sqlite::materialize_child(runtime, config, self, child_session_id)?;
        Ok(session.with_visible_skill_roots(visible_skill_roots))
    }

    /// Rebuild mutable session projections for a continuation without creating authority.
    ///
    /// Durable policy and active-skill state can change while a provider/tool loop is
    /// running. The current Session remains the authority ceiling: rematerialization
    /// may remove tools or visible skill roots, but it cannot add either. Host/base config,
    /// filesystem authority, capabilities, identity, and lifecycle stay owned by `self`;
    /// the effective browser/web projection may only narrow from that live ceiling.
    pub(crate) fn rematerialize(
        &self,
        runtime: &Runtime<RuntimeContextFactory>,
        config: &LoongConfig,
    ) -> Result<Self, String> {
        #[cfg(feature = "memory-sqlite")]
        let (requested_tool_view, requested_active_skill_roots, requested_runtime_narrowing) = {
            let projection =
                sqlite::load_rematerialization_projection(runtime, config, &self.session_id)?;
            (
                projection.tool_policy.effective_tool_view,
                projection.active_skill_roots,
                projection.tool_policy.effective_runtime_narrowing,
            )
        };

        #[cfg(not(feature = "memory-sqlite"))]
        let (requested_tool_view, requested_active_skill_roots, requested_runtime_narrowing) = (
            runtime_authority_tool_view(runtime, config),
            Vec::new(),
            None,
        );

        let requested_tool_runtime_config =
            crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
        let visible_skill_roots = crate::tools::skills::model_visible_skill_roots_with_config(
            &requested_tool_runtime_config,
        )
        .into_iter()
        .map(|path| super::absolute_lexical(path, &self.fs_resolution_root))
        .filter(|path| {
            self.visible_skill_roots
                .iter()
                .any(|authority_root| path.starts_with(authority_root))
        })
        .collect::<Vec<_>>();
        let active_skill_roots = requested_active_skill_roots
            .into_iter()
            .map(|path| super::absolute_lexical(path, &self.fs_resolution_root))
            .collect::<Vec<_>>();

        let mut session = self.clone();
        session.tool_view = requested_tool_view.intersect(&self.tool_view);
        session.active_skill_roots = active_skill_roots;
        let effective_runtime_narrowing =
            crate::tools::runtime_config::merge_runtime_narrowing_sources(
                self.resolved_runtime_narrowing().cloned(),
                requested_runtime_narrowing,
            );
        if let Some(runtime_narrowing) = effective_runtime_narrowing.as_ref() {
            session.tool_runtime_config = self.tool_runtime_config.narrowed(runtime_narrowing);
        }
        session.effective_runtime_narrowing =
            effective_runtime_narrowing.filter(|runtime_narrowing| !runtime_narrowing.is_empty());
        Ok(session.with_visible_skill_roots(visible_skill_roots))
    }
}

/// Merge configured legacy visibility with the paths that actually exist in Runtime.
///
/// This is the only typed-path projection used to materialize Session authority;
/// a missing registration therefore cannot be revived by the static catalog.
fn runtime_authority_tool_view(
    runtime: &Runtime<RuntimeContextFactory>,
    config: &LoongConfig,
) -> ToolView {
    let runtime_config =
        crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    crate::tools::runtime_visible_tool_view(runtime, &runtime_config, None)
}
