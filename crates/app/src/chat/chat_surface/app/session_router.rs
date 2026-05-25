#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SwitchConfirmState {
    pub(crate) pending_target_session_id: String,
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
