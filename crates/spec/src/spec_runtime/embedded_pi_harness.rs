use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::json;

use super::*;

pub struct EmbeddedPiHarness {
    pub seen: Mutex<Vec<String>>,
}

#[async_trait]
impl HarnessAdapter for EmbeddedPiHarness {
    fn name(&self) -> &str {
        "pi-local"
    }

    fn kind(&self) -> HarnessKind {
        HarnessKind::EmbeddedPi
    }

    async fn execute(&self, request: HarnessRequest) -> Result<HarnessOutcome, HarnessError> {
        match self.seen.lock() {
            Ok(mut guard) => guard.push(request.task_id.clone()),
            Err(_) => {
                return Err(HarnessError::Execution(
                    "EmbeddedPiHarness mutex poisoned".to_owned(),
                ));
            }
        }

        Ok(HarnessOutcome {
            status: "ok".to_owned(),
            output: json!({
                "adapter": "pi-local",
                "task": request.task_id,
                "objective": request.objective,
            }),
        })
    }
}
