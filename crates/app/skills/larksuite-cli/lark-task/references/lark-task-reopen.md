# task +reopen

> **Prerequisites:** Please read `../lark-shared/SKILL.md` to understand authentication, global parameters, and security rules.

Reopen a previously completed task.

## Recommended Commands

```bash
# Reopen a task
lark-cli task +reopen --task-id "t_xxx"
```

## Parameters

| Parameter | Required | Description |
|-----------|----------|-------------|
| `--task-id <id>` | Yes | The ID of the task to reopen. |

## Workflow

1. Confirm the task to reopen.
2. Execute the command.
3. Report success.

> [!CAUTION]
> This is a **Write Operation** -- You must confirm the user's intent before executing.
