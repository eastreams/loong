use std::{borrow::Cow, path::PathBuf};

use async_trait::async_trait;
use loong_contracts::capability::{Capabilities, Capability};
use loong_core::action::{Action, ActionMeta, Granted};
use serde_json::Value;

pub struct FsReadAction {
    pub path: PathBuf,
}

impl FsReadAction {
    const NAME: &str = "fs.read";
    const CAPABILITIES: Capabilities = Capabilities::singleton(Capability::FsRead);
}

impl ActionMeta for FsReadAction {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(Self::NAME)
    }
    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(Value::String(self.path.display().to_string()))
    }
    fn required_capabilities(&self) -> Capabilities {
        Self::CAPABILITIES
    }
}

#[async_trait]
impl<Cx: Sync> Action<Cx> for FsReadAction {
    type Output = Vec<u8>;
    // TODO: use proper error type
    type Error = ();

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        tokio::fs::read(&granted.as_ref().path)
            .await
            .map_err(|_| ())
    }
}
