use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;

use crate::{
    contracts::{ExecutionRoute, HarnessKind, HarnessOutcome, HarnessRequest},
    errors::HarnessError,
};

#[async_trait]
pub trait HarnessAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn kind(&self) -> HarnessKind;
    async fn execute(&self, request: HarnessRequest) -> Result<HarnessOutcome, HarnessError>;
}

#[derive(Default)]
pub struct HarnessBroker {
    adapters: BTreeMap<String, Arc<dyn HarnessAdapter>>,
}

impl HarnessBroker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            adapters: BTreeMap::new(),
        }
    }

    pub fn register<A: HarnessAdapter + 'static>(&mut self, adapter: A) {
        let key = adapter.name().to_owned();
        self.adapters.insert(key, Arc::new(adapter));
    }

    pub async fn execute(
        &self,
        route: &ExecutionRoute,
        request: HarnessRequest,
    ) -> Result<HarnessOutcome, HarnessError> {
        let adapter = if let Some(adapter_name) = &route.adapter {
            self.adapters
                .get(adapter_name)
                .ok_or_else(|| HarnessError::AdapterNotFound(adapter_name.clone()))?
                .clone()
        } else {
            self.adapters
                .values()
                .find(|candidate| candidate.kind() == route.harness_kind)
                .cloned()
                .ok_or_else(|| {
                    HarnessError::AdapterNotFound(format!("kind::{:?}", route.harness_kind))
                })?
        };

        if adapter.kind() != route.harness_kind {
            return Err(HarnessError::AdapterKindMismatch {
                adapter: adapter.name().to_owned(),
                expected: route.harness_kind,
                actual: adapter.kind(),
            });
        }

        return adapter.execute(request).await;
    }
}
