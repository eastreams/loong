#!/usr/bin/env bash

have_rg() {
  command -v rg >/dev/null 2>&1
}

count_pattern_in_file() {
  local pattern="$1"
  local file="$2"
  if have_rg; then
    rg -n "$pattern" "$file" | wc -l | tr -d '[:space:]'
  else
    { grep -En "$pattern" "$file" || true; } | wc -l | tr -d '[:space:]'
  fi
}

architecture_hotspot_keys() {
  cat <<'HOTSPOTS'
spec_runtime
spec_execution
provider_mod
memory_mod
HOTSPOTS
}

architecture_hotspot_spec() {
  case "$1" in
    spec_runtime)
      echo "crates/spec/src/spec_runtime.rs|3600|65"
      ;;
    spec_execution)
      echo "crates/spec/src/spec_execution.rs|3700|80"
      ;;
    provider_mod)
      echo "crates/app/src/provider/mod.rs|1000|20"
      ;;
    memory_mod)
      echo "crates/app/src/memory/mod.rs|650|16"
      ;;
    *)
      return 1
      ;;
  esac
}

architecture_file_line_count() {
  local file="$1"
  wc -l <"$file" | tr -d '[:space:]'
}

architecture_file_function_count() {
  local file="$1"
  count_pattern_in_file '^(pub[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+' "$file"
}

architecture_hotspot_rows() {
  local key spec file max_lines max_functions lines functions line_status fn_status
  while IFS= read -r key; do
    [[ -z "$key" ]] && continue
    spec="$(architecture_hotspot_spec "$key")" || return 1
    IFS='|' read -r file max_lines max_functions <<EOF_SPEC
$spec
EOF_SPEC
    if [[ ! -f "$file" ]]; then
      echo "missing hotspot file: $file" >&2
      return 1
    fi
    lines="$(architecture_file_line_count "$file")"
    functions="$(architecture_file_function_count "$file")"
    line_status="ok"
    fn_status="ok"
    if (( lines > max_lines )); then
      line_status="over"
    fi
    if (( functions > max_functions )); then
      fn_status="over"
    fi
    printf '%s|%s|%s|%s|%s|%s|%s|%s\n' \
      "$key" "$file" "$lines" "$max_lines" "$line_status" "$functions" "$max_functions" "$fn_status"
  done <<EOF_KEYS
$(architecture_hotspot_keys)
EOF_KEYS
}

architecture_boundary_check_keys() {
  cat <<'BOUNDARIES'
memory_literals
provider_mod_helper_definitions
spec_app_dependency
BOUNDARIES
}

architecture_memory_literal_hits() {
  if have_rg; then
    rg -n '"append_turn"|"window"|"clear_session"' crates/app/src --glob '!crates/app/src/memory/**' || true
  else
    grep -REn '"append_turn"|"window"|"clear_session"' crates/app/src --exclude-dir=memory || true
  fi
}

architecture_provider_mod_helper_definition_hits() {
  local file="crates/app/src/provider/mod.rs"
  if have_rg; then
    rg -n 'fn[[:space:]]+(build_completion_request_body|build_turn_request_body|parse_provider_api_error|extract_message_content|adapt_payload_mode_for_error|should_disable_tool_schema_for_error|should_try_next_model_on_error)\b' "$file" || true
  else
    grep -En 'fn[[:space:]]+(build_completion_request_body|build_turn_request_body|parse_provider_api_error|extract_message_content|adapt_payload_mode_for_error|should_disable_tool_schema_for_error|should_try_next_model_on_error)\b' "$file" || true
  fi
}

architecture_spec_app_dependency_hits() {
  local file="crates/spec/Cargo.toml"
  if have_rg; then
    rg -n '^loongclaw-app[[:space:]]*=' "$file" || true
  else
    grep -En '^loongclaw-app[[:space:]]*=' "$file" || true
  fi
}

architecture_boundary_pass_summary() {
  case "$1" in
    memory_literals)
      echo "memory operation literals are centralized in crates/app/src/memory/*"
      ;;
    provider_mod_helper_definitions)
      echo "provider/mod.rs keeps payload, parse, and recovery helper implementations outside the top-level module"
      ;;
    spec_app_dependency)
      echo "spec crate remains detached from app crate at the Cargo dependency boundary"
      ;;
    *)
      return 1
      ;;
  esac
}

architecture_boundary_fail_summary() {
  case "$1" in
    memory_literals)
      echo "memory operation literals found outside memory module boundary"
      ;;
    provider_mod_helper_definitions)
      echo "provider/mod.rs still defines payload, parse, or recovery helpers directly"
      ;;
    spec_app_dependency)
      echo "spec crate depends on app crate directly"
      ;;
    *)
      return 1
      ;;
  esac
}

architecture_boundary_hits() {
  case "$1" in
    memory_literals)
      architecture_memory_literal_hits
      ;;
    provider_mod_helper_definitions)
      architecture_provider_mod_helper_definition_hits
      ;;
    spec_app_dependency)
      architecture_spec_app_dependency_hits
      ;;
    *)
      return 1
      ;;
  esac
}

architecture_boundary_status() {
  local key="$1"
  local hits
  hits="$(architecture_boundary_hits "$key")"
  if [[ -n "$hits" ]]; then
    echo "FAIL"
  else
    echo "PASS"
  fi
}

architecture_boundary_detail_single_line() {
  local key="$1"
  local hits
  hits="$(architecture_boundary_hits "$key")"
  if [[ -z "$hits" ]]; then
    architecture_boundary_pass_summary "$key"
    return 0
  fi
  printf '%s\n' "$hits" | paste -sd ';' - | sed 's/;/; /g'
}
