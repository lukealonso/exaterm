use crate::config::DEFAULT_TERMINAL_ASSIST_MODEL;
pub use exaterm_types::synthesis::TerminalAssistSuggestion;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::time::Duration;

const OPENAI_TERMINAL_ASSIST_TIMEOUT: Duration = Duration::from_secs(20);
const TERMINAL_ASSIST_MAX_TOKENS: u16 = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SynthesisProvider {
    OpenAi,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderPreferences {
    pub skipped_providers: BTreeSet<SynthesisProvider>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OpenAiBackend {
    api_key: String,
    base_url: String,
}

#[derive(Clone, Debug)]
pub struct SynthesisBackendRegistry {
    openai: Option<OpenAiBackend>,
    terminal_assist_model: String,
}

#[derive(Debug)]
pub struct ProviderCallResult<T> {
    pub provider: Option<SynthesisProvider>,
    pub value: Result<T, String>,
    pub demoted_provider: Option<SynthesisProvider>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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

impl SynthesisBackendRegistry {
    pub fn from_env() -> Option<Self> {
        let openai = env::var("OPENAI_API_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|api_key| OpenAiBackend {
                api_key,
                base_url: openai_chat_completions_url(),
            });
        let registry = Self {
            openai,
            terminal_assist_model: normalize_terminal_assist_model(
                &env::var("EXATERM_TERMINAL_ASSIST_MODEL").unwrap_or_default(),
            ),
        };
        registry.is_available().then_some(registry)
    }

    pub fn suggest_terminal_assist_blocking(
        &self,
        preferences: &ProviderPreferences,
        evidence: &TerminalAssistEvidence,
    ) -> ProviderCallResult<TerminalAssistSuggestion> {
        self.run_with_fallback(preferences, |provider| match provider {
            SynthesisProvider::OpenAi => suggest_terminal_assist_openai_blocking(
                self.openai.as_ref().expect("openai backend must exist"),
                &self.terminal_assist_model,
                evidence,
            ),
        })
    }

    fn is_available(&self) -> bool {
        self.openai.is_some()
    }

    fn preferred_provider_order(
        &self,
        preferences: &ProviderPreferences,
    ) -> Vec<SynthesisProvider> {
        let mut providers = Vec::new();
        if self.openai.is_some()
            && !preferences
                .skipped_providers
                .contains(&SynthesisProvider::OpenAi)
        {
            providers.push(SynthesisProvider::OpenAi);
        }
        providers
    }

    fn run_with_fallback<T, F>(
        &self,
        preferences: &ProviderPreferences,
        mut call: F,
    ) -> ProviderCallResult<T>
    where
        F: FnMut(SynthesisProvider) -> Result<T, String>,
    {
        let providers = self.preferred_provider_order(preferences);
        if providers.is_empty() {
            return ProviderCallResult {
                provider: None,
                value: Err(
                    "no synthesis provider available after applying provider preferences".into(),
                ),
                demoted_provider: None,
            };
        }

        let mut first_failed_provider = None::<SynthesisProvider>;
        let mut last_error = None::<String>;
        for provider in providers {
            match call(provider) {
                Ok(value) => {
                    return ProviderCallResult {
                        provider: Some(provider),
                        value: Ok(value),
                        demoted_provider: first_failed_provider,
                    };
                }
                Err(error) => {
                    if first_failed_provider.is_none() {
                        first_failed_provider = Some(provider);
                        last_error = Some(error);
                    } else {
                        let previous_error = last_error.take().unwrap_or_default();
                        last_error = Some(format!(
                            "{} failed: {previous_error}; {} failed: {error}",
                            provider_label(
                                first_failed_provider.expect("first failure should exist")
                            ),
                            provider_label(provider),
                        ));
                    }
                }
            }
        }

        let failed_provider = first_failed_provider.expect("provider order was non-empty");
        ProviderCallResult {
            provider: Some(failed_provider),
            value: Err(last_error.expect("failed provider should have an error")),
            demoted_provider: None,
        }
    }
}

fn provider_label(provider: SynthesisProvider) -> &'static str {
    match provider {
        SynthesisProvider::OpenAi => "openai",
    }
}

fn openai_chat_completions_url() -> String {
    let base = env::var("EXATERM_OPENAI_BASE_URL")
        .or_else(|_| env::var("OPENAI_BASE_URL"))
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

pub fn normalize_terminal_assist_model(model: &str) -> String {
    let model = model.trim();
    if model.is_empty() {
        DEFAULT_TERMINAL_ASSIST_MODEL.into()
    } else {
        model.into()
    }
}

fn suggest_terminal_assist_openai_blocking(
    config: &OpenAiBackend,
    model: &str,
    evidence: &TerminalAssistEvidence,
) -> Result<TerminalAssistSuggestion, String> {
    let request_body = terminal_assist_openai_request_body(model, evidence);

    let client = reqwest::blocking::Client::builder()
        .http1_only()
        .connect_timeout(Duration::from_secs(4))
        .timeout(OPENAI_TERMINAL_ASSIST_TIMEOUT)
        .build()
        .map_err(format_error_chain)?;

    let response = client
        .post(&config.base_url)
        .bearer_auth(&config.api_key)
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .map_err(format_error_chain)?;

    let status = response.status();
    let payload: Value = response.json().map_err(format_error_chain)?;
    if !status.is_success() {
        return Err(payload.to_string());
    }

    let text = extract_response_text(&payload)
        .ok_or_else(|| format!("response did not include parseable text: {payload}"))?;
    parse_json_output::<TerminalAssistSuggestion>(&text, "model terminal assist response")
        .map(TerminalAssistSuggestion::sanitize)
}

fn terminal_assist_openai_request_body(model: &str, evidence: &TerminalAssistEvidence) -> Value {
    json!({
        "model": model,
        "max_completion_tokens": TERMINAL_ASSIST_MAX_TOKENS,
        "messages": [
            {
                "role": "system",
                "content": terminal_assist_system_prompt(),
            },
            {
                "role": "user",
                "content": terminal_assist_user_prompt(evidence),
            }
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "exaterm_terminal_assist",
                "strict": true,
                "schema": terminal_assist_schema(),
            }
        }
    })
}

fn parse_json_output<T>(text: &str, label: &str) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let candidates = json_output_candidates(text);
    let mut last_error = None;
    for candidate in &candidates {
        match serde_json::from_str::<T>(candidate) {
            Ok(value) => return Ok(value),
            Err(error) => last_error = Some(error),
        }
    }
    let error = last_error
        .map(|error| error.to_string())
        .unwrap_or_else(|| "no JSON candidate found".into());
    Err(format!(
        "failed to parse {label} as JSON: {error}; output: {text}"
    ))
}

