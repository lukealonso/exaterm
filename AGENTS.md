# Exaterm Contributor Guide

This file states the intended product philosophy and development posture for Exaterm.

It is not a full UX spec. It is the short operational contract future contributors should preserve.

For the fuller product framing, see:
- [docs/ux-spec.md](/home/luke/projects/exaterm/docs/ux-spec.md)
- [judgement/app.md](/home/luke/projects/exaterm/judgement/app.md)

## What Exaterm Is

Exaterm is a Linux desktop app for supervising multiple terminal-native coding agents at once.

Its job is not to replace Codex, Claude Code, or a normal terminal workflow. Its job is to make multi-agent supervision possible without forcing the operator to read several full terminal transcripts in parallel.

The core promise is:
- keep multiple sessions legible at a glance
- make progress, idleness, blockage, and failure easy to spot
- surface enough concrete evidence to verify whether an agent is really doing useful work
- let the operator intervene in a real terminal immediately

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
- idle timing and generic activity baselines

The model layer may refine and classify:
- thinking vs working
- idle vs blocked vs complete
- momentum
- risk posture
- terse operator summary
- auto-nudge text

The model must not be used as an excuse to skip building good observability.

### 5. Honest Degradation Matters

Exaterm must degrade honestly when it lacks deeper visibility.

Examples:
- plain SSH sessions are terminal-only supervision unless a remote foothold exists
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
- one terse operator-facing summary
- a few concrete evidence lines
- stable metric locations

### Focus and Intervention

Intervention should feel direct.

Rules:
- if a card is already showing a live terminal, interacting with it should stay in place
- focused mode should still feel like the same session/card system, not a different product
- avoid obvious or repetitive instructional chrome when the interaction is already clear

### Idleness

Idle is one of the most important normal states in the product.

It should be:
- easy to notice
- semantically correct
- not confused with active repaint churn or quiet-but-healthy work

### Auto-Nudge

Auto-nudge is an assistive feature, not a spam machine.

Rules:
- only fire for LLM-determined idle states
- only for sessions that actually look like coding agents
- use cooldowns
- prefer no nudge over a bad nudge

## Architecture Guidance

The desired architecture is:
- `model`: durable workspace/session state
- `runtime`: PTY/session transport and lifecycle
- `observation`: terminal history, process/file observation, evidence construction
- `supervision`: deterministic view-model shaping
- `synthesis`: LLM schemas, prompts, signatures, sanitization
- `ui`: GTK widgets, layout, presentation, and user interaction wiring

The codebase is moving toward that split. Keep pushing in that direction.

In particular:
- do not let `ui.rs` absorb more PTY/runtime logic
- do not let GTK widget concerns leak into reusable observation/runtime modules
- prefer adding testable helpers in non-UI modules instead of burying logic in callbacks

## Terminal and PTY Philosophy

Terminal fidelity matters because Exaterm supervises real terminal-native tools.

Rules:
- preserve native terminal behavior whenever possible
- avoid fake geometry churn
- avoid output corruption under TUI redraw or scrolling
- treat PTY resizing as meaningful application state, not a cosmetic detail

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
