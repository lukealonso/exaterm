# Exaterm Contributor Guide

This file states the intended product philosophy and development posture for Exaterm.

It is not a full UX spec. It is the short operational contract future contributors should preserve.

For the fuller product framing, see:
- [docs/ux-spec.md](/home/luke/projects/exaterm/docs/ux-spec.md)
- [judgement/app.md](/home/luke/projects/exaterm/judgement/app.md)

## What Exaterm Is

Exaterm is a beachhead-backed Linux desktop app for supervising terminal-native coding agents.

Its job is not to replace Codex, Claude Code, or a normal terminal workflow. Its job is to make multi-agent supervision possible without forcing the operator to read several full terminal transcripts in parallel.

The core promise is:
- keep terminal work legible at a glance
- make progress, stoppage, blockage, and failure easy to spot
- surface enough concrete evidence to verify whether an agent is really doing useful work
- let the operator intervene in a real terminal immediately
- make sessions persistent and reconnectable through a long-lived beachhead

## What Exaterm Is Not

Exaterm is not:
- a new agent shell
- an IDE
- a generic pane manager
- a dashboard-heavy terminal multiplexer
- a product that claims access to hidden model reasoning

When in doubt, preserve terminal-native workflows and avoid adding control-plane complexity that makes the app feel more like a dashboard than a supervision surface.

## Product Philosophy

### 1. Supervision First

The main product value is situational awareness across several agent sessions.

The operator should be able to answer:
- which sessions are making real progress
- which sessions are idle
- which sessions are blocked
- which sessions are failing
- which sessions are risky or drifting
- which session deserves attention next

If a change improves terminal fidelity but weakens supervisory clarity, it is probably the wrong trade.

### 2. Terminal-First, Then Progressive Density

Exaterm should unfold naturally from a familiar terminal experience.

That means:
- the app should launch into a real terminal, not an empty dashboard
- low-density layouts should feel close to a normal terminal workflow
- higher-density layouts may compress into supervision cards and scrollback
- the transition should feel like progressive abstraction, not mode-switch whiplash

The design target is:
- 1 session: full terminal-first
- 2 sessions: still terminal-first if viable
- higher density: adaptive mix of real terminals and supervisory cards depending on space

Exaterm should feel like a normal terminal first and only gradually reveal more supervision structure as density rises.

### 3. Real Evidence Beats Decorative Summaries

Exaterm should always prefer grounded evidence over polished but vague status copy.

Good signals:
- terminal history
- current updated terminal line
- subprocess/process-tree evidence
- file activity when trustworthy
- explicit idle timing

Bad signals:
- generic motivational summaries
- deterministic overclaims like “blocked” without clear evidence
- copy that sounds smart but is weakly grounded

### 4. The LLM Refines, It Does Not Invent the Substrate

The deterministic substrate owns:
- terminal capture
- painted-line / activity capture
- PTY state
- resize behavior
- process/file inspection when trustworthy
- generic activity baselines

The model layer may refine and classify:
- thinking vs working
- idle vs stopped vs blocked vs complete
- momentum
- risk posture
- one short operator-facing headline
- auto-nudge text

The model must not be used as an excuse to skip building good observability.

State semantics matter:
- `idle` means there is no meaningful active goal to resume; there is nothing to nudge
- `stopped` means the agent has unnecessarily paused after a coherent pass and a nudge may help
- `blocked` means human intervention is actually required; a nudge or simple continue will not fix it
- `complete` means the task is genuinely done

### 5. Honest Degradation Matters

Exaterm must degrade honestly when it lacks deeper visibility.

Examples:
- plain SSH terminals are not a special UI mode; the correct long-term model is always a beachhead-backed session, local or remote
- remote/local process and file claims should only appear when they are actually trustworthy
- if the model does not know, the UI should stay sparse rather than fabricate certainty

## UX Rules

### Cards

Battlefield cards should be:
- consistent in structure
- fast to scan
- minimal in wording
- rich in evidence when needed

Avoid:
- verbose structured report rows like `Intent:` / `Reality:` / `Output:`
- too many pills
- redundant labels
- multiple competing headlines

Preferred card hierarchy:
- identity
- state
- one short operator-facing headline
- a few concrete evidence lines
- stable metric locations

### Focus and Intervention

Intervention should feel direct.

Rules:
- if a card is already showing a live terminal, interacting with it should stay in place
- focused mode should still feel like the same session/card system, not a different product
- avoid obvious or repetitive instructional chrome when the interaction is already clear

### Idleness and Stoppage

`idle` and `stopped` are distinct and should stay distinct.

