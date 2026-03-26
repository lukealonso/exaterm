use crate::supervision::SignalTone;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::Path;

const DEFAULT_SUMMARY_MODEL: &str = "gpt-5-mini";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MismatchLevel {
    Low,
    Watch,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TacticalSynthesis {
    pub headline: Option<String>,
    pub primary_fragment: Option<String>,
    #[serde(default)]
    pub supporting_fragments: Vec<String>,
    pub alignment_fragment: Option<String>,
    pub mismatch_level: MismatchLevel,
    pub intervention_warranted: bool,
    pub confidence: f32,
}

impl TacticalSynthesis {
    pub fn sanitize(mut self) -> Self {
        self.headline = sanitize_optional(self.headline);
        self.primary_fragment = sanitize_optional(self.primary_fragment);
        self.alignment_fragment = sanitize_optional(self.alignment_fragment);
        self.supporting_fragments = self
            .supporting_fragments
            .into_iter()
            .filter_map(|fragment| sanitize_optional(Some(fragment)))
            .take(2)
            .collect();
        self.confidence = self.confidence.clamp(0.0, 1.0);
        self
    }

    pub fn signal_tone(&self) -> SignalTone {
        match self.mismatch_level {
            MismatchLevel::Low => SignalTone::Calm,
            MismatchLevel::Watch => SignalTone::Watch,
            MismatchLevel::High => SignalTone::Alert,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TacticalEvidence {
    pub session_name: String,
    pub task_label: String,
    pub battle_status: String,
    pub recency_label: String,
    pub deterministic_headline: String,
    pub deterministic_detail: Option<String>,
    pub deterministic_evidence: Vec<String>,
    pub deterministic_alignment: String,
    pub active_command: Option<String>,
    pub dominant_process: Option<String>,
    pub recent_files: Vec<String>,
    pub work_output_excerpt: Option<String>,
    pub idle_seconds: Option<u64>,
    pub recent_terminal_lines: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct OpenAiSynthesisConfig {
    pub api_key: String,
    pub model: String,
}

impl OpenAiSynthesisConfig {
    pub fn from_env() -> Option<Self> {
        load_dotenv_file();

        let api_key = env::var("OPENAI_API_KEY").ok()?.trim().to_string();
        if api_key.is_empty() {
            return None;
        }

        let requested_model = env::var("EXATERM_SUMMARY_MODEL").unwrap_or_default();
        Some(Self {
            api_key,
            model: normalize_summary_model(&requested_model),
        })
    }
}

pub fn load_dotenv_file() {
    let Ok(raw) = fs::read_to_string(Path::new(".env")) else {
        return;
    };

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || env::var_os(key).is_some() {
            continue;
        }
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if !value.is_empty() {
            env::set_var(key, value);
        }
    }
}

pub fn normalize_summary_model(model: &str) -> String {
    match model.trim() {
        "" => DEFAULT_SUMMARY_MODEL.into(),
        "gpt-5.4-mini" => DEFAULT_SUMMARY_MODEL.into(),
        "gpt-5.4" => "gpt-5".into(),
        other => other.into(),
    }
}

pub fn summary_signature(evidence: &TacticalEvidence) -> String {
    json!({
        "session_name": evidence.session_name,
        "task_label": evidence.task_label,
        "battle_status": evidence.battle_status,
        "deterministic_headline": evidence.deterministic_headline,
        "deterministic_detail": evidence.deterministic_detail,
        "deterministic_evidence": evidence.deterministic_evidence,
        "deterministic_alignment": evidence.deterministic_alignment,
        "active_command": evidence.active_command,
        "dominant_process": evidence.dominant_process,
        "recent_files": evidence.recent_files,
        "work_output_excerpt": evidence.work_output_excerpt,
        "idle_bucket": evidence.idle_seconds.map(|seconds| seconds / 15),
        "recent_terminal_lines": evidence.recent_terminal_lines,
    })
    .to_string()
}

pub fn summarize_blocking(
    config: &OpenAiSynthesisConfig,
    evidence: &TacticalEvidence,
) -> Result<TacticalSynthesis, String> {
    let request_body = json!({
        "model": config.model,
        "input": [
            {
                "role": "system",
                "content": tactical_system_prompt(),
            },
            {
                "role": "user",
                "content": format!(
                    "Summarize this supervised session into one tactical battlefield card object. Ground every field only in this evidence:\n{}",
                    serde_json::to_string_pretty(evidence).map_err(|error| error.to_string())?
                ),
            }
        ],
        "text": {
            "format": {
                "type": "json_schema",
                "name": "exaterm_tactical_summary",
                "strict": true,
                "schema": synthesis_schema(),
            }
        }
    });

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?;

    let response = client
        .post("https://api.openai.com/v1/responses")
        .bearer_auth(&config.api_key)
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .map_err(|error| error.to_string())?;

    let status = response.status();
    let payload: Value = response.json().map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(payload.to_string());
    }

    let text = extract_response_text(&payload)
        .ok_or_else(|| format!("response did not include parseable text: {payload}"))?;
    serde_json::from_str::<TacticalSynthesis>(&text)
        .map(TacticalSynthesis::sanitize)
        .map_err(|error| format!("failed to parse model synthesis: {error}; payload={text}"))
}

fn tactical_system_prompt() -> &'static str {
    "You are Exaterm's battlefield card synthesizer.\nReturn only a compact grounded tactical object.\nUse only the provided evidence.\nDo not invent hidden agent state, thoughts, or unseen files/processes.\nPrefer omission over speculation.\nThe card must stay terse and tactical, not verbose.\nIf the deterministic evidence is already good, preserve its shape rather than rewriting it theatrically."
}

fn synthesis_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "headline": { "type": ["string", "null"] },
            "primary_fragment": { "type": ["string", "null"] },
            "supporting_fragments": {
                "type": "array",
                "items": { "type": "string" },
                "maxItems": 2
            },
            "alignment_fragment": { "type": ["string", "null"] },
            "mismatch_level": {
                "type": "string",
                "enum": ["low", "watch", "high"]
            },
            "intervention_warranted": { "type": "boolean" },
            "confidence": { "type": "number" }
        },
        "required": [
            "headline",
            "primary_fragment",
            "supporting_fragments",
            "alignment_fragment",
            "mismatch_level",
            "intervention_warranted",
            "confidence"
        ],
        "additionalProperties": false
    })
}

