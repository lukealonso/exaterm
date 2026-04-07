// TypeScript interfaces matching exaterm-types serde JSON output.

// SessionId is a newtype tuple struct: serializes as a plain number.
export type SessionId = number;

export type SessionKind =
  | "WaitingShell"
  | "PlanningStream"
  | "RunningStream"
  | "BlockingPrompt"
  | "FailingTask";

// Unit variants serialize as strings, Failed(i32) as { "Failed": number }.
export type SessionStatus =
  | "Launching"
  | "Running"
  | "Waiting"
  | "Blocked"
  | "Complete"
  | "Detached"
  | { Failed: number };

export interface SessionLaunch {
  name: string;
  subtitle: string;
  program: string;
  args: string[];
  cwd: string | null;
  kind: SessionKind;
}

export interface SessionEvent {
  sequence: number;
  summary: string;
}

export interface SessionRecord {
  id: SessionId;
  launch: SessionLaunch;
  display_name: string | null;
  status: SessionStatus;
  pid: number | null;
  events: SessionEvent[];
}

export interface ObservationSnapshot {
  last_change_age_secs: number;
  recent_lines: string[];
  painted_line: string | null;
  shell_child_command: string | null;
  active_command: string | null;
  dominant_process: string | null;
  process_tree_excerpt: string | null;
  recent_files: string[];
  work_output_excerpt: string | null;
}

// snake_case via #[serde(rename_all = "snake_case")]
export type TacticalState =
  | "idle"
  | "stopped"
  | "thinking"
  | "working"
  | "blocked"
  | "failed"
  | "complete"
  | "detached";

// snake_case via #[serde(rename_all = "snake_case")]
export type AttentionLevel =
  | "autopilot"
  | "monitor"
  | "guide"
  | "intervene"
  | "takeover";

export interface TacticalSynthesis {
  tactical_state: TacticalState;
  tactical_state_brief: string | null;
  attention_level: AttentionLevel;
  attention_brief: string | null;
  headline: string | null;
}

export interface SessionSnapshot {
  record: SessionRecord;
  observation: ObservationSnapshot;
  summary: TacticalSynthesis | null;
  raw_stream_socket_name: string | null;
  auto_nudge_enabled: boolean;
  last_nudge: string | null;
  last_sent_age_secs: number | null;
}

export interface WorkspaceSnapshot {
  sessions: SessionSnapshot[];
}

export type ServerMessage =
  | { type: "workspace_snapshot"; snapshot: WorkspaceSnapshot }
  | { type: "error"; message: string };

export type ClientMessage =
  | { type: "attach_client" }
  | { type: "create_or_resume_default_workspace" }
  | { type: "add_terminals"; source_session: SessionId }
  | { type: "add_terminals_to"; source_session: SessionId; target_total: number }
  | { type: "add_one_terminal"; source_session: SessionId }
  | { type: "resize_terminal"; session_id: SessionId; rows: number; cols: number }
  | { type: "toggle_auto_nudge"; session_id: SessionId; enabled: boolean }
  | { type: "detach_client"; keep_alive: boolean }
  | { type: "terminate_workspace" };
