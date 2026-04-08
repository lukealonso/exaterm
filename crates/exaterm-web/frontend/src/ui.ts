import type {
  SessionSnapshot,
  WorkspaceSnapshot,
  ObservationSnapshot,
  TacticalSynthesis,
  AttentionLevel,
  SessionStatus,
  ClientMessage,
} from "./protocol";
import {
  attachTerminal,
  detachTerminal,
  hideTerminal,
  showTerminal,
  getTerminal,
  getAllTerminals,
  isSyncInputs,
  setSyncInputs,
  sendTextToSession,
  markTerminalDead,
} from "./terminal";

// --- Status derivation (port of supervision.rs) ---

type BattleCardStatus =
  | "idle"
  | "stopped"
  | "active"
  | "thinking"
  | "working"
  | "blocked"
  | "failed"
  | "complete"
  | "detached";

function deriveBattleCardStatus(
  status: SessionStatus,
  obs: ObservationSnapshot,
  summary: TacticalSynthesis | null
): BattleCardStatus {
  // For terminal states, always use the daemon's status (updates immediately)
  // rather than the LLM synthesis (which can lag by 5-30s).
  if (typeof status === "object" && "Failed" in status) return "failed";
  if (status === "Complete") return "complete";
  if (status === "Detached") return "detached";

  // For Running/Waiting, blend: if the daemon shows recent activity
  // (last_change < 5s) show "active" regardless of stale LLM state.
  if (summary && obs.last_change_age_secs < 5
      && (summary.tactical_state === "idle" || summary.tactical_state === "stopped")) {
    return "active";
  }

  if (summary) {
    switch (summary.tactical_state) {
      case "idle":
      case "stopped":
      case "thinking":
      case "working":
      case "blocked":
      case "failed":
      case "complete":
      case "detached":
        return summary.tactical_state;
      default:
        return "active";
    }
  }
  const shellReady = obs.active_command === "Interactive shell ready";
  const hasRuntimeEvidence =
    (obs.active_command !== null && obs.active_command !== "Interactive shell ready") ||
    obs.dominant_process !== null ||
    obs.work_output_excerpt !== null ||
    obs.recent_files.length > 0;

  // No summary available — derive from daemon status + observation.
  switch (status) {
    case "Blocked": return "active";
    case "Launching": return "active";
    case "Waiting":
      if (hasRuntimeEvidence) return "active";
      if (shellReady || obs.last_change_age_secs >= 30) return "idle";
      return "active";
    case "Running":
      if (obs.last_change_age_secs >= 30 && obs.active_command === null && obs.dominant_process === null)
        return "idle";
      return "active";
    default: return "active";
  }
}

function recencyLabel(idleSecs: number, status: BattleCardStatus): string {
  if (status === "idle" || status === "stopped") return `idle ${idleSecs}s`;
  if (idleSecs < 5) return "active now";
  return `active ${idleSecs}s ago`;
}

function statusChipLabel(status: BattleCardStatus, recency: string): string {
  if ((status === "idle" || status === "stopped") && recency.startsWith("idle ")) {
    const secs = recency.slice(5);
    return `${status.toUpperCase()} - ${secs}`;
  }
  return status.charAt(0).toUpperCase() + status.slice(1);
}

// --- Layout (port of layout.rs) ---

const EMBEDDED_TERMINAL_MIN_WIDTH = 8 * 80 + 72;
const EMBEDDED_TERMINAL_MIN_HEIGHT = 18 * 24 + 168;

export function battlefieldColumns(total: number, availableWidth: number, focused: boolean): number {
  if (total === 0) return 0;
  if (focused) return total;
  if (total === 1) return 1;
  if (total === 2) return Math.floor(availableWidth / 2) >= EMBEDDED_TERMINAL_MIN_WIDTH ? 2 : 1;
  if (total === 4) return 2;
  if (total === 6) return 3;
  if (total <= 4) return availableWidth >= 1800 ? total : 2;
  if (total === 5) return Math.max(3, Math.min(5, Math.floor(availableWidth / 420)));
  return Math.max(3, Math.min(Math.min(4, total), Math.floor(availableWidth / 380)));
}

export function battlefieldCanEmbedTerminals(
  total: number, columns: number, availableWidth: number, availableHeight: number
): boolean {
  if (total === 0 || columns === 0) return false;
  const tileWidth = (Math.max(0, availableWidth) - (columns - 1) * 12 - 24) / columns;
  const rows = Math.ceil(total / columns);
  const tileHeight = rows > 0 ? (Math.max(0, availableHeight) - (rows - 1) * 12 - 24) / rows : 0;
  return tileWidth >= EMBEDDED_TERMINAL_MIN_WIDTH && tileHeight >= EMBEDDED_TERMINAL_MIN_HEIGHT;
}

