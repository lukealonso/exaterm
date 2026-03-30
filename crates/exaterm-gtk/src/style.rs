use exaterm_ui::supervision::BattleCardStatus;
use gtk::gdk;
use gtk::prelude::*;
use std::path::PathBuf;

pub(crate) fn configure_app_icons(app_id: &str) {
    if let Some(display) = gdk::Display::default() {
        let icon_theme = gtk::IconTheme::for_display(&display);
        icon_theme.add_search_path(bundled_icon_search_path());
    }
    gtk::Window::set_default_icon_name(app_id);
}

pub(crate) fn apply_battle_status_style(label: &gtk::Label, status: BattleCardStatus) {
    for css in [
        "battle-idle",
        "battle-stopped",
        "battle-active",
        "battle-thinking",
        "battle-working",
        "battle-blocked",
        "battle-failed",
        "battle-complete",
        "battle-detached",
    ] {
        label.remove_css_class(css);
    }

    label.add_css_class(match status {
        BattleCardStatus::Idle => "battle-idle",
        BattleCardStatus::Stopped => "battle-stopped",
        BattleCardStatus::Active => "battle-active",
        BattleCardStatus::Thinking => "battle-thinking",
        BattleCardStatus::Working => "battle-working",
        BattleCardStatus::Blocked => "battle-blocked",
        BattleCardStatus::Failed => "battle-failed",
        BattleCardStatus::Complete => "battle-complete",
        BattleCardStatus::Detached => "battle-detached",
    });
}

pub(crate) fn status_chip_label(status: BattleCardStatus, recency_label: &str) -> String {
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

pub(crate) fn apply_battle_card_surface_style(frame: &gtk::Frame, status: BattleCardStatus) {
    for css in [
        "card-idle",
        "card-stopped",
        "card-active",
        "card-thinking",
        "card-working",
        "card-blocked",
        "card-failed",
        "card-complete",
        "card-detached",
    ] {
        frame.remove_css_class(css);
    }

    frame.add_css_class(match status {
        BattleCardStatus::Idle => "card-idle",
        BattleCardStatus::Stopped => "card-stopped",
        BattleCardStatus::Active => "card-active",
        BattleCardStatus::Thinking => "card-thinking",
        BattleCardStatus::Working => "card-working",
        BattleCardStatus::Blocked => "card-blocked",
        BattleCardStatus::Failed => "card-failed",
        BattleCardStatus::Complete => "card-complete",
        BattleCardStatus::Detached => "card-detached",
    });
}

pub(crate) fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&exaterm_ui::css::generate_application_css());

    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display should exist"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn bundled_icon_search_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons")
}
