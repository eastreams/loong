# Bash Exec Basic Tool Design

**Status:** Approved-for-planning draft based on the design decisions already confirmed in this thread.

## Relation to Issue #637

This design is not a full implementation of issue `#637` (`Shell command governance: Bash execution and Bash rule parsing support`).

It is the first implementation slice under that issue:

- implement the experimental `bash.exec` runtime surface
- keep `shell.exec` unchanged
- establish Bash runtime detection and visibility behavior

It intentionally does not implement the governance parts of `#637` yet, including:

- minimal-command-unit splitting
- `prefix_rule(...)`
- rule loading / precedence
- `Default(Allow|Deny)` evaluation for Bash command units

So this design should be read as “basic Bash tool support in service of `#637`”, not as “the complete delivery of `#637`”.

## Goal

Add the first implementation slice for issue `#637`: an experimental `bash.exec` tool that executes one Bash command string through the existing LoongClaw tool pipeline, without changing the current behavior of `shell.exec`.

## Problem

`shell.exec` currently models direct program execution: a command name plus optional argument list. That is useful for tightly governed single-program invocations, but it does not provide a basic Bash-native execution surface for ordinary shell workflows such as:

- `pwd`
- `cd subdir && cargo test`
- `git status; cargo fmt --all -- --check`

The immediate need is not command approval or rule parsing. The immediate need is a separate basic Bash tool that can run a shell command string safely enough to support future governance work, while leaving the existing `shell.exec` behavior untouched.

## Scope

This slice adds only the basic `bash.exec` tool surface.

Included:

- A new discoverable runtime tool named `bash.exec`
- Basic execution of a Bash command string via one Bash process per call
- Basic runtime availability detection for Bash
- Tool visibility behavior that hides `bash.exec` when Bash is unavailable
- A visible warning or log when Bash is unavailable and the tool is hidden
- Tool schema, search metadata, catalog wiring, and focused tests

Excluded:

- Approval flow
- Command governance rules
- AST-based command splitting
- `shell.exec` redirection or behavior changes
- Persistent sessions or cross-call shell state
- Environment-variable mutation semantics
- Pipeline, redirection, or other advanced shell-governance behavior

These exclusions are deliberate deferrals from `#637`, not contradictions of that issue's long-term direction.

## User-Facing Behavior

### Tool Name

The tool is exposed as `bash.exec`.

### Request Shape

The request payload contains:

- `command: string`
- `cwd?: string`
- `timeout_ms?: integer`

`args` is intentionally not part of `bash.exec`. The command is passed as one Bash command string.

### Execution Model

- Each tool call launches a fresh Bash process.
- By default the command runs as a non-login shell, using `bash -c <command>` or the platform-appropriate equivalent for the detected Bash runtime.
- A config-controlled login-shell mode may opt into `bash -lc <command>` when the user explicitly enables it.
- No shell state is preserved across calls.

This makes `bash.exec` suitable for single-shot shell workflows while keeping the implementation bounded.

### Runtime Availability

- If Bash is available on the host, `bash.exec` is exposed.
- If Bash is unavailable, `bash.exec` is hidden from the runtime tool surface.
- Hiding must not be silent. The runtime must emit a visible warning or log that Bash is unavailable and `bash.exec` is disabled.
- Bash availability should be probed when runtime tool policy is built, not only after a failed tool invocation.

On Windows, this requirement is phrased in terms of Bash availability rather than a single hard-coded Bash distribution. The implementation may probe Git Bash first or use another compatible Bash runtime, but the user-facing behavior is simply “Bash available” or “Bash unavailable”.

### Relationship to `shell.exec`

`bash.exec` is an experimental parallel tool. It exists to avoid changing the current `shell.exec` contract while LoongClaw adds basic Bash support.

This slice does not redirect `shell.exec`, merge the tools, or redefine `shell.exec` semantics.

### Result Shape

`bash.exec` should follow the existing core-tool result convention:

- successful execution returns `status = "ok"`
- a non-zero Bash exit code returns `status = "failed"` with captured stdout/stderr
- malformed payloads or runtime setup failures still return executor errors rather than synthetic tool-success payloads

## Architecture

The implementation must preserve the existing kernel-mediated execution path:

`CapabilityToken -> PolicyEngine.authorize(...) -> PolicyExtensionChain -> Execution -> Audit`

`bash.exec` should be wired into the same tool infrastructure used by other discoverable runtime tools:

- tool descriptor / catalog
- runtime tool view
- tool.search metadata
- core tool execution dispatch
- runtime configuration

The initial executor should reuse the existing shell-tool process-execution patterns where reasonable, but it must remain a distinct tool surface with its own request contract.

## Design Constraints

### 1. Separate Tool Boundary

Use a separate `bash.exec` tool now.

Reason:

- It lets LoongClaw add Bash support without silently changing `shell.exec`.
- It keeps the experimental shell-string surface isolated while the direct-exec tool remains stable.

### 2. Single-Shot Bash Execution

Use one fresh Bash process per call.

Reason:

- It is enough for immediate workflows such as `cd foo && cargo test`.
- It avoids dragging persistent shell session semantics into this slice.

### 3. Minimal Request Surface

Expose only:

- `command`
- `cwd`
- `timeout_ms`

Reason:

- This is the smallest interface that still supports useful Bash workflows.
- It avoids premature scope around environment overrides, interactive sessions, PTYs, or background lifecycle management.

### 4. Fail Closed on Missing Runtime

If Bash is unavailable, hide the tool and emit a warning/log.

Reason:

- The model should not be encouraged to call a tool that cannot succeed.
- Operators still need a visible explanation for why the tool is missing.

## Testing Requirements

The first implementation plan must use TDD.

Minimum behaviors to prove with focused tests:

- `bash.exec` is present in the runtime tool surface when Bash is available
- `bash.exec` is hidden when Bash is unavailable
- tool search metadata includes the new tool when available
- the tool rejects invalid payloads
- the tool executes a simple Bash command successfully
- the tool honors `cwd`
- the tool honors `timeout_ms`

## Non-Goals

This design intentionally does not answer:

- how command approval will work
- how Bash rules will be parsed
- how AST-based command splitting will work
- how `shell.exec` and `bash.exec` may converge later

Those belong to later governance-focused work. This slice exists only to establish the basic Bash execution surface that later work can build on.