pub fn extract_response_text(payload: &Value) -> Option<String> {
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

fn sanitize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!text.is_empty()).then_some(text)
    })
}

#[cfg(test)]
mod tests {
    use super::{
        extract_response_text, normalize_summary_model, summary_signature, MismatchLevel,
        TacticalEvidence, TacticalSynthesis,
    };
    use serde_json::json;

    #[test]
    fn normalizes_legacy_summary_model_aliases() {
        assert_eq!(normalize_summary_model("gpt-5.4-mini"), "gpt-5-mini");
        assert_eq!(normalize_summary_model(""), "gpt-5-mini");
    }

    #[test]
    fn extracts_text_from_responses_payload() {
        let payload = json!({
            "output": [
                {
                    "content": [
                        {
                            "type": "output_text",
                            "text": "{\"headline\":\"cargo test parser\",\"primary_fragment\":null,\"supporting_fragments\":[],\"alignment_fragment\":null,\"mismatch_level\":\"low\",\"intervention_warranted\":false,\"confidence\":0.72}"
                        }
                    ]
                }
            ]
        });

        let text = extract_response_text(&payload).expect("text should be extracted");
        assert!(text.contains("\"headline\":\"cargo test parser\""));
    }

    #[test]
    fn summary_signature_ignores_small_idle_tick_changes() {
        let mut evidence = TacticalEvidence {
            session_name: "Parser".into(),
            task_label: "Fix".into(),
            battle_status: "Idle".into(),
            recency_label: "idle 46s".into(),
            deterministic_headline: "rerunning parser tests".into(),
            deterministic_detail: Some("Quiet after edits in src/parser.rs".into()),
            deterministic_evidence: vec!["3 parser failures remain".into()],
            deterministic_alignment: "Recently active, now quiet".into(),
            active_command: None,
            dominant_process: None,
            recent_files: vec!["src/parser.rs".into()],
            work_output_excerpt: Some("3 parser failures remain".into()),
            idle_seconds: Some(46),
            recent_terminal_lines: vec!["Now rerunning the parser tests.".into()],
        };

        let first = summary_signature(&evidence);
        evidence.idle_seconds = Some(52);
        assert_eq!(summary_signature(&evidence), first);
    }

    #[test]
    fn sanitize_trims_and_limits_model_output() {
        let summary = TacticalSynthesis {
            headline: Some("  cargo   test parser ".into()),
            primary_fragment: Some(" 3 failures remain ".into()),
            supporting_fragments: vec![
                " src/parser.rs ".into(),
                " tests/parser.rs ".into(),
                " extra ".into(),
            ],
            alignment_fragment: Some(" low risk ".into()),
            mismatch_level: MismatchLevel::Low,
            intervention_warranted: false,
            confidence: 4.2,
        }
        .sanitize();

        assert_eq!(summary.headline.as_deref(), Some("cargo test parser"));
        assert_eq!(summary.supporting_fragments.len(), 2);
        assert_eq!(summary.confidence, 1.0);
    }
}
