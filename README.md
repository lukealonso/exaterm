# Exaterm

Exaterm is a terminal-first desktop app for supervising coding agents.

Instead of replacing the terminal, it lets you keep several agent sessions visible at once, watch what they are actually doing, and jump into a real terminal the moment one needs attention.

The current build is Linux-first and still moving quickly, but the core shape is in place:
- real terminal sessions, not fake transcript widgets
- a battlefield view that scales from one terminal upward
- lightweight LLM-backed headlines and status signals for each session
- a host session service behind the UI so sessions can outlive the window

## What It Feels Like

At low density, Exaterm behaves like a normal terminal app.

As you add more sessions, it progressively compresses into supervision cards:
- short headline
- recent terminal evidence
- momentum and risk
- state like `working`, `stopped`, `blocked`, or `failed`

The goal is simple: make it possible to supervise multiple coding agents without reading several full terminals in parallel all the time.

## Architecture

Exaterm is split into several crates:

- `crates/exaterm-types`
  - shared contract types only
  - model records/enums, protocol messages, synthesis result types
- `crates/exaterm-core`
  - headless daemon-side logic
  - observation, synthesis, daemon runtime, process/file inspection, protocol handling
- `crates/exatermd`
  - the headless host session service
  - owns PTYs, session state, summaries, and nudging
- `crates/exaterm-ui`
  - shared UI model
  - layout logic, supervision view state, and workspace view primitives shared between clients
- `crates/exaterm-gtk`
  - the GTK/VTE desktop client (Linux)
  - renders the UI, owns local display PTYs, and talks to the daemon
- `crates/exaterm-web`
  - browser-based web UI (axum + TypeScript/xterm.js)
  - connects to the daemon over Unix sockets, serves a single-page app with WebSocket relay

The UI is intended to connect to a host session service in normal operation.

Locally, the client talks to the session service over Unix domain sockets:
- one control socket for snapshots, commands, lifecycle, and model state
- one raw PTY byte socket per live session

The remote path uses the same session protocol over SSH-forwarded Unix sockets.

## Current Status

This is a working prototype, not a polished release.

What works well right now:
- low-latency terminal interaction through host sessions
- local persistent daemon-backed sessions
- terminal-native VTE rendering
- fixed-grid terminal layouts for supervising multiple sessions
- supervised group cards with Markdown summaries and supervisor-terminal toggles
- Ctrl-K terminal assist
- remote hosts over SSH in an experimental but working form

What is still evolving:
- remote session bootstrap and packaging
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

Exaterm can use OpenAI for Ctrl-K terminal assist.

On macOS, initialize the SwiftTerm submodule before building:

```bash
git submodule update --init --recursive
```

Required:
- no model credential is required to run the app

Configuration:
- open the launcher Settings panel before choosing a workspace
- leave the terminal audible bell disabled, or enable it there if wanted
- set the OpenAI API key, base URL, and Ctrl-K model there
- the default base URL is `https://api.openai.com/v1`
- the default Ctrl-K model is `gpt-5.5-nano`
- settings are stored in the Exaterm application config file at `$XDG_CONFIG_HOME/exaterm/config.json`, or `~/.config/exaterm/config.json`

Notes:
- Exaterm appends `/chat/completions` automatically when needed
- without an OpenAI API key in application settings, the app still runs, but Ctrl-K terminal assist is unavailable

## Building

From the repo root:

```bash
make
```

That builds the default native frontend for your platform and `exatermd`.

On Linux, that means:
- `exaterm-gtk`
- `exatermd`

On macOS, that means:
- `exaterm-macos`
- `exatermd`

To build only the web UI (no GTK or system UI libraries required):

```bash
make web
```

## Running

Local:

```bash
make run
```

That launches the native frontend for your platform and connects to local
host sessions, starting the service if needed.

You can also run the daemon directly:

```bash
make daemon
```

## Remote Mode

There is an experimental SSH path:

```bash
cargo run -p exaterm-gtk -- --ssh user@host
```

The intended direction is:
- copy a Linux `exatermd` to the remote host
- launch it remotely
- forward its Unix sockets back over SSH
- keep the UI talking to the same session protocol it uses locally

If you are on macOS, use `exaterm-macos` instead of `exaterm-gtk`.

Treat this as in-progress rather than finished product UX.

## Web UI

The web UI is a browser-based alternative to the GTK app. It connects to the same host session service, but it is not the primary platform and may lag the Linux GTK surface.

### Running locally

```bash
make web-run
```

This starts the web server on `http://127.0.0.1:9800`. Open that in a browser. The daemon is auto-spawned if not already running.

### Running over SSH

Run the web server on the remote host and forward the HTTP port:

```bash
ssh -L 9800:127.0.0.1:9800 user@host 'cargo run -p exaterm-web'
```

The daemon is auto-spawned on the remote host if not already running.

### CLI options

```text
--port <N>       Port to listen on (default: 9800)
--bind <addr>    Address to bind to (default: 127.0.0.1)
--no-embed       Serve frontend assets from disk instead of the embedded copy (for development)
```

### Security

The web server binds to localhost by default and has no authentication. Anyone who can reach the port gets full terminal access. This is safe when accessed through an SSH tunnel or on a single-user machine.

If you bind to a non-localhost address (e.g. `--bind 0.0.0.0`), the server will print a warning. Do not expose the web UI to an untrusted network without additional access control.

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
make web
make web-run
make web-test
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
