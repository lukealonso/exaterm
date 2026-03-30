# Exaterm

Exaterm is a terminal-first desktop app for supervising coding agents.

Instead of replacing the terminal, it lets you keep several agent sessions visible at once, watch what they are actually doing, and jump into a real terminal the moment one needs attention.

The current build is Linux-first and still moving quickly, but the core shape is in place:
- real terminal sessions, not fake transcript widgets
- a battlefield view that scales from one terminal upward
- lightweight LLM-backed headlines and status signals for each session
- a persistent beachhead daemon behind the UI so sessions can outlive the window

## What It Feels Like

At low density, Exaterm behaves like a normal terminal app.

As you add more sessions, it progressively compresses into supervision cards:
- short headline
- recent terminal evidence
- momentum and risk
- state like `working`, `stopped`, `blocked`, or `failed`

The goal is simple: make it possible to supervise multiple coding agents without reading several full terminals in parallel all the time.

## Architecture

Exaterm is split into five crates:

- `crates/exaterm-types`
  - shared contract types only
  - model records/enums, protocol messages, synthesis result types
- `crates/exaterm-core`
  - headless daemon-side logic
  - observation, synthesis, daemon runtime, process/file inspection, protocol handling
- `crates/exatermd`
  - the headless beachhead daemon
  - owns PTYs, session state, summaries, and nudging
- `crates/exaterm-ui`
  - shared UI model
  - layout logic, supervision view state, and workspace view primitives shared between clients
- `crates/exaterm-gtk`
  - the GTK/VTE desktop client (Linux)
  - renders the UI, owns local display PTYs, and talks to the daemon

The UI is intended to always be beachhead-backed in normal operation.

Locally, the client talks to the beachhead over Unix domain sockets:
- one control socket for snapshots, commands, lifecycle, and model state
- one raw PTY byte socket per live session

The remote path uses the same beachhead protocol over SSH-forwarded Unix sockets.

## Current Status

This is a working prototype, not a polished release.

What works well right now:
- low-latency terminal interaction through the beachhead
- local persistent daemon-backed sessions
- terminal-native VTE rendering
- battlefield/focus layouts for supervising multiple sessions
- LLM-backed summaries, naming, and auto-nudge behavior
- remote beachhead sessions over SSH in an experimental but working form

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

Required:
- `OPENAI_API_KEY`

Optional overrides:
- `EXATERM_OPENAI_BASE_URL`
  - preferred base URL override for an OpenAI-compatible API endpoint
- `OPENAI_BASE_URL`
  - fallback base URL override if `EXATERM_OPENAI_BASE_URL` is not set
- `EXATERM_SUMMARY_MODEL`
  - model override for session summaries
- `EXATERM_NAMING_MODEL`
  - model override for session naming
- `EXATERM_NUDGE_MODEL`
  - model override for auto-nudges

Notes:
- `OPENAI_*` is used for the API key and compatible base URL
- model overrides are Exaterm-specific: `EXATERM_SUMMARY_MODEL`, `EXATERM_NAMING_MODEL`, and `EXATERM_NUDGE_MODEL`
- if neither base URL variable is set, Exaterm uses `https://api.openai.com/v1`
- Exaterm appends `/chat/completions` automatically when needed
- these variables can also be provided in a repo-local `.env` file
- without `OPENAI_API_KEY`, the app still runs, but summaries, naming, and nudges stay disabled

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
It also builds both `exaterm` and `exatermd` first.

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
make daemon
make check
make test
make test-workspace
make core-test
make daemon-check
```

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
