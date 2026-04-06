# task +complete

> **Prerequisites:** Please read `../lark-shared/SKILL.md` to understand authentication, global parameters, and security rules.

Mark a task as completed.

## Recommended Commands

```bash
# Complete a task
lark-cli task +complete --task-id "t_xxx"
```

## Parameters

| Parameter | Required | Description |
|-----------|----------|-------------|
| `--task-id <id>` | Yes | The ID of the task to complete. |

## Workflow

1. Confirm the task to complete.
2. Execute the command.
3. Report success.

> [!CAUTION]
> This is a **Write Operation** -- You must confirm the user's intent before executing.
