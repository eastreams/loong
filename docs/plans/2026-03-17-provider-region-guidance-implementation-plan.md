## Implementation plan

1. Add failing tests first.
   - `crates/app/src/config/provider.rs`
   - `crates/daemon/src/onboard_cli.rs`
   - `crates/daemon/src/doctor_cli.rs`
   - `crates/app/src/provider/request_executor.rs`

2. Add provider-level region guidance helpers.
   - keep the data local to `config/provider.rs`
   - support current endpoint note + alternate endpoint hint
   - keep Bedrock on its existing templated path

3. Reuse the helpers in operator-facing summaries.
   - onboarding review digest
   - onboarding success summary

4. Reuse the helpers in failure surfaces.
   - onboard model-probe failures
   - doctor model-probe failures
   - doctor next-step generation
   - runtime `401/403` request errors

5. Run targeted verification.
   - unit tests for provider helpers
   - targeted daemon/app tests for changed messaging
   - broader Rust formatting and relevant test suites after targeted green

6. Deliver GitHub artifacts.
   - search for an existing tracking issue first
   - open or reuse an issue in English using the repository template structure
   - push the branch to the fork
   - open a PR against `alpha-test` with template-compliant validation notes and
     a closing clause