fn json_output_candidates(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    let mut candidates = Vec::new();
    let fenced_blocks = extract_fenced_json_blocks(trimmed);
    for fenced in fenced_blocks.into_iter().rev() {
        push_unique_candidate(&mut candidates, fenced);
    }
    if trimmed.starts_with('{') {
        push_unique_candidate(&mut candidates, trimmed.to_string());
    }
    let json_objects = extract_json_objects(trimmed);
    for object in json_objects.into_iter().rev() {
        push_unique_candidate(&mut candidates, object);
    }
    if candidates.is_empty() {
        candidates.push(trimmed.to_string());
    }
    candidates
}

fn push_unique_candidate(candidates: &mut Vec<String>, candidate: String) {
    let candidate = candidate.trim().to_string();
    if !candidate.is_empty() && !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

fn extract_fenced_json_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut remaining = text;
    while let Some(fence_start) = remaining.find("```") {
        let after_fence = &remaining[fence_start + 3..];
        let body_start = after_fence
            .find(|ch| ch == '\n' || ch == '\r')
            .map(|index| index + 1)
            .unwrap_or(0);
        let body = &after_fence[body_start..];
        let Some(fence_end) = body.find("```") else {
            break;
        };
        blocks.push(body[..fence_end].trim().to_string());
        remaining = &body[fence_end + 3..];
    }
    blocks
}

fn extract_json_objects(text: &str) -> Vec<String> {
    let mut objects = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' if depth > 0 => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(index);
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = start.take() {
                        objects.push(text[start..index + ch.len_utf8()].trim().to_string());
                    }
                }
            }
            _ => {}
        }
    }

    objects
}

fn terminal_assist_system_prompt() -> &'static str {
    r#"
You write one terminal insertion for Exaterm Ctrl-K assist.

The operator is editing a real terminal input line. Your output will be inserted into that terminal, not executed automatically.

Use only the provided evidence.
Return one compact JSON object only.
The final structured object in your response must be:
{"insert_text":"<one command or shell snippet>"}
Do not use markdown.
Do not wrap the command in code fences.
Do not explain your reasoning.
Do not ask questions.
Do not claim you ran anything.
Do not include surrounding prose, bullets, labels, or comments.

