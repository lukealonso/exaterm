use crate::supervision::SessionTileStatus;

pub fn status_chip_label(status: SessionTileStatus, recency_label: &str) -> String {
    if matches!(status, SessionTileStatus::Idle | SessionTileStatus::Stopped)
        && recency_label.starts_with("idle ")
    {
        let seconds = recency_label.trim_start_matches("idle ").trim();
        let label = match status {
            SessionTileStatus::Idle => "IDLE",
            SessionTileStatus::Stopped => "STOPPED",
            _ => unreachable!(),
        };
        return format!("{label} - {seconds}");
    }

    status.label().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_status_chip_includes_idle_recency() {
        assert_eq!(
            status_chip_label(SessionTileStatus::Idle, "idle 42s"),
            "IDLE - 42s"
        );
    }
}
