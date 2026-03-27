# Exaterm

Exaterm is a terminal-first desktop app for supervising coding agents.

Instead of replacing the terminal, it lets you keep several agent sessions visible at once, watch what they are actually doing, and jump into a real terminal the moment one needs attention.

The current build is Linux-first and still moving quickly, but the core shape is in place:
- real terminal sessions, not fake transcript widgets
- a battlefield view that scales from one terminal upward
- lightweight summaries and status signals for each session
- a persistent beachhead daemon behind the UI so sessions can outlive the window

## What It Feels Like

At low density, Exaterm behaves like a normal terminal app.

As you add more sessions, it progressively compresses into supervision cards:
- short brief
- recent terminal evidence
- momentum and risk
- state like `working`, `stopped`, `blocked`, or `failed`

The goal is simple: make it possible to supervise multiple coding agents without reading several full terminals in parallel all the time.

## Architecture

Exaterm is split into three crates:

- `crates/exaterm-core`
  - shared core logic
  - model, protocol, observation, supervision, synthesis, daemon runtime
- `crates/exatermd`
  - the headless beachhead daemon
  - owns PTYs, session state, summaries, and nudging
- `crates/exaterm`
  - the GTK/VTE desktop client
  - renders the UI and talks to the daemon

The UI is intended to always be beachhead-backed in normal operation.

Locally, the client talks to the beachhead over Unix domain sockets.
The long-term remote model is the same protocol over SSH-forwarded Unix sockets.

## Current Status

This is a working prototype, not a polished release.

What works well right now:
- low-latency terminal interaction through the beachhead
- local persistent daemon-backed sessions
- terminal-native VTE rendering
- battlefield/focus layouts for supervising multiple sessions
- LLM-backed summaries, naming, and auto-nudge behavior

What is still evolving:
- remote beachhead bootstrap and packaging
- portability beyond Linux
- session lifecycle UX
- packaging/distribution

## Requirements

You’ll need:
- Rust and Cargo
- GTK 4
- libadwaita
- VTE

The exact package names depend on distro.

Exaterm also uses the OpenAI API for summaries, naming, and nudges.

Environment variables:
- `OPENAI_API_KEY`
- optional: `EXATERM_OPENAI_BASE_URL`
- optional: `OPENAI_BASE_URL`
- optional: `EXATERM_SUMMARY_MODEL`
- optional: `EXATERM_NAMING_MODEL`
- optional: `EXATERM_NUDGE_MODEL`

## Building

From the repo root:

```bash
make
```

That builds both:
- `exaterm`
- `exatermd`

## Running

Local:

```bash
make run
```

That launches the GTK app and connects to a local beachhead, spawning one if needed.

You can also run the daemon directly:

```bash
make daemon
```

## Remote Mode

There is an experimental SSH path:

```bash
cargo run -p exaterm -- --ssh user@host
```

The intended direction is:
- copy a Linux `exatermd` to the remote host
- launch it remotely
- forward its Unix sockets back over SSH
- keep the UI talking to the same beachhead protocol it uses locally

Treat this as in-progress rather than finished product UX.

## Development Commands

Useful commands:

```bash
make
make run
make check
make test
make test-workspace
make core-test
make daemon-check
make probe
```

The raw-path latency probe lives in the core crate and is useful when working on beachhead transport performance.

## Why “Exaterm”?

The name is meant in the sense of an enormous number of terminals.

Not a pane manager. Not an IDE. Not a fake dashboard.

Just a calmer way to manage a lot of real terminal work at once.

## Notes

This project is opinionated:
- terminal fidelity matters
- the LLM should refine, not hallucinate the substrate
- evidence matters more than glossy summaries
- responsiveness matters more than architectural cleverness

If you want the contributor philosophy and architecture rules, see [AGENTS.md](/home/luke/projects/exaterm/AGENTS.md).
