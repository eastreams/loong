from __future__ import annotations

import json
import shlex
from collections.abc import Sequence

DEFAULT_CARGO_TARGET_DIR = "/tmp/loong-harbor-target"
LOCAL_BIN_DIR = "$HOME/.local/bin"
LOCAL_CARGO_BIN_DIR = "$HOME/.cargo/bin"
LOCAL_INSTALL_ROOT = "$HOME/.local"
LOCAL_CARGO_ENV_PATH = "$HOME/.cargo/env"
PRIMARY_BIN_NAME = "loong"
LEGACY_BIN_NAME = "loong"
TASK_CWD_ENV_NAME = "TASK_CWD"
TASK_CWD_ENV_EXPR = f"${TASK_CWD_ENV_NAME}"
RUNTIME_CONFIG_HEREDOC_DELIMITER = "LOONG_HARBOR_CONFIG_EOF"


def join_shell_lines(lines: Sequence[str]) -> str:
    joined_lines = "\n".join(lines)
    return joined_lines


def build_local_bin_path(bin_name: str) -> str:
    local_bin_path = f"{LOCAL_BIN_DIR}/{bin_name}"
    return local_bin_path


def sanitize_profile_id(value: str) -> str:
    sanitized_characters: list[str] = []
    last_character_was_underscore = False

    for character in value:
        if character.isalnum():
            sanitized_characters.append(character.lower())
            last_character_was_underscore = False
            continue

        if character in {"-", "_"} and not last_character_was_underscore:
            sanitized_characters.append("_")
            last_character_was_underscore = True
            continue

        if not last_character_was_underscore:
            sanitized_characters.append("_")
            last_character_was_underscore = True

    sanitized_text = "".join(sanitized_characters)
    stripped_text = sanitized_text.strip("_")

    if stripped_text:
        return stripped_text

    return "provider"


def build_cargo_install_line(quoted_daemon_crate_path: str, bin_name: str) -> str:
    install_root = LOCAL_INSTALL_ROOT
    install_line = (
        f'if cargo install --path {quoted_daemon_crate_path} --locked --force '
        f'--root "{install_root}" --bin {bin_name}; then'
    )
    return install_line


def build_symlink_line(source_bin_name: str, target_bin_name: str) -> str:
    source_path = build_local_bin_path(source_bin_name)
    target_path = build_local_bin_path(target_bin_name)
    symlink_line = f'  ln -sf "{source_path}" "{target_path}"'
    return symlink_line


def build_agent_dependency_install_command() -> str:
    lines: list[str] = []

    lines.append("set -euo pipefail")
    lines.append("if [ -f /etc/alpine-release ]; then")
    lines.append(
        "  apk add --no-cache bash curl git build-base pkgconf openssl-dev ca-certificates"
    )
    lines.append("elif command -v apt-get >/dev/null 2>&1; then")
    lines.append("  apt-get update")
    lines.append(
        "  DEBIAN_FRONTEND=noninteractive apt-get install -y curl git build-essential pkg-config libssl-dev ca-certificates"
    )
    lines.append("elif command -v yum >/dev/null 2>&1; then")
    lines.append(
        "  yum install -y curl git gcc gcc-c++ make pkgconfig openssl-devel ca-certificates"
    )
    lines.append("else")
    lines.append(
        '  echo "unsupported package manager: need curl git rust build dependencies" >&2'
    )
    lines.append("  exit 1")
    lines.append("fi")

    command = join_shell_lines(lines)
    return command


def build_agent_install_command(
    source_mount: str,
    cargo_target_dir: str = DEFAULT_CARGO_TARGET_DIR,
) -> str:
    daemon_crate_path = f"{source_mount}/crates/daemon"
    quoted_daemon_crate_path = shlex.quote(daemon_crate_path)
    quoted_cargo_target_dir = shlex.quote(cargo_target_dir)
    primary_bin_path = build_local_bin_path(PRIMARY_BIN_NAME)
    legacy_bin_path = build_local_bin_path(LEGACY_BIN_NAME)
    primary_install_line = build_cargo_install_line(
        quoted_daemon_crate_path=quoted_daemon_crate_path,
        bin_name=PRIMARY_BIN_NAME,
    )
    legacy_install_line = build_cargo_install_line(
        quoted_daemon_crate_path=quoted_daemon_crate_path,
        bin_name=LEGACY_BIN_NAME,
    )
    legacy_to_primary_symlink_line = build_symlink_line(
        source_bin_name=PRIMARY_BIN_NAME,
        target_bin_name=LEGACY_BIN_NAME,
    )

    lines: list[str] = []

    lines.append("set -euo pipefail")
    lines.append(f'export PATH="{LOCAL_BIN_DIR}:{LOCAL_CARGO_BIN_DIR}:$PATH"')
    lines.append("if ! command -v cargo >/dev/null 2>&1; then")
    lines.append("  cargo_install_succeeded=false")
    lines.append("  for attempt in 1 2 3; do")
    lines.append("    if curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal; then")
    lines.append("      cargo_install_succeeded=true")
    lines.append("      break")
    lines.append("    fi")
    lines.append("    sleep $((attempt * 2))")
    lines.append("  done")
    lines.append("  if [ \"$cargo_install_succeeded\" != \"true\" ]; then")
    lines.append('    echo "failed to install cargo with rustup" >&2')
    lines.append("    exit 1")
    lines.append("  fi")
    lines.append("fi")
    lines.append(f'if [ -f "{LOCAL_CARGO_ENV_PATH}" ]; then')
    lines.append(f'  . "{LOCAL_CARGO_ENV_PATH}"')
    lines.append("fi")
    lines.append(f"export CARGO_TARGET_DIR={quoted_cargo_target_dir}")
    lines.append(primary_install_line)
    lines.append(f"  {PRIMARY_BIN_NAME} --version")
    lines.append("  exit 0")
    lines.append("fi")
    lines.append(legacy_install_line)
    lines.append(f'  if [ ! -e "{primary_bin_path}" ]; then')
    lines.append(legacy_to_primary_symlink_line)
    lines.append("  fi")
    lines.append(f"  {PRIMARY_BIN_NAME} --version")
    lines.append("  exit 0")
    lines.append("fi")
    lines.append(f'if [ -x "{primary_bin_path}" ]; then')
    lines.append(f"  {PRIMARY_BIN_NAME} --version")
    lines.append("  exit 0")
    lines.append("fi")
    lines.append(f'if [ -x "{legacy_bin_path}" ]; then')
    lines.append(legacy_to_primary_symlink_line)
    lines.append(f"  {PRIMARY_BIN_NAME} --version")
    lines.append("  exit 0")
    lines.append("fi")
    lines.append(
        f'echo "failed to install {PRIMARY_BIN_NAME} from source mount {source_mount}" >&2'
    )
    lines.append("exit 1")

    command = join_shell_lines(lines)
    return command


