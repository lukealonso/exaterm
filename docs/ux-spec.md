# Exaterm UX Vision

## North Star

Exaterm is a commander's view for supervising 4 to 8 terminal-native coding agents at once.

Its job is not to replace the agent CLI. Its job is to make multi-agent supervision possible.

The core promise is:

- the operator can see what each agent appears to be doing
- the operator can see what the machine is actually doing
- the operator can spot mismatch, idleness, blockage, and failure quickly
- the operator can intervene in the real terminal in one step

Exaterm succeeds when the operator can maintain situational awareness across several concurrent agents without drowning in 8 separate terminal transcripts.

## Product Definition

Exaterm is:

- grid-first and supervision-first
- designed for 4 to 8 concurrent agent sessions
- centered on battle cards rather than raw shell panes
- built around correlating intent with observed execution
- denser and more operationally useful than any single agent CLI transcript
- respectful of native terminal workflows when intervention is needed

Exaterm is not:

- a replacement for Codex, Claude Code, or other terminal-native agents
- a custom agent shell that hides the agent's native interface
- an IDE
- a dashboard-heavy terminal multiplexer
- a product that claims access to hidden model reasoning

Core design rule: supervision comes first; terminal intervention stays one step away.

## Core User Job

The operator's top-level job is to keep tabs on several agents working in parallel, often on distinct tasks, and decide where attention belongs right now.

The operator is not primarily trying to read every terminal in full.

The operator is trying to answer:

- Which agents are actively making progress?
- Which agents are idle?
- Which agents are blocked?
- Which agents are failing?
- Which agents are doing something different from what they appear to claim?
- Which agent needs intervention first?

This is a battlefield view, not a single-session cockpit.

## Primary Working Posture

The default working posture is a grid of adaptive battle cards.

Each battle card represents one supervised agent session. A card can show traces of the real terminal, but the default overview is not a wall of raw terminal surfaces. The overview should instead present the smallest high-value unit of supervisory information: what the agent says it is doing, what the machine is actually doing, and whether those line up.

The operator should be able to sit in the main grid for most of the session and only drop into a real terminal when intervention is required.

## Information Model

Exaterm should distinguish three types of information.

### 1. Intent

Intent is the recent visible narrative of what the agent appears to be doing.

Examples:

- "Investigating failing parser tests"
- "Updating config loader"
- "I need to inspect the migration code"

Important constraint: this is not hidden chain-of-thought or internal reasoning. It is visible intent derived from the agent's terminal-visible output when available.

### 2. Observed Activity

Observed activity is what the machine can show directly or infer from evidence.

Examples:

- the active command or dominant subprocess
- recently spawned processes
- recent file writes, renames, or creations
- test progress
- build progress
- stdout or stderr emitted by child tools
- idle periods
- likely prompts for input

### 3. Inferred Narrative

Inferred narrative is a best-effort synthesis of intent plus observed activity.

Examples:

- "Agent says it is fixing parser tests; observed running `cargo test` and editing `src/parser.rs`."
- "Agent appears idle after claiming it would run tests."
- "Agent is likely blocked on a permission or input prompt."

This inferred layer is useful, but it must never pretend to be more certain than the evidence allows.

## Intent / Reality Correlation

The central UX idea in Exaterm is correlation between stated intent and observed reality.

The most valuable supervisory signal is often not the agent's visible narrative or the subprocess data independently. It is the relationship between them.

Healthy examples:

- the agent says it is running tests, and a test process is active
- the agent says it is editing parser code, and relevant files are changing
- the agent says it is investigating a failure, and the output stream shows the expected failing tool

Suspicious examples:

- the agent claims progress, but there is no meaningful command, file, or output activity
- the terminal narrative suggests one task, but the dominant subprocess is doing something unrelated
- the agent appears confident, but subprocess output shows repeated failure or retry loops
- the card shows extended idle time immediately after the agent claimed it would do something next

Exaterm should make these matches and mismatches visually obvious.

## Main Screen

The main screen is a resizable grid of session battle cards.

The grid must comfortably support supervision of 4 to 8 concurrent sessions without requiring the operator to manually toggle display modes just to stay oriented.

Recommended default layouts:

- 4 sessions: `2 x 2`
- 6 sessions: `3 x 2`
- 8 sessions: `4 x 2` on wide displays, or a denser fallback when needed

The overview should prefer a strict, even grid over freeform windowing. Equal card sizing makes state comparison faster and reduces visual hierarchy drift between sessions.

