import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import type { ClientMessage, SessionSnapshot } from "./protocol";

interface ExatermTestHooks {
  clearTerminalSelection(sessionId: number): boolean;
  connectionState(sessionId: number): number | null;
  terminalContainsText(sessionId: number, needle: string): boolean;
  terminalTextCenter(
    sessionId: number,
    needle: string
  ): { x: number; y: number } | null;
  selectTerminalText(sessionId: number, needle: string): boolean;
}

declare global {
  interface Window {
    __EXATERM_TEST__?: ExatermTestHooks;
  }
}

export interface ManagedTerminal {
  sessionId: number;
  term: Terminal;
  fit: FitAddon;
  wrapper: HTMLElement;
  ws: WebSocket;
  resizeObserver: ResizeObserver | null;
  /** When false, the stream WebSocket will not reconnect on close. */
  live: boolean;
}

const terminals = new Map<number, ManagedTerminal>();

let sendCommand: ((cmd: ClientMessage) => void) | null = null;
let syncInputsEnabled = false;

function findBufferText(term: Terminal, needle: string): { column: number; row: number } | null {
  const buffer = term.buffer.active;
  for (let row = 0; row < buffer.length; row++) {
    const line = buffer.getLine(row);
    const text = line?.translateToString(true) ?? "";
    const column = text.indexOf(needle);
    if (column >= 0) {
      return { column, row };
    }
  }
  return null;
}

function installTestHooks() {
  window.__EXATERM_TEST__ = {
    clearTerminalSelection(sessionId) {
      const managed = terminals.get(sessionId);
      if (!managed) return false;
      managed.term.clearSelection();
      return true;
    },
    connectionState(sessionId) {
      return terminals.get(sessionId)?.ws.readyState ?? null;
    },
    selectTerminalText(sessionId, needle) {
      const managed = terminals.get(sessionId);
      if (!managed) return false;
      const location = findBufferText(managed.term, needle);
      if (!location) return false;
      managed.term.select(location.column, location.row, needle.length);
      return true;
    },
    terminalContainsText(sessionId, needle) {
      const managed = terminals.get(sessionId);
      if (!managed) return false;
      return findBufferText(managed.term, needle) !== null;
    },
    terminalTextCenter(sessionId, needle) {
      const managed = terminals.get(sessionId);
      if (!managed) return null;
      const location = findBufferText(managed.term, needle);
      if (!location) return null;
      const screen = managed.term.element?.querySelector(".xterm-screen");
      if (!(screen instanceof HTMLElement)) return null;
      const rect = screen.getBoundingClientRect();
      const visibleRow = location.row - managed.term.buffer.active.viewportY;
      if (visibleRow < 0 || visibleRow >= managed.term.rows) return null;
      return {
        x: rect.left + ((location.column + needle.length / 2) / managed.term.cols) * rect.width,
        y: rect.top + ((visibleRow + 0.5) / managed.term.rows) * rect.height,
      };
    },
  };
}

function testHooksEnabled(): boolean {
  return document.body?.dataset.exatermTestHooks === "true";
}

if (testHooksEnabled()) {
  installTestHooks();
}

/**
 * Copy text to the clipboard, falling back to execCommand when the async
 * Clipboard API is unavailable (non-secure contexts: plain HTTP to a
 * non-loopback host, common when accessing the web UI over a LAN).
 */
export function copyToClipboard(text: string): boolean {
  if (!text) return false;
  if (navigator.clipboard && window.isSecureContext) {
    navigator.clipboard.writeText(text).catch(() => {
      execCommandCopy(text);
    });
    return true;
  }
  return execCommandCopy(text);
}

function execCommandCopy(text: string): boolean {
  const previousFocus =
    document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.top = "-1000px";
  textarea.style.opacity = "0";
  document.body.appendChild(textarea);
  textarea.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  }
  document.body.removeChild(textarea);
  if (previousFocus && previousFocus !== document.body && previousFocus.isConnected) {
    try {
      previousFocus.focus({ preventScroll: true });
    } catch {
      previousFocus.focus();
    }
  }
  return ok;
}

export function setSendCommand(fn: (cmd: ClientMessage) => void) {
  sendCommand = fn;
}

export function getTerminal(sessionId: number): ManagedTerminal | undefined {
  return terminals.get(sessionId);
}

export function getAllTerminals(): ManagedTerminal[] {
  return [...terminals.values()];
}

export function isSyncInputs(): boolean {
  return syncInputsEnabled;
}

export function setSyncInputs(enabled: boolean) {
  syncInputsEnabled = enabled;
}

/** Send raw text to a specific session's terminal via its WebSocket.
 *  If no terminal is attached, opens a one-shot WebSocket to deliver the text. */
