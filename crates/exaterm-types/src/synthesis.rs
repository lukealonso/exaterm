use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalAssistSuggestion {
    pub insert_text: String,
}

impl TerminalAssistSuggestion {
    pub fn sanitize(mut self) -> Self {
        self.insert_text = sanitize_terminal_insert_text(&self.insert_text);
        self
    }
}

fn sanitize_terminal_insert_text(value: &str) -> String {
    let trimmed = strip_terminal_insert_fence(value.trim());
    let first_line = trimmed
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end_matches('\r')
        .trim();
    first_line.chars().take(600).collect()
}

fn strip_terminal_insert_fence(value: &str) -> &str {
    let Some(rest) = value.strip_prefix("```") else {
        return value;
    };
    let body = rest
        .find(|ch| ch == '\n' || ch == '\r')
        .map(|index| &rest[index + 1..])
        .unwrap_or(rest);
    body.split_once("```")
        .map(|(body, _)| body.trim())
        .unwrap_or_else(|| body.trim())
}

#[cfg(test)]
mod tests {
    use super::TerminalAssistSuggestion;

    #[test]
    fn terminal_assist_sanitize_keeps_single_insert_line() {
        let suggestion = TerminalAssistSuggestion {
            insert_text: "  du -sh ./* | sort -h\n# explanation  ".into(),
        }
        .sanitize();

        assert_eq!(suggestion.insert_text, "du -sh ./* | sort -h");
    }

    #[test]
    fn terminal_assist_sanitize_strips_markdown_fence_inside_insert_text() {
        let suggestion = TerminalAssistSuggestion {
            insert_text: "```sh\ncargo test -p exaterm-core synthesis::tests\n```".into(),
        }
        .sanitize();

        assert_eq!(
            suggestion.insert_text,
            "cargo test -p exaterm-core synthesis::tests"
        );
    }

    #[test]
    fn terminal_assist_sanitize_preserves_paths_and_shell_operators() {
        let suggestion = TerminalAssistSuggestion {
            insert_text:
                "  rg -n \"TerminalAssist\" crates/exaterm-core/src/synthesis.rs && cargo check -p exaterm-core  "
                    .into(),
        }
        .sanitize();

        assert_eq!(
            suggestion.insert_text,
            "rg -n \"TerminalAssist\" crates/exaterm-core/src/synthesis.rs && cargo check -p exaterm-core"
        );
    }

    #[test]
    fn terminal_assist_sanitize_bounds_length() {
        let suggestion = TerminalAssistSuggestion {
            insert_text: "x".repeat(700),
        }
        .sanitize();

        assert_eq!(suggestion.insert_text.len(), 600);
    }
}
