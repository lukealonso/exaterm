import type {
  SessionSnapshot,
  WorkspaceSnapshot,
  ObservationSnapshot,
  SessionStatus,
  ClientMessage,
} from "./protocol";
import {
  attachTerminal,
  detachTerminal,
  showTerminal,
  getTerminal,
  getAllTerminals,
  isSyncInputs,
  setSyncInputs,
  sendTextToSession,
  markTerminalDead,
} from "./terminal";

// --- Status derivation (port of supervision.rs) ---

type SessionTileStatus =
  | "idle"
  | "stopped"
  | "active"
  | "thinking"
  | "working"
  | "blocked"
  | "failed"
  | "complete"
  | "detached";

function deriveSessionTileStatus(
  status: SessionStatus,
  obs: ObservationSnapshot
): SessionTileStatus {
  // For terminal states, always use the daemon's status (updates immediately)
  if (typeof status === "object" && "Failed" in status) return "failed";
  if (status === "Complete") return "complete";
  if (status === "Detached") return "detached";

  switch (status) {
    case "Blocked": return "active";
    case "Launching": return "active";
    case "Waiting":
      if (obs.last_change_age_secs >= 30) return "idle";
      return "active";
    case "Running":
      if (obs.last_change_age_secs >= 30) return "idle";
      return "active";
    default: return "active";
  }
}

function recencyLabel(idleSecs: number, status: SessionTileStatus): string {
  if (status === "idle" || status === "stopped") return `idle ${idleSecs}s`;
  if (idleSecs < 5) return "active now";
  return `active ${idleSecs}s ago`;
}

function statusChipLabel(status: SessionTileStatus, recency: string): string {
  if ((status === "idle" || status === "stopped") && recency.startsWith("idle ")) {
    const secs = recency.slice(5);
    return `${status.toUpperCase()} - ${secs}`;
  }
  return status.charAt(0).toUpperCase() + status.slice(1);
}

// --- Layout (port of layout.rs) ---

const EMBEDDED_TERMINAL_MIN_WIDTH = 8 * 80 + 72;

export function battlefieldColumns(total: number, availableWidth: number): number {
  if (total === 0) return 0;
  if (total === 1) return 1;
  if (total === 2) return Math.floor(availableWidth / 2) >= EMBEDDED_TERMINAL_MIN_WIDTH ? 2 : 1;
  if (total === 4) return 2;
  if (total === 6) return 3;
  if (total === 9) return 3;
  if (total <= 4) return availableWidth >= 1800 ? total : 2;
  if (total === 5) return Math.max(3, Math.min(5, Math.floor(availableWidth / 420)));
  return Math.max(3, Math.min(Math.min(4, total), Math.floor(availableWidth / 380)));
}

// --- Card DOM ---

interface CardElements {
  root: HTMLElement;
  title: HTMLElement;
  status: HTMLElement;
  recency: HTMLElement;
  terminalSlot: HTMLElement;
}

const cards = new Map<number, CardElements>();

function createCard(session: SessionSnapshot): CardElements {
  const root = document.createElement("div");
  root.className = "terminal-tile card-active";
  root.dataset.sessionId = String(session.record.id);

  root.innerHTML = `
    <div class="card-header-row">
      <div class="card-header-left">
        <span class="card-title"></span>
      </div>
      <span class="card-status tile-active"></span>
    </div>
    <div class="card-nudge-row">
      <button class="card-close-btn" title="Close shell (sends exit)">&#x2715;</button>
    </div>
    <div class="card-middle">
      <div class="card-terminal-slot"></div>
    </div>
    <div class="card-footer">
      <div class="card-recency"></div>
    </div>
  `;

  const el = (sel: string) => root.querySelector(sel) as HTMLElement;
  return {
    root,
    title: el(".card-title"),
    status: el(".card-status"),
    recency: el(".card-recency"),
    terminalSlot: el(".card-terminal-slot"),
  };
}

