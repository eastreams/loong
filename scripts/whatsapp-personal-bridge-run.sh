#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BRIDGE_DIR="$ROOT_DIR/runtime-plugins/whatsapp-personal-bridge"
CONFIG_PATH="${LOONG_CONFIG_PATH:-$HOME/.loong/config.toml}"
ACCOUNT_ID=""
SKIP_INSTALL=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --config)
      CONFIG_PATH="$2"
      shift 2
      ;;
    --account)
      ACCOUNT_ID="$2"
      shift 2
      ;;
    --skip-install)
      SKIP_INSTALL=1
      shift
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ ! -f "$CONFIG_PATH" ]]; then
  echo "config file not found: $CONFIG_PATH" >&2
  exit 1
fi

export LOONG_WHATSAPP_PERSONAL_BRIDGE_ROOT="$BRIDGE_DIR"
export LOONG_WHATSAPP_PERSONAL_RUNTIME_PLUGINS_ROOT="$ROOT_DIR/runtime-plugins"
export LOONG_WHATSAPP_PERSONAL_BRIDGE_CONFIG="$CONFIG_PATH"
export LOONG_WHATSAPP_PERSONAL_BRIDGE_ACCOUNT="$ACCOUNT_ID"

SETTINGS_ENV="$({ python3 - <<'PY'
from pathlib import Path
from urllib.parse import urlparse
import os, shlex, tomllib

config_path = Path(os.environ['LOONG_WHATSAPP_PERSONAL_BRIDGE_CONFIG'])
account_arg = os.environ.get('LOONG_WHATSAPP_PERSONAL_BRIDGE_ACCOUNT', '').strip()
config = tomllib.loads(config_path.read_text())
block = dict(config.get('whatsapp_personal') or {})
accounts = block.get('accounts') or {}

if account_arg:
    configured_account_id = account_arg
else:
    configured_account_id = (block.get('default_account') or '').strip() or 'default'

account = dict(accounts.get(configured_account_id) or {})

def resolve_value(key: str, env_key: str | None, default: str | None = None):
    if key in account and account[key] not in (None, ''):
        return account[key]
    if key in block and block[key] not in (None, ''):
        return block[key]
    env_pointer = None
    if env_key and account.get(env_key):
        env_pointer = str(account[env_key]).strip()
    elif env_key and block.get(env_key):
        env_pointer = str(block[env_key]).strip()
    if env_pointer:
        env_value = os.getenv(env_pointer)
        if env_value:
            return env_value
    return default

bridge_url = resolve_value('bridge_url', 'bridge_url_env', 'http://127.0.0.1:39731/bridge')
auth_dir = resolve_value(
    'auth_dir',
    'auth_dir_env',
    str(Path.home() / '.loong' / 'whatsapp-personal' / configured_account_id),
)
parsed = urlparse(bridge_url)
host = parsed.hostname or '127.0.0.1'
port = parsed.port or (443 if parsed.scheme == 'https' else 39731)
path = parsed.path or '/bridge'
roots = [str(value).strip() for value in (config.get('runtime_plugins', {}).get('roots') or []) if str(value).strip()]
bridge_root = str(Path(os.environ['LOONG_WHATSAPP_PERSONAL_BRIDGE_ROOT']))
runtime_plugins_root = str(Path(os.environ['LOONG_WHATSAPP_PERSONAL_RUNTIME_PLUGINS_ROOT']))
root_configured = any(value in ('.', './runtime-plugins', 'runtime-plugins', bridge_root, runtime_plugins_root) for value in roots)
for key, value in {
    'ACCOUNT_ID': configured_account_id,
    'HOST': host,
    'PORT': str(port),
    'PATH_NAME': path,
    'AUTH_DIR': auth_dir,
    'ROOT_CONFIGURED': '1' if root_configured else '0',
}.items():
    print(f'{key}={shlex.quote(value)}')
PY
} )"

eval "$SETTINGS_ENV"

if [[ $SKIP_INSTALL -ne 1 && ! -d "$BRIDGE_DIR/node_modules/@whiskeysockets/baileys" ]]; then
  echo "Installing WhatsApp Personal bridge dependencies in $BRIDGE_DIR ..."
  (cd "$BRIDGE_DIR" && npm install --no-fund --no-audit)
fi

if [[ "$ROOT_CONFIGURED" != "1" ]]; then
  cat >&2 <<MSG
warning: runtime_plugins.roots does not appear to include ./runtime-plugins yet.
Add this to your config for loong channels send/serve whatsapp-personal:

[runtime_plugins]
enabled = true
roots = ["./runtime-plugins"]
MSG
fi

echo "Starting WhatsApp Personal bridge"
echo "  account : $ACCOUNT_ID"
echo "  listen  : http://$HOST:$PORT$PATH_NAME"
echo "  auth_dir: $AUTH_DIR"
echo

exec node "$BRIDGE_DIR/bridge.mjs" --host "$HOST" --port "$PORT" --path "$PATH_NAME" --auth-dir "$AUTH_DIR"
