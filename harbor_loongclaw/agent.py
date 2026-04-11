from __future__ import annotations

import json
import shlex
import textwrap
from pathlib import Path, PurePosixPath

from harbor.agents.installed.base import BaseInstalledAgent, with_prompt_template
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext
from harbor.models.trial.paths import EnvironmentPaths


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
        return (
            'export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"; '
            "loong --version"
        )

    def _resolved_provider_kind(self) -> str:
        if self.model_name and "/" in self.model_name:
            provider_hint, _ = self.model_name.split("/", 1)
            if provider_hint.strip():
                return provider_hint.strip()
        return self.provider_kind.strip() or "openai"

    def _resolved_model_id(self) -> str:
        if not self.model_name:
            raise ValueError(
                "LoongClawInstalledAgent requires Harbor model_name, for example openai/gpt-5.4"
            )
        if "/" in self.model_name:
            _, model_id = self.model_name.split("/", 1)
            if model_id.strip():
                return model_id.strip()
        model_id = self.model_name.strip()
        if not model_id:
            raise ValueError("Harbor model_name resolved to an empty LoongClaw model id")
        return model_id

    def _profile_id(self) -> str:
        return self._resolved_provider_kind().replace("-", "_")

    def _env_output_path(self) -> PurePosixPath:
        return EnvironmentPaths.agent_dir / self._OUTPUT_FILENAME

    def _env_config_path(self) -> PurePosixPath:
        return EnvironmentPaths.agent_dir / self._CONFIG_FILENAME

    def _env_trajectory_path(self) -> PurePosixPath:
        return EnvironmentPaths.agent_dir / self._TRAJECTORY_FILENAME

    async def install(self, environment: BaseEnvironment) -> None:
        await self.exec_as_root(
            environment,
            command=textwrap.dedent(
                f"""
                if [ -f /etc/alpine-release ]; then
                  apk add --no-cache bash curl git build-base pkgconf openssl-dev ca-certificates
                elif command -v apt-get >/dev/null 2>&1; then
                  apt-get update
                  DEBIAN_FRONTEND=noninteractive apt-get install -y curl git build-essential pkg-config libssl-dev ca-certificates
                elif command -v yum >/dev/null 2>&1; then
                  yum install -y curl git gcc gcc-c++ make pkgconfig openssl-devel ca-certificates
                else
                  echo "unsupported package manager: need curl git rust build dependencies" >&2
                  exit 1
                fi
                """
            ).strip(),
        )

        await self.exec_as_agent(
            environment,
            command=textwrap.dedent(
                f"""
                export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
                if ! command -v cargo >/dev/null 2>&1; then
                  for attempt in 1 2 3; do
                    if curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal; then
                      break
                    fi
                    sleep $((attempt * 2))
                  done
                fi
                if [ -f "$HOME/.cargo/env" ]; then
                  . "$HOME/.cargo/env"
                fi
                if command -v cargo >/dev/null 2>&1; then
                  export CARGO_TARGET_DIR=/tmp/loongclaw-harbor-target
                  if cargo install --path {shlex.quote(f"{self.source_mount}/crates/daemon")} --locked --force --root "$HOME/.local" --bin loong; then
                    if command -v loong >/dev/null 2>&1; then
                      loong --version
                      exit 0
                    fi
                  fi
                  if cargo install --path {shlex.quote(f"{self.source_mount}/crates/daemon")} --locked --force --root "$HOME/.local" --bin loongclaw; then
                    if command -v loongclaw >/dev/null 2>&1; then
                      loongclaw --version
                    fi
                    if [ -x "$HOME/.local/bin/loongclaw" ] && [ ! -e "$HOME/.local/bin/loong" ]; then
                      ln -sf "$HOME/.local/bin/loongclaw" "$HOME/.local/bin/loong"
                    fi
                    if command -v loong >/dev/null 2>&1; then
                      loong --version
                      exit 0
                    fi
                  fi
                fi
                if [ -x "$HOME/.local/bin/loong" ]; then
                  "$HOME/.local/bin/loong" --version
                  exit 0
                fi
                if [ -x "$HOME/.local/bin/loongclaw" ]; then
                  ln -sf "$HOME/.local/bin/loongclaw" "$HOME/.local/bin/loong"
                  loong --version
                    exit 0
                  fi
                fi
                bash {shlex.quote(f"{self.source_mount}/scripts/install.sh")} --prefix "$HOME/.local/bin" --target-libc gnu
                loong --version
                """
            ).strip(),
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

        command = textwrap.dedent(
            f"""
            export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
            TASK_CWD="$(pwd)"
            cat > {shlex.quote(config_path)} <<EOF
            active_provider = {json.dumps(profile_id)}

            [providers.{profile_id}]
            kind = {json.dumps(provider_kind)}
            model = {json.dumps(model_id)}
            reasoning_effort = {json.dumps(self.reasoning_effort)}
            api_key = {{ env = {json.dumps(self.api_key_env)} }}

            [tools]
            file_root = "$TASK_CWD"
            shell_default_mode = {json.dumps(self.shell_default_mode)}

            [tools.bash]
            login_shell = false
            EOF

            loong validate-config --config {shlex.quote(config_path)}
            loong ask --config {shlex.quote(config_path)} --session {shlex.quote(self.session_name)} --message {shlex.quote(instruction)} 2>&1 | tee {shlex.quote(output_path)}
            if ! loong trajectory-export --config {shlex.quote(config_path)} --session {shlex.quote(self.session_name)} --output {shlex.quote(trajectory_path)}; then
              echo "warning: loong trajectory-export failed" >&2
            fi
            """
        ).strip()

        await self.exec_as_agent(environment, command=command)

    def populate_context_post_run(self, context: AgentContext) -> None:
        output_path = self.logs_dir / self._OUTPUT_FILENAME
        config_path = self.logs_dir / self._CONFIG_FILENAME
        trajectory_path = self.logs_dir / self._TRAJECTORY_FILENAME

        output_preview = None
        if output_path.exists():
            output_preview = output_path.read_text(errors="replace")[:4000]

        metadata = dict(context.metadata or {})
        metadata.update(
            {
                "provider_kind": self._resolved_provider_kind(),
                "model_id": self._resolved_model_id(),
                "reasoning_effort": self.reasoning_effort,
                "api_key_env": self.api_key_env,
                "session_name": self.session_name,
                "output_path": output_path.name if output_path.exists() else None,
                "config_path": config_path.name if config_path.exists() else None,
                "trajectory_path": trajectory_path.name if trajectory_path.exists() else None,
                "assistant_output_preview": output_preview,
            }
        )
        context.metadata = {key: value for key, value in metadata.items() if value is not None}
