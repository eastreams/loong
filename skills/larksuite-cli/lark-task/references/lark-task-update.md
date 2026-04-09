# task +update

> **Prerequisites:** Please read `../lark-shared/SKILL.md` to understand authentication, global parameters, and security rules.

Update an existing task in Lark.

## Recommended Commands

```bash
# Update task summary
lark-cli task +update --task-id "t_xxx" --summary "New Summary"

# Update multiple tasks' due dates
lark-cli task +update --task-id "t_xxx,t_yyy" --due "+2d"

# Update with JSON data
lark-cli task +update --task-id "t_xxx" --data '{"description": "New description"}'
```

## Parameters

| Parameter | Required | Description |
|-----------|----------|-------------|
| `--task-id <id>` | Yes | The ID of the task to update. Comma-separated list supported for multiple tasks. |
| `--summary <text>` | No | New summary/title for the task. |
| `--description <text>` | No | New description for the task. |
| `--due <time>` | No | New due date (supports relative time). |
| `--data <json>` | No | JSON payload for fields to update. |

## Workflow

1. Confirm with the user the tasks to update and the fields.
2. Execute `lark-cli task +update --task-id "..." ...`
3. Report the successful updates.

> [!CAUTION]
> This is a **Write Operation** -- You must confirm the user's intent before executing.
