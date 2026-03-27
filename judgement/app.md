# Exaterm

Exaterm is a Linux desktop app for supervising 4 to 8 terminal-native coding agents at once.

Its core job is to give an operator a battlefield view of several live agent sessions, make state and progress legible at a glance, surface enough under-the-covers evidence to verify whether an agent is really making progress, and let the operator jump into a real terminal instantly when intervention is needed.

Judge this product primarily as a multi-agent supervision surface, not as a general terminal emulator, not as a replacement shell for Codex or Claude Code, and not as an IDE. The north star for the product is described in [`docs/ux-spec.md`](../docs/ux-spec.md).

The most important parts of the experience are:

- Whether the main screen reads clearly as a supervision grid for 4 to 8 concurrent sessions rather than a generic pane manager or a wall of tiny terminals.
- Whether each session card makes the agent's state legible at a glance, especially whether it is `Idle`, `Thinking`, `Working`, `Blocked`, or `Failed`.
- Whether `Idle` has strong enough visual salience that a newly idle agent catches attention quickly without the whole interface becoming noisy.
- Whether each card inherently communicates what the agent appears to be doing and what the machine is actually doing without literal labeled report rows.
- Whether command-level evidence such as active subprocesses, file activity, and meaningful output excerpts helps the operator verify whether claimed progress is real.
- Whether the product helps the operator spot mismatch between visible agent narrative and observed execution.
- Whether model-assisted tactical synthesis, when present, makes the cards feel more intelligent and useful rather than more verbose or noisy.
- Whether model assistance appears to be reporting into distinct useful dimensions such as trajectory, confidence, operator action, and risk posture, rather than collapsing everything into one vague status line.
- Whether clicking a card cleanly promotes it into a large, uncluttered real-terminal view for direct intervention.

The workflows that deserve the most evaluation time are:

- Scanning the grid to understand what all visible agents are doing and decide which one matters most right now.
- Noticing when a session becomes idle and deciding whether that idle period is healthy waiting, suspicious inactivity, or a blocked state.
- Comparing a card's visible intent or narrative with its observed activity to see whether they line up.
- Using the under-the-covers evidence on a card to tell whether the agent is really working, confused, stalled, or failing.
- Promoting a selected session into focused terminal control and returning back to the grid without losing orientation.

Supporting surfaces still matter, but less:

- session creation and launch controls
- menus, preferences, and workspace chrome
- theming and visual polish outside the core supervision loop
- any secondary dialogs or controls that do not materially affect scan, verification, or intervention

Quality looks like this:

- The app reads immediately as a commander-style supervision surface for agent runs.
- The grid remains the main working posture for normal use.
- Cards are dense, consistent, and easy to compare across multiple sessions.
- State is readable before the operator reads detailed text.
- `Idle`, `Thinking`, `Working`, `Blocked`, and `Failed` are visually distinct enough to recognize with a quick scan.
- Idle is especially easy to spot because it is often the most actionable normal event.
- Each card provides enough signal about narrative, subprocess activity, file changes, and output to support quick judgment without turning into a labeled report template.
- The operator can tell whether an agent's visible narrative matches reality.
- The app exposes useful under-the-covers evidence without forcing the operator to abandon the main grid.
- Model assistance, if present, improves tactical usefulness by choosing better summaries rather than by adding more copy.
- When model assistance is present, any surfaced one-line justifications feel grounded in real terminal history rather than generic motivational filler.
- Clicking a session for intervention produces a large, comfortable, uncluttered terminal view that feels direct and native.
- Returning from intervention to the battlefield view is coherent and low-friction.
- Dense multi-session layouts remain legible at realistic working window sizes.
- The click-to-intervene gesture is discoverable through ambient cues, selection behavior, or surrounding chrome without resorting to per-card buttons.

Weak quality looks like this:

- The product feels like a generic multiplexer or pane manager rather than a supervision tool.
- The overview behaves like a wall of tiny terminals instead of a clear set of battle cards.
- State is ambiguous or weak enough that the operator must read each card carefully to know what matters.
- Idle sessions are easy to miss.
- Visual treatment makes `Thinking`, `Working`, `Blocked`, and `Failed` blur together.
- The UI shows lots of transcript or chrome but not enough evidence to verify real progress.
- The cards read like structured summaries with literal `Intent`, `Reality`, or `Output` rows instead of tactical states.
- Button-heavy card chrome competes with the battlefield scan loop.
- Whole-card intervention exists in principle but is hard to discover without trial-and-error clicking.
- Model-assisted copy makes the overview more verbose, repetitive, or noisy instead of sharper.
- Model-assisted copy collapses distinct concerns like momentum, confidence, and risk into one mushy sentence.
- It is difficult to tell whether an agent is actually doing work or only narrating work.
- The product requires too much clicking or mode-switching to get from scan to diagnosis to intervention.
- Entering a session for intervention still feels cluttered or indirect rather than like direct terminal control.
- Dense layouts become noisy or structurally hard to parse once several sessions are visible.

When judging this product, spend more time on the core supervision loop than on supporting chrome. The strongest evidence will come from whether the app helps the operator maintain awareness across several agents, catch idleness quickly, verify real work against stated intent, and intervene in the real terminal without losing the battlefield view.

Environment notes:

- This app targets Linux desktop usage.
- Prefer X11 for automation.
- Resize the main window to a realistic multi-session working size before judging layout density, state clarity, or supervision quality.