// --- Attention helpers ---

const ATTENTION_LEVELS: Record<AttentionLevel, { index: number; label: string; css: string; barCss: string }> = {
  autopilot: { index: 1, label: "AUTOPILOT", css: "focus-attention-1", barCss: "bar-attention-1" },
  monitor: { index: 2, label: "MONITOR", css: "focus-attention-2", barCss: "bar-attention-2" },
  guide: { index: 3, label: "GUIDE", css: "focus-attention-3", barCss: "bar-attention-3" },
  intervene: { index: 4, label: "INTERVENE", css: "focus-attention-4", barCss: "bar-attention-4" },
  takeover: { index: 5, label: "TAKEOVER", css: "focus-attention-5", barCss: "bar-attention-5" },
};

// --- Card DOM ---

interface CardElements {
  root: HTMLElement;
  title: HTMLElement;
  status: HTMLElement;
  headline: HTMLElement;
  attentionPill: HTMLElement;
  nudgeState: HTMLElement;
  middle: HTMLElement;
  momentumSegments: HTMLElement[];
  momentumReason: HTMLElement;
  recency: HTMLElement;
  terminalSlot: HTMLElement;
}

const cards = new Map<number, CardElements>();

function createCard(session: SessionSnapshot): CardElements {
  const root = document.createElement("div");
  root.className = "battle-card card-active";
  root.dataset.sessionId = String(session.record.id);

  root.innerHTML = `
    <div class="card-header-row">
      <div class="card-header-left">
        <span class="card-title"></span>
      </div>
      <span class="card-status battle-active"></span>
    </div>
    <div class="card-headline-row">
      <span class="card-headline"></span>
      <span class="card-attention-pill focus-attention-pill" style="display:none"></span>
    </div>
    <div class="card-nudge-row">
      <span class="card-nudge-state" style="display:none"></span>
      <button class="card-close-btn" title="Close shell (sends exit)">&#x2715;</button>
    </div>
    <div class="card-middle">
      <div class="card-terminal-slot"></div>
    </div>
    <div class="card-footer">
      <div class="bar-widget">
        <div class="bar-caption">Attention Condition</div>
        <div class="segmented-bar">
          <div class="bar-segment bar-empty"></div>
          <div class="bar-segment bar-empty"></div>
          <div class="bar-segment bar-empty"></div>
          <div class="bar-segment bar-empty"></div>
          <div class="bar-segment bar-empty"></div>
        </div>
        <div class="bar-reason"></div>
      </div>
      <div class="card-recency"></div>
    </div>
  `;

  const el = (sel: string) => root.querySelector(sel) as HTMLElement;
  return {
    root,
    title: el(".card-title"),
    status: el(".card-status"),
    headline: el(".card-headline"),
    attentionPill: el(".card-attention-pill"),
    nudgeState: el(".card-nudge-state"),
    middle: el(".card-middle"),
    momentumSegments: Array.from(root.querySelectorAll(".bar-segment")),
    momentumReason: el(".bar-reason"),
    recency: el(".card-recency"),
    terminalSlot: el(".card-terminal-slot"),
  };
}

const ALL_CARD_STATUS_CLASSES = [
  "card-idle", "card-stopped", "card-active", "card-thinking",
  "card-working", "card-blocked", "card-failed", "card-complete", "card-detached",
];
const ALL_BATTLE_STATUS_CLASSES = [
  "battle-idle", "battle-stopped", "battle-active", "battle-thinking",
  "battle-working", "battle-blocked", "battle-failed", "battle-complete", "battle-detached",
];
const ALL_ATTENTION_PILL_CLASSES = [
  "focus-attention-1", "focus-attention-2", "focus-attention-3",
  "focus-attention-4", "focus-attention-5",
];
const ALL_BAR_CLASSES = [
  "bar-attention-1", "bar-attention-2", "bar-attention-3",
  "bar-attention-4", "bar-attention-5", "bar-empty",
];

