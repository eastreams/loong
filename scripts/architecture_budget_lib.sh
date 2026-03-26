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
  local row
  local key

  while IFS= read -r row; do
    [[ -z "$row" ]] && continue
    IFS='|' read -r key _file _max_lines _max_functions _classes <<EOF_ROW
$row
EOF_ROW
    printf '%s\n' "$key"
  done <<EOF_ROWS
$(architecture_hotspot_metadata_rows)
EOF_ROWS
}

architecture_hotspot_metadata_rows() {
  cat <<'HOTSPOTS'
spec_runtime|crates/spec/src/spec_runtime.rs|3600|65|foundation
spec_execution|crates/spec/src/spec_execution.rs|3700|80|foundation
provider_mod|crates/app/src/provider/mod.rs|1000|20|foundation
memory_mod|crates/app/src/memory/mod.rs|650|16|foundation
acp_manager|crates/app/src/acp/manager.rs|3600|12|operational_density
acpx_runtime|crates/app/src/acp/acpx.rs|2800|65|operational_density
channel_registry|crates/app/src/channel/registry.rs|10000|90|structural_size
channel_config|crates/app/src/config/channels.rs|9000|90|structural_size
chat_runtime|crates/app/src/chat.rs|7300|160|structural_size,operational_density
channel_mod|crates/app/src/channel/mod.rs|6400|110|structural_size,operational_density
turn_coordinator|crates/app/src/conversation/turn_coordinator.rs|11200|120|structural_size,operational_density
tools_mod|crates/app/src/tools/mod.rs|15000|70|structural_size
daemon_lib|crates/daemon/src/lib.rs|6000|190|structural_size
onboard_cli|crates/daemon/src/onboard_cli.rs|9800|250|structural_size
HOTSPOTS
}

architecture_hotspot_metadata_for_key() {
  local requested_key="$1"
  local row
  local key

  while IFS= read -r row; do
    [[ -z "$row" ]] && continue
    IFS='|' read -r key _file _max_lines _max_functions _classes <<EOF_ROW
$row
EOF_ROW
    if [[ "$key" == "$requested_key" ]]; then
      printf '%s\n' "$row"
      return 0
    fi
  done <<EOF_ROWS
$(architecture_hotspot_metadata_rows)
EOF_ROWS

  return 1
}

architecture_hotspot_spec() {
  local requested_key="$1"
  local metadata_row
  local key
  local file
  local max_lines
  local max_functions
  local classes

  metadata_row="$(architecture_hotspot_metadata_for_key "$requested_key")" || return 1
  IFS='|' read -r key file max_lines max_functions classes <<EOF_ROW
$metadata_row
EOF_ROW

  printf '%s|%s|%s\n' "$file" "$max_lines" "$max_functions"
}

architecture_hotspot_classes() {
  local requested_key="$1"
  local metadata_row
  local key
  local file
  local max_lines
  local max_functions
  local classes

  metadata_row="$(architecture_hotspot_metadata_for_key "$requested_key")" || return 1
  IFS='|' read -r key file max_lines max_functions classes <<EOF_ROW
$metadata_row
EOF_ROW

  printf '%s\n' "$classes"
}

architecture_file_line_count() {
  local file="$1"
  wc -l <"$file" | tr -d '[:space:]'
}

architecture_file_function_count() {
  local file="$1"
  count_pattern_in_file '^(pub[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+' "$file"
}

architecture_hotspot_peak_usage_percent() {
  local lines="$1"
  local max_lines="$2"
  local functions="$3"
  local max_functions="$4"
  awk -v lines="$lines" -v max_lines="$max_lines" -v functions="$functions" -v max_functions="$max_functions" '
    BEGIN {
      line_pct = (lines / max_lines) * 100
      fn_pct = (functions / max_functions) * 100
      peak_pct = line_pct
      if (fn_pct > peak_pct) {
        peak_pct = fn_pct
      }
      printf "%.1f%%", peak_pct
    }
  '
}

architecture_hotspot_pressure() {
  local lines="$1"
  local max_lines="$2"
  local functions="$3"
  local max_functions="$4"

  if (( lines > max_lines || functions > max_functions )); then
    echo "BREACH"
    return 0
  fi

  if (( lines * 100 >= max_lines * 95 || functions * 100 >= max_functions * 95 )); then
    echo "TIGHT"
    return 0
  fi

  if (( lines * 100 >= max_lines * 85 || functions * 100 >= max_functions * 85 )); then
    echo "WATCH"
    return 0
  fi

  echo "HEALTHY"
}

architecture_hotspot_rows() {
  local metadata_row
  local key
  local file
  local classes
  local max_lines
  local max_functions
  local lines
  local functions
  local line_status
  local fn_status
  local peak_usage
  local pressure

  while IFS= read -r metadata_row; do
    [[ -z "$metadata_row" ]] && continue
    IFS='|' read -r key file max_lines max_functions classes <<EOF_ROW
$metadata_row
EOF_ROW
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
    peak_usage="$(architecture_hotspot_peak_usage_percent "$lines" "$max_lines" "$functions" "$max_functions")"
    pressure="$(architecture_hotspot_pressure "$lines" "$max_lines" "$functions" "$max_functions")"
    printf '%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s\n' \
      "$key" "$file" "$classes" "$lines" "$max_lines" "$line_status" "$functions" "$max_functions" "$fn_status" \
      "$peak_usage" "$pressure"
  done <<EOF_ROWS
$(architecture_hotspot_metadata_rows)
EOF_ROWS
}

architecture_boundary_check_keys() {
  cat <<'BOUNDARIES'
memory_literals
provider_mod_helper_definitions
conversation_provider_optional_binding_roundtrip
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

architecture_conversation_provider_optional_binding_roundtrip_hits() {
  local file="crates/app/src/conversation/runtime.rs"
  if have_rg; then
    rg -n 'ProviderRuntimeBinding::from_optional_kernel_context' "$file" || true
  else
    grep -En 'ProviderRuntimeBinding::from_optional_kernel_context' "$file" || true
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
    conversation_provider_optional_binding_roundtrip)
      echo "conversation/runtime.rs translates explicit conversation bindings into provider bindings without optional-kernel roundtrips"
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
    conversation_provider_optional_binding_roundtrip)
      echo "conversation/runtime.rs still rebuilds provider bindings from optional kernel context"
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
    conversation_provider_optional_binding_roundtrip)
      architecture_conversation_provider_optional_binding_roundtrip_hits
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
