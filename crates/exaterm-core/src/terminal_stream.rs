use std::time::{Duration, Instant};

const PAINT_CONSOLIDATE_SETTLE: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedLine {
    pub text: String,
    pub overwrite_count: usize,
}

#[derive(Debug, Default)]
pub struct PaintedLineTracker {
    current_line: String,
    last_emitted: Option<String>,
}

#[derive(Debug, Default)]
pub struct PaintConsolidator {
    pending: Option<String>,
    last_update_at: Option<Instant>,
    last_emitted: Option<String>,
}

#[derive(Debug, Default)]
pub struct TerminalStreamProcessor {
    carry: String,
    overwrite_count: usize,
    painted_line_tracker: PaintedLineTracker,
    paint_consolidator: PaintConsolidator,
}

#[derive(Debug, Default)]
pub struct StreamUpdate {
    pub semantic_lines: Vec<String>,
    pub painted_line: Option<String>,
}

impl StreamUpdate {
    pub fn is_empty(&self) -> bool {
        self.semantic_lines.is_empty() && self.painted_line.is_none()
    }
}

impl TerminalStreamProcessor {
    pub fn ingest(&mut self, chunk: &[u8]) -> StreamUpdate {
        let semantic_lines = decode_chunk(chunk, &mut self.carry, &mut self.overwrite_count)
            .into_iter()
            .map(|line| line.text)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();

        let painted_line = self
            .painted_line_tracker
            .ingest(chunk)
            .map(|line| {
                self.paint_consolidator.ingest(line);
            })
            .and_then(|_| self.paint_consolidator.maybe_emit())
            .or_else(|| self.paint_consolidator.maybe_emit());

        StreamUpdate {
            semantic_lines,
            painted_line,
        }
    }
}