The insert_text field must contain exactly one shell command or shell snippet suitable for insertion at the cursor.
Prefer a single command line.
Use a multi-command shell snippet only when the operator explicitly asks for it and it still belongs on one terminal input line.
Default to non-destructive commands: inspect, search, print, validate, test, or stage-free local edits only when the operator explicitly asks for an edit.
Do not suggest destructive commands such as rm -rf, git reset --hard, force push, broad chmod/chown, killall, destructive sudo operations, package removal, or data deletion unless the operator explicitly asks for that exact action and the evidence makes the target unambiguous.
If the operator request is unsafe, unclear, unrelated to terminal work, or cannot be answered from the evidence, return an empty insert_text string.

Respect the working_directory and recent_files evidence when choosing paths.
Do not invent files or directories that are not present in the evidence unless the operator explicitly asks to create them.
Prefer relative paths already visible in recent_files over absolute paths.
Return insertion text only; never include markdown or explanatory text inside insert_text.
"#
    .trim()
}

fn terminal_assist_user_prompt(evidence: &TerminalAssistEvidence) -> String {
    format!(
        "Return one shell command/snippet for this Ctrl-K terminal assist request. End with a compact JSON object containing only insert_text; insert_text must be insertion-only text for the originating terminal input line:\n{}",
        serde_json::to_string_pretty(evidence)
            .map_err(|error| error.to_string())
            .unwrap_or_default()
    )
}

fn terminal_assist_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "insert_text": { "type": "string" }
        },
        "required": ["insert_text"],
        "additionalProperties": false
    })
}

pub fn extract_response_text(payload: &Value) -> Option<String> {
    if let Some(text) = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
    {
        return Some(text.to_string());
    }

    if let Some(text) = payload.get("output_text").and_then(Value::as_str) {
        return Some(text.to_string());
    }

    payload
        .get("output")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find_map(|item| {
                item.get("content")
                    .and_then(Value::as_array)
                    .and_then(|content| {
                        content.iter().find_map(|part| {
                            part.get("text")
                                .and_then(Value::as_str)
                                .map(ToOwned::to_owned)
                                .or_else(|| {
                                    part.get("output_text")
                                        .and_then(Value::as_str)
                                        .map(ToOwned::to_owned)
                                })
                        })
                    })
            })
        })
}

