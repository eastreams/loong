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
- [Runtime Experiment](runtime-experiment.md)
- [Runtime Capability](runtime-capability.md)
- [Local Product Control Plane](local-product-control-plane.md)
- [Web UI](web-ui.md)
- [Prompt And Personality](prompt-and-personality.md)
- [Background Tasks](background-tasks.md)
- [Skills Discovery](skills-discovery.md)
- [Memory Profiles](memory-profiles.md)
- [Memory Retrieval](memory-retrieval.md)
- [Shell Completion](shell-completion.md)

## Notes

- `Installation`, `Onboarding`, `One-Shot Ask`, `Doctor`, `Browser Automation`, `Tool Surface`, and `Channel Setup` define the shipped first-run and support journey for the current MVP.
- `Runtime Experiment` defines the shipped local experiment-record surface layered on top of runtime snapshot and restore artifacts.
- `Runtime Capability` defines the shipped local capability-candidate review surface layered on top of runtime experiment artifacts.
- `Local Product Control Plane` defines the shared localhost-only product substrate that future HTTP and Web UI surfaces must consume instead of inventing a second runtime.
- `Background Tasks`, `Skills Discovery`, and `Memory Retrieval` define the next operator-facing productization surfaces that should be built on top of already-shipped runtime substrate.
- `Browser Automation Companion` and `Web UI` are expectation-setting specs for the next user-facing surfaces. They should not be documented as generally available before the implementation exists.

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
