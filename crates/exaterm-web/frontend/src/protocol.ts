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
}

export interface SessionSnapshot {
  record: SessionRecord;
  observation: ObservationSnapshot;
  raw_stream_socket_name: string | null;
}

export interface SupervisorActionRecord {
  sequence: number;
  summary: string;
  age_secs: number;
}

export type SupervisorProvider = "codex" | "claude" | "other";

export interface SupervisedGroupRecord {
  id: number;
  name: string;
  member_session_ids: SessionId[];
  supervisor_session_id: SessionId | null;
  provider: SupervisorProvider | null;
  goal: string | null;
  summary_markdown: string;
  supervisor_visible: boolean;
  summary_age_secs: number | null;
  latest_action_age_secs: number | null;
  actions: SupervisorActionRecord[];
}

export type WorkspaceItem =
  | { Session: SessionId }
  | { Group: number };

export interface WorkspaceSnapshot {
  items: WorkspaceItem[];
  sessions: SessionSnapshot[];
  groups: SupervisedGroupRecord[];
}

export interface TerminalDisplayCapabilities {
  kitty_graphics: boolean;
  sixel: boolean;
  vte_version: string | null;
}

export interface PetOrigin {
  seed_hash: string;
}

export interface PetComment {
  id: number;
  session_id: SessionId;
  name: string;
  appearance_ascii: string;
  message: string;
  ttl_secs: number;
}

export type ServerMessage =
  | { type: "workspace_snapshot"; snapshot: WorkspaceSnapshot }
  | { type: "terminal_assist_completed"; request_id: number; session_id: SessionId; inserted: boolean; error: string | null }
  | { type: "pet_comment"; comment: PetComment }
  | { type: "error"; message: string };

export type ClientMessage =
  | { type: "attach_client" }
  | { type: "set_terminal_display_capabilities"; capabilities: TerminalDisplayCapabilities }
  | { type: "set_pet_origin"; origin: PetOrigin }
  | { type: "create_or_resume_default_workspace" }
  | { type: "add_terminals"; source_session: SessionId }
  | { type: "add_terminals_to"; source_session: SessionId; target_total: number }
  | { type: "add_one_terminal"; source_session: SessionId }
  | { type: "resize_terminal"; session_id: SessionId; rows: number; cols: number }
  | { type: "close_session"; session_id: SessionId }
  | { type: "create_supervised_group"; name: string; session_ids: SessionId[]; goal: string | null }
  | { type: "set_group_supervisor_visible"; group_id: number; visible: boolean }
  | { type: "send_message_to_agent"; group_id: number; session_id: SessionId; message: string }
  | { type: "update_group_summary"; group_id: number; markdown: string }
  | { type: "request_terminal_assist"; request_id: number; session_id: SessionId; prompt: string }
  | { type: "detach_client"; keep_alive: boolean }
  | { type: "terminate_workspace" };
