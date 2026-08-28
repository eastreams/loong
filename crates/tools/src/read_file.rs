use async_trait::async_trait;
use kernel::access::fs::FsReadError;
use schemars::JsonSchema;
use tool_host::{ToolContext, ToolImpl};

pub struct ReadFileTool;

#[derive(JsonSchema, serde::Deserialize)]
pub struct ReadFileInput {
    pub path: String,
}

#[derive(JsonSchema, serde::Serialize)]
pub struct ReadFileOutput {
    pub content: String,
}

#[async_trait]
impl ToolImpl for ReadFileTool {
    type Input = ReadFileInput;
    type Output = ReadFileOutput;
    type Error = FsReadError;

    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Read a text file inside the workspace"
    }

    async fn execute(
        &self,
        ctx: &ToolContext<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        let bytes = ctx.fs().read(&input.path).await?;
        let content = String::from_utf8_lossy(&bytes).into_owned();

        Ok(ReadFileOutput { content })
    }
}