const ALL_CARD_STATUS_CLASSES = [
  "card-idle", "card-stopped", "card-active", "card-thinking",
  "card-working", "card-blocked", "card-failed", "card-complete", "card-detached",
];
const ALL_TILE_STATUS_CLASSES = [
  "tile-idle", "tile-stopped", "tile-active", "tile-thinking",
  "tile-working", "tile-blocked", "tile-failed", "tile-complete", "tile-detached",
];
function updateCard(card: CardElements, session: SessionSnapshot) {
  const status = deriveSessionTileStatus(session.record.status, session.observation);
  const recency = recencyLabel(session.observation.last_change_age_secs, status);

  // Title
  card.title.textContent = session.record.display_name || session.record.launch.name;

  // Status chip
  card.status.textContent = statusChipLabel(status, recency);
  ALL_TILE_STATUS_CLASSES.forEach((c) => card.status.classList.remove(c));
  card.status.classList.add(`tile-${status}`);

  // Card background
  ALL_CARD_STATUS_CLASSES.forEach((c) => card.root.classList.remove(c));
  card.root.classList.add(`card-${status}`);

  // Recency
  card.recency.textContent = recency;

  // Close button — context-aware: exit running sessions, dismiss dead ones.
  const isDeadSession =
    session.record.status === "Complete" ||
    session.record.status === "Detached" ||
    (typeof session.record.status === "object" && "Failed" in session.record.status);
  if (isDeadSession) {
    markTerminalDead(session.record.id);
  }
  const closeBtn = card.root.querySelector<HTMLElement>(".card-close-btn")!;
  closeBtn.title = isDeadSession ? "Dismiss card" : "Close shell (sends exit)";
  closeBtn.onclick = (e) => {
    e.stopPropagation();
    if (isDeadSession) {
      dismissedSessionIds.add(session.record.id);
      selectedSessionId = null;
      render();
    } else {
      sendTextToSession(session.record.id, "exit\n");
    }
  };

  card.title.parentElement!.parentElement!.style.display = "";
  card.root.querySelector<HTMLElement>(".card-footer")!.style.display = "";

  // Terminal tiles always embed the live terminal when a raw stream exists.
  if (session.raw_stream_socket_name) {
    card.terminalSlot.style.display = "";
    const existing = getTerminal(session.record.id);
    if (existing) {
      showTerminal(session.record.id, card.terminalSlot);
    } else {
      attachTerminal(session, card.terminalSlot);
    }
  } else {
    card.terminalSlot.replaceChildren();
  }
}

// --- Context Menu ---

let contextMenuEl: HTMLElement | null = null;
let contextMenuSessionId: number | null = null;

function createContextMenu(): HTMLElement {
  const menu = document.createElement("div");
  menu.className = "context-menu hidden";
  menu.innerHTML = `
    <div class="context-menu-item" data-action="copy">Copy</div>
    <div class="context-menu-item" data-action="paste">Paste</div>
    <div class="context-menu-divider"></div>
    <div class="context-menu-item" data-action="add-terminals">Add Terminals</div>
    <div class="context-menu-divider"></div>
    <div class="context-menu-item" data-action="insert-number-1">Insert Terminal Number (1-base)</div>
    <div class="context-menu-item" data-action="insert-number-0">Insert Terminal Number (0-base)</div>
    <div class="context-menu-divider"></div>
    <div class="context-menu-item" data-action="sync-inputs">
      <span class="context-menu-check"></span>Synchronize Inputs
    </div>
  `;

  menu.addEventListener("click", (e) => {
    const item = (e.target as HTMLElement).closest("[data-action]") as HTMLElement | null;
    if (!item || contextMenuSessionId === null || !item.dataset.action) return;
    handleContextMenuAction(item.dataset.action, contextMenuSessionId);
    hideContextMenu();
  });

  document.body.appendChild(menu);
  return menu;
}

function showContextMenu(x: number, y: number, sessionId: number) {
  if (!contextMenuEl) contextMenuEl = createContextMenu();
  contextMenuSessionId = sessionId;

  // Update sync inputs checkmark.
  const syncItem = contextMenuEl.querySelector('[data-action="sync-inputs"] .context-menu-check')!;
  syncItem.textContent = isSyncInputs() ? "\u2713 " : "";

  // Update copy enabled state.
  const copyItem = contextMenuEl.querySelector('[data-action="copy"]') as HTMLElement;
  const managed = getTerminal(sessionId);
  const hasSelection = managed?.term.hasSelection() ?? false;
  copyItem.classList.toggle("disabled", !hasSelection);

  // Update add terminals enabled state — always enabled since we send add_one_terminal.
  const addItem = contextMenuEl.querySelector('[data-action="add-terminals"]') as HTMLElement;
  addItem.classList.remove("disabled");

  contextMenuEl.style.left = `${x}px`;
  contextMenuEl.style.top = `${y}px`;
  contextMenuEl.classList.remove("hidden");

  // Close on next click anywhere.
  setTimeout(() => {
    document.addEventListener("click", hideContextMenu, { once: true });
  }, 0);
}

function hideContextMenu() {
  if (contextMenuEl) contextMenuEl.classList.add("hidden");
  contextMenuSessionId = null;
}

