# Browser Companion Preview

Use this managed skill when a task needs richer browser automation through the
`agent-browser` CLI than the built-in `browser.open`, `browser.extract`, and
`browser.click` tools can reliably provide.

This preview is loaded through `external_skills.invoke` and currently routes
work through `exec`. It does not yet provide the same bounded,
profile-isolated safety model as the built-in browser tools.

## Preconditions

- This preview expects the `agent-browser` CLI to be installed and available on
  `PATH`.
- This preview uses `exec` to call `agent-browser`, so shell policy must allow
  the `agent-browser` command.
- Enabling this preview usually means the operator has also enabled the
  external-skills runtime with installed-skill auto exposure for the current
  config.
- If those prerequisites are missing, say so plainly and stop instead of
  pretending the browser task completed.

## Operating Rules

1. Treat `agent-browser` as the execution adapter for multi-step page work.
2. Keep the user informed with short progress updates before major browser
   actions.
3. Use the ref-based workflow:
   - `agent-browser open <url>`
   - `agent-browser snapshot -i`
   - interact with `click`, `fill`, `select`, `press`, or `scroll`
   - re-run `agent-browser snapshot -i` after navigation or DOM changes
4. For extraction tasks, prefer:
   - `agent-browser get text body`
   - `agent-browser get text @eN`
   - `agent-browser screenshot --full`
5. If a task needs login, 2FA, arbitrary JavaScript execution, destructive form
   submission, or unsupported browser state management, explain that this is a
   limited preview and stop for operator approval instead of improvising.

## Command Patterns

```text
agent-browser open https://example.com
agent-browser snapshot -i
agent-browser click @e3
agent-browser wait --load networkidle
agent-browser snapshot -i
agent-browser get text body
agent-browser screenshot --full
```

## Response Style

- Summarize what page or step you are on.
- Surface blockers immediately.
- Do not dump raw browser output unless the user asks for it.