The main screen may include:

- grid canvas
- a thin workspace bar for global counts or filters
- optional workspace-level attention queue or filter controls
- optional detail expansion area if it does not compete with the grid

The grid remains the dominant region.

The workspace bar should stay minimal. It can show lightweight counts such as:

- `Idle 2`
- `Working 4`
- `Failed 1`

It should not compete visually with the card grid.

## Session Object

A session is the primary object in the system. A session represents one supervised terminal-native agent run.

Each session has:

- a stable session identity
- a launch configuration
- a native terminal surface
- runtime state
- recent visible intent
- recent command and subprocess activity
- recent file activity
- work-output streams
- event history
- derived supervisory signals

## Battle Card

A battle card is the default grid representation of one session.

The battle card is the core UX unit of Exaterm. It should provide enough information for prioritization and light diagnosis without forcing the operator into the raw terminal or a secondary inspector.

Each battle card contains:

- session identity
- high-level status
- recency and idle information
- the minimum state-appropriate narrative needed to understand what is happening
- the minimum evidence needed to decide whether intervention may be warranted
- whole-card click affordance for intervention

Recommended overview size:

- width roughly `340-420px`
- height roughly `190-260px`

That is enough room for a dense, readable supervisory summary without turning the card into a miniature dashboard.

## Battle Card Anatomy

The default card structure should be consistent across all sessions, but the information density inside the card should adapt to the tactical state of the session. The operator should learn one stable scan pattern and reuse it everywhere without every card turning into the same labeled report.

Recommended card bands:

1. Header
2. Tactical body
3. Supporting evidence

### Header

The header is compact and always visible.

Header contents:

- session name
- task or agent label
- high-level status
- recency indicator
- active command or dominant subprocess label

The header should make it easy to answer, in under a second, whether this session is active, idle, blocked, or failing.

The dominant visual elements in the header should be:

- state
- recency
- session identity

The active command label should be present, but subordinate to state.

### Tactical Body

The card body should answer the operator's real question for that state, not literally render a schema with lines like `Intent:`, `Reality:`, and `Output:` on every card.

The card should inherently communicate:

- what the session appears to be doing
- whether it seems healthy, suspicious, blocked, or stalled
- what single fact matters most right now

This means the body should be state-shaped:

- `Idle` cards emphasize idle age, last meaningful action, and why the idleness may or may not matter
- `Working` cards emphasize the dominant active task plus one strong evidence fragment
- `Thinking` cards emphasize the current direction or narrative with a calmer treatment
- `Blocked` cards emphasize the blocking cause
- `Failed` cards emphasize the failure headline and one concrete clue

Do not waste space stating the obvious. For example, an idle card should not spend its main line saying that nothing is running. The idle treatment and idle age already imply that the session is not actively progressing.

Do not make the overview card look like a dense inspector with multiple explicit labels and stacked report rows. It should feel like an unfolding tactical picture of the battlefield.

### Supporting Evidence

Below the tactical body, the card may show one or two compact evidence fragments when they materially help the operator judge whether intervention is warranted.

Candidate contents:

- active command
- dominant subprocess
- recently written files
- failing test summary
- current build step
- linter error
- compiler error location
- migration output
- likely prompt-for-input state
- retry loop or repeated failure hint

The goal is not exhaustive telemetry. The goal is rapid supervisory orientation.

These evidence fragments should be compact and concrete. They should not read like a narrated explanation of the card.

File activity is especially important when it grounds whether the agent is actually changing the codebase and where, but it should appear only when it adds decision value.

In tighter layouts, evidence may compress into short fragments such as:

- `Files: src/parser.rs, tests/parser.rs`
- `Output: 3 parser tests failing`
- `Output: cargo test still running`

### Intervention Affordance

The whole card is the intervention affordance.

There should be no per-card `Intervene` button in overview mode. There should generally be no buttons on the face of the battle card at all.

Clicking a card means: promote this session into direct terminal control.

The card should therefore feel immediately actionable without adding extra pill buttons, secondary controls, or cluttered action rows.

## Example Battle Card

The product should converge on cards that read roughly like this:

```text
┌ Agent 3 · Parser Fix                 IDLE · idle 48s ┐
│ rerunning parser tests                                    │
│ quiet after src/parser.rs, tests/parser.rs                │
│ 3 parser failures still visible                           │
└────────────────────────────────────────────────────────────┘
```

