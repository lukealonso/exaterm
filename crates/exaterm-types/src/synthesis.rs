use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticalState {
    Idle,
    Stopped,
    Thinking,
    Working,
    Blocked,
    Failed,
    Complete,
    Detached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionLevel {
    Autopilot,
    Monitor,
    Guide,
    Intervene,
    Takeover,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TacticalSynthesis {
    pub tactical_state: TacticalState,
    pub tactical_state_brief: Option<String>,
    pub attention_level: AttentionLevel,
    pub attention_brief: Option<String>,
    pub headline: Option<String>,
    #[serde(default)]
    pub tool_not_likely_coding_agent: bool,
}

impl TacticalSynthesis {
    pub fn sanitize(mut self) -> Self {
        self.headline = sanitize_optional(self.headline);
        self.tactical_state_brief = sanitize_optional(self.tactical_state_brief);
        self.attention_brief = sanitize_optional(self.attention_brief);
        if self.tool_not_likely_coding_agent {
            self.tactical_state = TacticalState::Idle;
            self.attention_level = AttentionLevel::Autopilot;
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameSuggestion {
    pub name: String,
}

impl NameSuggestion {
    pub fn sanitize(mut self) -> Self {
        self.name = sanitize_name(&self.name);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NudgeSuggestion {
    pub text: String,
}

impl NudgeSuggestion {
    pub fn sanitize(mut self) -> Self {
        self.text = sanitize_optional(Some(self.text))
            .unwrap_or_default()
            .chars()
            .take(120)
            .collect();
        self
    }
}

fn sanitize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!text.is_empty()).then_some(text)
    })
}

fn sanitize_name(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let truncated = trimmed.chars().take(40).collect::<String>();
    let bounded = if truncated.chars().count() < trimmed.chars().count() {
        truncated
            .rfind(char::is_whitespace)
            .map(|index| truncated[..index].trim_end().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or(truncated)
    } else {
        truncated
    };

    bounded
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '.' | ',' | ':' | ';'))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        AttentionLevel, NameSuggestion, NudgeSuggestion, TacticalState, TacticalSynthesis,
    };

    #[test]
    fn tactical_synthesis_sanitize_trims_fields() {
        let summary = TacticalSynthesis {
            tactical_state: TacticalState::Stopped,
            tactical_state_brief: Some("  stopped   cleanly  ".into()),
            attention_level: AttentionLevel::Guide,
            attention_brief: Some("  likely needs   a small nudge  ".into()),
            headline: Some("  parser   pass ".into()),
            tool_not_likely_coding_agent: false,
        }
        .sanitize();

        assert_eq!(summary.headline.as_deref(), Some("parser pass"));
        assert_eq!(
            summary.tactical_state_brief.as_deref(),
            Some("stopped cleanly")
        );
        assert_eq!(
            summary.attention_brief.as_deref(),
            Some("likely needs a small nudge")
        );
    }

    #[test]
    fn tactical_synthesis_sanitize_coerces_non_coding_sessions_to_idle_autopilot() {
        let summary = TacticalSynthesis {
            tactical_state: TacticalState::Working,
            tactical_state_brief: Some("shell is active".into()),
            attention_level: AttentionLevel::Guide,
            attention_brief: Some("watch closely".into()),
            headline: Some("tail -f is updating".into()),
            tool_not_likely_coding_agent: true,
        }
        .sanitize();

        assert_eq!(summary.tactical_state, TacticalState::Idle);
        assert_eq!(summary.attention_level, AttentionLevel::Autopilot);
    }

    #[test]
    fn tactical_synthesis_deserializes_without_new_boolean() {
        let summary: TacticalSynthesis = serde_json::from_str(
            r#"{
                "tactical_state":"working",
                "tactical_state_brief":"running tests",
                "attention_level":"monitor",
                "attention_brief":"worth watching",
                "headline":"cargo test parser"
            }"#,
        )
        .expect("deserialize legacy tactical synthesis");

        assert!(!summary.tool_not_likely_coding_agent);
    }

    #[test]
    fn name_suggestion_sanitize_bounds_length() {
        let suggestion = NameSuggestion {
            name: "  a very long parser repair name that should definitely be shortened  ".into(),
        }
        .sanitize();
        assert!(suggestion.name.len() <= 40);
        assert!(!suggestion.name.is_empty());
    }

    #[test]
    fn nudge_suggestion_sanitize_trims() {
        let suggestion = NudgeSuggestion {
            text: "   Keep going on the next concrete failure.   ".into(),
        }
        .sanitize();
        assert_eq!(suggestion.text, "Keep going on the next concrete failure.");
    }
}
