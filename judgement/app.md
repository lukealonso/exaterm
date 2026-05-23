# Exaterm

Exaterm is a Linux GTK desktop app for supervising terminal-native coding agents through host-backed terminal sessions.

Judge this product primarily as a terminal-first multi-agent supervision surface, not as a general terminal emulator, not as a replacement shell for Codex or Claude Code, and not as an IDE. The north star for the product is described in [`docs/ux-spec.md`](../docs/ux-spec.md).

The most important parts of the experience are:

- Whether the app launches into real terminal tiles rather than an empty dashboard.
- Whether fixed grid layouts keep several sessions tidy and legible without manual pane management.
- Whether terminal sessions remain terminals at all densities instead of collapsing into old battle cards, scrollback previews, top-rail tabs, or focus-mode proxies.
- Whether host-backed sessions and reconnection feel central to the workflow.
- Whether supervised group cards summarize groups without hiding the member terminals permanently.
- Whether the group card can toggle to reveal the supervisor terminal for custom instructions.
- Whether Ctrl-K offers quick terminal help for the selected or active terminal without feeling like a separate chat product.
- Whether direct intervention happens by typing in the real terminal in place.

The workflows that deserve the most evaluation time are:

- Opening the app and confirming the first screen is a usable terminal.
- Adding multiple terminals and checking that the grid remains stable and predictable.
- Creating or viewing a supervised group, then entering the group to see the worker terminals as normal terminal tiles.
- Toggling the supervisor terminal on a group card and returning to the Markdown summary.
- Using Ctrl-K to ask for a command and checking that the suggested text is inserted into the intended terminal.
- Disconnecting or reconnecting through the host session path where practical.

Quality looks like this:

- The app feels calm, technical, dense, and operational.
- Terminal fidelity is preserved: TUIs, scrollback, selection, resizing, and input remain native-feeling.
- The grid is structured enough that the operator does not waste attention tidying panes.
- Supervised group summaries are short, grounded, and readable, including Markdown tables when helpful.
- Supervisor actions are explicit: messages to workers and summary updates are separate operations.
- Process and file evidence, when surfaced through supervisor tooling, is grounded and not overclaimed.
- Ctrl-K suggestions are safe, inspectable, and appropriate to the visible terminal context.
- The UI does not claim access to hidden model reasoning.

Weak quality looks like this:

- Ordinary terminal sessions collapse into summary cards or scrollback cards.
- A separate focus mode or top-rail card system reappears.
- The app feels like a dashboard instead of a terminal-first supervision tool.
- Supervised group cards become verbose report templates.
- The supervisor summary is coupled to command injection.
- Auto-nudge controls or hidden nudge loops are visible or implied.
- Per-session title or tactical summarizers drive the UI.
- Process/file observation is shown as decorative certainty rather than used cautiously through MCP or terminal assist.
- Ctrl-K inserts risky commands without making them inspectable.

Environment notes:

- This app targets Linux desktop usage.
- Prefer X11 for automation.
- Resize the main window to a realistic multi-session working size before judging layout density, terminal fidelity, and group-card behavior.
