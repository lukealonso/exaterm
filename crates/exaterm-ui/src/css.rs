use crate::supervision::BattleCardStatus;
use crate::theme::{self, Color, Gradient};

fn css_color(c: &Color) -> String {
    if (c.a - 1.0).abs() < f32::EPSILON {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    } else {
        format!("rgba({}, {}, {}, {})", c.r, c.g, c.b, c.a)
    }
}

fn css_gradient(deg: u16, g: &Gradient) -> String {
    format!(
        "linear-gradient({deg}deg, {} 0%, {} 100%)",
        css_color(&g.top),
        css_color(&g.bottom),
    )
}

fn card_status_css(name: &str, status: BattleCardStatus) -> String {
    let t = theme::card_theme(status);
    format!(
        "\
        .{name} {{\n\
        {i}background: {bg};\n\
        {i}border-color: {bc};\n\
        }}",
        bg = css_gradient(180, &t.background),
        bc = css_color(&t.border_color),
        i = INDENT,
    )
}

fn chip_status_css(name: &str, status: BattleCardStatus) -> String {
    let t = theme::status_chip_theme(status);
    format!(
        "\
        .{name} {{\n\
        {i}color: {fg};\n\
        {i}background: {bg};\n\
        {i}border-color: {bc};\n\
        }}",
        fg = css_color(&t.text_color),
        bg = css_color(&t.background),
        bc = css_color(&t.border_color),
        i = INDENT,
    )
}

const INDENT: &str = "            ";