def build_runtime_config_text(
    profile_id: str,
    provider_kind: str,
    model_id: str,
    reasoning_effort: str,
    api_key_env: str,
    shell_default_mode: str,
) -> str:
    lines: list[str] = []

    serialized_profile_id = json.dumps(profile_id)
    serialized_provider_kind = json.dumps(provider_kind)
    serialized_model_id = json.dumps(model_id)
    serialized_reasoning_effort = json.dumps(reasoning_effort)
    serialized_api_key_env = json.dumps(api_key_env)
    serialized_shell_default_mode = json.dumps(shell_default_mode)
    serialized_file_root = json.dumps(TASK_CWD_ENV_EXPR)
    serialized_profile_section_key = json.dumps(profile_id)

    lines.append(f"active_provider = {serialized_profile_id}")
    lines.append("")
    lines.append(f"[providers.{serialized_profile_section_key}]")
    lines.append(f"kind = {serialized_provider_kind}")
    lines.append(f"model = {serialized_model_id}")
    lines.append(f"reasoning_effort = {serialized_reasoning_effort}")
    lines.append(f"api_key = {{ env = {serialized_api_key_env} }}")
    lines.append("")
    lines.append("[tools]")
    lines.append(f"file_root = {serialized_file_root}")
    lines.append(f"shell_default_mode = {serialized_shell_default_mode}")
    lines.append("")
    lines.append("[tools.bash]")
    lines.append("login_shell = false")

    config_text = "\n".join(lines)
    return config_text


def build_agent_run_command(
    profile_id: str,
    provider_kind: str,
    model_id: str,
    reasoning_effort: str,
    api_key_env: str,
    shell_default_mode: str,
    session_name: str,
    instruction: str,
    config_path: str,
    output_path: str,
    trajectory_path: str,
) -> str:
    runtime_config_text = build_runtime_config_text(
        profile_id=profile_id,
        provider_kind=provider_kind,
        model_id=model_id,
        reasoning_effort=reasoning_effort,
        api_key_env=api_key_env,
        shell_default_mode=shell_default_mode,
    )

    quoted_config_path = shlex.quote(config_path)
    quoted_output_path = shlex.quote(output_path)
    quoted_trajectory_path = shlex.quote(trajectory_path)
    quoted_instruction = shlex.quote(instruction)
    quoted_session_name = shlex.quote(session_name)
    validate_config_command = f"loong validate-config --config {quoted_config_path}"
    ask_command = (
        f"loong ask --config {quoted_config_path} --session "
        f"{quoted_session_name} --message {quoted_instruction} 2>&1 | tee "
        f"{quoted_output_path}"
    )
    export_trajectory_command = (
        f"if ! loong trajectory-export --config {quoted_config_path} --session "
        f"{quoted_session_name} --output {quoted_trajectory_path}; then"
    )
    export_warning_line = f'  echo "warning: {PRIMARY_BIN_NAME} trajectory-export failed" >&2'

    lines: list[str] = []

    lines.append("set -euo pipefail")
    lines.append(f'export PATH="{LOCAL_BIN_DIR}:{LOCAL_CARGO_BIN_DIR}:$PATH"')
    lines.append(f'export {TASK_CWD_ENV_NAME}="$(pwd)"')
    lines.append(
        f"cat > {quoted_config_path} <<'{RUNTIME_CONFIG_HEREDOC_DELIMITER}'"
    )
    lines.append(runtime_config_text)
    lines.append(RUNTIME_CONFIG_HEREDOC_DELIMITER)
    lines.append(validate_config_command)
    lines.append(ask_command)
    lines.append(export_trajectory_command)
    lines.append(export_warning_line)
    lines.append("fi")

    command = join_shell_lines(lines)
    return command