pub fn decode_chunk(
    chunk: &[u8],
    carry: &mut String,
    overwrite_count: &mut usize,
) -> Vec<DecodedLine> {
    let mut lines = Vec::new();
    let mut index = 0usize;
    let mut printable = Vec::new();

    while index < chunk.len() {
        let flush_printable = |printable: &mut Vec<u8>, carry: &mut String| {
            if !printable.is_empty() {
                carry.push_str(&String::from_utf8_lossy(printable));
                printable.clear();
            }
        };

        match chunk[index] {
            0x1b => {
                flush_printable(&mut printable, carry);
                index += 1;
                if index < chunk.len() {
                    match chunk[index] {
                        b'[' => {
                            index += 1;
                            let start = index;
                            while index < chunk.len() {
                                let byte = chunk[index];
                                index += 1;
                                if (byte as char).is_ascii_alphabetic() || byte == b'~' {
                                    if csi_implies_rewrite(&chunk[start..index]) {
                                        carry.clear();
                                        *overwrite_count += 1;
                                    }
                                    break;
                                }
                            }
                        }
                        b']' => {
                            index += 1;
                            while index < chunk.len() {
                                match chunk[index] {
                                    0x07 => {
                                        index += 1;
                                        break;
                                    }
                                    0x1b if index + 1 < chunk.len()
                                        && chunk[index + 1] == b'\\' =>
                                    {
                                        index += 2;
                                        break;
                                    }
                                    _ => index += 1,
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            b'\r' => {
                flush_printable(&mut printable, carry);
                if index + 1 < chunk.len() && chunk[index + 1] == b'\n' {
                    if !carry.is_empty() {
                        lines.push(DecodedLine {
                            text: carry.trim_end().to_string(),
                            overwrite_count: *overwrite_count,
                        });
                        carry.clear();
                        *overwrite_count = 0;
                    }
                    index += 2;
                } else {
                    carry.clear();
                    *overwrite_count += 1;
                    index += 1;
                }
            }
            b'\n' => {
                flush_printable(&mut printable, carry);
                if !carry.is_empty() {
                    lines.push(DecodedLine {
                        text: carry.trim_end().to_string(),
                        overwrite_count: *overwrite_count,
                    });
                    carry.clear();
                    *overwrite_count = 0;
                }
                index += 1;
            }
            0x08 => {
                flush_printable(&mut printable, carry);
                carry.pop();
                index += 1;
            }
            byte if !byte.is_ascii_control() || byte == b'\t' => {
                printable.push(byte);
                index += 1;
            }
            _ => {
                index += 1;
            }
        }
    }

    if !printable.is_empty() {
        carry.push_str(&String::from_utf8_lossy(&printable));
    }

    lines
}

pub fn csi_implies_rewrite(sequence: &[u8]) -> bool {
    let Some(final_byte) = sequence.last().copied() else {
        return false;
    };

    matches!(final_byte, b'G' | b'H' | b'f' | b'K' | b'P' | b'X')
}

impl PaintedLineTracker {
    pub fn ingest(&mut self, chunk: &[u8]) -> Option<String> {
        let mut index = 0usize;
        let mut printable = Vec::new();
        let mut candidate = None::<String>;

        let flush_printable =
            |printable: &mut Vec<u8>, current_line: &mut String, candidate: &mut Option<String>| {
                if !printable.is_empty() {
                    current_line.push_str(&String::from_utf8_lossy(printable));
                    printable.clear();
                    let trimmed = current_line.trim();
                    if !trimmed.is_empty() {
                        *candidate = Some(trimmed.to_string());
                    }
                }
            };

        while index < chunk.len() {
            match chunk[index] {
                0x1b => {
                    flush_printable(&mut printable, &mut self.current_line, &mut candidate);
                    index += 1;
                    if index < chunk.len() {
                        match chunk[index] {
                            b'[' => {
                                index += 1;
                                let start = index;
                                while index < chunk.len() {
                                    let byte = chunk[index];
                                    index += 1;
                                    if (byte as char).is_ascii_alphabetic() || byte == b'~' {
                                        if csi_implies_rewrite(&chunk[start..index]) {
                                            self.current_line.clear();
                                        }
                                        break;
                                    }
                                }
                            }
                            b']' => {
                                index += 1;
                                while index < chunk.len() {
                                    match chunk[index] {
                                        0x07 => {
                                            index += 1;
                                            break;
                                        }
                                        0x1b if index + 1 < chunk.len()
                                            && chunk[index + 1] == b'\\' =>
                                        {
                                            index += 2;
                                            break;
                                        }
                                        _ => index += 1,
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                b'\r' => {
                    flush_printable(&mut printable, &mut self.current_line, &mut candidate);
                    self.current_line.clear();
                    if index + 1 < chunk.len() && chunk[index + 1] == b'\n' {
                        index += 2;
                    } else {
                        index += 1;
                    }
                }
                b'\n' => {
                    flush_printable(&mut printable, &mut self.current_line, &mut candidate);
                    self.current_line.clear();
                    index += 1;
                }
                0x08 => {
                    flush_printable(&mut printable, &mut self.current_line, &mut candidate);
                    self.current_line.pop();
                    let trimmed = self.current_line.trim();
                    if !trimmed.is_empty() {
                        candidate = Some(trimmed.to_string());
                    }
                    index += 1;
                }
                byte if !byte.is_ascii_control() || byte == b'\t' => {
                    printable.push(byte);
                    index += 1;
                }
                _ => {
                    index += 1;
                }
            }
        }

        flush_printable(&mut printable, &mut self.current_line, &mut candidate);

        match candidate {
            Some(line) if self.last_emitted.as_ref() != Some(&line) => {
                self.last_emitted = Some(line.clone());
                Some(line)
            }
            _ => None,
        }
    }
}

impl PaintConsolidator {
    pub fn ingest(&mut self, line: String) {
        self.ingest_at(line, Instant::now());
    }

    pub fn maybe_emit(&mut self) -> Option<String> {
        self.maybe_emit_at(Instant::now())
    }

    fn ingest_at(&mut self, line: String, now: Instant) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }

        self.pending = Some(match self.pending.take() {
            Some(existing) => merge_paint_lines(&existing, trimmed),
            None => trimmed.to_string(),
        });
        self.last_update_at = Some(now);
    }

    fn maybe_emit_at(&mut self, now: Instant) -> Option<String> {
        let pending = self.pending.clone()?;
        let settled = self.last_update_at.is_some_and(|last_update_at| {
            now.duration_since(last_update_at) >= PAINT_CONSOLIDATE_SETTLE
        });
        if !settled {
            return None;
        }
        if !looks_consolidated_worthy(&pending) {
            return None;
        }
        if self.last_emitted.as_ref() == Some(&pending) {
            return None;
        }
        self.last_emitted = Some(pending.clone());
        Some(pending)
    }
}

pub fn merge_paint_lines(existing: &str, incoming: &str) -> String {
    if incoming == existing {
        return existing.to_string();
    }
    if incoming.chars().all(|ch| ch.is_ascii_digit()) && looks_wordish(existing) {
        return format!("{existing} {incoming}");
    }
    if is_tiny_paint_fragment(incoming) {
        return existing.to_string();
    }
    if incoming.len() >= existing.len() && incoming.starts_with(existing) {
        return incoming.to_string();
    }
    if existing.len() >= incoming.len()
        && (existing.starts_with(incoming) || existing.contains(incoming))
    {
        return existing.to_string();
    }
    if incoming.len() > existing.len() && incoming.contains(existing) {
        return incoming.to_string();
    }
    incoming.to_string()
}

fn is_tiny_paint_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed == "•" {
        return true;
    }
    let visible = trimmed.chars().filter(|ch| !ch.is_whitespace()).count();
    let alpha = trimmed.chars().filter(|ch| ch.is_alphabetic()).count();
    visible <= 2 || (visible <= 4 && alpha <= 1)
}

fn looks_wordish(text: &str) -> bool {
    let alpha = text.chars().filter(|ch| ch.is_alphabetic()).count();
    alpha >= 4
}

fn looks_consolidated_worthy(text: &str) -> bool {
    let visible = text.chars().filter(|ch| !ch.is_whitespace()).count();
    let alpha = text.chars().filter(|ch| ch.is_alphabetic()).count();
    visible >= 4 || alpha >= 3
}

#[cfg(test)]
mod tests {
    use super::{
        DecodedLine, PaintConsolidator, PaintedLineTracker, csi_implies_rewrite, decode_chunk,
        merge_paint_lines,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn decodes_chunks_into_lines() {
        let mut carry = String::new();
        let mut overwrite_count = 0usize;
        let lines = decode_chunk(b"hello\r\nworld\npartial", &mut carry, &mut overwrite_count);
        assert_eq!(
            lines,
            vec![
                DecodedLine {
                    text: "hello".to_string(),
                    overwrite_count: 0,
                },
                DecodedLine {
                    text: "world".to_string(),
                    overwrite_count: 0,
                }
            ]
        );
        assert_eq!(carry, "partial");
    }

    #[test]
    fn carriage_return_overwrites_in_place_status_updates() {
        let mut carry = String::new();
        let mut overwrite_count = 0usize;
        let lines = decode_chunk(
            b"Working 1\rWorking 2\rWorking 3\nsteady line\n",
            &mut carry,
            &mut overwrite_count,
        );
        assert_eq!(
            lines,
            vec![
                DecodedLine {
                    text: "Working 3".to_string(),
                    overwrite_count: 2,
                },
                DecodedLine {
                    text: "steady line".to_string(),
                    overwrite_count: 0,
                }
            ]
        );
        assert!(carry.is_empty());
    }

    #[test]
    fn rewrite_like_csi_sequences_increment_overwrite_count() {
        let mut carry = String::new();
        let mut overwrite_count = 0usize;
        let lines = decode_chunk(b"alpha\x1b[2Kbeta\n", &mut carry, &mut overwrite_count);
        assert_eq!(
            lines,
            vec![DecodedLine {
                text: "beta".to_string(),
                overwrite_count: 1,
            }]
        );
    }

    #[test]
    fn recognizes_rewrite_like_csi_ops() {
        assert!(csi_implies_rewrite(b"2K"));
        assert!(csi_implies_rewrite(b"1G"));
        assert!(!csi_implies_rewrite(b"31m"));
    }

    #[test]
    fn painted_line_tracker_follows_overwrites() {
        let mut tracker = PaintedLineTracker::default();
        let painted = tracker
            .ingest(b"Working 1\rWorking 2\rWorking 3")
            .expect("painted line should update");
        assert_eq!(painted, "Working 3");
    }

    #[test]
    fn painted_line_tracker_follows_rewrite_like_csi() {
        let mut tracker = PaintedLineTracker::default();
        let painted = tracker
            .ingest(b"alpha\x1b[2Kbeta")
            .expect("painted line should update");
        assert_eq!(painted, "beta");
    }

    #[test]
    fn consolidator_merges_prefix_fragments() {
        assert_eq!(merge_paint_lines("Work", "Worki"), "Worki");
        assert_eq!(merge_paint_lines("Working", "orking"), "Working");
        assert_eq!(merge_paint_lines("Working", "1"), "Working 1");
    }

    #[test]
    fn consolidator_emits_settled_snapshots() {
        let mut consolidator = PaintConsolidator::default();
        let now = Instant::now();
        consolidator.ingest_at("W".into(), now);
        consolidator.ingest_at("Wo".into(), now + Duration::from_millis(10));
        consolidator.ingest_at("Wor".into(), now + Duration::from_millis(20));
        consolidator.ingest_at("Working".into(), now + Duration::from_millis(40));
        let painted = consolidator
            .maybe_emit_at(now + Duration::from_millis(250))
            .expect("settled snapshot should emit");
        assert_eq!(painted, "Working");
    }

    #[test]
    fn consolidator_allows_new_sentence_to_replace_status() {
        let mut consolidator = PaintConsolidator::default();
        let now = Instant::now();
        consolidator.ingest_at("Working".into(), now);
        consolidator.ingest_at(
            "Reviewing the current repository state first".into(),
            now + Duration::from_millis(40),
        );
        let painted = consolidator
            .maybe_emit_at(now + Duration::from_millis(250))
            .expect("sentence snapshot should emit");
        assert_eq!(painted, "Reviewing the current repository state first");
    }
}