function handleContextMenuAction(action: string, sessionId: number) {
  const managed = getTerminal(sessionId);
  switch (action) {
    case "copy":
      if (managed?.term.hasSelection()) {
        navigator.clipboard.writeText(managed.term.getSelection());
      }
      break;
    case "paste":
      navigator.clipboard.readText()
        .then((text) => {
          sendTextToSession(sessionId, text);
        })
        .catch(() => {
          console.warn("Clipboard access denied or unavailable");
        });
      break;
    case "add-terminals":
      if (onSendCommand) {
        onSendCommand({ type: "add_one_terminal", source_session: sessionId });
      }
      break;
    case "insert-number-1":
      insertTerminalNumber(sessionId, true);
      break;
    case "insert-number-0":
      insertTerminalNumber(sessionId, false);
      break;
    case "sync-inputs":
      setSyncInputs(!isSyncInputs());
      break;
  }
}

function insertTerminalNumber(sourceSessionId: number, oneBased: boolean) {
  const ids = currentSnapshot.sessions
    .filter((s) => !dismissedSessionIds.has(s.record.id))
    .map((s) => s.record.id);
  if (isSyncInputs()) {
    // Send each session's own index to its terminal.
    ids.forEach((id, i) => {
      const num = oneBased ? i + 1 : i;
      sendTextToSession(id, String(num));
    });
  } else {
    const idx = ids.indexOf(sourceSessionId);
    if (idx >= 0) {
      const num = oneBased ? idx + 1 : idx;
      sendTextToSession(sourceSessionId, String(num));
    }
  }
}

// --- Battlefield Grid ---

let gridEl: HTMLElement | null = null;
let resizeObserver: ResizeObserver | null = null;
let currentSnapshot: WorkspaceSnapshot = { items: [], sessions: [], groups: [] };
let onSendCommand: ((cmd: ClientMessage) => void) | null = null;
let selectedSessionId: number | null = null;
const dismissedSessionIds = new Set<number>();

export function init(appEl: HTMLElement, sendFn: (cmd: ClientMessage) => void) {
  onSendCommand = sendFn;

  gridEl = document.createElement("div");
  gridEl.className = "battlefield-grid";
  appEl.innerHTML = "";
  appEl.appendChild(gridEl);

  let renderTimer: ReturnType<typeof setTimeout> | null = null;
  resizeObserver = new ResizeObserver(() => {
    if (renderTimer) clearTimeout(renderTimer);
    renderTimer = setTimeout(() => render(), 100);
  });
  resizeObserver.observe(gridEl);

  // Selection on pointerdown so it fires even when xterm.js swallows the
  // subsequent click (e.g. to start a text selection inside the terminal).
  gridEl.addEventListener("pointerdown", (e) => {
    const cardEl = (e.target as HTMLElement).closest(".terminal-tile") as HTMLElement | null;
    if (!cardEl || !cardEl.dataset.sessionId) return;
    if (e.button !== 0) return; // left-click only
    const sid = Number(cardEl.dataset.sessionId);
    if (selectedSessionId !== sid) {
      selectedSessionId = sid;
      render();
    }
  });

  // Click behavior matches GTK terminal tiles: select the tile and focus the
  // terminal in place.
  gridEl.addEventListener("click", (e) => {
    const cardEl = (e.target as HTMLElement).closest(".terminal-tile") as HTMLElement | null;
    if (!cardEl || !cardEl.dataset.sessionId) return;
    const sid = Number(cardEl.dataset.sessionId);

    if (selectedSessionId !== sid) {
      selectedSessionId = sid;
      render();
    }
    const managed = getTerminal(sid);
    if (managed) managed.term.focus();
  });

  // Right-click: context menu.
  gridEl.addEventListener("contextmenu", (e) => {
    const cardEl = (e.target as HTMLElement).closest(".terminal-tile") as HTMLElement | null;
    if (!cardEl || !cardEl.dataset.sessionId) return;
    e.preventDefault();
    showContextMenu(e.clientX, e.clientY, Number(cardEl.dataset.sessionId));
  });

  // Keyboard shortcuts (capture phase to beat xterm.js).
  document.addEventListener("keydown", (e) => {
    // Enter (no modifier) in the grid — matches GTK behavior:
    // - If an embedded terminal already has focus: let Enter through to terminal.
    // - Otherwise focus the selected terminal.
    if (e.key === "Enter" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey
        && selectedSessionId !== null) {
      // If ANY terminal has focus, let Enter through — don't steal it.
      const anyTerminalFocused = getAllTerminals().some(
        (t) => t.term.element?.contains(document.activeElement)
      );
      if (anyTerminalFocused) {
        return;
      }
      e.preventDefault();
      e.stopPropagation();
      const managed = getTerminal(selectedSessionId);
      if (managed) {
        managed.term.focus();
        render();
      }
      return;
    }

    // Ctrl/Cmd+Shift+N: add shells.
    if ((e.key === "N" || (e.key === "n" && e.shiftKey)) && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      e.stopPropagation();
      const first = currentSnapshot.sessions.find(
        (s) => !dismissedSessionIds.has(s.record.id)
      );
      if (first && onSendCommand) {
        onSendCommand({ type: "add_one_terminal", source_session: first.record.id });
      }
      return;
    }

    // [ / ] with Ctrl/Cmd: navigate tiles in the grid.
    if ((e.key === "[" || e.key === "]") && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      e.stopPropagation();
      const sessions = currentSnapshot.sessions.filter(
        (s) => !dismissedSessionIds.has(s.record.id)
      );
      if (sessions.length === 0) return;
      const ids = sessions.map((s) => s.record.id);
      const currentIdx = selectedSessionId !== null ? ids.indexOf(selectedSessionId) : -1;
      let nextIdx: number;
      if (e.key === "]") {
        nextIdx = currentIdx < ids.length - 1 ? currentIdx + 1 : 0;
      } else {
        nextIdx = currentIdx > 0 ? currentIdx - 1 : ids.length - 1;
      }
      selectCard(ids[nextIdx]);
    }
  }, true);
}

