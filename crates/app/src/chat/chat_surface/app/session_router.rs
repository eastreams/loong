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
#[derive(Default)]
pub(crate) struct SessionRouter {
    active_route: Option<ActiveSessionRoute>,
    created_this_run_session_ids: Vec<String>,
}

#[allow(dead_code)]
impl SessionRouter {
    pub(crate) fn new(active_route: ActiveSessionRoute) -> Self {
        let mut router = Self::default();
        router.install_route(active_route);
        router
    }

    pub(crate) fn active_route(&self) -> &ActiveSessionRoute {
        self.active_route
            .as_ref()
            .expect("session router must have an active route")
    }

    pub(crate) fn active_route_mut(&mut self) -> &mut ActiveSessionRoute {
        self.active_route
            .as_mut()
            .expect("session router must have an active route")
    }

    pub(crate) fn active_session_id(&self) -> &str {
        self.active_route().runtime.session_id.as_str()
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
        self.active_route = Some(active_route);
    }
}