export function sendTextToSession(sessionId: number, text: string) {
  const managed = terminals.get(sessionId);
  if (managed && managed.ws.readyState === WebSocket.OPEN) {
    managed.ws.send(new TextEncoder().encode(text));
    return;
  }
  // No active terminal — open a temporary stream connection.
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const ws = new WebSocket(
    `${protocol}//${location.host}/ws/stream/${sessionId}`
  );
  ws.binaryType = "arraybuffer";
  ws.onopen = () => {
    ws.send(new TextEncoder().encode(text));
    setTimeout(() => ws.close(), 500);
  };
  ws.onerror = () => ws.close();
}

export function attachTerminal(
  session: SessionSnapshot,
  container: HTMLElement
): ManagedTerminal {
  const existing = terminals.get(session.record.id);
  if (existing) {
    if (existing.wrapper.parentElement !== container) {
      container.innerHTML = "";
      container.appendChild(existing.wrapper);
      existing.fit.fit();
    }
    return existing;
  }

  container.innerHTML = "";

  const wrapper = document.createElement("div");
  wrapper.className = "xterm-wrapper";
  container.appendChild(wrapper);

  const term = new Terminal({
    cursorBlink: true,
    fontSize: 14,
    fontFamily: "'Cascadia Code', 'Fira Code', 'JetBrains Mono', monospace",
    scrollback: 100_000,
    scrollOnUserInput: true,
    // Let Mac users bypass app mouse tracking with Option+drag (xterm.js
    // already bypasses it on Shift+drag on all platforms). TUIs like
    // Claude Code enable mouse reporting, which otherwise eats the drag.
    macOptionClickForcesSelection: true,
    theme: {
      background: "#070d14",
      foreground: "#e2e8f0",
      cursor: "#e2e8f0",
      selectionBackground: "rgba(96, 165, 250, 0.55)",
      selectionInactiveBackground: "rgba(96, 165, 250, 0.35)",
    },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(wrapper);
  fit.fit();

  // OSC 52 clipboard passthrough. When a TUI inside the session (tmux,
  // vim, helix, ...) is configured to emit OSC 52 on copy, xterm.js forwards
  // the sequence here and we write the decoded payload to the browser
  // clipboard. This is the only reliable path to the system clipboard when
  // the selection is captured by the guest app (e.g. `tmux set -g mouse on`
  // with `set -g set-clipboard on`). We deliberately ignore queries
  // (payload === "?") so remote code can't exfiltrate clipboard contents.
  term.parser.registerOscHandler(52, (data) => {
    // data looks like `<targets>;<base64>` — e.g. `c;SGVsbG8=`.
    const semi = data.indexOf(";");
    if (semi < 0) return false;
    const payload = data.slice(semi + 1);
    if (!payload || payload === "?") return true; // refuse reads, but claim handled
    let text: string;
    try {
      text = atob(payload);
    } catch {
      return false;
    }
    // atob yields a binary string; decode as UTF-8.
    try {
      const bytes = Uint8Array.from(text, (ch) => ch.charCodeAt(0));
      text = new TextDecoder("utf-8").decode(bytes);
    } catch {
      // Fall back to the raw binary string if decoding fails.
    }
    copyToClipboard(text);
    return true;
  });

  // Bulletproof copy shortcuts. A TUI with mouse reporting on (Claude Code,
  // codex, tmux, vim) eats plain drag, so the user may Shift+drag to select
  // — and then needs a key they can actually press. Ctrl+Shift+C is the
  // standard Linux/Windows terminal copy shortcut; Cmd+C on Mac matches
  // native apps. We only intercept when there is a selection, so copy never
  // steals Ctrl+C / Cmd+C away from the shell.
  const sid = session.record.id;
  term.attachCustomKeyEventHandler((event) => {
    if (event.type !== "keydown") return true;
    const key = event.key.toLowerCase();
    const isMac = navigator.platform.toLowerCase().includes("mac");
    const copyCombo =
      (event.ctrlKey && event.shiftKey && !event.altKey && key === "c") ||
      (isMac && event.metaKey && !event.shiftKey && !event.altKey && key === "c");
    if (!copyCombo) return true;
    const text = term.hasSelection() ? term.getSelection() : "";
    if (!text) return true; // let Ctrl+C / Cmd+C pass through as SIGINT / native
    copyToClipboard(text);
    event.preventDefault();
    event.stopPropagation();
    return false;
  });

  // Send initial terminal size to the daemon immediately after fit.
  // Browsers don't guarantee a ResizeObserver callback on first attach,
  // so the PTY could stay at 80x24 without this.
  if (sendCommand) {
    sendCommand({
      type: "resize_terminal",
      session_id: session.record.id,
      rows: term.rows,
      cols: term.cols,
    });
  }

  // Auto-copy on select (matches GTK's connect_selection_changed behavior).
  term.onSelectionChange(() => {
    if (term.hasSelection()) {
      copyToClipboard(term.getSelection());
    }
  });

  // Stream WebSocket with auto-reconnect.
  let ws: WebSocket = connectStream(sid, term);
  let reconnecting = false;

  function connectStream(sessionId: number, terminal: Terminal): WebSocket {
    const protocol = location.protocol === "https:" ? "wss:" : "ws:";
    const sock = new WebSocket(
      `${protocol}//${location.host}/ws/stream/${sessionId}`
    );
    sock.binaryType = "arraybuffer";
    // After the replay buffer streams in, scroll to bottom.
    let scrollTimer: ReturnType<typeof setTimeout> | null = null;
    sock.onmessage = (event) => {
      terminal.write(new Uint8Array(event.data));
      // Reset the scroll-to-bottom timer on each message. Once data
      // stops arriving (replay complete), scroll to bottom.
      if (scrollTimer) clearTimeout(scrollTimer);
      scrollTimer = setTimeout(() => terminal.scrollToBottom(), 200);
    };
    sock.onclose = () => {
      // Reconnect if the same terminal instance is still managed, live,
      // and not replaced by a new attach.
      const currentEntry = terminals.get(sessionId);
      if (!reconnecting && currentEntry && currentEntry.term === term && currentEntry.live) {
        reconnecting = true;
        setTimeout(() => {
          const entry = terminals.get(sessionId);
          if (entry && entry.term === term && entry.live) {
            ws = connectStream(sessionId, term);
            entry.ws = ws;
          }
          reconnecting = false;
        }, 1000);
      }
    };
    return sock;
  }

  term.onData((data) => {
    if (syncInputsEnabled) {
      const encoded = new TextEncoder().encode(data);
      for (const t of terminals.values()) {
        if (t.ws.readyState === WebSocket.OPEN) {
          t.ws.send(encoded);
        }
      }
    } else {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(new TextEncoder().encode(data));
      }
    }
  });

  let resizeTimer: ReturnType<typeof setTimeout> | null = null;
  const resizeObserver = new ResizeObserver(() => {
    if (resizeTimer) clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      fit.fit();
      if (sendCommand) {
        sendCommand({
          type: "resize_terminal",
          session_id: session.record.id,
          rows: term.rows,
          cols: term.cols,
        });
      }
    }, 50);
  });
  resizeObserver.observe(wrapper);

  const managed: ManagedTerminal = {
    sessionId: session.record.id,
    term,
    fit,
    wrapper,
    ws,
    resizeObserver,
    live: true,
  };
  terminals.set(session.record.id, managed);
  return managed;
}

