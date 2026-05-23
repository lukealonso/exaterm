# Exaterm UX Vision

## North Star

Exaterm is a host-backed Linux desktop app for supervising terminal-native coding agents.

Its job is not to replace Codex, Claude Code, or a normal terminal workflow. Its job is to keep several real terminal sessions legible, persistent, and easy to intervene in while a supervisor agent can summarize and coordinate a group when the operator asks for that structure.

The core promise is:

- terminals stay terminals
- fixed grids keep the workspace tidy without manual pane management
- sessions persist and reconnect through Hosts
- supervised groups summarize multi-agent work without hiding the worker terminals forever
- Ctrl-K gives fast command-line help in the current terminal

For the supervised-group and supervisor MCP contract, see [supervised-groups.md](supervised-groups.md).

## Product Definition

Exaterm is:

- terminal-first
- grid-first
- host-backed in normal operation
- built for local or remote Linux hosts
- designed for supervising terminal-native coding agents without replacing their CLIs

Exaterm is not:

- a new agent shell
- an IDE
- a generic pane manager
- a dashboard-heavy terminal multiplexer
- a product that claims access to hidden model reasoning
- a per-session AI summary surface

Core design rule: direct terminal control is the default, and supervision is layered on top only where it earns its keep.

## Main Screen

The main screen is a fixed, adaptive grid of workspace items.

Workspace items are:

- terminal tiles
- supervised group cards

A terminal tile always contains a real terminal surface. It must not silently collapse into a summary card, scrollback preview, tab rail, or top-rail proxy.

A supervised group card represents several worker terminals and an optional supervisor terminal. The card is the only overview card in the main model. It is not a replacement for every session.

Recommended layout behavior:

- 1 terminal: full terminal-first surface
- 2 terminals: terminal-first split when space allows
- 3 or more items: fixed grid with stable cells
- group contents: clicking into a group shows its member terminals as normal terminal tiles

The grid should prefer predictable equal sizing over freeform pane management. The operator should not have to maintain tidy layouts by hand.

## Supervised Groups

A supervised group has:

- 1 to N worker terminals
- an optional supervisor terminal
- a Markdown summary owned by the supervisor
- recent supervisor actions
- a toggle that reveals the supervisor terminal inside the group card

Top-level behavior:

- the group occupies one card in the grid
- worker terminals outside the group remain normal terminal tiles
- entering the group expands its members into normal terminal tiles
- returning to the top-level grid restores the group card

The supervisor terminal is not another top-level worker by default. It becomes visible only when the user toggles the group card into supervisor-terminal mode.

## Supervisor Summary

The group summary is Markdown produced by the supervisor agent.

It should be:

- grounded in visible terminal evidence and explicit worker state
- brief enough to scan
- allowed to use natural Markdown tables when useful
- free-form, not locked to a fixed schema

The supervisor protocol intentionally separates:

- sending a message to a worker terminal
- updating the user-facing Markdown summary

Command injection must not be coupled to summary updates. A supervisor can ask a worker to continue, inspect, test, or report back without changing the summary. It can also update the summary without sending terminal input.

## MCP Contract

The supervisor uses MCP tools exposed by the host session service.

Expected tools:

- `exaterm_list_groups`: discover supervised groups and workspace items
- `exaterm_get_group`: inspect a group, members, observations, current summary, and recent supervisor actions
- `exaterm_send_message_to_agent`: send text to a specific worker terminal in the group
- `exaterm_update_group_summary`: replace the Markdown summary shown on the group card

The MCP layer is the right place to expose deeper deterministic evidence such as recent terminal history, process hints, and file activity when trustworthy.

The normal UI protocol should stay sparse. It should not publish old per-session tactical summaries, title suggestions, auto-nudge state, or process/file spying payloads just to decorate tiles.

## Ctrl-K Terminal Assist

Ctrl-K opens a prompt scoped to the current terminal or selected terminal.

The user asks for terminal help in plain English, for example:

- "Find how much disk space I'm using"
- "Show large files under this repo"
- "Run the smallest useful test for this crate"

The assistant returns insertable terminal text. It should favor safe, inspectable commands and avoid destructive commands unless the user explicitly asks for them.

Completion behavior:

- insert text into the target terminal
- do not automatically press Enter unless the product intentionally adds that later
- use recent terminal history and internal deterministic observation as evidence
- keep the interaction lightweight enough to feel like command-line autocomplete help, not a chat surface

## Observation Boundaries

Deterministic observation is still valuable, but it has a narrow role.

The daemon may track:

- terminal output
- painted line and activity timing
- process tree hints
- file activity hints
- raw stream attachment state

These signals are used for:

- MCP group inspection
- Ctrl-K terminal assist evidence
- honest internal state maintenance

They should not recreate the old per-session battle-card stack, title summarizer, auto-nudge loop, or UI-visible process/file dashboard.

If a signal is uncertain, the system should omit it or mark it as uncertain. It must not invent confidence.

## Interaction Model

Primary interactions:

- click a terminal tile to focus that real terminal in place
- right-click a terminal for terminal-native actions
- Ctrl-K to ask for command-line help
- create a supervised group from selected or visible terminals
- click a supervised group card to enter its member-terminal view
- toggle the supervisor terminal from the group card when custom instructions are needed

Removed interactions:

- per-session battle-card collapse
- scrollback-preview cards
- top-rail card tabs
- focused intervention mode as a separate product state
- auto-nudge pills or controls

The operator should always feel one click away from typing in the real terminal, not one mode transition away.

## Visual Direction

The interface should feel:

- technical
- calm
- dense
- operational

It should not feel:

- playful
- dashboard-corporate
- overloaded with AI branding
- verbose for its own sake

Terminal tiles should be visually quiet and stable. Supervised group cards can carry richer summary content, but they should still avoid excessive chrome.

Important visual rules:

- no cards inside cards
- no decorative summary widgets on terminal tiles
- no button-heavy per-terminal controls
- stable grid dimensions
- readable terminal content over ornamental status treatment
- Markdown summaries should be comfortable to scan, including tables

## Architecture Direction

The intended architecture is:

- `exaterm-types`: shared protocol and model contracts
- `exaterm-core`: daemon-side PTY ownership, observation, terminal assist, MCP tools, and host-session state
- `exatermd`: pure daemon binary
- `exaterm-ui`: shared layout and view state that does not depend on GTK or VTE
- `exaterm-gtk`: Linux GTK/VTE client and primary UI

Host rules:

- GTK is a host-session client in normal operation
- the session service owns canonical PTYs and persistence-oriented state
- local and remote hosts use the same client abstraction
- local UI ownership is only for explicit fake/demo/gallery paths

Transport rules:

- raw byte stream for terminal I/O
- control/model channel for snapshots, commands, lifecycle, MCP, and terminal assist
- Unix sockets locally
- SSH-forwarded Unix sockets for remote operation

## V1 Scope

Must-have:

- host-backed terminal grid
- fixed adaptive layout
- supervised group creation
- group card with Markdown summary
- group member-terminal view
- supervisor terminal visibility toggle
- MCP tools for group detail, worker messages, and summary updates
- Ctrl-K terminal assist
- direct terminal intervention in place
- honest degradation when observations are unavailable

Out of scope:

- old battle-card collapse for every session
- per-session title summarizer
- per-session tactical summarizer
- auto-nudge
- process/file evidence as UI decoration
- hidden model reasoning claims
