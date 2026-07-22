use crate::model::SessionId;
use exaterm_types::synthesis::TerminalAssistSuggestion;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

const APP_SERVER_START_TIMEOUT: Duration = Duration::from_secs(8);
const APP_SERVER_RPC_TIMEOUT: Duration = Duration::from_secs(120);

const TERMINAL_ASSIST_DEVELOPER_INSTRUCTIONS: &str = r#"
You are the Ctrl-K terminal command assistant inside Exaterm.

For every turn, return exactly one JSON object matching the supplied output schema. The insert_text value is inserted at the cursor in a real terminal but is never executed automatically.

Use the current terminal evidence in the user message and the preceding turns in this thread. Treat follow-up requests as corrections or refinements of earlier suggestions. Return one shell command or compact one-line shell snippet. Do not include markdown, code fences, prose, labels, comments, or a trailing newline in insert_text.

Prefer non-destructive inspection, search, validation, and test commands. Do not propose destructive commands such as rm -rf, git reset --hard, force push, broad chmod or chown, killall, destructive sudo operations, package removal, or data deletion unless the operator explicitly requests that exact action and the target is unambiguous. If the request is unsafe, unrelated to terminal work, or cannot be answered reliably, return an empty insert_text.

You may inspect the working tree when useful, but never edit files or run commands that change repository or system state.
"#;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerminalAssistEvidence {
    pub session_name: String,
    pub operator_prompt: String,
    pub current_input: String,
    pub working_directory: Option<String>,
    pub shell_child_command: Option<String>,
    pub active_command: Option<String>,
    pub dominant_process: Option<String>,
    pub process_tree_excerpt: Option<String>,
    pub recent_files: Vec<String>,
    pub terminal_status_line: Option<String>,
    pub recent_terminal_history: Vec<String>,
}

pub struct CodexAppServerClient {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    child: Child,
    next_request_id: u64,
    pending_messages: VecDeque<Value>,
    terminal_threads: BTreeMap<SessionId, String>,
}

impl CodexAppServerClient {
    pub fn launch() -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("failed to reserve Codex app-server port: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("failed to read Codex app-server port: {error}"))?
            .port();
        drop(listener);

        let url = format!("ws://127.0.0.1:{port}");
        let mut child = Command::new("codex")
            .args(["app-server", "--listen", &url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("failed to start `codex app-server`: {error}"))?;

        let deadline = Instant::now() + APP_SERVER_START_TIMEOUT;
        let socket = loop {
            match tungstenite::connect(url.as_str()) {
                Ok((socket, _)) => break socket,
                Err(error) => {
                    if let Ok(Some(status)) = child.try_wait() {
                        return Err(format!(
                            "`codex app-server` exited during startup: {status}"
                        ));
                    }
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!(
                            "timed out connecting to `codex app-server` at {url}: {error}"
                        ));
                    }
                    thread::sleep(Duration::from_millis(40));
                }
            }
        };