function updateCard(card: CardElements, session: SessionSnapshot, embedTerminal: boolean) {
  const status = deriveBattleCardStatus(session.record.status, session.observation, session.summary);
  const recency = recencyLabel(session.observation.last_change_age_secs, status);
  const hasSummary = session.summary !== null;

  // Title
  card.title.textContent = session.record.display_name || session.record.launch.name;

  // Status chip
  card.status.textContent = statusChipLabel(status, recency);
  ALL_BATTLE_STATUS_CLASSES.forEach((c) => card.status.classList.remove(c));
  card.status.classList.add(`battle-${status}`);

  // Card background
  ALL_CARD_STATUS_CLASSES.forEach((c) => card.root.classList.remove(c));
  card.root.classList.add(`card-${status}`);

  // Headline
  const headline = session.summary?.headline ?? "";
  card.headline.textContent = headline;
  card.headline.style.display = headline ? "" : "none";

  // Attention pill
  if (session.summary) {
    const attn = ATTENTION_LEVELS[session.summary.attention_level];
    card.attentionPill.textContent = attn.label;
    ALL_ATTENTION_PILL_CLASSES.forEach((c) => card.attentionPill.classList.remove(c));
    card.attentionPill.classList.add(attn.css);
    card.attentionPill.style.display = "";
    card.attentionPill.title = session.summary.attention_brief ?? "";
  } else {
    card.attentionPill.style.display = "none";
  }

  // Nudge state with hover and cooldown
  const nudgeEnabled = session.auto_nudge_enabled;
  const nudgeCooldown = nudgeEnabled && session.last_sent_age_secs !== null && session.last_sent_age_secs < 120;
  let defaultLabel: string;
  if (nudgeCooldown) {
    defaultLabel = "AUTONUDGE COOLDOWN";
    card.nudgeState.className = "card-nudge-state card-control-state card-control-cooldown";
  } else if (nudgeEnabled && session.last_nudge) {
    defaultLabel = "AUTONUDGE NUDGED";
    card.nudgeState.className = "card-nudge-state card-control-state card-control-nudged";
  } else if (nudgeEnabled) {
    defaultLabel = "AUTONUDGE ARMED";
    card.nudgeState.className = "card-nudge-state card-control-state card-control-armed";
  } else {
    defaultLabel = "AUTONUDGE OFF";
    card.nudgeState.className = "card-nudge-state card-control-state card-control-off";
  }
  card.nudgeState.textContent = defaultLabel;
  card.nudgeState.style.display = "";

  const hoverLabel = nudgeEnabled ? "DISARM AUTONUDGE" : "ARM AUTONUDGE";
  card.nudgeState.onmouseenter = () => { card.nudgeState.textContent = hoverLabel; };
  card.nudgeState.onmouseleave = () => { card.nudgeState.textContent = defaultLabel; };
  card.nudgeState.onclick = (e) => {
    e.stopPropagation();
    if (onSendCommand) {
      onSendCommand({ type: "toggle_auto_nudge", session_id: session.record.id, enabled: !nudgeEnabled });
    }
  };

  // Momentum bar
  if (session.summary) {
    const attn = ATTENTION_LEVELS[session.summary.attention_level];
    card.momentumSegments.forEach((seg, i) => {
      ALL_BAR_CLASSES.forEach((c) => seg.classList.remove(c));
      seg.classList.add(i < attn.index ? attn.barCss : "bar-empty");
    });
    const reason = session.summary.attention_brief ?? "";
    card.momentumReason.textContent = reason;
    card.momentumReason.style.display = reason ? "" : "none";
  } else {
    card.momentumSegments.forEach((seg) => {
      ALL_BAR_CLASSES.forEach((c) => seg.classList.remove(c));
      seg.classList.add("bar-empty");
    });
    card.momentumReason.textContent = "";
    card.momentumReason.style.display = "none";
  }

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
      if (focusedSessionId === session.record.id) {
        focusedSessionId = null;
        selectedSessionId = null;
      }
      render();
    } else {
      sendTextToSession(session.record.id, "exit\n");
    }
  };

  // Chrome visibility (SparseShell vs Summarized)
  const showChrome = hasSummary;
  card.title.parentElement!.parentElement!.style.display = showChrome ? "" : "none";
  card.headline.parentElement!.style.display = showChrome ? "" : "none";
  card.root.querySelector<HTMLElement>(".card-footer")!.style.display = showChrome ? "" : "none";

  // Terminal embedding
  if (embedTerminal && session.raw_stream_socket_name) {
    card.terminalSlot.style.display = "";
    card.terminalSlot.classList.remove("scrollback-terminal-hidden");
    const existing = getTerminal(session.record.id);
    if (existing) {
      // Terminal exists (was hidden) — re-show it in this slot.
      showTerminal(session.record.id, card.terminalSlot);
    } else {
      // No terminal yet — create one.
      attachTerminal(session, card.terminalSlot);
    }
  } else {
    // Not embedding: hide the terminal (preserve scrollback) instead
    // of disposing it. Only dispose for dead sessions.
    const existing = getTerminal(session.record.id);
    if (existing) {
      hideTerminal(session.record.id);
    }
    // Show scrollback preview. Prefer the xterm buffer (has live content
    // from the alternate screen buffer) over observation.recent_lines
    // (which is stale for TUI apps like Claude Code).
    card.terminalSlot.classList.add("scrollback-terminal-hidden");
    card.terminalSlot.style.display = "";
    const termBuffer = existing?.term.buffer.active;
    let previewLines: string[];
    if (termBuffer && termBuffer.length > 0) {
      previewLines = [];
      const lastRow = Math.min(termBuffer.length - 1, termBuffer.baseY + termBuffer.cursorY);
      const startRow = Math.max(0, lastRow - 11);
      for (let i = startRow; i <= lastRow; i++) {
        const line = termBuffer.getLine(i)?.translateToString(true) ?? "";
        if (line.trim()) previewLines.push(line);
      }
    } else {
      previewLines = session.observation.recent_lines.slice(-12);
    }
    const scrollbackText = previewLines.join("\n");
    const pre = card.terminalSlot.querySelector(".card-scrollback-text");
    if (previewLines.length > 0) {
      if (!pre || pre.textContent !== scrollbackText) {
        const el = document.createElement("pre");
        el.className = "card-scrollback-text";
        el.textContent = scrollbackText;
        card.terminalSlot.replaceChildren(el);
      }
    } else {
      if (!card.terminalSlot.querySelector(".card-scrollback-empty")) {
        const el = document.createElement("div");
        el.className = "card-scrollback-empty";
        el.textContent = "Waiting for output...";
        card.terminalSlot.replaceChildren(el);
      }
    }
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
let currentSnapshot: WorkspaceSnapshot = { sessions: [] };
let onSendCommand: ((cmd: ClientMessage) => void) | null = null;
let selectedSessionId: number | null = null;
let focusedSessionId: number | null = null;
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
    const cardEl = (e.target as HTMLElement).closest(".battle-card") as HTMLElement | null;
    if (!cardEl || !cardEl.dataset.sessionId) return;
    if (e.button !== 0) return; // left-click only
    const sid = Number(cardEl.dataset.sessionId);
    if (focusedSessionId === null && selectedSessionId !== sid) {
      selectedSessionId = sid;
      render();
    }
  });

  // Click behavior matches GTK:
  // - In focus mode: click focused card → return to battlefield.
  // - In battlefield with embedded terminal: click → focus terminal input.
  // - In battlefield without embedded terminal: click → enter focus mode.
  gridEl.addEventListener("click", (e) => {
    const cardEl = (e.target as HTMLElement).closest(".battle-card") as HTMLElement | null;
    if (!cardEl || !cardEl.dataset.sessionId) return;
    const sid = Number(cardEl.dataset.sessionId);

    if (focusedSessionId !== null) {
      if (sid === focusedSessionId) {
        focusedSessionId = null;
        selectedSessionId = null;
        render();
        return;
      }
      focusedSessionId = sid;
      selectedSessionId = sid;
      render();
      return;
    }

    // Check if the terminal is actually embedded (visible) in the card,
    // not just alive in the background.
    const termSlot = cards.get(sid)?.terminalSlot;
    const terminalEmbedded = termSlot && !termSlot.classList.contains("scrollback-terminal-hidden");
    if (terminalEmbedded) {
      // Card has a visible embedded terminal: focus it.
      // Selection was already set by pointerdown; re-render only if needed.
      if (selectedSessionId !== sid) {
        selectedSessionId = sid;
        render();
      }
      const managed = getTerminal(sid);
      if (managed) managed.term.focus();
    } else {
      // Card doesn't embed a terminal: enter focus mode directly.
      focusCard(sid);
    }
  });

  // Right-click: context menu.
  gridEl.addEventListener("contextmenu", (e) => {
    const cardEl = (e.target as HTMLElement).closest(".battle-card") as HTMLElement | null;
    if (!cardEl || !cardEl.dataset.sessionId) return;
    e.preventDefault();
    showContextMenu(e.clientX, e.clientY, Number(cardEl.dataset.sessionId));
  });

  // Keyboard shortcuts (capture phase to beat xterm.js).
  document.addEventListener("keydown", (e) => {
    // Escape: exit focus mode.
    if (e.key === "Escape" && focusedSessionId !== null) {
      e.preventDefault();
      e.stopPropagation();
      focusedSessionId = null;
      selectedSessionId = null;
      render();
      return;
    }

    // Enter (no modifier) in battlefield — matches GTK behavior:
    // - If an embedded terminal already has focus: let Enter through to terminal.
    // - If selected card has embedded terminal but not focused: focus the terminal.
    // - If selected card has no embedded terminal: enter focus mode.
    if (e.key === "Enter" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey
        && selectedSessionId !== null && focusedSessionId === null) {
      // If ANY terminal has focus, let Enter through — don't steal it.
      const anyTerminalFocused = getAllTerminals().some(
        (t) => t.term.element?.contains(document.activeElement)
      );
      if (anyTerminalFocused) {
        return;
      }
      e.preventDefault();
      e.stopPropagation();
      const selectedSlot = cards.get(selectedSessionId)?.terminalSlot;
      const embedded = selectedSlot && !selectedSlot.classList.contains("scrollback-terminal-hidden");
      const managed = getTerminal(selectedSessionId);
      if (embedded && managed) {
        managed.term.focus();
        render();
      } else {
        focusCard(selectedSessionId);
      }
      return;
    }

    // Ctrl/Cmd+Enter: always enter focus mode (web-specific shortcut).
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)
        && selectedSessionId !== null && focusedSessionId === null) {
      e.preventDefault();
      e.stopPropagation();
      focusCard(selectedSessionId);
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

    // [ / ] with Ctrl/Cmd: navigate cards in battlefield.
    if (focusedSessionId === null && (e.key === "[" || e.key === "]") && (e.ctrlKey || e.metaKey)) {
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
  if (focusedSessionId !== null) {
    if (!snapshot.sessions.some((s) => s.record.id === focusedSessionId)) {
      focusedSessionId = null;
      selectedSessionId = null;
    }
  }
  render();
}

export function restartWorkspace() {
  dismissedSessionIds.clear();
  focusedSessionId = null;
  selectedSessionId = null;
  // Detach all terminals so fresh ones are created.
  for (const [id] of cards) {
    detachTerminal(id);
  }
  cards.clear();
  if (gridEl) gridEl.innerHTML = "";
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
  const managed = getTerminal(sessionId);
  if (managed) managed.term.focus();
}

function focusCard(sessionId: number) {
  focusedSessionId = sessionId;
  selectedSessionId = sessionId;
  render();
  requestAnimationFrame(() => {
    const managed = getTerminal(sessionId);
    if (managed) {
      managed.fit.fit();
      managed.term.focus();
    }
  });
}

function render() {
  if (!gridEl) return;

  // Preserve keyboard focus across render — snapshot updates must not
  // steal focus from the terminal the user is typing in.
  const activeElement = document.activeElement;

  const sessions = currentSnapshot.sessions.filter(
    (s) => !dismissedSessionIds.has(s.record.id)
  );

  // Clear stale selection/focus that references a dismissed or removed session.
  const visibleIds = new Set(sessions.map((s) => s.record.id));
  if (focusedSessionId !== null && !visibleIds.has(focusedSessionId)) {
    focusedSessionId = null;
  }
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

  const isFocused = focusedSessionId !== null;
  const width = gridEl.clientWidth;
  const height = gridEl.clientHeight;

  if (isFocused) {
    gridEl.className = "battlefield-grid focus-mode single-session";
    gridEl.style.gridTemplateColumns = "1fr";
    gridEl.style.gridTemplateRows = "1fr";
  } else {
    const cols = battlefieldColumns(sessions.length, width, false);
    gridEl.className = "battlefield-grid";
    gridEl.classList.toggle("single-session", sessions.length === 1);
    gridEl.style.gridTemplateColumns = `repeat(${cols}, 1fr)`;
    gridEl.style.gridTemplateRows = "";
  }

  const canEmbed = isFocused
    ? true
    : sessions.length === 1 ||
      battlefieldCanEmbedTerminals(sessions.length, battlefieldColumns(sessions.length, width, false), width, height);

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
    const isFocusedCard = isFocused && session.record.id === focusedSessionId;
    const isHidden = isFocused && !isFocusedCard;
    const embed = isFocusedCard || (!isFocused && canEmbed);

    card.root.classList.toggle("selected-card", selectedSessionId === session.record.id && !isFocused);
    card.root.classList.toggle("focused-card", isFocusedCard);
    card.root.style.display = isHidden ? "none" : "";

    updateCard(card, session, embed);
  }

  // Restore focus if it was stolen during render.
  if (activeElement && activeElement !== document.activeElement
      && document.body.contains(activeElement)) {
    (activeElement as HTMLElement).focus?.();
  }
}
