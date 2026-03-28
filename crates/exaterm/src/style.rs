use crate::supervision::BattleCardStatus;
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
    provider.load_from_string(
        "
        window {
            background: #000000;
        }

        flowboxchild {
            padding: 0;
            background: transparent;
            box-shadow: none;
            outline: none;
        }

        flowboxchild:selected {
            background: transparent;
            box-shadow: none;
            outline: none;
        }

        flowboxchild:selected > * {
            box-shadow: none;
        }

        flowboxchild.selected-card > * {
            border-color: rgba(113, 197, 255, 0.98);
            box-shadow: 0 0 0 1px rgba(113, 197, 255, 0.92), 0 22px 44px rgba(13, 92, 151, 0.24);
        }

        .workspace-summary {
            color: rgba(199, 210, 222, 0.9);
            font-size: 13px;
            letter-spacing: 0.08em;
            text-transform: uppercase;
        }

        .workspace-hint {
            color: rgba(189, 204, 219, 0.74);
            font-size: 12px;
        }

        .empty-state {
            margin-top: 40px;
            margin-bottom: 56px;
        }

        .empty-title {
            color: #f8fafc;
            font-size: 28px;
            font-weight: 800;
        }

        .empty-body {
            color: rgba(198, 211, 225, 0.82);
            font-size: 15px;
            line-height: 1.45;
        }

        .battle-card {
            border-radius: 24px;
            border: 1px solid rgba(163, 175, 194, 0.16);
            background: rgba(10, 18, 28, 0.95);
            box-shadow: 0 24px 46px rgba(0, 0, 0, 0.28);
            min-width: 392px;
            min-height: 220px;
        }

        .battle-card.single-card {
            min-width: 0;
            min-height: 0;
        }

        .battle-card.scrollback-card {
            min-width: 0;
            min-height: 0;
        }

        .card-terminal-slot {
            border-radius: 20px;
            border: 1px solid rgba(120, 136, 158, 0.2);
            background: rgba(7, 13, 20, 0.96);
            min-height: 0;
            padding: 10px;
        }

        .card-terminal-slot.scrollback-terminal-hidden {
            min-height: 0;
            padding: 0;
            border-color: transparent;
            background: transparent;
        }

        .card-header-row {
            min-height: 34px;
        }

        .card-body-stack {
            margin-top: 2px;
        }

        .card-bottom-stack,
        .card-footer-stack {
            margin-top: 0;
        }

        .card-scrollback-band {
            border-radius: 14px;
            border: 1px solid rgba(173, 188, 204, 0.08);
            background: rgba(8, 14, 22, 0.34);
            padding: 8px 10px;
            min-height: 0;
        }

        .card-scrollback-line {
            color: rgba(202, 214, 227, 0.88);
            font-size: 11px;
            font-family: Monospace;
            line-height: 1.1;
        }

        .card-bars-row {
            margin-top: 0;
        }

        .card-title {
            font-weight: 800;
            font-size: 18px;
            color: #f8fafc;
        }

        .card-subtitle {
            color: rgba(196, 208, 222, 0.66);
            font-size: 12px;
            letter-spacing: 0.04em;
            text-transform: uppercase;
        }

        .card-status {
            font-weight: 800;
            font-size: 10px;
            letter-spacing: 0.08em;
            text-transform: uppercase;
            border-radius: 999px;
            padding: 4px 10px;
            border: 1px solid rgba(190, 202, 217, 0.2);
        }

        .card-recency {
            color: rgba(188, 201, 216, 0.88);
            font-size: 12px;
            font-weight: 700;
            letter-spacing: 0.03em;
        }

        .card-headline {
            color: rgba(222, 232, 242, 0.82);
            font-weight: 700;
            font-size: 14px;
            line-height: 1.18;
        }

        .card-evidence {
            color: rgba(198, 212, 227, 0.88);
            font-size: 12px;
            font-family: Monospace;
            background: rgba(11, 18, 28, 0.32);
            border-radius: 11px;
            border: 1px solid rgba(173, 188, 204, 0.12);
            padding: 7px 10px;
        }

        .card-alert {
            color: rgba(202, 214, 227, 0.78);
            font-size: 11px;
            font-weight: 600;
            line-height: 1.2;
            margin: 0;
        }

        .card-control-row {
            min-height: 28px;
            margin-top: -2px;
            margin-bottom: -2px;
        }

        .card-control-label {
            color: rgba(203, 214, 226, 0.72);
            font-size: 10px;
            font-weight: 700;
            letter-spacing: 0.08em;
            text-transform: uppercase;
        }

        .card-control-state {
            font-size: 10px;
            font-weight: 800;
            letter-spacing: 0.08em;
            text-transform: uppercase;
            border-radius: 999px;
            padding: 4px 10px;
            border: 1px solid rgba(190, 202, 217, 0.16);
        }

        .card-control-off {
            color: rgba(214, 222, 230, 0.84);
            background: rgba(84, 97, 112, 0.18);
            border-color: rgba(163, 175, 194, 0.16);
        }

        .card-control-armed {
            color: #fde68a;
            background: rgba(120, 87, 10, 0.22);
            border-color: rgba(250, 204, 21, 0.22);
        }

        .card-control-nudged {
            color: #86efac;
            background: rgba(17, 88, 51, 0.22);
            border-color: rgba(74, 222, 128, 0.2);
        }

        .card-control-cooldown {
            color: #93c5fd;
            background: rgba(33, 82, 145, 0.22);
            border-color: rgba(96, 165, 250, 0.2);
        }

        .bar-widget {
            border-radius: 12px;
            border: 1px solid rgba(173, 188, 204, 0.08);
            background: rgba(11, 18, 28, 0.18);
            padding: 9px 11px;
        }

        .bar-caption {
            color: rgba(186, 200, 214, 0.62);
            font-size: 10px;
            letter-spacing: 0.08em;
            text-transform: uppercase;
        }

        .segmented-bar {
            min-height: 8px;
        }

        .bar-segment {
            min-height: 8px;
            border-radius: 999px;
        }

        .bar-empty {
            background: rgba(163, 175, 194, 0.14);
        }

        .bar-attention-1 {
            background: linear-gradient(90deg, rgba(110, 231, 183, 0.88) 0%, rgba(52, 211, 153, 0.92) 100%);
        }

        .bar-attention-2 {
            background: linear-gradient(90deg, rgba(96, 165, 250, 0.88) 0%, rgba(59, 130, 246, 0.92) 100%);
        }

        .bar-attention-3 {
            background: linear-gradient(90deg, rgba(250, 204, 21, 0.88) 0%, rgba(251, 146, 60, 0.92) 100%);
        }

        .bar-attention-4 {
            background: linear-gradient(90deg, rgba(248, 113, 113, 0.9) 0%, rgba(239, 68, 68, 0.94) 100%);
        }

        .bar-attention-5 {
            background: linear-gradient(90deg, rgba(244, 63, 94, 0.92) 0%, rgba(190, 24, 93, 0.96) 100%);
        }

        .bar-reason {
            color: rgba(226, 234, 242, 0.9);
            font-size: 13px;
            font-weight: 650;
            line-height: 1.28;
        }

        .focus-title {
            color: #f8fafc;
            font-size: 20px;
            font-weight: 800;
        }

        .focus-subtitle {
            color: rgba(196, 208, 222, 0.78);
            font-size: 14px;
            margin-bottom: 6px;
        }

        .focus-frame {
            border-radius: 24px;
            border: 1px solid rgba(120, 136, 158, 0.2);
            background: rgba(7, 13, 20, 0.96);
            padding: 10px;
        }

        .focus-panel {
            margin-top: 4px;
        }

        .pill {
            border-radius: 999px;
            padding: 6px 14px;
        }

        .pill {
            background: rgba(119, 198, 255, 0.16);
            color: #dbeafe;
        }

        flowboxchild.focused-card > * {
            border-color: rgba(110, 231, 183, 0.92);
            box-shadow: 0 0 0 1px rgba(110, 231, 183, 0.78), 0 20px 38px rgba(7, 88, 57, 0.22);
        }

        .card-idle {
            background: linear-gradient(180deg, rgba(21, 24, 30, 0.98) 0%, rgba(12, 14, 19, 0.97) 100%);
            border-color: rgba(21, 24, 30, 0.96);
        }

        .card-stopped {
            background: linear-gradient(180deg, rgba(54, 43, 11, 0.98) 0%, rgba(23, 21, 9, 0.97) 100%);
            border-color: rgba(54, 43, 11, 0.96);
        }

        .card-active {
            background: linear-gradient(180deg, rgba(14, 33, 52, 0.98) 0%, rgba(9, 18, 31, 0.97) 100%);
            border-color: rgba(14, 33, 52, 0.96);
        }

        .card-thinking {
            background: linear-gradient(180deg, rgba(9, 44, 29, 0.98) 0%, rgba(9, 23, 16, 0.97) 100%);
            border-color: rgba(9, 44, 29, 0.96);
        }

        .card-working {
            background: linear-gradient(180deg, rgba(9, 44, 29, 0.98) 0%, rgba(9, 23, 16, 0.97) 100%);
            border-color: rgba(9, 44, 29, 0.96);
        }

        .card-blocked {
            background: linear-gradient(180deg, rgba(55, 18, 22, 0.98) 0%, rgba(27, 11, 14, 0.97) 100%);
            border-color: rgba(55, 18, 22, 0.96);
        }

        .card-failed {
            background: linear-gradient(180deg, rgba(55, 18, 22, 0.98) 0%, rgba(27, 11, 14, 0.97) 100%);
            border-color: rgba(55, 18, 22, 0.96);
        }

        .card-complete {
            background: linear-gradient(180deg, rgba(11, 40, 41, 0.98) 0%, rgba(7, 20, 22, 0.97) 100%);
            border-color: rgba(11, 40, 41, 0.96);
        }

        .card-detached {
            background: linear-gradient(180deg, rgba(36, 18, 51, 0.98) 0%, rgba(16, 9, 25, 0.97) 100%);
            border-color: rgba(36, 18, 51, 0.96);
        }

        .battle-idle {
            color: #cbd5e1;
            background: rgba(71, 85, 105, 0.18);
            border-color: rgba(148, 163, 184, 0.22);
        }

        .battle-stopped {
            color: #fde68a;
            background: rgba(120, 87, 10, 0.22);
            border-color: rgba(250, 204, 21, 0.28);
        }

        .battle-active {
            color: #93c5fd;
            background: rgba(33, 82, 145, 0.22);
            border-color: rgba(96, 165, 250, 0.26);
        }

        .battle-thinking {
            color: #86efac;
            background: rgba(17, 88, 51, 0.24);
            border-color: rgba(74, 222, 128, 0.24);
        }

        .battle-working {
            color: #86efac;
            background: rgba(17, 88, 51, 0.24);
            border-color: rgba(74, 222, 128, 0.24);
        }

        .battle-blocked {
            color: #fca5a5;
            background: rgba(114, 28, 35, 0.24);
            border-color: rgba(248, 113, 113, 0.24);
        }

        .battle-failed {
            color: #fca5a5;
            background: rgba(114, 28, 35, 0.24);
            border-color: rgba(248, 113, 113, 0.24);
        }

        .battle-complete {
            color: #99f6e4;
            background: rgba(16, 77, 77, 0.22);
            border-color: rgba(94, 234, 212, 0.24);
        }

        .battle-detached {
            color: #e9d5ff;
            background: rgba(74, 34, 112, 0.22);
            border-color: rgba(192, 132, 252, 0.24);
        }

        .focus-mode flowboxchild .battle-card {
            min-width: 176px;
            min-height: 182px;
            border-radius: 18px;
            box-shadow: 0 14px 28px rgba(0, 0, 0, 0.22);
        }

        .focus-mode flowboxchild .card-title {
            font-size: 15px;
        }

        .focus-mode flowboxchild .card-status,
        .focus-mode flowboxchild .card-recency {
            font-size: 10px;
        }

        .focus-mode flowboxchild .card-header-row {
            min-height: 28px;
        }

        .focus-mode flowboxchild .card-bottom-stack {
            margin-top: 0;
        }

        .focus-mode flowboxchild .card-alert {
            color: rgba(206, 217, 229, 0.84);
            font-size: 12px;
            font-weight: 600;
            line-height: 1.3;
            padding: 0;
            background: transparent;
            border-color: transparent;
            min-height: 112px;
            margin-top: 6px;
            margin-bottom: 0;
            margin-left: 0;
            margin-right: 0;
        }

        .focus-mode flowboxchild .card-headline,
        .focus-mode flowboxchild .card-scrollback-band,
        .focus-mode flowboxchild .bar-widget {
        }

        terminal {
            border-radius: 18px;
            padding: 12px;
        }
        ",
    );

    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display should exist"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn bundled_icon_search_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons")
}