This is the right density target: enough information to decide whether the session is healthy without replaying the entire terminal transcript.

The important point is that the card inherently answers the operator's tactical questions without spelling out a report schema. It should not literally render `Intent`, `Reality`, or `Output` labels unless there is a rare, state-specific reason to do so.

## Adaptive Density

Battle cards should automatically change density based on available space, session urgency, and operator focus.

The operator should not need to click around just to make the overview usable.

### Overview Density

At overview density, the card should emphasize:

- status
- recency
- idle detection
- active command
- one short tactical summary chosen for the current state
- one or two compact evidence fragments when they matter

At this density, the card is optimized for scanning 4 to 8 sessions quickly.

Overview cards should not advertise intervention through explicit inline buttons. The action is implicit in the card itself.

If the product needs to improve discoverability, it should prefer lightweight ambient hints in surrounding chrome or first-run guidance rather than putting buttons or action rows onto the face of every card.

### Selected Density

When selected for inspection, a card may expand in place and reveal richer detail without immediately leaving the battlefield view.

Selected state can reveal:

- a larger recent terminal excerpt
- more detailed subprocess information
- richer file activity
- a short recent event timeline
- a larger work-output window

This state is for diagnosis while preserving workspace context.

### Intervention Focus Mode

When the operator clicks a card with the intent to intervene, that session should come front and center in a focused intervention view.

In this mode:

- the real embedded terminal becomes the primary surface
- surrounding workspace clutter drops away
- the terminal is sized up dynamically to a comfortable, reasonable working size
- the operator should feel like they are directly inside that agent's native terminal
- returning to the battlefield view should be immediate and predictable

This is not a different product mode. It is a fast promotion from supervision to direct control.

### Intervention Density

When the operator chooses to intervene, the real embedded terminal becomes primary for that session.

The transition should be one step and reversible without losing overall workspace awareness.

## Status Model

Status is intentionally coarse, operator-facing, and action-oriented.

The most important day-to-day supervisory states are:

- `Idle`
- `Thinking`
- `Working`
- `Failed`

Additional edge or terminal states may exist:

- `Blocked`
- `Complete`
- `Detached`

Definitions:

- `Idle`: the session appears live but no meaningful command, process, file, or output activity has been observed recently
- `Thinking`: the agent appears to be producing visible narrative, planning text, or lightweight terminal interaction, but there is not yet strong evidence of substantial tool execution or file activity
- `Working`: tool runs, subprocess activity, file changes, or meaningful output indicate concrete execution is underway
- `Failed`: the session or main launched activity exited unexpectedly or entered a clear error condition
- `Blocked`: the session likely needs user input, permission, missing dependency resolution, or an explicit confirmation step
- `Complete`: the intended task appears finished
- `Detached`: the terminal backend or session observability channel is no longer healthy

The distinction between `Thinking` and `Working` matters. A session that is narrating or planning without visible execution is in a different operational state from a session that is actively running tests, editing files, or driving tools.

Status assignment must be explainable from evidence. The UI should never imply confidence it does not have.

## State Priority

Not all states deserve equal visual weight.

In normal supervision:

- `Idle` should be the most attention-grabbing state because it is the most common actionable event
- `Failed` should be immediately unmistakable and severe
- `Blocked` should be visually clear and distinct from idle
- `Working` should read as healthy forward motion, not as an alert
- `Thinking` should read as live but pre-execution, calmer than working and much calmer than idle or failed

The operator should be able to scan the grid and spot a newly idle agent before reading any text.

## Idle Detection

Idle detection is first-class actionable intelligence.

The operator should be able to tell:

- that a session is idle
- how long it has been idle
- whether the idle period followed claimed next steps
- whether the session is probably harmlessly waiting or suspiciously stalled

An idle agent is not just a quiet agent. The product should distinguish:

- quiet but healthy long-running work
- quiet because the agent is waiting on external output
- quiet because nothing is happening
- quiet because the session is blocked on input

Recency cues should always be visible, for example:

- `active 4s ago`
- `idle 52s`
- `no file writes for 2m`
- `no subprocess output since last command`

Idle should also have strong visual salience in the grid. In most workflows, a newly idle agent is the event most likely to require operator judgment, so the card should make that transition obvious without turning the whole interface into an alarm system.

## Observability Boundaries

Exaterm v1 should aggressively pursue useful command-level observability without requiring deep agent-specific integrations.

Available without deep integration:

