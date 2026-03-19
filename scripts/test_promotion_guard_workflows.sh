#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

assert_guard_condition() {
  local workflow_path="$1"
  local job_name="$2"
  local expected_ref="$3"

  ruby - "$workflow_path" "$job_name" "$expected_ref" <<'RUBY'
require "yaml"

workflow_path, job_name, expected_ref = ARGV
workflow = YAML.load_file(workflow_path)
condition = workflow.fetch("jobs").fetch(job_name).fetch("if")

required_fragments = [
  "github.event.pull_request.head.repo.full_name != github.repository",
  "github.event.pull_request.head.ref != '#{expected_ref}'"
]

missing = required_fragments.reject { |fragment| condition.include?(fragment) }
unless missing.empty?
  warn "workflow #{workflow_path} is missing required guard fragments: #{missing.join(', ')}"
  warn "actual condition: #{condition}"
  exit 1
end
RUBY
}

cd "$REPO_ROOT"

assert_guard_condition ".github/workflows/enforce-dev-to-main.yml" "block-non-dev-source" "dev"
assert_guard_condition ".github/workflows/enforce-main-to-release.yml" "block-non-main-source" "main"

echo "promotion guard workflow checks passed"
