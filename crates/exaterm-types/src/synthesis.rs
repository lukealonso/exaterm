use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticalState {
    Idle,
    Stopped,
    Active,
    Thinking,
    Working,
    Blocked,
    Failed,
    Complete,
    Detached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressState {
    SteadyProgress,
    Verifying,
    Exploring,
    WaitingForNudge,
    Blocked,
    Flailing,
    ConvergedWaiting,
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MomentumState {
    Strong,
    Steady,
    Fragile,
    Stalled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorAction {
    None,
    Watch,
    Nudge,
    Intervene,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskPosture {
    Low,
    Watch,
    High,
    Extreme,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MismatchLevel {
    Low,
    Watch,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TacticalSynthesis {
    pub tactical_state: Option<TacticalState>,
    pub tactical_state_brief: Option<String>,
    pub progress_state: Option<ProgressState>,
    pub progress_state_brief: Option<String>,
    pub momentum_state: Option<MomentumState>,
    pub momentum_state_brief: Option<String>,
    pub operator_action: Option<OperatorAction>,
    pub operator_action_brief: Option<String>,
    pub terse_operator_summary: Option<String>,
    pub headline: Option<String>,
    pub primary_fragment: Option<String>,
    #[serde(default)]
    pub supporting_fragments: Vec<String>,
    pub alignment_fragment: Option<String>,
    pub risk_posture: Option<RiskPosture>,
    pub risk_brief: Option<String>,
    pub mismatch_level: MismatchLevel,
    pub mismatch_brief: Option<String>,
    pub intervention_warranted: bool,
}

impl TacticalSynthesis {
    pub fn sanitize(mut self) -> Self {
        self.headline = sanitize_optional(self.headline);
        self.primary_fragment = sanitize_optional(self.primary_fragment);
        self.alignment_fragment = sanitize_optional(self.alignment_fragment);
        self.tactical_state_brief = sanitize_optional(self.tactical_state_brief);
        self.progress_state_brief = sanitize_optional(self.progress_state_brief);
        self.momentum_state_brief = sanitize_optional(self.momentum_state_brief);
        self.operator_action_brief = sanitize_optional(self.operator_action_brief);
        self.terse_operator_summary = sanitize_optional(self.terse_operator_summary);
        self.risk_brief = sanitize_optional(self.risk_brief);
        self.mismatch_brief = sanitize_optional(self.mismatch_brief);
        self.supporting_fragments = self
            .supporting_fragments
            .into_iter()
            .filter_map(|fragment| sanitize_optional(Some(fragment)))
            .take(2)
            .collect();
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
    use super::{MismatchLevel, NameSuggestion, NudgeSuggestion, TacticalSynthesis};

    #[test]
    fn tactical_synthesis_sanitize_trims_fields() {
        let summary = TacticalSynthesis {
            tactical_state: None,
            tactical_state_brief: Some("  stopped   cleanly  ".into()),
            progress_state: None,
            progress_state_brief: None,
            momentum_state: None,
            momentum_state_brief: None,
            operator_action: None,
            operator_action_brief: None,
            terse_operator_summary: Some("  waiting   for   continue ".into()),
            headline: Some("  parser   pass ".into()),
            primary_fragment: None,
            supporting_fragments: vec!["  one  ".into(), " ".into(), "two".into()],
            alignment_fragment: None,
            risk_posture: None,
            risk_brief: None,
            mismatch_level: MismatchLevel::Low,
            mismatch_brief: None,
            intervention_warranted: false,
        }
        .sanitize();

        assert_eq!(summary.headline.as_deref(), Some("parser pass"));
        assert_eq!(summary.tactical_state_brief.as_deref(), Some("stopped cleanly"));
        assert_eq!(summary.terse_operator_summary.as_deref(), Some("waiting for continue"));
        assert_eq!(summary.supporting_fragments, vec!["one".to_string(), "two".to_string()]);
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
