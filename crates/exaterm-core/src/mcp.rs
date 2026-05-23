use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

pub const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";

pub type ToolCallOutcome = Result<ToolCallResult, ToolCallError>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

impl ServerInfo {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(rename = "outputSchema", skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}

impl ToolDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: Some(description.into()),
            input_schema,
            output_schema: None,
        }
    }

    pub fn with_output_schema(mut self, output_schema: Value) -> Self {
        self.output_schema = Some(output_schema);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToolContent {
    Text { text: String },
}

impl ToolContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolCallResult {
    pub content: Vec<ToolContent>,
    #[serde(rename = "structuredContent", skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(rename = "isError", default, skip_serializing_if = "is_false")]
    pub is_error: bool,
}

impl ToolCallResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::text(text)],
            structured_content: None,
            is_error: false,
        }
    }

    pub fn structured(structured_content: Value, fallback_text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::text(fallback_text)],
            structured_content: Some(structured_content),
            is_error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::text(text)],
            structured_content: None,
            is_error: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCallError {
    pub message: String,
}

impl ToolCallError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<&str> for ToolCallError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl From<String> for ToolCallError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

pub trait ToolDispatcher {
    fn call_tool(&self, name: &str, arguments: Value) -> ToolCallOutcome;
}

impl<F> ToolDispatcher for F
where
    F: Fn(&str, Value) -> ToolCallOutcome,
{
    fn call_tool(&self, name: &str, arguments: Value) -> ToolCallOutcome {
        self(name, arguments)
    }
}

pub struct McpServer<D> {
    server_info: ServerInfo,
    tools: Vec<ToolDefinition>,
    dispatcher: D,
}

impl<D> McpServer<D>
where
    D: ToolDispatcher,
{
    pub fn new(server_info: ServerInfo, tools: Vec<ToolDefinition>, dispatcher: D) -> Self {
        Self {
            server_info,
            tools,
            dispatcher,
        }
    }

    pub fn handle_line(&self, line: &str) -> Option<String> {
        let request = match serde_json::from_str::<JsonRpcRequest>(line) {
            Ok(request) => request,
            Err(error) => {
                let response =
                    JsonRpcResponse::error(None, -32700, format!("Parse error: {error}"));
                return Some(serialize_line(&response));
            }
        };

        if request.id.is_none() {
            return None;
        }

        Some(serialize_line(&self.handle_request(request)))
    }

    pub fn serve<R, W>(&self, reader: R, mut writer: W) -> io::Result<()>
    where
        R: BufRead,
        W: Write,
    {
        for line in reader.lines() {
            if let Some(response) = self.handle_line(&line?) {
                writer.write_all(response.as_bytes())?;
                writer.write_all(b"\n")?;
                writer.flush()?;
            }
        }
        Ok(())
    }

    fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id.clone();
        match request.method.as_str() {
            "initialize" => JsonRpcResponse::result(id, self.initialize_result(request.params)),
            "tools/list" => JsonRpcResponse::result(id, json!({ "tools": self.tools })),
            "tools/call" => match parse_tool_call_params(request.params) {
                Ok(params) => {
                    let result = self
                        .dispatcher
                        .call_tool(
                            &params.name,
                            params
                                .arguments
                                .unwrap_or(Value::Object(Default::default())),
                        )
                        .unwrap_or_else(|error| ToolCallResult::error(error.message));
                    JsonRpcResponse::result(id, json!(result))
                }
                Err(message) => JsonRpcResponse::error(id, -32602, message),
            },
            _ => {
                JsonRpcResponse::error(id, -32601, format!("Method not found: {}", request.method))
            }
        }
    }

    fn initialize_result(&self, params: Option<Value>) -> Value {
        let protocol_version = params
            .as_ref()
            .and_then(|params| params.get("protocolVersion"))
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_PROTOCOL_VERSION);

        json!({
            "protocolVersion": protocol_version,
            "capabilities": {
                "tools": {}
            },
            "serverInfo": self.server_info
        })
    }
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    fn result(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    name: String,
    arguments: Option<Value>,
}

fn parse_tool_call_params(params: Option<Value>) -> Result<ToolCallParams, String> {
    let params = params.ok_or_else(|| "tools/call params are required".to_string())?;
    serde_json::from_value(params).map_err(|error| format!("Invalid tools/call params: {error}"))
}

