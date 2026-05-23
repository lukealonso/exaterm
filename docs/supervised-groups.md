# Supervised Groups

Supervised groups collapse several worker terminals into one overview card while keeping every worker terminal directly reachable. They are meant for terminal-native coding agents, not for replacing the agents' own CLIs.

## Semantics

A supervised group has:

- one group id and display name
- one or more worker terminal sessions
- an optional supervisor terminal session
- one freeform Markdown summary for the operator
- a small recent action log

In the top-level battlefield, the group occupies one card. Entering the group shows the worker terminals as normal terminal cards. The supervisor terminal is separate from the workers; the group card can toggle between the Markdown summary and the supervisor terminal so the operator can give custom instructions without making the supervisor another top-level worker.

## Supervisor Posture

The supervisor should keep a light touch. Its job is to monitor, prod, redirect, and cross-pollinate when there is evidence that doing so will help.

Default behavior:

- gather state before acting
- prefer terminal evidence, process evidence, recent output, file activity, and explicit worker state over polished guesses
- do nothing when workers are actively exploring productively
- prod stalled workers sparingly
- redirect workers stuck in repeated loops with short, specific guidance
- cross-pollinate useful ideas after another worker stalls or asks for direction, not immediately on every new idea
- preserve diversity across workers
- call out uncertainty instead of inventing certainty

The supervisor should not claim access to hidden model reasoning. It should describe visible evidence and concrete actions.

## MCP Transport

The daemon exposes an MCP endpoint for supervisor agents:

- `EXATERM_MCP_SOCKET`: Unix socket path
- `EXATERM_MCP_TRANSPORT=unix-jsonrpc-lines`

The socket speaks newline-delimited JSON-RPC. The supervisor should use MCP tools as the source of truth for group discovery and interactions.

## MCP Tools

`exaterm_list_groups`

Lists supervised groups and visible workspace items.

`exaterm_get_group`

Reads one group's metadata, worker sessions, deterministic observations, current Markdown summary, and recent supervisor actions.

Arguments:

- `group_id`: integer

`exaterm_send_message_to_agent`

Sends a direct message into one worker terminal. This is for prods, redirects, and cross-pollinated ideas.

Arguments:

- `group_id`: integer
- `session_id`: integer
- `message`: string

`exaterm_update_group_summary`

Replaces the operator-facing group summary. This is intentionally separate from worker messaging.

Arguments:

- `group_id`: integer
- `markdown`: string

## Summary Guidance

The summary is freeform Markdown. There is no fixed schema.

Use whatever structure best helps the operator scan the group. Markdown tables are encouraged when they naturally fit, especially for multi-worker status:

```markdown
| Worker | State | Evidence | Next |
|---|---|---|---|
| 3 | Working | running tests after parser edit | monitor |
| 4 | Stalling | no output or file activity for 8m despite prod | prod sent |
```

Good summaries are short, grounded, and operational. Include concrete evidence such as commands, files, errors, elapsed idle time, recent actions, and what changed since the last update. Avoid verbose report rows, generic encouragement, and certainty that the evidence does not support.

Use group progress states conservatively:

- `Active`: useful work is still moving, including ordinary debugging, compile errors, failing tests, retries, or one worker being stuck while others can continue.
- `Stalling`: despite supervisor efforts, forward progress is not being made.
- `Blocked`: a substantial proportion of the agents cannot proceed at all.
- `Complete`: the group goal is genuinely done.

Do not label the overall group blocked just because a worker hit an error, a test failed, or one lane needs redirection.

## Command Separation

Worker messages and summary updates are separate operations:

- send a message when a worker needs input
- update the summary when the operator-facing picture changes

Do not use summary updates as a side channel for command injection. Do not require a worker message just to refresh the summary.