export function update(snapshot: WorkspaceSnapshot) {
  currentSnapshot = snapshot;
  const countEl = document.getElementById("session-count");
  if (countEl) {
    const n = snapshot.sessions.filter((s) => !dismissedSessionIds.has(s.record.id)).length;
    countEl.textContent = n > 0 ? `${n} session${n > 1 ? "s" : ""}` : "";
  }
  render();
}

export function restartWorkspace() {
  currentSnapshot = { items: [], sessions: [], groups: [] };
  dismissedSessionIds.clear();
  selectedSessionId = null;
  // Detach all terminals so fresh ones are created.
  for (const [id] of cards) {
    detachTerminal(id);
  }
  cards.clear();
  if (gridEl) gridEl.innerHTML = "";
  const countEl = document.getElementById("session-count");
  if (countEl) countEl.textContent = "";
}

export function getFirstSessionId(): number | null {
  const sessions = currentSnapshot.sessions.filter(
    (s) => !dismissedSessionIds.has(s.record.id)
  );
  return sessions.length > 0 ? sessions[0].record.id : null;
}

function selectCard(sessionId: number) {
  selectedSessionId = sessionId;
  render();
}

function render() {
  if (!gridEl) return;

  // Preserve keyboard focus across render — snapshot updates must not
  // steal focus from the terminal the user is typing in.
  const activeElement = document.activeElement;

  const sessions = currentSnapshot.sessions.filter(
    (s) => !dismissedSessionIds.has(s.record.id)
  );

  // Clear stale selection that references a dismissed or removed session.
  const visibleIds = new Set(sessions.map((s) => s.record.id));
  if (selectedSessionId !== null && !visibleIds.has(selectedSessionId)) {
    selectedSessionId = null;
  }

  if (sessions.length === 0) {
    for (const [id, card] of cards) {
      card.root.remove();
      detachTerminal(id);
      cards.delete(id);
    }
    gridEl.innerHTML = `<div class="empty-state">
      <div class="empty-title">No Live Sessions Yet</div>
      <div class="empty-body">Use Add Shell to start a real terminal-native agent or open an operator shell.<br>Exaterm opens into an empty battlefield so the workspace begins with your own sessions.</div>
    </div>`;
    gridEl.className = "battlefield-grid";
    gridEl.style.gridTemplateColumns = "1fr";
    return;
  }

  const emptyState = gridEl.querySelector(".empty-state");
  if (emptyState) emptyState.remove();

  const width = gridEl.clientWidth;

  const cols = battlefieldColumns(sessions.length, width);
  gridEl.className = "battlefield-grid";
  gridEl.classList.toggle("single-session", sessions.length === 1);
  gridEl.style.gridTemplateColumns = `repeat(${cols}, 1fr)`;
  gridEl.style.gridTemplateRows = "";

  const activeIds = new Set(sessions.map((s) => s.record.id));
  for (const [id, card] of cards) {
    if (!activeIds.has(id)) {
      card.root.remove();
      detachTerminal(id);
      cards.delete(id);
    }
  }

  for (const session of sessions) {
    if (!cards.has(session.record.id)) {
      const card = createCard(session);
      cards.set(session.record.id, card);
      gridEl.appendChild(card.root);
    }
  }

  for (const session of sessions) {
    const card = cards.get(session.record.id)!;
    card.root.classList.toggle("selected-card", selectedSessionId === session.record.id);
    card.root.style.display = "";

    updateCard(card, session);
  }

  // Restore focus if it was stolen during render.
  if (activeElement && activeElement !== document.activeElement
      && document.body.contains(activeElement)) {
    (activeElement as HTMLElement).focus?.();
  }
}
