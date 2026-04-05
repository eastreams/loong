# task +assign

> **Prerequisites:** Please read `../lark-shared/SKILL.md` to understand authentication, global parameters, and security rules.

Assign or remove members (assignees) from a task.

## Recommended Commands

```bash
# Add an assignee
lark-cli task +assign --task-id "t_xxx" --add "ou_aaa"

# Transfer an assignee (remove old, add new)
lark-cli task +assign --task-id "t_xxx" --remove "ou_old" --add "ou_new"

# Add multiple assignees
lark-cli task +assign --task-id "t_xxx" --add "ou_aaa,ou_bbb"
```

## Parameters

| Parameter | Required | Description |
|-----------|----------|-------------|
| `--task-id <id>` | Yes | The ID of the task to modify. |
| `--add <ids>` | No | Comma-separated list of user `open_id`s to add as assignees. |
| `--remove <ids>` | No | Comma-separated list of user `open_id`s to remove from assignees. |

## Workflow

1. Confirm the task and members to add/remove.
2. Execute the command.
3. Report success and the new count of assignees.

> [!CAUTION]
> This is a **Write Operation** -- You must confirm the user's intent before executing.
EOF~