- PTY/session capture
- terminal stream capture
- process tree tracking via `/proc`
- operator controls
- event derivation from process and stream observations

Potentially available through wrapping, launch control, tracing, or adapters:

- subprocess stdio capture
- attribution of observed output to the main process or selected child processes
- file activity capture
- test/build progress extraction
- known tool pattern extraction

Not assumed in v1:

- hidden model reasoning
- internal tool-call state from every agent framework
- perfect stdout attribution in every environment
- perfect attribution of every file change to the exact responsible subprocess

Important constraint: when observability is uncertain, the UI must explicitly label that uncertainty rather than pretending to know more than it does.

## Generic Extraction Strategy

The product should not depend on an LLM just to parse terminal activity, but Exaterm should use model-assisted synthesis as part of the core supervision design when it materially improves tactical usefulness.

The architecture should therefore be hybrid:

- a deterministic evidence pipeline as the source of truth
- a model-assisted synthesis layer that turns recent evidence into smarter tactical summaries

The deterministic baseline pipeline should:

- strip control sequences
- segment recent PTY output into chunks
- classify chunks into likely narrative text, command text, tool output, prompts, and noise
- correlate chunks with process launches, exits, and file activity
- derive structured recent evidence from those observations

The model-assisted layer should selectively help with:

- choosing the most important tactical fragment to surface on a card
- compressing noisy recent activity into one useful summary instead of several mediocre ones
- synthesizing visible narrative with observed execution into a concise alignment or mismatch judgment
- turning raw evidence into a battle-card presentation that feels intelligent rather than mechanical
- classifying trajectory states that are hard to detect heuristically, such as `waiting_for_nudge`, healthy verification loops, converged waiting, or flailing
- judging whether the agent sounds coherent and on track versus uncertain, confused, or risky

The model-assisted layer should not be responsible for:

- basic status derivation
- idle timing
- raw process or file observation
- pretending to know more than the evidence supports
- inventing hidden reasoning or private model state

If model synthesis is unavailable, the product must still function using deterministic evidence only. However, the intended north star experience assumes model assistance for better tactical summarization.

Model calls should be:

- driven by meaningful evidence changes, not constant repaints
- aggressively cached
- bounded to a recent evidence window that is large enough to reveal short-term trajectory, not just the latest line
- conservative about confidence

## LLM Supervision Dimensions

The model-assisted synthesis layer should report into several distinct operator-facing dimensions rather than one overloaded summary state.

Recommended dimensions:

- `tactical_state`: the broad present-tense state, such as `Idle`, `Active`, `Working`, `Blocked`, `Failed`, or `Complete`
- `progress_state`: the trajectory or momentum, such as steady progress, verifying, exploring, waiting for nudge, flailing, converged waiting, or blocked
- `confidence_state`: how coherent and self-consistent the agent appears from visible evidence
- `operator_action`: whether the operator should watch, nudge, intervene, or do nothing
- `risk_posture`: whether the current behavior appears low-risk, watch-worthy, high-risk, or extreme
- `mismatch_level`: how much the visible narrative and machine evidence diverge

Each dimension should come with a terse grounded justification. These justifications exist so the UI can selectively surface the most valuable explanation for a given card without rendering the whole schema literally.

Examples of the intended shape:

- `progress_state = waiting_for_nudge` with a short reason that the agent paused after a crisp checkpoint
- `confidence_state = uncertain` with a short reason that the narrative keeps restarting without narrowing the issue
- `risk_posture = high` with a short reason that the agent is bypassing validation or taking shortcuts

The UI does not need to show every dimension on every card. The synthesis model should still provide them so Exaterm can decide which dimensions matter most for the current tactical state and density mode.

## Command-Level Visibility

A core promise of Exaterm is that the operator can see under the covers when needed without leaving the main grid.

Important signals include:

- active command
- recently launched subprocesses
- dominant subprocess
- grouped work output from child tools
- meaningful stderr
- recent file writes
- recent file creations or renames
- high-signal events like retries, failures, quiet periods, or input prompts

The UI should show enough of this by default to let the operator verify whether the session is progressing in reality, not just in rhetoric.

## Recent Terminal Narrative

The product should still preserve recent terminal-visible narrative because it helps the operator understand the agent's intent and reasoning style.

However, this should be treated as visible terminal narrative, not hidden reasoning.

The UX should prefer wording such as:

- recent intent
- recent narrative
- stated next step
- visible terminal summary

