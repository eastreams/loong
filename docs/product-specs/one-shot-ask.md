# One-Shot Ask

## User Story

As a first-time or script-oriented user, I want a one-shot assistant command so
that I can get an answer immediately without entering an interactive shell.

## Acceptance Criteria

- [ ] LoongClaw exposes `loong ask --message "..."` as a first-class CLI
      command.
- [ ] `ask` reuses the same config load, provider routing, memory behavior, and
      ACP options as CLI chat.
- [ ] `ask` rejects empty or whitespace-only messages before starting a turn.
- [ ] `ask` prints one assistant response and exits without REPL-only prompts or
      slash-command behavior.
- [ ] Onboarding and `doctor` can both promote a concrete `ask` example as the
      first visible success path for a healthy local setup.
- [ ] When surfaced outside `ask` itself, the one-shot handoff is labeled in
      product-facing language such as "first answer" rather than only as a
      technical example.
- [ ] `ask` is not interrupted by optional operator-personalization flows; any
      personalization suggestions stay outside the one-shot path.
- [ ] `ask` help text points users toward `loong chat` for interactive
      follow-up.

## Out of Scope

- Interactive history navigation
- REPL slash commands
- A separate one-shot runtime distinct from chat
