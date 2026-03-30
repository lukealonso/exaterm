use crate::supervision::BattleCardStatus;
use exaterm_types::synthesis::{AttentionLevel, TacticalSynthesis};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttentionPresentation {
    pub fill: usize,
    pub label: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentedBarPresentation {
    pub fill: usize,
    pub css_class: &'static str,
    pub label: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NudgeStateTone {
    Off,
    Armed,
    Cooldown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NudgeStatePresentation {
    pub label: &'static str,
    pub css_class: &'static str,
    pub tone: NudgeStateTone,
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
        let presentation = attention_level_presentation(summary.attention_level);
        (presentation, summary.attention_brief.clone())
    })
}

pub fn attention_level_presentation(level: AttentionLevel) -> AttentionPresentation {
    match level {
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
    }
}

pub fn attention_bar_presentation(
    summary: Option<&TacticalSynthesis>,
) -> Option<(SegmentedBarPresentation, Option<String>)> {
    summary.map(|summary| {
        let presentation = match summary.attention_level {
            AttentionLevel::Autopilot => SegmentedBarPresentation {
                fill: 1,
                css_class: "bar-attention-1",
                label: "ATTENTION CONDITION",
            },
            AttentionLevel::Monitor => SegmentedBarPresentation {
                fill: 2,
                css_class: "bar-attention-2",
                label: "ATTENTION CONDITION",
            },
            AttentionLevel::Guide => SegmentedBarPresentation {
                fill: 3,
                css_class: "bar-attention-3",
                label: "ATTENTION CONDITION",
            },
            AttentionLevel::Intervene => SegmentedBarPresentation {
                fill: 4,
                css_class: "bar-attention-4",
                label: "ATTENTION CONDITION",
            },
            AttentionLevel::Takeover => SegmentedBarPresentation {
                fill: 5,
                css_class: "bar-attention-5",
                label: "ATTENTION CONDITION",
            },
        };
        (presentation, summary.attention_brief.clone())
    })
}

pub fn nudge_state_presentation(
    enabled: bool,
    cooldown_active: bool,
    hovered: bool,
) -> NudgeStatePresentation {
    if hovered {
        if enabled {
            return NudgeStatePresentation {
                label: "DISARM AUTONUDGE",
                css_class: "card-control-cooldown",
                tone: NudgeStateTone::Cooldown,
            };
        }
        return NudgeStatePresentation {
            label: "ARM AUTONUDGE",
            css_class: "card-control-off",
            tone: NudgeStateTone::Off,
        };
    }

    if cooldown_active {
        return NudgeStatePresentation {
            label: "AUTONUDGE COOLDOWN",
            css_class: "card-control-cooldown",
            tone: NudgeStateTone::Cooldown,
        };
    }

    if enabled {
        return NudgeStatePresentation {
            label: "AUTONUDGE ARMED",
            css_class: "card-control-armed",
            tone: NudgeStateTone::Armed,
        };
    }

    NudgeStatePresentation {
        label: "AUTONUDGE OFF",
        css_class: "card-control-off",
        tone: NudgeStateTone::Off,
    }
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

    #[test]
    fn attention_bar_presentation_maps_monitor() {
        let summary = TacticalSynthesis {
            tactical_state: TacticalState::Working,
            tactical_state_brief: None,
            attention_level: AttentionLevel::Monitor,
            attention_brief: Some("Watch for drift".into()),
            headline: Some("Steady".into()),
        };
        let (presentation, reason) = attention_bar_presentation(Some(&summary)).unwrap();
        assert_eq!(presentation.fill, 2);
        assert_eq!(presentation.css_class, "bar-attention-2");
        assert_eq!(presentation.label, "ATTENTION CONDITION");
        assert_eq!(reason.as_deref(), Some("Watch for drift"));
    }

    #[test]
    fn nudge_state_presentation_prefers_hover_actions() {
        let presentation = nudge_state_presentation(true, false, true);
        assert_eq!(presentation.label, "DISARM AUTONUDGE");
        assert_eq!(presentation.css_class, "card-control-cooldown");
        assert_eq!(presentation.tone, NudgeStateTone::Cooldown);
    }

    #[test]
    fn nudge_state_presentation_marks_armed_state() {
        let presentation = nudge_state_presentation(true, false, false);
        assert_eq!(presentation.label, "AUTONUDGE ARMED");
        assert_eq!(presentation.css_class, "card-control-armed");
        assert_eq!(presentation.tone, NudgeStateTone::Armed);
    }
}
