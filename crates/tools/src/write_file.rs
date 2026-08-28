use std::borrow::Cow;

use async_trait::async_trait;
use kernel::access::fs::FsWriteError;
use schemars::JsonSchema;
use tool_host::{ToolContext, ToolImpl};

pub struct WriteFileTool;

#[derive(JsonSchema, serde::Deserialize)]
pub struct WriteFileInput {
    pub path: String,
    pub content: String,
}

#[derive(JsonSchema, serde::Serialize)]
pub struct WriteFileOutput {
    pub written: String,
}

#[async_trait]
impl ToolImpl for WriteFileTool {
    type Input = WriteFileInput;
    type Output = WriteFileOutput;
    type Error = FsWriteError;

    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed("write_file")
    }

    fn description(&self) -> Cow<'_, str> {
        Cow::Borrowed("Write a text file inside the workspace")
    }

    async fn execute(
        &self,
        ctx: &ToolContext<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        let path = input.path.clone();
        ctx.fs()
            .write(&input.path, input.content.into_bytes())
            .await?;

        Ok(WriteFileOutput { written: path })
    }
}