fn serialize_line<T>(value: &T) -> String
where
    T: Serialize,
{
    serde_json::to_string(value).expect("JSON-RPC response serialization should not fail")
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_server<F>(dispatcher: F) -> McpServer<F>
    where
        F: Fn(&str, Value) -> ToolCallOutcome,
    {
        McpServer::new(
            ServerInfo::new("exaterm-test", "0.1.0"),
            vec![ToolDefinition::new(
                "session_status",
                "Read session status",
                json!({
                    "type": "object",
                    "properties": {
                        "sessionId": { "type": "string" }
                    },
                    "required": ["sessionId"]
                }),
            )],
            dispatcher,
        )
    }

    fn response_value(line: &str) -> Value {
        serde_json::from_str(line).expect("response should be valid JSON")
    }

    #[test]
    fn initialize_serializes_server_capabilities() {
        let server = test_server(|_, _| Ok(ToolCallResult::text("unused")));

        let response = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":"init-1","method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
            )
            .expect("request should produce a response");

        assert_eq!(
            response_value(&response),
            json!({
                "jsonrpc": "2.0",
                "id": "init-1",
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "exaterm-test",
                        "version": "0.1.0"
                    }
                }
            })
        );
    }

    #[test]
    fn tools_list_serializes_tool_definitions() {
        let server = test_server(|_, _| Ok(ToolCallResult::text("unused")));

        let response = server
            .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
            .expect("request should produce a response");

        assert_eq!(
            response_value(&response),
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "tools": [{
                        "name": "session_status",
                        "description": "Read session status",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "sessionId": { "type": "string" }
                            },
                            "required": ["sessionId"]
                        }
                    }]
                }
            })
        );
    }

    #[test]
    fn tools_call_success_serializes_structured_content_and_text_fallback() {
        let server = test_server(|name, arguments| {
            assert_eq!(name, "session_status");
            assert_eq!(arguments, json!({ "sessionId": "s1" }));
            Ok(ToolCallResult::structured(
                json!({ "sessionId": "s1", "state": "running" }),
                "s1 is running",
            ))
        });

        let response = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":"call-1","method":"tools/call","params":{"name":"session_status","arguments":{"sessionId":"s1"}}}"#,
            )
            .expect("request should produce a response");

        assert_eq!(
            response_value(&response),
            json!({
                "jsonrpc": "2.0",
                "id": "call-1",
                "result": {
                    "content": [{
                        "type": "text",
                        "text": "s1 is running"
                    }],
                    "structuredContent": {
                        "sessionId": "s1",
                        "state": "running"
                    }
                }
            })
        );
    }

    #[test]
    fn tools_call_error_serializes_as_mcp_tool_error_result() {
        let server = test_server(|_, _| Err(ToolCallError::new("session not found")));

        let response = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"session_status","arguments":{"sessionId":"missing"}}}"#,
            )
            .expect("request should produce a response");

        assert_eq!(
            response_value(&response),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "content": [{
                        "type": "text",
                        "text": "session not found"
                    }],
                    "isError": true
                }
            })
        );
    }

    #[test]
    fn method_not_found_serializes_json_rpc_error() {
        let server = test_server(|_, _| Ok(ToolCallResult::text("unused")));

        let response = server
            .handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"unknown/method"}"#)
            .expect("request should produce a response");

        assert_eq!(
            response_value(&response),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "error": {
                    "code": -32601,
                    "message": "Method not found: unknown/method"
                }
            })
        );
    }

    #[test]
    fn parse_error_serializes_json_rpc_error_without_id() {
        let server = test_server(|_, _| Ok(ToolCallResult::text("unused")));

        let response = server
            .handle_line(r#"{"jsonrpc":"2.0","id": "#)
            .expect("parse error should produce a response");
        let response = response_value(&response);

        assert_eq!(response["jsonrpc"], "2.0");
        assert!(response.get("id").is_none());
        assert_eq!(response["error"]["code"], -32700);
        assert!(response["error"]["message"]
            .as_str()
            .expect("error message should be a string")
            .starts_with("Parse error:"),);
    }
}