Rules:
- `idle` is passive, muted, and not nudgeable
- `stopped` is the nudgeable paused state and should be more attention-grabbing than idle
- `blocked` is more severe than `stopped`
- do not confuse quiet repaint churn or a checkpoint pause with healthy active work

### Auto-Nudge

Auto-nudge is an assistive feature, not a spam machine.

Rules:
- only fire for LLM-determined `stopped` states
- only for sessions that actually look like coding agents
- use cooldowns
- prefer no nudge over a bad nudge

## Architecture Guidance

The current intended architecture is a five-part workspace:
- `crates/exaterm-types`
  - pure shared contract types
  - model records/enums
  - protocol payloads
  - synthesis result types
- `crates/exaterm-core`
  - headless daemon-side logic
  - launch/model helpers
  - headless runtime and PTY ownership
  - observation
  - synthesis
  - file/process inspection
  - daemon protocol handling
- `crates/exatermd`
  - pure daemon binary
  - no GTK
  - no VTE
  - owns the canonical session state and LLM work
- `crates/exaterm-ui`
  - shared UI model
  - layout logic, supervision view state, and workspace view primitives
  - no GTK, no VTE — pure logic shared between clients
- `crates/exaterm-gtk`
  - GTK/VTE client (Linux)
  - interaction, local display PTYs, and rendering
  - frontend-only supervision and presentation logic

### Beachhead Rules

The UI should always be beachhead-backed in normal operation.

That means:
- the GTK app is a client of a beachhead
- the beachhead owns canonical PTYs, observations, summaries, naming, nudging, and persistence-oriented state
- the UI must not silently fall back to owning live sessions itself
- local and remote beachheads must be hidden behind the same client abstraction

The only acceptable non-beachhead exception is explicit fake/demo/gallery mode.

### Transport Rules

Use two transport planes:
- raw byte stream
  - low-latency terminal I/O only
  - keep it off JSON and off slow UI/model paths
- control/model channel
  - snapshots, commands, lifecycle, errors, and state updates
  - readability and evolvability matter more than micro-optimization here

For local transport, prefer Unix domain sockets.

For remote transport, the intended direction is:
- remote `exatermd`
- SSH as the outer transport
- SSH-forwarded Unix sockets
- same protocol as local

### Code Placement Rules

In particular:
- do not let `crates/exaterm-gtk/src/ui.rs` absorb more headless runtime logic
- do not let GTK or VTE leak back into `exaterm-core` or `exaterm-ui`
- do not let behavior-heavy helpers or execution policy creep into `exaterm-types`
- keep frontend prose/presentation shaping out of the daemon and shared contract crates
- prefer testable helpers in core modules instead of burying logic in GTK callbacks
- keep `exatermd` pure enough to ship independently to remote Linux hosts

## Terminal and PTY Philosophy

Terminal fidelity matters because Exaterm supervises real terminal-native tools.

Rules:
- preserve native terminal behavior whenever possible
- avoid fake geometry churn
- avoid output corruption under TUI redraw or scrolling
- treat PTY resizing as meaningful application state, not a cosmetic detail
- keep the hot terminal path wake-driven and near-realtime
- keep model/control traffic off the raw byte path

The canonical session PTY belongs to the beachhead.

The GTK client may use a local display PTY for VTE, but that display PTY is not the source of truth. It is only a rendering/input adapter.

When hidden terminals need geometry:
- prefer a real measured size
- otherwise prefer a predicted focused size
- avoid pushing arbitrary fallback geometry into live agent processes unless unavoidable

## Review Standard

When reviewing changes, prioritize:
- behavioral regressions
- terminal fidelity regressions
- incorrect supervisory claims
- state-model confusion
- excessive UI chrome
- evidence quality regressions

Treat these as lower priority:
- purely stylistic lint cleanup
- refactors that do not materially improve architecture or correctness
- cosmetic polishing that does not improve scan, evidence, or intervention

## Change Discipline

Preferred way to work:
- make focused changes
- verify with `cargo check` and relevant tests
- commit working checkpoints frequently

Do not:
- sneak unrelated cleanup into feature changes
- rewrite large stable areas without a clear architectural gain
- replace grounded behavior with hand-wavy heuristics

When changing beachhead/client boundaries:
- prefer one clear abstraction over local-vs-remote branches in the UI
- keep raw-path latency measurable
- preserve terminal responsiveness before adding protocol sophistication

## Naming and Tone

Exaterm means a very large number of terminals. Preserve that feeling.

The product should feel:
- technical
- calm
- direct
- operational

It should not feel:
- playful
- dashboard-corporate
- overloaded with AI branding
- verbose for its own sake
