import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import type { ClientMessage, SessionSnapshot } from "./protocol";

export interface ManagedTerminal {
  sessionId: number;
  term: Terminal;
  fit: FitAddon;
  ws: WebSocket;
  resizeObserver: ResizeObserver | null;
}

const terminals = new Map<number, ManagedTerminal>();

let sendCommand: ((cmd: ClientMessage) => void) | null = null;
let syncInputsEnabled = false;

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
    const wrapper = container.querySelector(".xterm-wrapper");
    if (existing.term.element && existing.term.element.parentElement !== wrapper) {
      container.innerHTML = "";
      const newWrapper = document.createElement("div");
      newWrapper.className = "xterm-wrapper";
      container.appendChild(newWrapper);
      newWrapper.appendChild(existing.term.element);
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
    theme: {
      background: "#070d14",
      foreground: "#e2e8f0",
      cursor: "#e2e8f0",
      selectionBackground: "rgba(96, 165, 250, 0.3)",
    },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(wrapper);
  fit.fit();

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
      navigator.clipboard.writeText(term.getSelection()).catch(() => {});
    }
  });

  // Stream WebSocket with auto-reconnect.
  const sid = session.record.id;
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
      // Reconnect if the terminal is still in the map (not disposed).
      if (!reconnecting && terminals.has(sessionId)) {
        reconnecting = true;
        setTimeout(() => {
          if (terminals.has(sessionId)) {
            ws = connectStream(sessionId, terminal);
            // Update the managed entry's ws reference.
            const entry = terminals.get(sessionId);
            if (entry) entry.ws = ws;
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
    ws,
    resizeObserver,
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
  // Move the xterm element out of the visible DOM into a hidden holder.
  if (managed.term.element) {
    if (!hiddenHolder) {
      hiddenHolder = document.createElement("div");
      hiddenHolder.style.display = "none";
      document.body.appendChild(hiddenHolder);
    }
    hiddenHolder.appendChild(managed.term.element);
  }
}

/** Re-show a hidden terminal in a container. */
export function showTerminal(sessionId: number, container: HTMLElement) {
  const managed = terminals.get(sessionId);
  if (!managed) return;

  // If the terminal is already in this container, do nothing.
  const existingWrapper = container.querySelector(".xterm-wrapper");
  if (existingWrapper && managed.term.element?.parentElement === existingWrapper) {
    return;
  }

  // Ensure the container has a wrapper.
  let wrapper = existingWrapper as HTMLElement | null;
  if (!wrapper) {
    container.innerHTML = "";
    wrapper = document.createElement("div");
    wrapper.className = "xterm-wrapper";
    container.appendChild(wrapper);
  }
  if (managed.term.element) {
    wrapper.appendChild(managed.term.element);
  }
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
      ro.observe(wrapper!);
    });
  });
}

let hiddenHolder: HTMLElement | null = null;

export function detachTerminal(sessionId: number) {
  const managed = terminals.get(sessionId);
  if (!managed) return;
  managed.resizeObserver?.disconnect();
  managed.ws.close();
  managed.term.dispose();
  terminals.delete(sessionId);
}

