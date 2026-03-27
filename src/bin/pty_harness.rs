use exaterm::procfs;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::collections::VecDeque;
use std::io::Read;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const MAX_RECENT_LINES: usize = 200;
const PAINT_CONSOLIDATE_SETTLE: Duration = Duration::from_millis(100);
#[derive(Debug, Clone, PartialEq, Eq)]
struct DecodedLine {
    text: String,
    overwrite_count: usize,
}

#[derive(Debug, Default)]
struct PaintedLineTracker {
    current_line: String,
    last_emitted: Option<String>,
}

#[derive(Debug, Default)]
struct PaintConsolidator {
    pending: Option<String>,
    last_update_at: Option<Instant>,
    last_emitted: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (max_seconds, trace_control, paint_stream, command) =
        parse_args(std::env::args().skip(1).collect())?;
    let command_label = shell_join(&command);
    println!("[event {}] launching {}", timestamp_now(), command_label);

    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 40,
        cols: 160,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut builder = CommandBuilder::new(&command[0]);
    for arg in command.iter().skip(1) {
        builder.arg(arg);
    }
    builder.cwd(std::env::current_dir()?);

    let mut child = pair.slave.spawn_command(builder)?;
    drop(pair.slave);

    let pid = child.process_id().map(|id| id as u32);
    if let Some(pid) = pid {
        println!("[event {}] spawned pid {}", timestamp_now(), pid);
    }

    let mut reader = pair.master.try_clone_reader()?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let start = Instant::now();
    let mut carry = String::new();
    let mut overwrite_count = 0usize;
    let mut painted_line_tracker = PaintedLineTracker::default();
    let mut paint_consolidator = PaintConsolidator::default();
    let mut recent_lines = VecDeque::new();
    let mut last_output_at = Instant::now();
    let mut last_dominant_process = None::<String>;
    let mut last_process_poll = Instant::now() - Duration::from_secs(10);
    let mut last_idle_bucket = None::<u64>;
    loop {
        while let Ok(chunk) = rx.try_recv() {
            if paint_stream {
                if let Some(line) = painted_line_tracker.ingest(&chunk) {
                    paint_consolidator.ingest(line);
                }
            }
            for line in decode_chunk(&chunk, &mut carry, &mut overwrite_count) {
                let mut trimmed = line.text.clone();
                if trimmed.is_empty() {
                    continue;
                }
                if trace_control && line.overwrite_count > 0 {
                    trimmed = format!("[rewrites {}] {}", line.overwrite_count, trimmed);
                }
                if last_output_at.elapsed().as_secs() >= 15 {
                    println!(
                        "[progress {}] output resumed after {}s idle",
                        timestamp_now(),
                        last_output_at.elapsed().as_secs()
                    );
                }
                last_output_at = Instant::now();
                last_idle_bucket = None;
                push_recent(&mut recent_lines, trimmed.clone());
                println!("[line {}] {}", timestamp_now(), trimmed);
            }
        }

        if paint_stream {
            if let Some(line) = paint_consolidator.maybe_emit() {
                println!("[paint {}] {}", timestamp_now(), line);
            }
        }

        if let Some(pid) = pid.filter(|_| last_process_poll.elapsed() >= Duration::from_secs(2)) {
            last_process_poll = Instant::now();
            let dominant = procfs::dominant_child_command(pid).ok().flatten();
            if dominant != last_dominant_process {
                last_dominant_process = dominant.clone();
                if let Some(dominant) = dominant {
                    println!("[event {}] dominant process -> {}", timestamp_now(), dominant);
                }
            }
        }

        let idle_seconds = last_output_at.elapsed().as_secs();
        let idle_bucket = match idle_seconds {
            0..=14 => None,
            15..=29 => Some(15),
            30..=59 => Some(30),
            _ => Some(60),
        };
        if idle_bucket.is_some() && idle_bucket != last_idle_bucket {
            last_idle_bucket = idle_bucket;
            let last_line = recent_lines.back().cloned().unwrap_or_else(|| "no output yet".into());
            println!(
                "[progress {}] idle {}s | last meaningful line: {}",
                timestamp_now(),
                idle_seconds,
                last_line
            );
        }

        if last_process_poll.elapsed() >= Duration::from_secs(2) {
            if let Some(pid) = pid {
                if let Ok(tree) = procfs::format_process_tree(pid) {
                    let compact = tree
                        .lines()
                        .take(4)
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<_>>()
                        .join(" | ");
                    if !compact.is_empty() {
                        println!("[proc {}] {}", timestamp_now(), compact);
                    }
                }
                last_process_poll = Instant::now();
            }
        }

        if let Some(status) = child.try_wait()? {
            println!("[event {}] child exited with {}", timestamp_now(), status.exit_code());
            break;
        }

        if start.elapsed() >= max_seconds {
            println!(
                "[event {}] max runtime {}s reached, terminating child",
                timestamp_now(),
                max_seconds.as_secs()
            );
            child.kill()?;
            let status = child.wait()?;
            println!("[event {}] child exited with {}", timestamp_now(), status.exit_code());
            break;
        }

        std::thread::sleep(Duration::from_millis(250));
    }