and avoid claiming access to private reasoning that the product does not in fact possess.

When a model is used to synthesize card copy, that synthesis should still be grounded in visible terminal narrative and observed evidence rather than implied private reasoning.

## Detail Model

The default expectation is that most useful supervision happens inside battle cards.

If a secondary detail surface exists, it should be an escalation path, not the core model.

Possible uses for a deeper detail view:

- full combined subprocess output
- longer recent terminal window
- extended event timeline
- richer process tree

But the north star is clear: battle cards should carry the default supervisory burden.

## Interaction Model

The interaction loop should be:

1. scan the grid
2. identify the session needing attention
3. inspect the card's tactical presentation and evidence
4. optionally expand or enrich the selected session without relying on on-card buttons
5. click the session to promote it front and center into pure-terminal intervention view if direct control is required
6. return to the battlefield view without losing orientation

Mouse and keyboard support should both preserve this loop.

Required capabilities:

- move between sessions quickly
- focus a selected session
- expand a selected card
- enter the real terminal in one action by clicking the card itself
- promote a session into a centered, dynamically sized intervention view
- return from intervention back to the grid cleanly
- cycle attention-worthy sessions

## Main User Flows

### 1. Scan the Battlefield

Goal: understand what all agents are doing and decide where attention belongs first.

Flow:

1. Operator sees 4 to 8 battle cards.
2. Cards show status, recency, tactical state, and high-signal evidence fragments.
3. Idle, blocked, failed, and suspicious mismatch states stand out.
4. Operator selects the session that deserves attention first.

Success criteria:

- no deep drill-down is needed for first-pass prioritization
- the operator can tell what all agents are roughly doing from the main grid
- the operator can spot idleness and mismatch immediately
- a newly idle session is obvious within a fast scan of the grid

### 2. Verify Claimed Progress

Goal: check whether the agent's visible narrative matches reality.

Flow:

1. Operator notices a card claiming progress.
2. The card also shows subprocess, file, and output evidence.
3. Operator checks whether the evidence aligns with the narrative.
4. Operator either leaves the session alone or escalates attention.

Success criteria:

- mismatch between claim and reality is easy to spot
- the operator can trust the card enough to avoid unnecessary intervention

### 3. Detect Suspicious Idleness

Goal: determine whether an idle agent is harmlessly waiting or has stalled.

Flow:

1. A card enters `Idle`.
2. The card shows idle age and recent preceding activity.
3. The operator checks whether the session is waiting on a known long-running command, external dependency, or input prompt.
4. The operator decides whether to intervene.

Success criteria:

- idle detection is prominent
- the operator can distinguish waiting from suspicious inactivity

### 4. Inspect Under-the-Covers Activity

Goal: understand what is happening beneath the agent's visible transcript.

Flow:

1. Operator selects a card or otherwise marks it as the current subject.
2. The card expands in place or gains richer density without requiring visible on-card buttons.
3. The operator reviews active commands, subprocesses, file writes, and recent work-output excerpts.
4. The operator decides whether the session is healthy, confused, blocked, or failing.

Success criteria:

- deeper operational context is available without abandoning the battlefield view
- the UI reveals what the machine is doing, not only what the agent says

### 5. Intervene in the Native Terminal

Goal: provide direct input or corrective action using the real agent interface.

Flow:

1. Operator clicks the session needing intervention.
2. That session is promoted front and center into a focused intervention view.
3. The real terminal is shown at a comfortable working size with surrounding clutter minimized.
4. Operator types directly into the embedded native TUI.
5. Operator returns to the battle card grid after intervention.

Success criteria:

- intervention always happens in the real terminal
- the promoted terminal view is large enough to work comfortably
- the operator does not feel trapped in a cluttered split view while intervening
- no custom abstraction sits between the operator and the agent when direct control is needed

## Visual Guidance

### General Feel

The interface should feel calm, dense, and operational.

It should not feel like:

- 8 tiny terminals fighting for attention
- a dashboard with noisy widgets
- a chat transcript manager

It should feel like:

- a tactical supervision surface
- a set of concise operational summaries
- a place where unusual states stand out immediately

### Layout Principles

The visual layout should follow a few hard rules:

- state before prose
- evidence before decoration
- consistent card structure across the whole grid
- equal card sizing in overview mode
- no default sidebars that compete with the battlefield
- the grid is for supervision; the promoted terminal is for control
- no button-heavy card chrome
- cards should read like tactical states, not labeled reports

