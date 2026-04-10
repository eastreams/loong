# task +tasklist-task-add

> **Prerequisites:** Please read `../lark-shared/SKILL.md` to understand authentication, global parameters, and security rules.

Add existing tasks to a tasklist.

## Recommended Commands

```bash
# Add a single task to a tasklist
lark-cli task +tasklist-task-add --tasklist-id "tl_xxx" --task-id "t_aaa"

# Add multiple tasks to a tasklist
lark-cli task +tasklist-task-add --tasklist-id "tl_xxx" --task-id "t_aaa,t_bbb,t_ccc"
```

## Parameters

| Parameter | Required | Description |
|-----------|----------|-------------|
| `--tasklist-id <id>` | Yes | The GUID of the tasklist, or a full AppLink URL. |
| `--task-id <ids>` | Yes | Comma-separated list of task IDs to add to the tasklist. |

## Workflow

1. Confirm the tasklist and the tasks to add.
2. Execute the command `lark-cli task +tasklist-task-add ...`.
3. Report the result (successful vs failed tasks).

> [!CAUTION]
> This is a **Write Operation** -- You must confirm the user's intent before executing.