fn format_error_chain(error: impl Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(error) = source {
        message.push_str(": ");
        message.push_str(&error.to_string());
        source = error.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::{
        extract_response_text, normalize_terminal_assist_model, openai_chat_completions_url,
        terminal_assist_openai_request_body, terminal_assist_schema, terminal_assist_system_prompt,
        terminal_assist_user_prompt, OpenAiBackend, ProviderPreferences, SynthesisBackendRegistry,
        SynthesisProvider, TerminalAssistEvidence, TerminalAssistSuggestion,
        DEFAULT_TERMINAL_ASSIST_MODEL, TERMINAL_ASSIST_MAX_TOKENS,
    };
    use serde_json::json;
    use std::collections::BTreeSet;
    use std::sync::Mutex;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn test_registry() -> SynthesisBackendRegistry {
        SynthesisBackendRegistry {
            openai: None,
            terminal_assist_model: DEFAULT_TERMINAL_ASSIST_MODEL.into(),
        }
    }

    #[test]
    fn terminal_assist_model_defaults_and_preserves_exact_name() {
        assert_eq!(
            normalize_terminal_assist_model("gpt-5.5-nano"),
            "gpt-5.5-nano"
        );
        assert_eq!(DEFAULT_TERMINAL_ASSIST_MODEL, "gpt-5.5-nano");
        assert_eq!(
            normalize_terminal_assist_model(""),
            DEFAULT_TERMINAL_ASSIST_MODEL
        );
        assert_eq!(normalize_terminal_assist_model("gpt-5.4"), "gpt-5.4");
    }

    #[test]
    fn terminal_assist_schema_requires_insert_text_only() {
        let schema = terminal_assist_schema();

        assert_eq!(schema["properties"]["insert_text"]["type"], "string");
        assert_eq!(schema["required"], json!(["insert_text"]));
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn terminal_assist_prompt_requires_json_insertion_only_and_non_destructive_default() {
        let prompt = terminal_assist_system_prompt();

        assert!(prompt.contains("Return one compact JSON object only."));
        assert!(prompt.contains(r#"{"insert_text":"<one command or shell snippet>"}"#));
        assert!(prompt.contains("Do not use markdown."));
        assert!(prompt.contains("exactly one shell command or shell snippet"));
        assert!(prompt.contains("suitable for insertion at the cursor"));
        assert!(prompt.contains("Default to non-destructive commands"));
        assert!(prompt.contains("Do not suggest destructive commands"));
        assert!(prompt.contains("rm -rf"));
        assert!(prompt.contains("git reset --hard"));
    }

    #[test]
    fn terminal_assist_user_prompt_includes_relevant_evidence_fields() {
        let evidence = TerminalAssistEvidence {
            session_name: "Parser".into(),
            operator_prompt: "rerun the parser test".into(),
            current_input: "cargo test ".into(),
            working_directory: Some("/home/luke/projects/exaterm".into()),
            shell_child_command: Some("codex".into()),
            active_command: Some("cargo test parser".into()),
            dominant_process: Some("cargo".into()),
            process_tree_excerpt: Some("bash | codex | cargo".into()),
            recent_files: vec!["crates/exaterm-core/src/synthesis.rs".into()],
            terminal_status_line: Some("parser tests failed".into()),
            recent_terminal_history: vec!["error: parser snapshot changed".into()],
        };

        let prompt = terminal_assist_user_prompt(&evidence);

        assert!(prompt.contains("\"operator_prompt\": \"rerun the parser test\""));
        assert!(prompt.contains("\"current_input\": \"cargo test \""));
        assert!(prompt.contains("\"working_directory\": \"/home/luke/projects/exaterm\""));
        assert!(prompt.contains("\"shell_child_command\": \"codex\""));
        assert!(prompt.contains("\"active_command\": \"cargo test parser\""));
        assert!(prompt.contains("\"dominant_process\": \"cargo\""));
        assert!(prompt.contains("\"process_tree_excerpt\": \"bash | codex | cargo\""));
        assert!(prompt.contains("\"recent_files\""));
        assert!(prompt.contains("crates/exaterm-core/src/synthesis.rs"));
        assert!(prompt.contains("\"terminal_status_line\": \"parser tests failed\""));
        assert!(prompt.contains("\"recent_terminal_history\""));
        assert!(prompt.contains("originating terminal input line"));
        assert!(prompt.contains("insert_text must be insertion-only text"));
    }

    #[test]
    fn terminal_assist_openai_request_uses_fast_json_only_model_call() {
        let evidence = TerminalAssistEvidence {
            session_name: "Shell".into(),
            operator_prompt: "disk usage".into(),
            current_input: String::new(),
            working_directory: Some("/tmp/project".into()),
            shell_child_command: None,
            active_command: None,
            dominant_process: None,
            process_tree_excerpt: None,
            recent_files: Vec::new(),
            terminal_status_line: None,
            recent_terminal_history: Vec::new(),
        };

        let body = terminal_assist_openai_request_body(DEFAULT_TERMINAL_ASSIST_MODEL, &evidence);

        assert_eq!(body["model"], DEFAULT_TERMINAL_ASSIST_MODEL);
        assert_eq!(body["max_completion_tokens"], TERMINAL_ASSIST_MAX_TOKENS);
        assert!(body.get("reasoning_effort").is_none());
        assert_eq!(
            body["response_format"]["json_schema"]["schema"]["required"],
            json!(["insert_text"])
        );
    }

    #[test]
    fn openai_chat_completions_url_defaults_to_openai() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::env::remove_var("EXATERM_OPENAI_BASE_URL");
        std::env::remove_var("OPENAI_BASE_URL");
        assert_eq!(
            openai_chat_completions_url(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn openai_chat_completions_url_uses_configured_base() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::env::set_var("EXATERM_OPENAI_BASE_URL", "https://example.test/v1/");
        assert_eq!(
            openai_chat_completions_url(),
            "https://example.test/v1/chat/completions"
        );
        std::env::remove_var("EXATERM_OPENAI_BASE_URL");
    }

    #[test]
    fn extracts_text_from_chat_completions_payload() {
        let payload = json!({
            "choices": [
                {
                    "message": {
                        "content": "{\"insert_text\":\"du -sh .\"}"
                    }
                }
            ]
        });

        let text = extract_response_text(&payload).expect("text should be extracted");
        assert!(text.contains("\"insert_text\":\"du -sh .\""));
    }

    #[test]
    fn extracts_text_from_responses_payload() {
        let payload = json!({
            "output": [
                {
                    "content": [
                        {
                            "type": "output_text",
                            "text": "{\"insert_text\":\"cargo test\"}"
                        }
                    ]
                }
            ]
        });

        let text = extract_response_text(&payload).expect("text should be extracted");
        assert!(text.contains("\"insert_text\":\"cargo test\""));
    }

    #[test]
    fn parse_json_output_accepts_fenced_json() {
        let parsed = super::parse_json_output::<TerminalAssistSuggestion>(
            "```json\n{\"insert_text\":\"du -sh .\"}\n```",
            "fenced",
        )
        .expect("fenced json should parse");
        assert_eq!(parsed.insert_text, "du -sh .");
    }

    #[test]
    fn parse_json_output_accepts_fenced_json_with_leading_text() {
        let parsed = super::parse_json_output::<TerminalAssistSuggestion>(
            "Here is the result:\n```json\n{\"insert_text\":\"cargo test\"}\n```",
            "fenced-leading",
        )
        .expect("fenced json with leading text should parse");
        assert_eq!(parsed.insert_text, "cargo test");
    }

    #[test]
    fn parse_json_output_prefers_final_structured_object() {
        let parsed = super::parse_json_output::<TerminalAssistSuggestion>(
            "Need to inspect disk usage.\n{\"insert_text\":\"echo wrong\"}\nFinal:\n{\"insert_text\":\"du -sh .\"}",
            "final-json",
        )
        .expect("final json should parse");
        assert_eq!(parsed.insert_text, "du -sh .");
    }

    #[test]
    fn backend_registry_uses_openai_when_configured() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::env::set_var("OPENAI_API_KEY", "test-key");

        let registry = SynthesisBackendRegistry::from_env().expect("registry should exist");
        assert_eq!(
            registry.preferred_provider_order(&ProviderPreferences::default()),
            vec![SynthesisProvider::OpenAi]
        );

        std::env::remove_var("OPENAI_API_KEY");
    }

    #[test]
    fn backend_registry_requires_openai_for_terminal_assist() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::env::remove_var("OPENAI_API_KEY");

        assert!(SynthesisBackendRegistry::from_env().is_none());
    }

    #[test]
    fn backend_registry_skips_openai_when_previously_demoted() {
        let registry = SynthesisBackendRegistry {
            openai: Some(OpenAiBackend {
                api_key: "test-key".into(),
                base_url: "https://example.invalid/v1/chat/completions".into(),
            }),
            terminal_assist_model: DEFAULT_TERMINAL_ASSIST_MODEL.into(),
        };

        let skipped = BTreeSet::from([SynthesisProvider::OpenAi]);
        assert_eq!(
            registry.preferred_provider_order(&ProviderPreferences {
                skipped_providers: skipped,
            }),
            Vec::<SynthesisProvider>::new()
        );
    }

    #[test]
    fn backend_registry_is_none_when_no_provider_exists() {
        let _guard = ENV_MUTEX
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::env::remove_var("OPENAI_API_KEY");
        assert!(SynthesisBackendRegistry::from_env().is_none());
    }

    #[test]
    fn run_with_fallback_does_not_use_cli_without_openai() {
        let registry = test_registry();
        let mut calls = Vec::new();
        let result = registry.run_with_fallback(&ProviderPreferences::default(), |provider| {
            calls.push(provider);
            Ok("ok".to_string())
        });

        assert!(calls.is_empty());
        assert_eq!(result.provider, None);
        assert_eq!(result.demoted_provider, None);
        assert!(result.value.is_err());
    }

    #[test]
    fn run_with_fallback_returns_openai_error_without_cli_fallback() {
        let registry = SynthesisBackendRegistry {
            openai: Some(OpenAiBackend {
                api_key: "test-key".into(),
                base_url: "https://example.invalid/v1/chat/completions".into(),
            }),
            terminal_assist_model: DEFAULT_TERMINAL_ASSIST_MODEL.into(),
        };
        let mut calls = Vec::new();
        let result: super::ProviderCallResult<String> =
            registry.run_with_fallback(&ProviderPreferences::default(), |provider| {
                calls.push(provider);
                match provider {
                    SynthesisProvider::OpenAi => Err("openai failed".into()),
                }
            });

        assert_eq!(calls, vec![SynthesisProvider::OpenAi]);
        assert_eq!(result.provider, Some(SynthesisProvider::OpenAi));
        assert_eq!(result.demoted_provider, None);
        assert!(result.value.is_err());
    }

    #[test]
    fn run_with_fallback_skips_openai_when_preference_is_set() {
        let registry = SynthesisBackendRegistry {
            openai: Some(OpenAiBackend {
                api_key: "test-key".into(),
                base_url: "https://example.invalid/v1/chat/completions".into(),
            }),
            terminal_assist_model: DEFAULT_TERMINAL_ASSIST_MODEL.into(),
        };
        let mut calls = Vec::new();
        let skipped = BTreeSet::from([SynthesisProvider::OpenAi]);
        let result = registry.run_with_fallback(
            &ProviderPreferences {
                skipped_providers: skipped,
            },
            |provider| {
                calls.push(provider);
                Ok("ok".to_string())
            },
        );

        assert!(calls.is_empty());
        assert_eq!(result.provider, None);
        assert_eq!(result.demoted_provider, None);
        assert!(result.value.is_err());
    }
}
