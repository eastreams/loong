# Channel Serve Ownership Phase 13 Design

**Scope**

Phase 13 adds a shared serve-start ownership gate for long-lived channel workers.

Phase 12 extracted shared command-context construction and shared serve runtime
startup/shutdown. What still remains unsafe is duplicate startup:

- the same Telegram account can be launched twice
- the same Feishu/Lark webhook account can be launched twice
- runtime status can report duplicates, but startup still allows them

**Problem Statement**

Duplicate long-lived channel workers are not just noisy. They create real
behavioral hazards:

- duplicate Telegram polling can race ack offsets
- duplicate Feishu webhook servers can create routing ambiguity
- operators get warned only after duplicate state exists

The runtime state layer already knows how to summarize running vs. stale
instances. The missing piece is to use that information as a startup gate.

**Chosen Design**

Add a shared ownership check inside the serve-runtime wrapper:

1. Before starting a new runtime tracker, inspect account-scoped runtime state
   for the same platform + operation + account.
2. If an active running instance exists, reject startup.
3. If only stale or stopped instances exist, allow startup.

The ownership gate should be shared for Telegram and Feishu serve flows by
living inside `with_channel_serve_runtime(...)`.

**Behavior**

- reject when a running instance already exists for the same account/operation
- include account id, pid when available, and running instance count in the
  error
- allow takeover when the previous instance is stale

**Why This Matters**

This is the first real supervisor policy, not just shared plumbing.

It upgrades the runtime wrapper from:

- "start a tracker around a serve body"

to:

- "enforce single active ownership for a serve slot"

That is the correct next increment before adding restart/backoff and richer
supervisor state transitions.