        let mut client = Self {
            socket,
            child,
            next_request_id: 1,
            pending_messages: VecDeque::new(),
            terminal_threads: BTreeMap::new(),
        };
        client.set_socket_timeout(Duration::from_secs(15))?;
        client.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "exaterm",
                    "title": "Exaterm",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }),
        )?;
        client.send(json!({"method": "initialized", "params": {}}))?;
        client.set_socket_timeout(APP_SERVER_RPC_TIMEOUT)?;
        Ok(client)
    }

    pub fn suggest_terminal_assist(
        &mut self,
        session_id: SessionId,
        evidence: &TerminalAssistEvidence,
    ) -> Result<TerminalAssistSuggestion, String> {
        let thread_id = match self.terminal_threads.get(&session_id) {
            Some(thread_id) => thread_id.clone(),
            None => {
                let thread_id = self.start_terminal_thread(evidence)?;
                self.terminal_threads.insert(session_id, thread_id.clone());
                thread_id
            }
        };

        let result = self.request(
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{
                    "type": "text",
                    "text": terminal_assist_user_prompt(evidence),
                }],
                "outputSchema": terminal_assist_output_schema(),
            }),
        )?;
        let turn_id = result
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Codex turn/start response had no turn id: {result}"))?
            .to_string();

        self.wait_for_terminal_assist(&thread_id, &turn_id)
    }

    fn start_terminal_thread(
        &mut self,
        evidence: &TerminalAssistEvidence,
    ) -> Result<String, String> {
        let mut params = json!({
            "approvalPolicy": "never",
            "sandbox": "read-only",
            "personality": "pragmatic",
            "ephemeral": true,
            "developerInstructions": TERMINAL_ASSIST_DEVELOPER_INSTRUCTIONS,
        });
        if let Some(cwd) = evidence
            .working_directory
            .as_deref()
            .filter(|cwd| !cwd.trim().is_empty())
        {
            params["cwd"] = Value::String(cwd.to_string());
        }
        let result = self.request("thread/start", params)?;
        result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| format!("Codex thread/start response had no thread id: {result}"))
    }

    fn wait_for_terminal_assist(
        &mut self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<TerminalAssistSuggestion, String> {
        let mut completed_agent_message = None::<String>;
        let mut streamed_agent_message = String::new();

        loop {
            let message = match self.pending_messages.pop_front() {
                Some(message) => message,
                None => self.read()?,
            };

            if message.get("id").is_some() && message.get("method").is_some() {
                self.reject_server_request(&message)?;
                continue;
            }

            let method = message.get("method").and_then(Value::as_str);
            let params = message.get("params").unwrap_or(&Value::Null);
            match method {
                Some("item/agentMessage/delta")
                    if notification_matches(params, thread_id, turn_id) =>
                {
                    if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                        streamed_agent_message.push_str(delta);
                    }
                }
                Some("item/completed") if notification_matches(params, thread_id, turn_id) => {
                    if params.pointer("/item/type").and_then(Value::as_str) == Some("agentMessage")
                    {
                        completed_agent_message = params
                            .pointer("/item/text")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned);
                    }
                }
                Some("turn/completed") if notification_matches(params, thread_id, turn_id) => {
                    let status = params.pointer("/turn/status").and_then(Value::as_str);
                    if status != Some("completed") {
                        let error = params
                            .pointer("/turn/error/message")
                            .and_then(Value::as_str)
                            .unwrap_or("turn did not complete");
                        return Err(format!(
                            "Codex terminal assist failed ({status:?}): {error}"
                        ));
                    }
                    let final_message = completed_turn_agent_message(params)
                        .or(completed_agent_message)
                        .filter(|message| !message.trim().is_empty())
                        .unwrap_or(streamed_agent_message);
                    return parse_terminal_assist_response(&final_message);
                }
                _ => {}
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        for attempt in 0..=4u32 {
            let id = self.next_request_id;
            self.next_request_id = self.next_request_id.saturating_add(1);
            self.send(json!({"method": method, "id": id, "params": params.clone()}))?;

            loop {
                let message = self.read()?;
                if message.get("id").and_then(Value::as_u64) == Some(id)
                    && message.get("method").is_none()
                {
                    if let Some(error) = message.get("error") {
                        if error.get("code").and_then(Value::as_i64) == Some(-32001) && attempt < 4
                        {
                            let backoff_ms = 50u64 * (1u64 << attempt) + (id % 23);
                            thread::sleep(Duration::from_millis(backoff_ms));
                            break;
                        }
                        return Err(format!("Codex app-server {method} failed: {error}"));
                    }
                    return message
                        .get("result")
                        .cloned()
                        .ok_or_else(|| format!("Codex app-server {method} returned no result"));
                }
                self.pending_messages.push_back(message);
            }
        }
        unreachable!("Codex app-server overload retry loop always returns")
    }

    fn reject_server_request(&mut self, request: &Value) -> Result<(), String> {
        let Some(id) = request.get("id").cloned() else {
            return Ok(());
        };
        self.send(json!({
            "id": id,
            "error": {
                "code": -32601,
                "message": "Exaterm terminal assist does not handle interactive server requests",
            },
        }))
    }

    fn send(&mut self, value: Value) -> Result<(), String> {
        self.socket
            .send(Message::text(value.to_string()))
            .map_err(|error| format!("failed to write Codex app-server WebSocket: {error}"))
    }

    fn read(&mut self) -> Result<Value, String> {
        loop {
            let message = self
                .socket
                .read()
                .map_err(|error| format!("failed to read Codex app-server WebSocket: {error}"))?;
            match message {
                Message::Text(text) => {
                    return serde_json::from_str(text.as_str())
                        .map_err(|error| format!("invalid Codex app-server JSON: {error}"));
                }
                Message::Binary(bytes) => {
                    return serde_json::from_slice(&bytes)
                        .map_err(|error| format!("invalid Codex app-server JSON: {error}"));
                }
                Message::Close(frame) => {
                    return Err(format!("Codex app-server closed its WebSocket: {frame:?}"));
                }
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }

    fn set_socket_timeout(&mut self, timeout: Duration) -> Result<(), String> {
        if let MaybeTlsStream::Plain(stream) = self.socket.get_mut() {
            stream
                .set_read_timeout(Some(timeout))
                .and_then(|()| stream.set_write_timeout(Some(timeout)))
                .map_err(|error| format!("failed to configure Codex app-server socket: {error}"))?;
        }
        Ok(())
    }
}

impl Drop for CodexAppServerClient {
    fn drop(&mut self) {
        let _ = self.socket.close(None);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn notification_matches(params: &Value, thread_id: &str, turn_id: &str) -> bool {
    params.get("threadId").and_then(Value::as_str) == Some(thread_id)
        && (params.get("turnId").and_then(Value::as_str) == Some(turn_id)
            || params.pointer("/turn/id").and_then(Value::as_str) == Some(turn_id))
}

fn completed_turn_agent_message(params: &Value) -> Option<String> {
    params
        .pointer("/turn/items")?
        .as_array()?
        .iter()
        .rev()
        .find(|item| item.get("type").and_then(Value::as_str) == Some("agentMessage"))?
        .get("text")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn terminal_assist_user_prompt(evidence: &TerminalAssistEvidence) -> String {
    format!(
        "Suggest the terminal insertion requested by the operator. Current terminal evidence:\n{}",
        serde_json::to_string_pretty(evidence).unwrap_or_else(|_| "{}".into())
    )
}

fn terminal_assist_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "insert_text": {"type": "string"},
        },
        "required": ["insert_text"],
        "additionalProperties": false,
    })
}

fn parse_terminal_assist_response(text: &str) -> Result<TerminalAssistSuggestion, String> {
    let trimmed = text.trim();
    let suggestion = serde_json::from_str::<TerminalAssistSuggestion>(trimmed).or_else(|_| {
        let start = trimmed.find('{').ok_or(())?;
        let end = trimmed.rfind('}').ok_or(())?;
        serde_json::from_str::<TerminalAssistSuggestion>(&trimmed[start..=end]).map_err(|_| ())
    });
    suggestion
        .map(TerminalAssistSuggestion::sanitize)
        .map_err(|()| format!("Codex terminal assist returned invalid output: {trimmed}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence() -> TerminalAssistEvidence {
        TerminalAssistEvidence {
            session_name: "Shell 2".into(),
            operator_prompt: "find the largest Rust files".into(),
            current_input: "rg".into(),
            working_directory: Some("/tmp/repo".into()),
            shell_child_command: Some("bash".into()),
            active_command: None,
            dominant_process: None,
            process_tree_excerpt: Some("bash".into()),
            recent_files: vec!["src/main.rs".into()],
            terminal_status_line: Some("$ rg".into()),
            recent_terminal_history: vec!["$ cargo test".into()],
        }
    }

    #[test]
    fn terminal_assist_uses_strict_insert_text_schema() {
        let schema = terminal_assist_output_schema();
        assert_eq!(schema["required"], json!(["insert_text"]));
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn terminal_assist_prompt_includes_current_evidence_and_operator_request() {
        let prompt = terminal_assist_user_prompt(&evidence());
        assert!(prompt.contains("find the largest Rust files"));
        assert!(prompt.contains("src/main.rs"));
        assert!(prompt.contains("/tmp/repo"));
    }

    #[test]
    fn terminal_assist_response_is_sanitized_before_insertion() {
        let suggestion = parse_terminal_assist_response(
            r#"{"insert_text":"rg --files -g '*.rs' | xargs wc -l\nexplanation"}"#,
        )
        .expect("valid response");
        assert_eq!(suggestion.insert_text, "rg --files -g '*.rs' | xargs wc -l");
    }

    #[test]
    fn completed_turn_uses_last_agent_message() {
        let params = json!({
            "turn": {
                "items": [
                    {"type": "agentMessage", "text": "first"},
                    {"type": "reasoning", "summary": []},
                    {"type": "agentMessage", "text": "final"},
                ]
            }
        });
        assert_eq!(
            completed_turn_agent_message(&params).as_deref(),
            Some("final")
        );
    }

    #[test]
    #[ignore = "requires an installed Codex CLI"]
    fn codex_app_server_websocket_initializes() {
        let mut client =
            CodexAppServerClient::launch().expect("Codex app-server should initialize");
        let mut evidence = evidence();
        evidence.working_directory = std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string());
        let thread_id = client
            .start_terminal_thread(&evidence)
            .expect("Codex app-server should start a terminal thread");
        assert!(!thread_id.is_empty());
    }
}