    Ok(())
}

fn parse_args(args: Vec<String>) -> Result<(Duration, bool, bool, Vec<String>), String> {
    let mut max_seconds = 45u64;
    let mut trace_control = false;
    let mut paint_stream = false;
    let mut index = 0usize;

    while index < args.len() {
        match args[index].as_str() {
            "--max-seconds" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--max-seconds requires a value".to_string())?;
                max_seconds = value
                    .parse::<u64>()
                    .map_err(|_| format!("invalid --max-seconds value: {value}"))?;
                index += 2;
            }
            "--trace-control" => {
                trace_control = true;
                index += 1;
            }
            "--paint-stream" => {
                paint_stream = true;
                index += 1;
            }
            _ => break,
        }
    }

    let command = if index < args.len() {
        args[index..].to_vec()
    } else {
        vec!["codex".into(), "do a code review in this repo".into()]
    };

    if command.is_empty() {
        return Err("missing command".into());
    }

    Ok((Duration::from_secs(max_seconds), trace_control, paint_stream, command))
}

fn decode_chunk(chunk: &[u8], carry: &mut String, overwrite_count: &mut usize) -> Vec<DecodedLine> {
    let mut lines = Vec::new();
    let mut index = 0usize;
    let mut printable = Vec::new();

    while index < chunk.len() {
        let flush_printable =
            |printable: &mut Vec<u8>, carry: &mut String| {
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
                                    0x1b if index + 1 < chunk.len() && chunk[index + 1] == b'\\' => {
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

fn csi_implies_rewrite(sequence: &[u8]) -> bool {
    let Some(final_byte) = sequence.last().copied() else {
        return false;
    };

    matches!(final_byte, b'G' | b'H' | b'f' | b'K' | b'P' | b'X')
}

impl PaintedLineTracker {
    fn ingest(&mut self, chunk: &[u8]) -> Option<String> {
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
                                        0x1b
                                            if index + 1 < chunk.len()
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
    fn ingest(&mut self, line: String) {
        self.ingest_at(line, Instant::now());
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

    fn maybe_emit(&mut self) -> Option<String> {
        self.maybe_emit_at(Instant::now())
    }

    fn maybe_emit_at(&mut self, now: Instant) -> Option<String> {
        let pending = self.pending.clone()?;
        let settled = self
            .last_update_at
            .is_some_and(|last_update_at| now.duration_since(last_update_at) >= PAINT_CONSOLIDATE_SETTLE);
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

fn merge_paint_lines(existing: &str, incoming: &str) -> String {
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

fn push_recent(recent: &mut VecDeque<String>, line: String) {
    recent.push_back(line);
    while recent.len() > MAX_RECENT_LINES {
        recent.pop_front();
    }
}

fn shell_join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| {
            if part.chars().all(|ch| ch.is_ascii_alphanumeric() || "-_./".contains(ch)) {
                part.clone()
            } else {
                format!("{part:?}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn timestamp_now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let day = seconds % 86_400;
    let hours = day / 3_600;
    let minutes = (day % 3_600) / 60;
    let secs = day % 60;
    format!("{hours:02}:{minutes:02}:{secs:02}")
}

#[cfg(test)]
mod tests {
    use super::{
        csi_implies_rewrite, decode_chunk, merge_paint_lines, parse_args, DecodedLine,
        PaintConsolidator, PaintedLineTracker,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn parses_optional_max_seconds() {
        let (duration, trace_control, paint_stream, command) = parse_args(vec![
            "--max-seconds".into(),
            "12".into(),
            "--trace-control".into(),
            "--paint-stream".into(),
            "codex".into(),
            "review".into(),
        ])
        .expect("args should parse");

        assert_eq!(duration.as_secs(), 12);
        assert!(trace_control);
        assert!(paint_stream);
        assert_eq!(command, vec!["codex".to_string(), "review".to_string()]);
    }

    #[test]
    fn defaults_to_codex_review_command() {
        let (_, trace_control, paint_stream, command) =
            parse_args(Vec::new()).expect("default args should parse");
        assert!(!trace_control);
        assert!(!paint_stream);
        assert_eq!(command[0], "codex");
    }

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