The operator should always know where to look first: the cards, then the selected session, then the terminal if intervention is needed.

### State Visibility

State must be readable before the operator reads detailed text.

Every card should communicate state through a compact but unmistakable combination of:

- color
- contrast
- border or fill treatment
- status label
- recency text
- restrained motion only when justified

The states `Idle`, `Thinking`, `Working`, `Blocked`, and `Failed` should not blur together. The operator should be able to identify them with peripheral vision.

### State Hierarchy

Suggested visual hierarchy:

- `Idle`: highest routine salience; this should catch the eye because it often signals that operator attention may now be useful
- `Failed`: strongest error treatment; unmistakable and severe
- `Blocked`: urgent but different from failure; should suggest "needs input" rather than "crashed"
- `Working`: positive forward-motion treatment; visible but calm
- `Thinking`: live-but-pre-execution treatment; present and readable, but deliberately quieter than working

`Idle` should not look like a muted or forgotten state. It should look like a live session that has stopped progressing and may now deserve attention.

### Density

Density should favor supervisory signal over decorative chrome.

Default rules:

- keep labels short
- make recency and status obvious
- surface command-level information early
- keep output excerpts compact and high-signal
- avoid making the operator hunt for idle or blocked states
- avoid rendering the information model literally as stacked `Intent` / `Reality` / `Output` rows on every card

At overview density, state treatment matters more than decorative richness. If space is limited, preserve the state signal and idle recency before preserving additional narrative detail.

The card should feel like a compressed tactical surface, not a miniature terminal, not a mini dashboard, and not a structured report template.

### Motion

Use motion sparingly.

Motion may help with:

- newly idle sessions
- newly failed sessions
- newly blocked sessions
- recently changed attention state

Persistent animation across many cards will become noise.

Recommended rule: motion should be event-based and decay quickly. A newly idle card may pulse or brighten briefly, then settle into a strong static idle treatment.

### Small States

When space becomes tight:

- preserve status first
- preserve recency and idle cues second
- preserve active command and intent summary third
- collapse raw terminal visibility before removing supervisory signals

### Click Behavior

Click behavior should be decisive:

- clicking a card promotes that session front and center into pure-terminal focus mode
- returning from intervention should restore the original battlefield immediately

Avoid requiring multiple expansion steps before the operator can take control, and avoid forcing the operator to aim for small action buttons inside cards.

## Engineering Direction

Recommended implementation direction:

- Rust application
- GTK4/libadwaita frontend
- VTE-backed terminal surfaces for intervention
- session model for intent, observed activity, and derived status
- observability adapters for process, output, and file activity
- battle-card layout system with adaptive density
- promotion path from battle card to centered terminal intervention view

High-level architectural areas:

- terminal/session host
- event and stream ingestion
- deterministic activity classifier
- session-state derivation
- battle-card presentation model
- intervention/focus manager

## V1 Scope

Must-have:

- adaptive grid of battle cards for 4 to 8 sessions
- clear per-card status and recency
- first-class idle detection
- visible recent intent or narrative summary
- visible command and subprocess summary
- visible file activity summary when available
- visible work-output excerpt
- one-step transition into the real terminal
- front-and-center terminal promotion with dynamic sizing
- keyboard and mouse support for scan and intervene
- buttonless overview cards where click means intervene
- hybrid deterministic plus model-assisted tactical summarization

Should-have:

- mismatch detection between intent and observed activity
- grouped work-output view
- richer test/build progress extraction
- best-effort file attribution
- expanded in-place detail state for selected cards

Out of scope for v1:

- hidden reasoning capture
- perfect per-subprocess attribution in every environment
- deep protocol integrations for every agent product
- IDE-like code navigation
- orchestration workflows beyond supervision and intervention

## Acceptance Criteria

The UX is successful if:

- operators can supervise 4 to 8 terminal-native agents from one screen
- operators can understand what each agent is doing without opening every terminal
- operators can immediately tell when an agent has gone idle
- `Idle`, `Thinking`, `Working`, `Blocked`, and `Failed` are visually distinct at a glance
- operators can see enough under-the-covers activity to verify whether claimed progress is real
- operators can spot mismatch between intent and reality quickly
- operators can intervene in the native terminal in one step
- selected sessions can be promoted into a large, uncluttered terminal view for direct control
- the main grid remains the primary working posture
- overview cards feel like a dynamic battlefield, not a stack of labeled summaries or pills