/// Generates the full GTK CSS stylesheet from theme constants.
pub fn generate_application_css() -> String {
    let i = INDENT;

    let mut parts: Vec<String> = Vec::new();

    // --- structural rules (no theme mapping) ---

    parts.push(format!(
        "\
        window {{\n\
        {i}background: #000000;\n\
        }}"
    ));

    parts.push(format!(
        "\
        flowboxchild {{\n\
        {i}padding: 0;\n\
        {i}background: transparent;\n\
        {i}box-shadow: none;\n\
        {i}outline: none;\n\
        }}"
    ));

    parts.push(format!(
        "\
        flowboxchild:selected {{\n\
        {i}background: transparent;\n\
        {i}box-shadow: none;\n\
        {i}outline: none;\n\
        }}"
    ));

    parts.push(format!(
        "\
        flowboxchild:selected > * {{\n\
        {i}box-shadow: none;\n\
        }}"
    ));

    parts.push(format!(
        "\
        flowboxchild.selected-card > * {{\n\
        {i}border-color: rgba(113, 197, 255, 0.98);\n\
        {i}box-shadow: 0 0 0 1px rgba(113, 197, 255, 0.92), 0 22px 44px rgba(13, 92, 151, 0.24);\n\
        }}"
    ));

    parts.push(format!(
        "\
        .workspace-summary {{\n\
        {i}color: rgba(199, 210, 222, 0.9);\n\
        {i}font-size: 13px;\n\
        {i}letter-spacing: 0.08em;\n\
        {i}text-transform: uppercase;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .workspace-hint {{\n\
        {i}color: rgba(189, 204, 219, 0.74);\n\
        {i}font-size: 12px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .empty-state {{\n\
        {i}margin-top: 40px;\n\
        {i}margin-bottom: 56px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .empty-title {{\n\
        {i}color: #f8fafc;\n\
        {i}font-size: 28px;\n\
        {i}font-weight: 800;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .empty-body {{\n\
        {i}color: rgba(198, 211, 225, 0.82);\n\
        {i}font-size: 15px;\n\
        {i}line-height: 1.45;\n\
        }}"
    ));

    // .battle-card — uses card theme constants for shared values
    let base_card = theme::card_theme(BattleCardStatus::Idle);
    parts.push(format!(
        "\
        .battle-card {{\n\
        {i}border-radius: {br}px;\n\
        {i}border: 1px solid rgba(163, 175, 194, 0.16);\n\
        {i}background: rgba(10, 18, 28, 0.95);\n\
        {i}box-shadow: 0 {oy}px {bl}px {sc};\n\
        {i}min-width: {mw}px;\n\
        {i}min-height: {mh}px;\n\
        }}",
        br = base_card.border_radius as u32,
        oy = base_card.shadow.offset_y as u32,
        bl = base_card.shadow.blur as u32,
        sc = css_color(&base_card.shadow.color),
        mw = base_card.min_width as u32,
        mh = base_card.min_height as u32,
    ));

    parts.push(format!(
        "\
        .battle-card.single-card {{\n\
        {i}min-width: 0;\n\
        {i}min-height: 0;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .battle-card.scrollback-card {{\n\
        {i}min-width: 0;\n\
        {i}min-height: 0;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .card-terminal-slot {{\n\
        {i}border-radius: 20px;\n\
        {i}border: 1px solid rgba(120, 136, 158, 0.2);\n\
        {i}background: rgba(7, 13, 20, 0.96);\n\
        {i}min-height: 0;\n\
        {i}padding: 10px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .card-terminal-slot.scrollback-terminal-hidden {{\n\
        {i}min-height: 0;\n\
        {i}padding: 0;\n\
        {i}border-color: transparent;\n\
        {i}background: transparent;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .card-header-row {{\n\
        {i}min-height: 34px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .card-body-stack {{\n\
        {i}margin-top: 2px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .card-bottom-stack,\n\
        .card-footer-stack {{\n\
        {i}margin-top: 0;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .card-scrollback-band {{\n\
        {i}border-radius: 14px;\n\
        {i}border: 1px solid rgba(173, 188, 204, 0.08);\n\
        {i}background: rgba(8, 14, 22, 0.34);\n\
        {i}padding: 8px 10px;\n\
        {i}min-height: 0;\n\
        }}"
    ));

    // .card-scrollback-line — uses scrollback_line_font()
    let sl_font = theme::scrollback_line_font();
    parts.push(format!(
        "\
        .card-scrollback-line {{\n\
        {i}color: rgba(202, 214, 227, 0.88);\n\
        {i}font-size: {sz}px;\n\
        {i}font-family: Monospace;\n\
        {i}line-height: 1.1;\n\
        }}",
        sz = sl_font.size as u32,
    ));

    parts.push(format!(
        "\
        .card-bars-row {{\n\
        {i}margin-top: 0;\n\
        }}"
    ));

    // .card-title — uses card_title_font()
    let tf = theme::card_title_font();
    parts.push(format!(
        "\
        .card-title {{\n\
        {i}font-weight: {w};\n\
        {i}font-size: {s}px;\n\
        {i}color: #f8fafc;\n\
        }}",
        w = tf.weight,
        s = tf.size as u32,
    ));

    // .card-subtitle — uses card_subtitle_font()
    let sf = theme::card_subtitle_font();
    parts.push(format!(
        "\
        .card-subtitle {{\n\
        {i}color: rgba(196, 208, 222, 0.66);\n\
        {i}font-size: {s}px;\n\
        {i}letter-spacing: {ls}em;\n\
        {i}text-transform: uppercase;\n\
        }}",
        s = sf.size as u32,
        ls = sf.letter_spacing,
    ));

    // .card-status — uses card_status_font()
    let csf = theme::card_status_font();
    parts.push(format!(
        "\
        .card-status {{\n\
        {i}font-weight: {w};\n\
        {i}font-size: {s}px;\n\
        {i}letter-spacing: {ls}em;\n\
        {i}text-transform: uppercase;\n\
        {i}border-radius: 999px;\n\
        {i}padding: 4px 10px;\n\
        {i}border: 1px solid rgba(190, 202, 217, 0.2);\n\
        }}",
        w = csf.weight,
        s = csf.size as u32,
        ls = csf.letter_spacing,
    ));

    // .card-recency — uses card_recency_font()
    let rf = theme::card_recency_font();
    parts.push(format!(
        "\
        .card-recency {{\n\
        {i}color: rgba(188, 201, 216, 0.88);\n\
        {i}font-size: {s}px;\n\
        {i}font-weight: {w};\n\
        {i}letter-spacing: {ls}em;\n\
        }}",
        s = rf.size as u32,
        w = rf.weight,
        ls = rf.letter_spacing,
    ));

    // .card-headline — uses card_headline_font()
    let hf = theme::card_headline_font();
    parts.push(format!(
        "\
        .card-headline {{\n\
        {i}color: #f8fafc;\n\
        {i}font-weight: {w};\n\
        {i}font-size: {s}px;\n\
        {i}line-height: 1.12;\n\
        }}",
        w = hf.weight,
        s = hf.size as u32,
    ));

    // .card-detail — uses card_detail_font()
    let df = theme::card_detail_font();
    parts.push(format!(
        "\
        .card-detail {{\n\
        {i}color: rgba(226, 234, 242, 0.94);\n\
        {i}font-size: {s}px;\n\
        {i}font-weight: {w};\n\
        {i}line-height: 1.25;\n\
        }}",
        s = df.size as u32,
        w = df.weight,
    ));

    // .card-evidence — uses card_evidence_font()
    let ef = theme::card_evidence_font();
    parts.push(format!(
        "\
        .card-evidence {{\n\
        {i}color: rgba(198, 212, 227, 0.88);\n\
        {i}font-size: {s}px;\n\
        {i}font-family: Monospace;\n\
        {i}background: rgba(11, 18, 28, 0.32);\n\
        {i}border-radius: 11px;\n\
        {i}border: 1px solid rgba(173, 188, 204, 0.12);\n\
        {i}padding: 7px 10px;\n\
        }}",
        s = ef.size as u32,
    ));

    // .card-alert — uses card_alert_font()
    let af = theme::card_alert_font();
    parts.push(format!(
        "\
        .card-alert {{\n\
        {i}color: rgba(202, 214, 227, 0.78);\n\
        {i}font-size: {s}px;\n\
        {i}font-weight: {w};\n\
        {i}line-height: 1.2;\n\
        {i}margin: 0;\n\
        }}",
        s = af.size as u32,
        w = af.weight,
    ));

    parts.push(format!(
        "\
        .card-control-row {{\n\
        {i}min-height: 28px;\n\
        {i}margin-top: -2px;\n\
        {i}margin-bottom: -2px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .card-control-label {{\n\
        {i}color: rgba(203, 214, 226, 0.72);\n\
        {i}font-size: 10px;\n\
        {i}font-weight: 700;\n\
        {i}letter-spacing: 0.08em;\n\
        {i}text-transform: uppercase;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .card-control-state {{\n\
        {i}font-size: 10px;\n\
        {i}font-weight: 800;\n\
        {i}letter-spacing: 0.08em;\n\
        {i}text-transform: uppercase;\n\
        {i}border-radius: 999px;\n\
        {i}padding: 4px 10px;\n\
        {i}border: 1px solid rgba(190, 202, 217, 0.16);\n\
        }}"
    ));

    parts.push(format!(
        "\
        .card-control-off {{\n\
        {i}color: rgba(214, 222, 230, 0.84);\n\
        {i}background: rgba(84, 97, 112, 0.18);\n\
        {i}border-color: rgba(163, 175, 194, 0.16);\n\
        }}"
    ));

    parts.push(format!(
        "\
        .card-control-armed {{\n\
        {i}color: #fde68a;\n\
        {i}background: rgba(120, 87, 10, 0.22);\n\
        {i}border-color: rgba(250, 204, 21, 0.22);\n\
        }}"
    ));

    parts.push(format!(
        "\
        .card-control-nudged {{\n\
        {i}color: #86efac;\n\
        {i}background: rgba(17, 88, 51, 0.22);\n\
        {i}border-color: rgba(74, 222, 128, 0.2);\n\
        }}"
    ));

    parts.push(format!(
        "\
        .card-control-cooldown {{\n\
        {i}color: #93c5fd;\n\
        {i}background: rgba(33, 82, 145, 0.22);\n\
        {i}border-color: rgba(96, 165, 250, 0.2);\n\
        }}"
    ));

    // .bar-widget
    parts.push(format!(
        "\
        .bar-widget {{\n\
        {i}border-radius: 12px;\n\
        {i}border: 1px solid rgba(173, 188, 204, 0.08);\n\
        {i}background: rgba(11, 18, 28, 0.18);\n\
        {i}padding: 7px 9px;\n\
        }}"
    ));

    // .bar-caption — uses bar_caption_font()
    let bcf = theme::bar_caption_font();
    parts.push(format!(
        "\
        .bar-caption {{\n\
        {i}color: rgba(186, 200, 214, 0.62);\n\
        {i}font-size: {s}px;\n\
        {i}letter-spacing: {ls}em;\n\
        {i}text-transform: uppercase;\n\
        }}",
        s = bcf.size as u32,
        ls = bcf.letter_spacing,
    ));

    parts.push(format!(
        "\
        .segmented-bar {{\n\
        {i}min-height: 8px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .bar-segment {{\n\
        {i}min-height: 8px;\n\
        {i}border-radius: 999px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .bar-empty {{\n\
        {i}background: rgba(163, 175, 194, 0.14);\n\
        }}"
    ));

    parts.push(format!("\
        .bar-calm {{\n\
        {i}background: linear-gradient(90deg, rgba(110, 231, 183, 0.88) 0%, rgba(52, 211, 153, 0.92) 100%);\n\
        }}"));

    parts.push(format!("\
        .bar-watch {{\n\
        {i}background: linear-gradient(90deg, rgba(250, 204, 21, 0.88) 0%, rgba(251, 146, 60, 0.92) 100%);\n\
        }}"));

    parts.push(format!("\
        .bar-alert {{\n\
        {i}background: linear-gradient(90deg, rgba(248, 113, 113, 0.9) 0%, rgba(239, 68, 68, 0.94) 100%);\n\
        }}"));

    // .bar-reason — uses bar_reason_font()
    let brf = theme::bar_reason_font();
    parts.push(format!(
        "\
        .bar-reason {{\n\
        {i}color: rgba(186, 200, 214, 0.56);\n\
        {i}font-size: {s}px;\n\
        {i}line-height: 1.2;\n\
        }}",
        s = brf.size as u32,
    ));

    // .focus-title — uses focus_title_font()
    let ftf = theme::focus_title_font();
    parts.push(format!(
        "\
        .focus-title {{\n\
        {i}color: #f8fafc;\n\
        {i}font-size: {s}px;\n\
        {i}font-weight: {w};\n\
        }}",
        s = ftf.size as u32,
        w = ftf.weight,
    ));

    // .focus-subtitle — uses focus_subtitle_font()
    let fsf = theme::focus_subtitle_font();
    parts.push(format!(
        "\
        .focus-subtitle {{\n\
        {i}color: rgba(196, 208, 222, 0.78);\n\
        {i}font-size: {s}px;\n\
        {i}margin-bottom: 6px;\n\
        }}",
        s = fsf.size as u32,
    ));

    parts.push(format!(
        "\
        .focus-frame {{\n\
        {i}border-radius: 24px;\n\
        {i}border: 1px solid rgba(120, 136, 158, 0.2);\n\
        {i}background: rgba(7, 13, 20, 0.96);\n\
        {i}padding: 10px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .focus-panel {{\n\
        {i}margin-top: 4px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .pill {{\n\
        {i}border-radius: 999px;\n\
        {i}padding: 6px 14px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .pill {{\n\
        {i}background: rgba(119, 198, 255, 0.16);\n\
        {i}color: #dbeafe;\n\
        }}"
    ));

    parts.push(format!(
        "\
        flowboxchild.focused-card > * {{\n\
        {i}border-color: rgba(110, 231, 183, 0.92);\n\
        {i}box-shadow: 0 0 0 1px rgba(110, 231, 183, 0.78), 0 20px 38px rgba(7, 88, 57, 0.22);\n\
        }}"
    ));

    // --- card status gradients (from theme) ---
    let card_statuses = [
        ("card-idle", BattleCardStatus::Idle),
        ("card-stopped", BattleCardStatus::Stopped),
        ("card-active", BattleCardStatus::Active),
        ("card-thinking", BattleCardStatus::Thinking),
        ("card-working", BattleCardStatus::Working),
        ("card-blocked", BattleCardStatus::Blocked),
        ("card-failed", BattleCardStatus::Failed),
        ("card-complete", BattleCardStatus::Complete),
        ("card-detached", BattleCardStatus::Detached),
    ];
    for (name, status) in card_statuses {
        parts.push(card_status_css(name, status));
    }

    // --- battle chip statuses (from theme) ---
    let chip_statuses = [
        ("battle-idle", BattleCardStatus::Idle),
        ("battle-stopped", BattleCardStatus::Stopped),
        ("battle-active", BattleCardStatus::Active),
        ("battle-thinking", BattleCardStatus::Thinking),
        ("battle-working", BattleCardStatus::Working),
        ("battle-blocked", BattleCardStatus::Blocked),
        ("battle-failed", BattleCardStatus::Failed),
        ("battle-complete", BattleCardStatus::Complete),
        ("battle-detached", BattleCardStatus::Detached),
    ];
    for (name, status) in chip_statuses {
        parts.push(chip_status_css(name, status));
    }

    // --- focus mode overrides ---
    parts.push(format!(
        "\
        .focus-mode flowboxchild .battle-card {{\n\
        {i}min-width: 176px;\n\
        {i}min-height: 182px;\n\
        {i}border-radius: 18px;\n\
        {i}box-shadow: 0 14px 28px rgba(0, 0, 0, 0.22);\n\
        }}"
    ));

    parts.push(format!(
        "\
        .focus-mode flowboxchild .card-title {{\n\
        {i}font-size: 15px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .focus-mode flowboxchild .card-status,\n\
        .focus-mode flowboxchild .card-recency {{\n\
        {i}font-size: 10px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .focus-mode flowboxchild .card-header-row {{\n\
        {i}min-height: 28px;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .focus-mode flowboxchild .card-bottom-stack {{\n\
        {i}margin-top: 0;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .focus-mode flowboxchild .card-alert {{\n\
        {i}color: rgba(206, 217, 229, 0.84);\n\
        {i}font-size: 12px;\n\
        {i}font-weight: 600;\n\
        {i}line-height: 1.3;\n\
        {i}padding: 0;\n\
        {i}background: transparent;\n\
        {i}border-color: transparent;\n\
        {i}min-height: 112px;\n\
        {i}margin-top: 6px;\n\
        {i}margin-bottom: 0;\n\
        {i}margin-left: 0;\n\
        {i}margin-right: 0;\n\
        }}"
    ));

    parts.push(format!(
        "\
        .focus-mode flowboxchild .card-headline,\n\
        .focus-mode flowboxchild .card-detail,\n\
        .focus-mode flowboxchild .card-scrollback-band,\n\
        .focus-mode flowboxchild .bar-widget {{\n\
        }}"
    ));

    parts.push(format!(
        "\
        terminal {{\n\
        {i}border-radius: 18px;\n\
        {i}padding: 12px;\n\
        }}"
    ));

    // Assemble with the same leading-newline / 8-space-indented formatting
    // as the original hardcoded string.
    let body = parts
        .iter()
        .map(|block| {
            block
                .lines()
                .map(|line| format!("        {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    format!("\n{body}\n        ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The original hardcoded CSS from exaterm-gtk style.rs, used as the
    /// canonical reference for snapshot comparison.
    const ORIGINAL_CSS: &str = "
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
            color: #f8fafc;
            font-weight: 800;
            font-size: 20px;
            line-height: 1.12;
        }

        .card-detail {
            color: rgba(226, 234, 242, 0.94);
            font-size: 15px;
            font-weight: 650;
            line-height: 1.25;
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
            padding: 7px 9px;
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

        .bar-calm {
            background: linear-gradient(90deg, rgba(110, 231, 183, 0.88) 0%, rgba(52, 211, 153, 0.92) 100%);
        }

        .bar-watch {
            background: linear-gradient(90deg, rgba(250, 204, 21, 0.88) 0%, rgba(251, 146, 60, 0.92) 100%);
        }

        .bar-alert {
            background: linear-gradient(90deg, rgba(248, 113, 113, 0.9) 0%, rgba(239, 68, 68, 0.94) 100%);
        }

        .bar-reason {
            color: rgba(186, 200, 214, 0.56);
            font-size: 10px;
            line-height: 1.2;
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
        .focus-mode flowboxchild .card-detail,
        .focus-mode flowboxchild .card-scrollback-band,
        .focus-mode flowboxchild .bar-widget {
        }

        terminal {
            border-radius: 18px;
            padding: 12px;
        }
        ";

    fn normalize_whitespace(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn contains_battle_card_selector() {
        let css = generate_application_css();
        assert!(
            css.contains(".battle-card {"),
            "missing .battle-card selector"
        );
    }

    #[test]
    fn contains_all_card_status_selectors() {
        let css = generate_application_css();
        for name in [
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
            assert!(
                css.contains(&format!(".{name} {{")),
                "missing .{name} selector"
            );
        }
    }

    #[test]
    fn contains_all_battle_chip_selectors() {
        let css = generate_application_css();
        for name in [
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
            assert!(
                css.contains(&format!(".{name} {{")),
                "missing .{name} selector"
            );
        }
    }

    #[test]
    fn card_title_has_correct_properties() {
        let css = generate_application_css();
        let title_start = css
            .find(".card-title {")
            .expect("missing .card-title selector");
        let title_section = &css[title_start..css[title_start..].find('}').unwrap() + title_start];
        assert!(
            title_section.contains("font-weight: 800"),
            "card-title missing font-weight: 800"
        );
        assert!(
            title_section.contains("font-size: 18px"),
            "card-title missing font-size: 18px"
        );
    }

    #[test]
    fn card_active_gradient_matches() {
        let css = generate_application_css();
        let active_start = css
            .find(".card-active {")
            .expect("missing .card-active selector");
        let active_section =
            &css[active_start..css[active_start..].find('}').unwrap() + active_start];
        assert!(
            active_section.contains("rgba(14, 33, 52, 0.98)"),
            "card-active missing rgba(14, 33, 52, 0.98)"
        );
    }

    #[test]
    fn terminal_selector_present() {
        let css = generate_application_css();
        assert!(css.contains("terminal {"), "missing terminal selector");
    }

    #[test]
    fn snapshot_matches_original() {
        let generated = generate_application_css();
        let gen_norm = normalize_whitespace(&generated);
        let orig_norm = normalize_whitespace(ORIGINAL_CSS);
        assert_eq!(gen_norm, orig_norm, "generated CSS does not match original");
    }
}
