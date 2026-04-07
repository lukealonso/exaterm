import "@xterm/xterm/css/xterm.css";
import type { ServerMessage, ClientMessage } from "./protocol";
import { setSendCommand } from "./terminal";
import { init as initUi, update as updateUi, getFirstSessionId, restartWorkspace } from "./ui";

// --- State ---

let controlWs: WebSocket | null = null;
const appEl = document.getElementById("app")!;
const overlayEl = document.getElementById("reconnect-overlay")!;

// --- Control WebSocket ---

function connectControl() {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const ws = new WebSocket(`${protocol}//${location.host}/ws/control`);
  controlWs = ws;

  ws.onopen = () => {
    overlayEl.classList.add("hidden");
  };

  ws.onmessage = (event) => {
    try {
      const msg: ServerMessage = JSON.parse(event.data);
      if (msg.type === "workspace_snapshot") {
        updateUi(msg.snapshot);
      }
    } catch (e) {
      console.error("failed to parse server message:", e);
    }
  };

  ws.onclose = () => {
    controlWs = null;
    overlayEl.classList.remove("hidden");
    setTimeout(connectControl, 2000);
  };

  ws.onerror = () => {
    ws.close();
  };
}

function sendCommand(cmd: ClientMessage) {
  if (controlWs && controlWs.readyState === WebSocket.OPEN) {
    controlWs.send(JSON.stringify(cmd));
  }
}

// --- Toolbar ---

const shortcutsOverlay = document.getElementById("shortcuts-overlay")!;
document.getElementById("shortcuts-btn")!.addEventListener("click", () => {
  shortcutsOverlay.classList.toggle("hidden");
});
document.getElementById("shortcuts-close-btn")!.addEventListener("click", () => {
  shortcutsOverlay.classList.add("hidden");
});
shortcutsOverlay.addEventListener("click", (e) => {
  if (e.target === shortcutsOverlay) shortcutsOverlay.classList.add("hidden");
});

document.getElementById("restart-btn")!.addEventListener("click", () => {
  if (!confirm("Terminate all sessions and start fresh?")) return;
  sendCommand({ type: "terminate_workspace" });
  restartWorkspace();
  // After termination, the daemon will send an empty snapshot.
  // Then we request a new default workspace.
  setTimeout(() => {
    sendCommand({ type: "create_or_resume_default_workspace" });
  }, 500);
});

const addShellBtn = document.getElementById("add-shell-btn")!;
addShellBtn.addEventListener("click", () => {
  const first = getFirstSessionId();
  if (first !== null) {
    sendCommand({ type: "add_one_terminal", source_session: first });
  }
});

// --- Init ---

setSendCommand(sendCommand);
initUi(appEl, sendCommand);
connectControl();
