/// Generates the GTK stylesheet used by the Linux client.
pub fn generate_application_css() -> String {
    r#"
window {
    background: #000000;
}

.battlefield-root {
    background: #000000;
}

.battlefield-toolbar {
    margin-bottom: 4px;
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

.terminal-tile,
.supervised-group-card {
    border-radius: 24px;
    border: 1px solid rgba(163, 175, 194, 0.16);
    box-shadow: 0 24px 46px rgba(0, 0, 0, 0.28);
}

.terminal-tile {
    background: linear-gradient(180deg, rgba(21, 24, 30, 0.98) 0%, rgba(12, 14, 19, 0.97) 100%);
    border-color: rgba(21, 24, 30, 0.96);
    min-width: 280px;
    min-height: 160px;
}

.supervised-group-card {
    background: linear-gradient(180deg, rgba(16, 36, 38, 0.98) 0%, rgba(8, 20, 24, 0.98) 100%);
    border-color: rgba(45, 212, 191, 0.22);
    min-width: 392px;
    min-height: 220px;
}

.supervised-group-card.group-assessment-watching {
    background: linear-gradient(180deg, rgba(18, 43, 47, 0.98) 0%, rgba(8, 22, 28, 0.98) 100%);
    border-color: rgba(45, 212, 191, 0.24);
}

.supervised-group-card.group-assessment-active {
    background: linear-gradient(180deg, rgba(16, 47, 39, 0.98) 0%, rgba(7, 24, 22, 0.98) 100%);
    border-color: rgba(74, 222, 128, 0.28);
}

.supervised-group-card.group-assessment-stalling {
    background: linear-gradient(180deg, rgba(58, 42, 14, 0.98) 0%, rgba(29, 20, 8, 0.98) 100%);
    border-color: rgba(245, 158, 11, 0.34);
}

.supervised-group-card.group-assessment-blocked {
    background: linear-gradient(180deg, rgba(59, 20, 24, 0.98) 0%, rgba(30, 9, 13, 0.98) 100%);
    border-color: rgba(248, 113, 113, 0.34);
}

.supervised-group-card.group-assessment-complete {
    background: linear-gradient(180deg, rgba(22, 49, 36, 0.98) 0%, rgba(9, 24, 20, 0.98) 100%);
    border-color: rgba(52, 211, 153, 0.3);
}

.terminal-tile.single-card,
.supervised-group-card.single-card {
    min-width: 0;
    min-height: 0;
}

.terminal-tile.selected-card {
    background: linear-gradient(180deg, rgba(24, 28, 36, 0.98) 0%, rgba(14, 17, 24, 0.97) 100%);
}

.supervised-group-card.selected-card {
    box-shadow: 0 24px 46px rgba(0, 0, 0, 0.28), inset 0 0 0 999px rgba(255, 255, 255, 0.035);
}

.card-terminal-slot {
    border-radius: 20px;
    border: 1px solid rgba(74, 94, 118, 0.36);
    background: #000000;
    min-height: 0;
    padding: 16px;
}

.card-header-row {
    min-height: 34px;
}

.card-title {
    color: #f8fafc;
    font-size: 18px;
    font-weight: 800;
}

.card-title-stack {
    min-width: 0;
}

.group-subtitle {
    color: rgba(208, 224, 232, 0.66);
    font-size: 12px;
    font-weight: 650;
    margin-top: 2px;
}

.card-status {
    color: rgba(199, 218, 240, 0.92);
    background: rgba(30, 58, 95, 0.62);
    border: 1px solid rgba(96, 165, 250, 0.2);
    border-radius: 999px;
    font-size: 11px;
    font-weight: 800;
    letter-spacing: 0.08em;
    padding: 4px 10px;
    text-transform: uppercase;
}

.card-status.group-status-watching {
    color: rgba(204, 251, 241, 0.94);
    background: rgba(20, 184, 166, 0.16);
    border-color: rgba(45, 212, 191, 0.32);
}

.card-status.group-status-active {
    color: rgba(220, 252, 231, 0.95);
    background: rgba(34, 197, 94, 0.15);
    border-color: rgba(74, 222, 128, 0.32);
}

.card-status.group-status-stalling {
    color: rgba(254, 243, 199, 0.95);
    background: rgba(245, 158, 11, 0.16);
    border-color: rgba(251, 191, 36, 0.36);
}

.card-status.group-status-blocked {
    color: rgba(254, 226, 226, 0.96);
    background: rgba(239, 68, 68, 0.18);
    border-color: rgba(248, 113, 113, 0.38);
}

.card-status.group-status-complete {
    color: rgba(209, 250, 229, 0.96);
    background: rgba(16, 185, 129, 0.16);
    border-color: rgba(52, 211, 153, 0.34);
}

.card-headline {
    color: #f8fafc;
    font-size: 15px;
    font-weight: 650;
    line-height: 1.2;
}

.card-recency {
    color: rgba(188, 201, 216, 0.82);
    font-size: 12px;
    font-weight: 600;
}

.group-summary-frame {
    border-radius: 16px;
    border: 1px solid rgba(204, 251, 241, 0.11);
    background: rgba(0, 0, 0, 0.24);
}

.group-summary-content {
    color: rgba(241, 245, 249, 0.94);
}

.markdown-heading {
    color: #f8fafc;
    font-size: 18px;
    font-weight: 800;
    margin-bottom: 3px;
}

.markdown-heading-small {
    font-size: 16px;
}

.markdown-paragraph,
.markdown-list-item {
    color: rgba(230, 240, 246, 0.9);
    font-size: 15px;
    line-height: 1.4;
}

.markdown-muted {
    color: rgba(181, 197, 208, 0.68);
    font-size: 14px;
    font-weight: 650;
}

.markdown-code-block {
    color: rgba(226, 232, 240, 0.92);
    background: rgba(0, 0, 0, 0.36);
    border: 1px solid rgba(148, 163, 184, 0.12);
    border-radius: 10px;
    font-size: 14px;
    padding: 8px 10px;
}

.markdown-table {
    border-radius: 10px;
    background: rgba(0, 0, 0, 0.2);
}

.markdown-table-cell {
    color: rgba(231, 238, 244, 0.9);
    border: 1px solid rgba(148, 163, 184, 0.12);
    font-size: 14px;
    padding: 8px 10px;
}

.markdown-table-header {
    color: rgba(248, 250, 252, 0.96);
    background: rgba(255, 255, 255, 0.045);
    font-weight: 800;
}

.toolbar-add-button,
.toolbar-toggle-button {
    background: rgba(30, 38, 50, 0.72);
    color: rgba(188, 201, 216, 0.86);
    border: 1px solid rgba(163, 175, 194, 0.14);
    border-radius: 10px;
    font-size: 12px;
    font-weight: 700;
    min-height: 0;
    padding: 4px 14px;
}

.toolbar-add-button:hover,
.toolbar-toggle-button:hover {
    background: rgba(37, 72, 118, 0.82);
    border-color: rgba(96, 165, 250, 0.32);
}

.toolbar-toggle-button:checked {
    background: rgba(30, 58, 95, 0.82);
    color: rgba(199, 218, 240, 0.94);
    border-color: rgba(96, 165, 250, 0.28);
}

.launcher-window {
    background: #000000;
}

.launcher-title {
    color: #f8fafc;
    font-size: 30px;
    font-weight: 850;
}

.launcher-subtitle,
.launcher-muted {
    color: rgba(190, 204, 219, 0.72);
    font-size: 13px;
}

.launcher-panel {
    background: linear-gradient(180deg, rgba(15, 23, 34, 0.98) 0%, rgba(8, 12, 19, 0.98) 100%);
    border: 1px solid rgba(98, 125, 154, 0.22);
    border-radius: 18px;
    padding: 16px;
    min-width: 0;
}

.launcher-panel-title {
    color: rgba(248, 250, 252, 0.96);
    font-size: 17px;
    font-weight: 800;
}

.launcher-list,
.launcher-remote-list {
    background: rgba(0, 0, 0, 0.22);
    border: 1px solid rgba(148, 163, 184, 0.11);
    border-radius: 12px;
}

.launcher-remote-list {
    min-height: 68px;
}

.launcher-row {
    background: transparent;
    border-radius: 10px;
    margin: 4px;
}

.launcher-row:hover {
    background: rgba(37, 72, 118, 0.34);
}

.launcher-row:selected {
    background: rgba(37, 99, 145, 0.46);
}

.launcher-row-title {
    color: rgba(241, 245, 249, 0.95);
    font-size: 13px;
    font-weight: 750;
}

.launcher-row-detail {
    color: rgba(174, 190, 208, 0.72);
    font-size: 12px;
}

.launcher-entry {
    background: rgba(0, 0, 0, 0.32);
    color: rgba(241, 245, 249, 0.94);
    border: 1px solid rgba(148, 163, 184, 0.16);
    border-radius: 10px;
    min-height: 0;
}

.launcher-entry:focus {
    border-color: rgba(96, 165, 250, 0.42);
}

.launcher-primary-button,
.launcher-secondary-button {
    border-radius: 10px;
    font-size: 12px;
    font-weight: 750;
    min-height: 0;
    padding: 5px 14px;
}

.launcher-primary-button {
    background: rgba(30, 94, 140, 0.84);
    color: rgba(239, 246, 255, 0.96);
    border: 1px solid rgba(125, 211, 252, 0.28);
}

.launcher-primary-button:hover {
    background: rgba(37, 112, 166, 0.92);
    border-color: rgba(125, 211, 252, 0.42);
}

.launcher-secondary-button {
    background: rgba(30, 38, 50, 0.72);
    color: rgba(212, 224, 238, 0.88);
    border: 1px solid rgba(163, 175, 194, 0.15);
}

.launcher-secondary-button:hover {
    background: rgba(48, 61, 78, 0.82);
    border-color: rgba(203, 213, 225, 0.25);
}

.terminal-assist-entry {
    min-width: 0;
}

.terminal-assist-overlay {
    background: rgba(0, 0, 0, 0.62);
    border-radius: 18px;
    padding: 18px;
}

.terminal-assist-panel {
    background: rgba(9, 16, 25, 0.96);
    border: 1px solid rgba(125, 211, 252, 0.22);
    border-radius: 16px;
    box-shadow: 0 18px 42px rgba(0, 0, 0, 0.44);
    min-width: 0;
    padding: 14px;
}

.terminal-assist-status {
    color: rgba(226, 240, 250, 0.92);
    font-size: 13px;
    font-weight: 750;
}

.terminal-assist-cancel {
    background: rgba(30, 38, 50, 0.72);
    color: rgba(218, 228, 238, 0.88);
    border: 1px solid rgba(163, 175, 194, 0.16);
    border-radius: 10px;
    font-size: 12px;
    font-weight: 700;
    min-height: 0;
    padding: 4px 12px;
}

.terminal-assist-cancel:hover {
    background: rgba(69, 85, 108, 0.82);
    border-color: rgba(203, 213, 225, 0.28);
}

.terminal-dim-overlay {
    background: rgba(0, 0, 0, 0.10);
    border-radius: 18px;
}

terminal {
    border-radius: 18px;
    padding: 12px;
}
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::generate_application_css;

    #[test]
    fn contains_terminal_and_group_surface_selectors() {
        let css = generate_application_css();
        assert!(css.contains(".terminal-tile,"));
        assert!(css.contains(".supervised-group-card {"));
    }

    #[test]
    fn terminal_tiles_keep_minimal_card_shell() {
        let css = generate_application_css();
        assert!(css.contains("border-radius: 24px;"));
        assert!(css.contains("box-shadow: 0 24px 46px rgba(0, 0, 0, 0.28);"));
        assert!(css.contains("background: linear-gradient(180deg, rgba(21, 24, 30, 0.98) 0%, rgba(12, 14, 19, 0.97) 100%);"));
        assert!(css.contains("min-width: 280px;"));
        assert!(css.contains("min-height: 160px;"));
        assert!(!css.contains(".terminal-card .card-terminal-slot"));
    }

    #[test]
    fn selected_card_brightens_surface_without_border_indicator() {
        let css = generate_application_css();
        let selected_start = css
            .find(".terminal-tile.selected-card")
            .expect("missing selected-card selector");
        let selected_section =
            &css[selected_start..css[selected_start..].find('}').unwrap() + selected_start];
        assert!(selected_section.contains("background: linear-gradient"));
        assert!(!selected_section.contains("border-color"));
        assert!(!selected_section.contains("box-shadow"));
    }

    #[test]
    fn supervised_group_cards_have_distinct_assessment_colors() {
        let css = generate_application_css();
        assert!(css.contains(".supervised-group-card.group-assessment-watching"));
        assert!(css.contains(".supervised-group-card.group-assessment-active"));
        assert!(css.contains(".supervised-group-card.group-assessment-stalling"));
        assert!(css.contains(".supervised-group-card.group-assessment-blocked"));
        assert!(css.contains(".supervised-group-card.group-assessment-complete"));
    }

    #[test]
    fn supervised_group_summary_has_markdown_styles() {
        let css = generate_application_css();
        assert!(css.contains(".group-summary-content"));
        assert!(css.contains(".markdown-table-cell"));
        assert!(css.contains(".markdown-code-block"));
    }

    #[test]
    fn terminal_slot_keeps_framed_well() {
        let css = generate_application_css();
        assert!(css.contains("border-radius: 20px;"));
        assert!(css.contains("border: 1px solid rgba(74, 94, 118, 0.36);"));
        assert!(css.contains("background: #000000;"));
        assert!(css.contains("padding: 16px;"));
    }

    #[test]
    fn contains_terminal_assist_selector() {
        let css = generate_application_css();
        assert!(css.contains(".terminal-assist-entry"));
        assert!(css.contains(".terminal-assist-overlay"));
        assert!(css.contains(".terminal-assist-panel"));
    }

    #[test]
    fn contains_launcher_selectors() {
        let css = generate_application_css();
        assert!(css.contains(".launcher-panel"));
        assert!(css.contains(".launcher-row:selected"));
        assert!(css.contains(".launcher-primary-button"));
    }

    #[test]
    fn contains_unfocused_terminal_overlay_selector() {
        let css = generate_application_css();
        assert!(css.contains(".terminal-dim-overlay"));
        assert!(css.contains("background: rgba(0, 0, 0, 0.10);"));
    }
}
