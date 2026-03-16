# Product Specs

User-facing product requirements and specifications for LoongClaw.

## Structure

Product specs describe **what** the product does from the user's perspective, not implementation internals or scheduling details.

## Specs

- [Installation](installation.md)
- [Onboarding](onboarding.md)
- [One-Shot Ask](one-shot-ask.md)
- [Doctor](doctor.md)
- [Browser Automation](browser-automation.md)
- [Browser Automation Companion](browser-automation-companion.md)
- [Channel Setup](channel-setup.md)
- [Tool Surface](tool-surface.md)
- [WebChat](webchat.md)
- [Prompt And Personality](prompt-and-personality.md)
- [Memory Profiles](memory-profiles.md)

## Notes

- `Installation`, `Onboarding`, `One-Shot Ask`, `Doctor`, `Browser Automation`, `Tool Surface`, and `Channel Setup` define the shipped first-run and support journey for the current MVP.
- `Browser Automation Companion` and `WebChat` are expectation-setting specs for the next user-facing surfaces. They should not be documented as generally available before the implementation exists.

Template for new specs:

```markdown
# [Feature Name]

## User Story
As a [role], I want [capability] so that [benefit].

## Acceptance Criteria
- [ ] Criterion 1
- [ ] Criterion 2

## Out of Scope
- Item 1
```
