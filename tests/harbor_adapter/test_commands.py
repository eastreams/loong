from __future__ import annotations

import subprocess
import tomllib
import unittest

from harbor_loongclaw.commands import build_agent_install_command
from harbor_loongclaw.commands import build_agent_run_command
from harbor_loongclaw.commands import build_runtime_config_text
from harbor_loongclaw.commands import sanitize_profile_id


def assert_shell_parses(script: str) -> None:
    completed = subprocess.run(
        ["bash", "-n"],
        check=False,
        input=script,
        text=True,
        capture_output=True,
    )

    if completed.returncode == 0:
        return

    error_message = completed.stderr.strip()
    script_preview = script
    if len(script_preview) > 500:
        truncated_script_preview = script_preview[:500]
        script_preview = f"{truncated_script_preview}..."

    error_message = (
        f"{error_message}\n\nscript preview:\n{script_preview}"
    )
    raise AssertionError(error_message)


class HarborLoongClawCommandTests(unittest.TestCase):
    def test_install_command_is_valid_shell(self) -> None:
        command = build_agent_install_command("/opt/loongclaw-src")
        assert_shell_parses(command)

    def test_install_command_uses_cargo_install_without_source_fallback(self) -> None:
        command = build_agent_install_command("/opt/loongclaw-src")

        self.assertIn("--bin loong", command)
        self.assertIn("--bin loongclaw", command)
        self.assertNotIn("scripts/install.sh", command)
        self.assertIn(
            'ln -sf "$HOME/.local/bin/loong" "$HOME/.local/bin/loongclaw"',
            command,
        )
        self.assertIn('echo "failed to install cargo with rustup" >&2', command)

    def test_runtime_config_text_is_valid_toml(self) -> None:
        config_text = build_runtime_config_text(
            profile_id="openai",
            provider_kind="openai",
            model_id="gpt-5.4",
            reasoning_effort="xhigh",
            api_key_env="OPENAI_API_KEY",
            shell_default_mode="allow",
        )

        parsed = tomllib.loads(config_text)

        provider = parsed["providers"]["openai"]
        tools = parsed["tools"]
        bash_tools = tools["bash"]

        self.assertEqual(parsed["active_provider"], "openai")
        self.assertEqual(provider["kind"], "openai")
        self.assertEqual(provider["model"], "gpt-5.4")
        self.assertEqual(provider["reasoning_effort"], "xhigh")
        self.assertEqual(provider["api_key"]["env"], "OPENAI_API_KEY")
        self.assertEqual(tools["file_root"], "$TASK_CWD")
        self.assertEqual(tools["shell_default_mode"], "allow")
        self.assertFalse(bash_tools["login_shell"])

    def test_runtime_config_text_quotes_sanitized_profile_keys(self) -> None:
        profile_id = sanitize_profile_id("OpenAI Compat [beta]")
        config_text = build_runtime_config_text(
            profile_id=profile_id,
            provider_kind="openai",
            model_id="gpt-5.4",
            reasoning_effort="xhigh",
            api_key_env="OPENAI_API_KEY",
            shell_default_mode="allow",
        )

        parsed = tomllib.loads(config_text)
        provider = parsed["providers"][profile_id]

        self.assertEqual(profile_id, "openai_compat_beta")
        self.assertEqual(parsed["active_provider"], profile_id)
        self.assertEqual(provider["kind"], "openai")

    def test_run_command_is_valid_shell(self) -> None:
        command = build_agent_run_command(
            profile_id="openai",
            provider_kind="openai",
            model_id="gpt-5.4",
            reasoning_effort="xhigh",
            api_key_env="OPENAI_API_KEY",
            shell_default_mode="allow",
            session_name="harbor",
            instruction="say hello",
            config_path="/logs/agent/loongclaw-config.toml",
            output_path="/logs/agent/loongclaw.txt",
            trajectory_path="/logs/agent/loongclaw-trajectory.json",
        )

        assert_shell_parses(command)

    def test_run_command_keeps_validate_ask_and_trajectory_contract(self) -> None:
        command = build_agent_run_command(
            profile_id="openai",
            provider_kind="openai",
            model_id="gpt-5.4",
            reasoning_effort="xhigh",
            api_key_env="OPENAI_API_KEY",
            shell_default_mode="allow",
            session_name="harbor",
            instruction="say hello",
            config_path="/logs/agent/loongclaw-config.toml",
            output_path="/logs/agent/loongclaw.txt",
            trajectory_path="/logs/agent/loongclaw-trajectory.json",
        )

        self.assertIn("loong validate-config", command)
        self.assertIn("loong ask", command)
        self.assertIn("loong trajectory-export", command)
        self.assertIn('export TASK_CWD="$(pwd)"', command)
        self.assertIn(
            "<<'LOONGCLAW_HARBOR_CONFIG_EOF'",
            command,
        )

        validate_position = command.index("loong validate-config")
        ask_position = command.index("loong ask")
        export_position = command.index("loong trajectory-export")

        self.assertLess(validate_position, ask_position)
        self.assertLess(ask_position, export_position)

    def test_sanitize_profile_id_replaces_unsafe_characters(self) -> None:
        profile_id = sanitize_profile_id(" openai.compat [beta] ")

        self.assertEqual(profile_id, "openai_compat_beta")

    def test_sanitize_profile_id_falls_back_when_value_is_empty(self) -> None:
        profile_id = sanitize_profile_id("[] /")

        self.assertEqual(profile_id, "provider")


if __name__ == "__main__":
    unittest.main()
