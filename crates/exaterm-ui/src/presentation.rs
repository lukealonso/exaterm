use crate::supervision::BattleCardStatus;
use exaterm_types::synthesis::{AttentionLevel, TacticalSynthesis};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttentionPresentation {
    pub fill: usize,
    pub label: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChromeVisibility {
    pub title_visible: bool,
    pub headline_visible: bool,
    pub status_visible: bool,
    pub header_visible: bool,
    pub bars_visible: bool,
    pub nudge_state_visible: bool,
    pub nudge_row_visible: bool,
}

pub fn status_chip_label(status: BattleCardStatus, recency_label: &str) -> String {
    if matches!(status, BattleCardStatus::Idle | BattleCardStatus::Stopped)
        && recency_label.starts_with("idle ")
    {
        let seconds = recency_label.trim_start_matches("idle ").trim();
        let label = match status {
            BattleCardStatus::Idle => "IDLE",
            BattleCardStatus::Stopped => "STOPPED",
            _ => unreachable!(),
        };
        return format!("{label} - {seconds}");
    }

    status.label().to_string()
}

pub fn attention_presentation(
    summary: Option<&TacticalSynthesis>,
) -> Option<(AttentionPresentation, Option<String>)> {
    summary.map(|summary| {
        let presentation = match summary.attention_level {
            AttentionLevel::Autopilot => AttentionPresentation {
                fill: 1,
                label: "AUTOPILOT",
            },
            AttentionLevel::Monitor => AttentionPresentation {
                fill: 2,
                label: "MONITOR",
            },
            AttentionLevel::Guide => AttentionPresentation {
                fill: 3,
                label: "GUIDE",
            },
            AttentionLevel::Intervene => AttentionPresentation {
                fill: 4,
                label: "INTERVENE",
            },
            AttentionLevel::Takeover => AttentionPresentation {
                fill: 5,
                label: "TAKEOVER",
            },
        };
        (presentation, summary.attention_brief.clone())
    })
}

pub fn combined_focus_summary_text(headline: &str, attention_brief: Option<&str>) -> String {
    let headline = headline.trim();
    let attention_brief = attention_brief.unwrap_or("").trim();
    match (headline.is_empty(), attention_brief.is_empty()) {
        (false, false) => {
            let separator =
                if headline.ends_with('.') || headline.ends_with('!') || headline.ends_with('?') {
                    " "
                } else {
                    ". "
                };
            format!("{headline}{separator}{attention_brief}")
        }
        (false, true) => headline.to_string(),
        (true, false) => attention_brief.to_string(),
        (true, true) => String::new(),
    }
}

pub fn chrome_visibility(
    summarized: bool,
    focus_mode: bool,
    has_operator_summary: bool,
) -> ChromeVisibility {
    let title_visible = summarized;
    let status_visible = summarized;
    ChromeVisibility {
        title_visible,
        headline_visible: summarized,
        status_visible,
        header_visible: title_visible || status_visible,
        bars_visible: summarized && !focus_mode,
        nudge_state_visible: summarized && !focus_mode,
        nudge_row_visible: has_operator_summary || (summarized && !focus_mode),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exaterm_types::synthesis::{AttentionLevel, TacticalState, TacticalSynthesis};

    #[test]
    fn idle_status_chip_includes_idle_recency() {
        assert_eq!(
            status_chip_label(BattleCardStatus::Idle, "idle 42s"),
            "IDLE - 42s"
        );
    }

    #[test]
    fn combined_focus_summary_joins_headline_and_attention() {
        assert_eq!(
            combined_focus_summary_text("Parser done", Some("Needs review")),
            "Parser done. Needs review"
        );
    }

    #[test]
    fn chrome_visibility_hides_summary_fields_when_unsummarized() {
        let visibility = chrome_visibility(false, false, false);
        assert!(!visibility.title_visible);
        assert!(!visibility.bars_visible);
    }

    #[test]
    fn attention_presentation_maps_takeover() {
        let summary = TacticalSynthesis {
            tactical_state: TacticalState::Blocked,
            tactical_state_brief: None,
            attention_level: AttentionLevel::Takeover,
            attention_brief: Some("Human takeover needed".into()),
            headline: Some("Blocked".into()),
        };
        let (presentation, reason) = attention_presentation(Some(&summary)).unwrap();
        assert_eq!(presentation.fill, 5);
        assert_eq!(presentation.label, "TAKEOVER");
        assert_eq!(reason.as_deref(), Some("Human takeover needed"));
    }
}
