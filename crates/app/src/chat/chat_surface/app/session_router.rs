#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteOrigin {
    Existing,
    CreatedThisRun,
}

#[allow(dead_code)]
impl RouteOrigin {
    pub(crate) fn is_created_this_run(self) -> bool {
        matches!(self, Self::CreatedThisRun)
    }
}

#[allow(dead_code)]
pub(crate) struct ActiveSessionRoute {
    pub(crate) runtime: crate::chat::CliTurnRuntime,
    pub(crate) route_origin: RouteOrigin,
}

#[allow(dead_code)]
impl ActiveSessionRoute {
    pub(crate) fn from_runtime(runtime: crate::chat::CliTurnRuntime) -> Self {
        match runtime.session_origin {
            crate::chat::CliRuntimeSessionOrigin::Existing => Self::for_existing(runtime),
            crate::chat::CliRuntimeSessionOrigin::CreatedThisRun => {
                Self::for_created_this_run(runtime)
            }
        }
    }

    pub(crate) fn for_existing(runtime: crate::chat::CliTurnRuntime) -> Self {
        Self {
            runtime,
            route_origin: RouteOrigin::Existing,
        }
    }

    pub(crate) fn for_created_this_run(runtime: crate::chat::CliTurnRuntime) -> Self {
        Self {
            runtime,
            route_origin: RouteOrigin::CreatedThisRun,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SessionRouterVisualState {
    pub(crate) switch_confirm: Option<SwitchConfirmState>,
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
        if active_route.route_origin.is_created_this_run() {
            self.created_this_run_session_ids
                .push(active_route.runtime.session_id.clone());
        }
        self.active_route = Some(active_route);
    }
}