/** Hide a terminal without disposing it — preserves scrollback and WebSocket. */
export function hideTerminal(sessionId: number) {
  const managed = terminals.get(sessionId);
  if (!managed) return;
  managed.resizeObserver?.disconnect();
  managed.resizeObserver = null;
  managed.term.clearSelection();
  managed.term.blur();
  managed.wrapper.style.display = "none";
}

/** Re-show a hidden terminal in a container. */
export function showTerminal(sessionId: number, container: HTMLElement) {
  const managed = terminals.get(sessionId);
  if (!managed) return;

  // If the terminal is already in this container, do nothing.
  if (
    managed.wrapper.parentElement === container &&
    managed.wrapper.style.display !== "none"
  ) {
    return;
  }

  if (managed.wrapper.parentElement !== container) {
    container.replaceChildren(managed.wrapper);
  } else {
    container
      .querySelectorAll(".card-scrollback-text, .card-scrollback-empty")
      .forEach((node) => node.remove());
  }
  managed.wrapper.style.display = "";
  // Re-attach resize observer.
  let resizeTimer: ReturnType<typeof setTimeout> | null = null;
  const ro = new ResizeObserver(() => {
    if (resizeTimer) clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      managed.fit.fit();
      if (sendCommand) {
        sendCommand({
          type: "resize_terminal",
          session_id: sessionId,
          rows: managed.term.rows,
          cols: managed.term.cols,
        });
      }
    }, 50);
  });
  managed.resizeObserver = ro;
  // Delay fit + observer until the layout has settled to avoid
  // triggering a resize while the element is being re-parented
  // (which causes garbled characters from the shell redraw).
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      managed.fit.fit();
      ro.observe(managed.wrapper);
    });
  });
}

/** Stop reconnecting the stream WebSocket for a dead session.
 *  The terminal stays in the map (scrollback preserved) until detached. */
export function markTerminalDead(sessionId: number) {
  const managed = terminals.get(sessionId);
  if (managed) managed.live = false;
}

export function detachTerminal(sessionId: number) {
  const managed = terminals.get(sessionId);
  if (!managed) return;
  managed.resizeObserver?.disconnect();
  managed.ws.close();
  managed.term.dispose();
  terminals.delete(sessionId);
}
