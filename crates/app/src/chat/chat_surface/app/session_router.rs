#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SwitchConfirmState {
    pub(crate) pending_target_session_id: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionTransitionReason {
    UserRequestedNew,
    UserRequestedResume,
}

#[allow(dead_code)]
pub(crate) struct SessionTransitionOutcome {
    pub(crate) message: String,
}

#[allow(dead_code)]
pub(crate) struct ActiveSessionRoute {
    pub(crate) runtime: crate::chat::CliTurnRuntime,
}

#[allow(dead_code)]
impl ActiveSessionRoute {
    pub(crate) fn route_origin(&self) -> crate::chat::CliRuntimeSessionOrigin {
        self.runtime.session_origin
    }

    pub(crate) fn from_runtime(runtime: crate::chat::CliTurnRuntime) -> Self {
        Self { runtime }
    }
}

#[allow(dead_code)]
pub(crate) struct SessionRouter {
    active_route: ActiveSessionRoute,
    created_this_run_session_ids: Vec<String>,
    switch_confirm: Option<SwitchConfirmState>,
}

#[allow(dead_code)]
impl SessionRouter {
    pub(crate) fn new(active_route: ActiveSessionRoute) -> Self {
        let mut created_this_run_session_ids = Vec::new();
        if matches!(
            active_route.route_origin(),
            crate::chat::CliRuntimeSessionOrigin::CreatedThisRun
        ) {
            created_this_run_session_ids.push(active_route.runtime.session_id.clone());
        }

        Self {
            active_route,
            created_this_run_session_ids,
            switch_confirm: None,
        }
    }

    pub(crate) fn active_route(&self) -> &ActiveSessionRoute {
        &self.active_route
    }

    pub(crate) fn active_route_mut(&mut self) -> &mut ActiveSessionRoute {
        &mut self.active_route
    }

    pub(crate) fn active_runtime(&self) -> &crate::chat::CliTurnRuntime {
        &self.active_route.runtime
    }

    pub(crate) fn active_runtime_mut(&mut self) -> &mut crate::chat::CliTurnRuntime {
        &mut self.active_route.runtime
    }

    pub(crate) fn active_session_id(&self) -> &str {
        self.active_runtime().session_id.as_str()
    }

    pub(crate) fn created_this_run_session_ids(&self) -> Vec<String> {
        self.created_this_run_session_ids.clone()
    }

    pub(crate) fn install_created_route(&mut self, active_route: ActiveSessionRoute) {
        self.install_route(active_route);
    }

    pub(crate) fn begin_create_new_session(
        &mut self,
        reason: SessionTransitionReason,
    ) -> crate::CliResult<SessionTransitionOutcome> {
        let route = self.rebuild_route(None, crate::chat::CliSessionRequirement::AllowImplicitDefault)?;
        let session_id = route.runtime.session_id.clone();
        self.install_created_route(route);
        self.switch_confirm = None;
        Ok(SessionTransitionOutcome {
            message: session_transition_success_message(reason, session_id.as_str()),
        })
    }

    pub(crate) fn begin_resume_session(
        &mut self,
        target_session_id: &str,
        reason: SessionTransitionReason,
    ) -> crate::CliResult<SessionTransitionOutcome> {
        let route = self.rebuild_route(
            Some(target_session_id),
            crate::chat::CliSessionRequirement::RequireExplicit,
        )?;
        let session_id = route.runtime.session_id.clone();
        self.install_route(route);
        self.switch_confirm = None;
        Ok(SessionTransitionOutcome {
            message: session_transition_success_message(reason, session_id.as_str()),
        })
    }

    #[cfg(feature = "memory-sqlite")]
    pub(crate) fn latest_resume_target_session_id(&self) -> crate::CliResult<Option<String>> {
        crate::session::latest_resumable_root_session_id(&self.active_runtime().memory_config)
    }

    fn rebuild_route(
        &self,
        session_hint: Option<&str>,
        session_requirement: crate::chat::CliSessionRequirement,
    ) -> crate::CliResult<ActiveSessionRoute> {
        let preserved_options = crate::chat::CliChatOptions {
            acp_requested: false,
            acp_event_stream: false,
            acp_bootstrap_mcp_servers: self
                .active_runtime()
                .effective_bootstrap_mcp_servers
                .clone(),
            acp_working_directory: self.active_runtime().effective_working_directory.clone(),
        };
        let runtime = crate::chat::initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx(
            self.active_runtime().resolved_path.clone(),
            self.active_runtime().config.clone(),
            session_hint,
            &preserved_options,
            self.active_runtime().runtime_kernel.cloned_kernel_context(),
            session_requirement,
        )?;
        Ok(ActiveSessionRoute::from_runtime(runtime))
    }

    fn install_route(&mut self, active_route: ActiveSessionRoute) {
        if matches!(
            active_route.route_origin(),
            crate::chat::CliRuntimeSessionOrigin::CreatedThisRun
        ) && !self
            .created_this_run_session_ids
            .contains(&active_route.runtime.session_id)
        {
            self.created_this_run_session_ids
                .push(active_route.runtime.session_id.clone());
        }
        self.active_route = active_route;
    }
}

fn session_transition_success_message(
    reason: SessionTransitionReason,
    session_id: &str,
) -> String {
    match reason {
        SessionTransitionReason::UserRequestedNew => {
            format!("Started a new session: {session_id}")
        }
        SessionTransitionReason::UserRequestedResume => {
            format!("Resumed session: {session_id}")
        }
    }
}
