from __future__ import annotations

from pathlib import PurePosixPath

from harbor.agents.installed.base import BaseInstalledAgent, with_prompt_template
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext
from harbor.models.trial.paths import EnvironmentPaths

from .commands import build_agent_dependency_install_command
from .commands import build_agent_install_command
from .commands import build_agent_run_command
from .commands import sanitize_profile_id


class LoongClawInstalledAgent(BaseInstalledAgent):
    """Harbor adapter that installs and runs the local LoongClaw workspace."""

    _OUTPUT_FILENAME = "loongclaw.txt"
    _CONFIG_FILENAME = "loongclaw-config.toml"
    _TRAJECTORY_FILENAME = "loongclaw-trajectory.json"

    def __init__(
        self,
        *args,
        reasoning_effort: str = "xhigh",
        api_key_env: str = "OPENAI_API_KEY",
        provider_kind: str = "openai",
        source_mount: str = "/opt/loongclaw-src",
        session_name: str = "harbor",
        shell_default_mode: str = "allow",
        **kwargs,
    ):
        super().__init__(*args, **kwargs)
        self.reasoning_effort = reasoning_effort
        self.api_key_env = api_key_env
        self.provider_kind = provider_kind
        self.source_mount = source_mount
        self.session_name = session_name
        self.shell_default_mode = shell_default_mode

    @staticmethod
    def name() -> str:
        return "loongclaw"

    def get_version_command(self) -> str | None:
        command = 'export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"; loong --version'
        return command

    def _resolved_provider_kind(self) -> str:
        if self.model_name and "/" in self.model_name:
            provider_hint, _ = self.model_name.split("/", 1)
            if provider_hint.strip():
                return provider_hint.strip()

        fallback_provider_kind = self.provider_kind.strip() or "openai"
        return fallback_provider_kind

    def _resolved_model_id(self) -> str:
        if not self.model_name:
            raise ValueError(
                "LoongClawInstalledAgent requires Harbor model_name, for example openai/gpt-5.4"
            )

        if "/" in self.model_name:
            _, model_id = self.model_name.split("/", 1)
            if model_id.strip():
                return model_id.strip()

        stripped_model_id = self.model_name.strip()
        if not stripped_model_id:
            raise ValueError("Harbor model_name resolved to an empty LoongClaw model id")

        return stripped_model_id

    def _profile_id(self) -> str:
        resolved_provider_kind = self._resolved_provider_kind()
        profile_id = sanitize_profile_id(resolved_provider_kind)
        return profile_id

    def _env_output_path(self) -> PurePosixPath:
        output_path = EnvironmentPaths.agent_dir / self._OUTPUT_FILENAME
        return output_path

    def _env_config_path(self) -> PurePosixPath:
        config_path = EnvironmentPaths.agent_dir / self._CONFIG_FILENAME
        return config_path

    def _env_trajectory_path(self) -> PurePosixPath:
        trajectory_path = EnvironmentPaths.agent_dir / self._TRAJECTORY_FILENAME
        return trajectory_path

    async def install(self, environment: BaseEnvironment) -> None:
        dependency_install_command = build_agent_dependency_install_command()

        await self.exec_as_root(
            environment,
            command=dependency_install_command,
        )

        agent_install_command = build_agent_install_command(
            source_mount=self.source_mount,
        )

        await self.exec_as_agent(
            environment,
            command=agent_install_command,
        )

    @with_prompt_template
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        provider_kind = self._resolved_provider_kind()
        model_id = self._resolved_model_id()
        profile_id = self._profile_id()
        config_path = str(self._env_config_path())
        output_path = str(self._env_output_path())
        trajectory_path = str(self._env_trajectory_path())

        command = build_agent_run_command(
            profile_id=profile_id,
            provider_kind=provider_kind,
            model_id=model_id,
            reasoning_effort=self.reasoning_effort,
            api_key_env=self.api_key_env,
            shell_default_mode=self.shell_default_mode,
            session_name=self.session_name,
            instruction=instruction,
            config_path=config_path,
            output_path=output_path,
            trajectory_path=trajectory_path,
        )

        await self.exec_as_agent(environment, command=command)

    def populate_context_post_run(self, context: AgentContext) -> None:
        output_path = self.logs_dir / self._OUTPUT_FILENAME
        config_path = self.logs_dir / self._CONFIG_FILENAME
        trajectory_path = self.logs_dir / self._TRAJECTORY_FILENAME

        output_preview = None
        if output_path.exists():
            output_preview = output_path.read_text(errors="replace")[:4000]

        metadata = dict(context.metadata or {})

        metadata["provider_kind"] = self._resolved_provider_kind()
        metadata["model_id"] = self._resolved_model_id()
        metadata["reasoning_effort"] = self.reasoning_effort
        metadata["api_key_env"] = self.api_key_env
        metadata["session_name"] = self.session_name
        metadata["output_path"] = output_path.name if output_path.exists() else None
        metadata["config_path"] = config_path.name if config_path.exists() else None
        metadata["trajectory_path"] = (
            trajectory_path.name if trajectory_path.exists() else None
        )
        metadata["assistant_output_preview"] = output_preview

        filtered_metadata: dict[str, object] = {}
        for key, value in metadata.items():
            if value is None:
                continue

            filtered_metadata[key] = value

        context.metadata = filtered_metadata
