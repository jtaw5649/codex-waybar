use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    fs::{self, File},
    io::{self, BufRead, BufReader, ErrorKind, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use clap::Parser;
use dirs::home_dir;
use glob::glob;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Parser, Debug)]
#[command(author, version, about = "Publish Codex reasoning updates for Waybar")]
struct Args {
    /// Print the contents of a cache file once and exit
    #[arg(long)]
    print_cache: Option<PathBuf>,

    /// Explicit session log file to read (skip auto-discovery)
    #[arg(long)]
    session_file: Option<PathBuf>,

    /// Explicit Codex session id (falls back to latest seen in history)
    #[arg(long)]
    session_id: Option<String>,

    /// Path to Codex history.jsonl (defaults to ~/.codex/history.jsonl)
    #[arg(long)]
    history_path: Option<PathBuf>,

    /// Root of Codex sessions directory (defaults to ~/.codex/sessions)
    #[arg(long)]
    sessions_root: Option<PathBuf>,

    /// Poll interval in milliseconds while tailing
    #[arg(long, default_value_t = 250)]
    poll_ms: u64,

    /// Re-check history for a fresher session every N seconds
    #[arg(long, default_value_t = 5)]
    session_refresh_secs: u64,

    /// Track up to N recent Codex sessions concurrently
    #[arg(long, default_value_t = 4)]
    session_window: usize,

    /// Maximum characters to emit for the Waybar label
    #[arg(long, default_value_t = 120)]
    max_chars: usize,

    /// Write the most recent payload to the specified cache file
    #[arg(long)]
    cache_file: Option<PathBuf>,

    /// Replay the entire log from the beginning instead of tailing new entries
    #[arg(long)]
    start_at_beginning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WaybarOutput {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tooltip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alt: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    class: Vec<String>,
}

#[derive(Debug, Clone)]
struct RenderedEvent {
    payload: WaybarOutput,
    timestamp: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionEvent {
    session_id: String,
    event: RenderedEvent,
}

#[derive(Debug)]
struct SessionState {
    path: PathBuf,
    offset: u64,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if let Some(cache_path) = &args.print_cache {
        return print_cache(cache_path);
    }

    let cache_path = args
        .cache_file
        .as_deref()
        .context("--cache-file is required unless --print-cache is used")?;

    let history_path = args
        .history_path
        .unwrap_or(default_history_path().context("Unable to determine default history path")?);
    let sessions_root = args
        .sessions_root
        .unwrap_or(default_sessions_root().context("Unable to determine default sessions path")?);

    let poll_interval = Duration::from_millis(args.poll_ms);
    let session_refresh_interval = Duration::from_secs(args.session_refresh_secs);
    let mut last_session_refresh = Instant::now() - session_refresh_interval;

    let auto_discover = args.session_file.is_none() && args.session_id.is_none();
    let mut tracked_sessions: Vec<String> = if auto_discover {
        recent_session_ids(&history_path, args.session_window)?
    } else {
        match (&args.session_id, &args.session_file) {
            (Some(id), _) => vec![id.clone()],
            (None, Some(path)) => vec![
                infer_session_id_from_path(path)
                    .context("Failed to infer session id from --session-file")?,
            ],
            (None, None) => Vec::new(),
        }
    };

    let mut explicit_paths: HashMap<String, PathBuf> = HashMap::new();
    if let Some(path) = &args.session_file {
        if let Some(session_id) = tracked_sessions.get(0) {
            explicit_paths.insert(session_id.clone(), path.clone());
        }
    }

    let mut session_states: HashMap<String, SessionState> = HashMap::new();
    let mut last_emitted: Option<SessionEvent> = None;

    bootstrap_sessions(
        &mut session_states,
        &mut last_emitted,
        &tracked_sessions,
        &explicit_paths,
        &sessions_root,
        args.max_chars,
        args.start_at_beginning,
        cache_path,
    )?;

    loop {
        if auto_discover && last_session_refresh.elapsed() >= session_refresh_interval {
            tracked_sessions = recent_session_ids(&history_path, args.session_window)?;
            last_session_refresh = Instant::now();
        }

        if tracked_sessions.is_empty() {
            thread::sleep(poll_interval);
            continue;
        }

        session_states.retain(|id, _| tracked_sessions.contains(id));

        let mut newest_event: Option<SessionEvent> = None;

        for session_id in &tracked_sessions {
            match session_states.entry(session_id.clone()) {
                Entry::Vacant(entry) => {
                    let explicit = explicit_paths.get(session_id);
                    if let Some((state, initial_event)) = initialize_session_state(
                        session_id,
                        explicit,
                        &sessions_root,
                        args.max_chars,
                        args.start_at_beginning,
                    )? {
                        if let Some(event) = initial_event {
                            newest_event = select_newer_event(
                                newest_event,
                                SessionEvent {
                                    session_id: session_id.clone(),
                                    event,
                                },
                            );
                        }
                        entry.insert(state);
                    }
                }
                Entry::Occupied(mut entry) => {
                    let mut reinitialize = false;
                    {
                        let state = entry.get_mut();
                        match read_new_lines(&state.path, &mut state.offset) {
                            Ok(lines) => {
                                for line in lines {
                                    match process_log_line(&line, args.max_chars) {
                                        Ok(Some(event)) => {
                                            newest_event = select_newer_event(
                                                newest_event,
                                                SessionEvent {
                                                    session_id: session_id.clone(),
                                                    event,
                                                },
                                            );
                                        }
                                        Ok(None) => {}
                                        Err(err) => {
                                            eprintln!("Failed to process log entry: {err:?}");
                                        }
                                    }
                                }
                            }
                            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                                reinitialize = true;
                            }
                            Err(err) => {
                                eprintln!("Error reading {}: {err}", state.path.display());
                            }
                        }
                    }

                    if reinitialize {
                        let explicit = explicit_paths.get(session_id);
                        match initialize_session_state(
                            session_id,
                            explicit,
                            &sessions_root,
                            args.max_chars,
                            args.start_at_beginning,
                        )? {
                            Some((state, initial_event)) => {
                                if let Some(event) = initial_event {
                                    newest_event = select_newer_event(
                                        newest_event,
                                        SessionEvent {
                                            session_id: session_id.clone(),
                                            event,
                                        },
                                    );
                                }
                                entry.insert(state);
                            }
                            None => {
                                entry.remove();
                            }
                        }
                    }
                }
            }
        }

        if let Some(event) = newest_event {
            if should_emit(&last_emitted, &event) {
                emit_payload(&event.event.payload, cache_path)?;
                last_emitted = Some(event);
            }
        }

        thread::sleep(poll_interval);
    }
}

fn default_history_path() -> Result<PathBuf> {
    let mut path = home_dir().context("Home directory not found")?;
    path.push(".codex");
    path.push("history.jsonl");
    Ok(path)
}

fn default_sessions_root() -> Result<PathBuf> {
    let mut path = home_dir().context("Home directory not found")?;
    path.push(".codex");
    path.push("sessions");
    Ok(path)
}

fn infer_session_id_from_path(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split('-').last())
        .and_then(|segment| segment.strip_suffix(".jsonl"))
        .map(|s| s.to_string())
}

fn locate_session_file(root: &Path, session_id: &str) -> Result<Option<PathBuf>> {
    let pattern = format!("{}/**/*{}*.jsonl", root.display(), session_id);
    let mut newest_path: Option<PathBuf> = None;
    let mut newest_mtime: Option<std::time::SystemTime> = None;

    for entry in glob(&pattern)? {
        if let Ok(path) = entry {
            if let Ok(metadata) = fs::metadata(&path) {
                let mtime = metadata
                    .modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                if newest_mtime.map_or(true, |current| mtime > current) {
                    newest_mtime = Some(mtime);
                    newest_path = Some(path);
                }
            }
        }
    }

    Ok(newest_path)
}

fn recent_session_ids(history_path: &Path, limit: usize) -> Result<Vec<String>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let file = match File::open(history_path) {
        Ok(f) => f,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };

    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    for line in reader.lines() {
        match line {
            Ok(line) => lines.push(line),
            Err(err) => {
                eprintln!("Skipping malformed history entry: {err}");
            }
        }
    }

    let mut seen = HashSet::new();
    let mut ordered = Vec::new();

    for line in lines.iter().rev() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<Value>(line) {
            if let Some(id) = val.get("session_id").and_then(|v| v.as_str()) {
                if seen.insert(id.to_string()) {
                    ordered.push(id.to_string());
                    if ordered.len() == limit {
                        break;
                    }
                }
            }
        }
    }

    ordered.reverse();
    Ok(ordered)
}

fn initialize_session_state(
    session_id: &str,
    explicit_path: Option<&PathBuf>,
    sessions_root: &Path,
    max_chars: usize,
    start_at_beginning: bool,
) -> Result<Option<(SessionState, Option<RenderedEvent>)>> {
    let path = match explicit_path {
        Some(path) => path.clone(),
        None => match locate_session_file(sessions_root, session_id)? {
            Some(path) => path,
            None => return Ok(None),
        },
    };

    let mut offset = 0;
    let event = prime_session(&path, &mut offset, max_chars, start_at_beginning)?;
    Ok(Some((SessionState { path, offset }, event)))
}

fn bootstrap_sessions(
    session_states: &mut HashMap<String, SessionState>,
    last_emitted: &mut Option<SessionEvent>,
    tracked_sessions: &[String],
    explicit_paths: &HashMap<String, PathBuf>,
    sessions_root: &Path,
    max_chars: usize,
    start_at_beginning: bool,
    cache_path: &Path,
) -> Result<()> {
    let mut newest_event: Option<SessionEvent> = None;

    for session_id in tracked_sessions {
        if session_states.contains_key(session_id) {
            continue;
        }
        let explicit = explicit_paths.get(session_id);
        if let Some((state, initial_event)) = initialize_session_state(
            session_id,
            explicit,
            sessions_root,
            max_chars,
            start_at_beginning,
        )? {
            if let Some(event) = initial_event {
                newest_event = select_newer_event(
                    newest_event,
                    SessionEvent {
                        session_id: session_id.clone(),
                        event,
                    },
                );
            }
            session_states.insert(session_id.clone(), state);
        }
    }

    if let Some(event) = newest_event {
        emit_payload(&event.event.payload, cache_path)?;
        *last_emitted = Some(event);
    }

    Ok(())
}

fn select_newer_event(
    current: Option<SessionEvent>,
    candidate: SessionEvent,
) -> Option<SessionEvent> {
    match current {
        None => Some(candidate),
        Some(existing) => {
            if is_newer_timestamp(
                candidate.event.timestamp.as_ref(),
                existing.event.timestamp.as_ref(),
            ) || (candidate.event.timestamp == existing.event.timestamp
                && candidate.event.payload != existing.event.payload)
            {
                Some(candidate)
            } else {
                Some(existing)
            }
        }
    }
}

fn is_newer_timestamp(candidate: Option<&String>, current: Option<&String>) -> bool {
    match (candidate, current) {
        (Some(candidate), Some(current)) => candidate > current,
        (Some(_), None) => true,
        _ => false,
    }
}

fn should_emit(last_emitted: &Option<SessionEvent>, candidate: &SessionEvent) -> bool {
    match last_emitted {
        None => true,
        Some(previous) => {
            previous.session_id != candidate.session_id
                || previous.event.timestamp != candidate.event.timestamp
                || previous.event.payload != candidate.event.payload
        }
    }
}

fn read_new_lines(path: &Path, offset: &mut u64) -> io::Result<Vec<String>> {
    let file = File::open(path)?;
    let file_len = file.metadata()?.len();
    if *offset > file_len {
        *offset = 0;
    }

    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(*offset))?;

    let mut lines = Vec::new();
    loop {
        let mut buffer = String::new();
        let bytes = reader.read_line(&mut buffer)?;
        if bytes == 0 {
            break;
        }
        *offset += bytes as u64;
        if let Some(stripped) = buffer.strip_suffix('\n') {
            lines.push(stripped.to_string());
        } else {
            lines.push(buffer);
        }
    }
    Ok(lines)
}

fn prime_session(
    path: &Path,
    offset: &mut u64,
    max_chars: usize,
    start_at_beginning: bool,
) -> Result<Option<RenderedEvent>> {
    let metadata = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            *offset = 0;
            return Ok(None);
        }
        Err(err) => return Err(err.into()),
    };

    if start_at_beginning {
        *offset = 0;
    } else {
        *offset = metadata.len();
    }

    let file = match File::open(path) {
        Ok(f) => f,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            *offset = 0;
            return Ok(None);
        }
        Err(err) => return Err(err.into()),
    };
    let reader = BufReader::new(file);
    let mut last_event: Option<RenderedEvent> = None;
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                *offset = 0;
                return Ok(None);
            }
            Err(err) => return Err(err.into()),
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(event) = process_log_line(&line, max_chars)? {
            last_event = Some(event);
        }
    }

    *offset = metadata.len();

    Ok(last_event)
}

fn process_log_line(line: &str, max_chars: usize) -> Result<Option<RenderedEvent>> {
    if line.trim().is_empty() {
        return Ok(None);
    }

    let value: Value = match serde_json::from_str(line) {
        Ok(val) => val,
        Err(err) => {
            eprintln!("Skipping malformed log entry: {err}");
            return Ok(None);
        }
    };

    let payload = match value.get("payload") {
        Some(payload) => payload,
        None => return Ok(None),
    };

    if payload
        .get("type")
        .and_then(Value::as_str)
        .map(|t| t != "agent_reasoning")
        .unwrap_or(true)
    {
        return Ok(None);
    }

    let raw_text = payload
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();

    if raw_text.is_empty() {
        return Ok(None);
    }

    let sanitized = sanitize_text(raw_text);
    let truncated = truncate_text(&sanitized, max_chars);
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let phase = extract_phase(raw_text);

    let mut classes = vec!["codex".to_string(), "agent-reasoning".to_string()];
    if let Some(ref label) = phase {
        if let Some(slug) = slugify(label) {
            classes.push(format!("phase-{}", slug));
        }
    }

    let tooltip = build_tooltip(timestamp.as_deref(), raw_text, &sanitized, &truncated);
    let display_text = phase.clone().unwrap_or_else(|| truncated.clone());

    Ok(Some(RenderedEvent {
        payload: WaybarOutput {
            text: display_text,
            tooltip,
            alt: phase,
            class: classes,
        },
        timestamp,
    }))
}

fn sanitize_text(input: &str) -> String {
    let mut text = input.replace('\n', " ").replace('\r', " ");
    text = text.replace("**", "");
    collapse_whitespace(&text)
}

fn collapse_whitespace(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_space = false;
    for ch in input.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim().to_string()
}

fn truncate_text(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_string();
    }
    let mut truncated = String::new();
    for ch in text.chars() {
        let next_len = truncated.len() + ch.len_utf8();
        if next_len > max_len {
            truncated = truncated.trim_end().to_owned();
            truncated.push('…');
            return truncated;
        }
        truncated.push(ch);
    }
    truncated
}

fn extract_phase(raw: &str) -> Option<String> {
    if let Some(stripped) = raw.strip_prefix("**") {
        if let Some(end) = stripped.find("**") {
            return Some(stripped[..end].trim().to_string());
        }
    }
    None
}

fn slugify(input: &str) -> Option<String> {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if ch.is_whitespace() || ch == '-' || ch == '_' {
            if !slug.is_empty() && !last_dash {
                slug.push('-');
                last_dash = true;
            }
        } else if !slug.is_empty() && !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() { None } else { Some(slug) }
}

fn build_tooltip(
    timestamp: Option<&str>,
    raw_text: &str,
    sanitized: &str,
    truncated: &str,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(ts) = timestamp {
        parts.push(ts.to_string());
    }
    let raw_trimmed = raw_text.trim();
    if !raw_trimmed.is_empty() && raw_trimmed != sanitized {
        parts.push(raw_trimmed.to_string());
    } else if sanitized != truncated {
        parts.push(sanitized.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn emit_payload(payload: &WaybarOutput, cache_path: &Path) -> Result<()> {
    write_payload_to_cache(payload, cache_path)?;
    Ok(())
}

fn write_payload_to_cache(payload: &WaybarOutput, cache_path: &Path) -> Result<()> {
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let temp_path = cache_path.with_extension("tmp");
    {
        let mut file = File::create(&temp_path)?;
        serde_json::to_writer(&mut file, payload)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    fs::rename(&temp_path, cache_path)?;

    Ok(())
}

fn print_cache(path: &Path) -> Result<()> {
    match fs::read_to_string(path) {
        Ok(content) => {
            print!("{}", content);
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let payload = WaybarOutput {
                text: "Waiting for Codex…".to_string(),
                tooltip: None,
                alt: Some("initializing".to_string()),
                class: vec!["codex".to_owned(), "agent-reasoning".to_owned()],
            };
            println!("{}", serde_json::to_string(&payload)?);
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    }

    io::stdout().flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::io::Write;
    use tempfile::{NamedTempFile, tempdir};

    #[test]
    fn prime_session_returns_none_when_file_missing() -> Result<()> {
        let dir = tempdir()?;
        let session_path = dir.path().join("missing-session.jsonl");
        let mut offset = 42;

        let result = prime_session(&session_path, &mut offset, 120, false)?;

        assert!(result.is_none());
        assert_eq!(offset, 0);
        Ok(())
    }

    #[test]
    fn prime_session_reads_last_payload() -> Result<()> {
        let dir = tempdir()?;
        let session_path = dir.path().join("session.jsonl");
        let mut file = File::create(&session_path)?;
        let payload_one = json!({
            "timestamp": "2025-10-29T12:00:00Z",
            "type": "event_msg",
            "payload": { "type": "agent_reasoning", "text": "**First step** details" }
        });
        let payload_two = json!({
            "timestamp": "2025-10-29T12:01:00Z",
            "type": "event_msg",
            "payload": { "type": "agent_reasoning", "text": "Second step" }
        });
        writeln!(file, "{payload_one}")?;
        writeln!(file, "{payload_two}")?;

        let mut offset = 0;
        let result = prime_session(&session_path, &mut offset, 120, false)?;

        assert!(result.is_some());
        let event = result.unwrap();
        assert_eq!(event.payload.text, "Second step");
        assert_eq!(event.timestamp.as_deref(), Some("2025-10-29T12:01:00Z"));
        Ok(())
    }

    #[test]
    fn read_new_lines_resets_offset_when_file_shrinks() -> Result<()> {
        let temp = NamedTempFile::new()?;
        fs::write(temp.path(), "line1\nline2\n")?;
        let mut offset = fs::metadata(temp.path())?.len();

        fs::write(temp.path(), "line3\n")?;

        let lines = read_new_lines(temp.path(), &mut offset)?;
        assert_eq!(lines, vec!["line3".to_string()]);
        assert_eq!(offset, fs::metadata(temp.path())?.len());
        Ok(())
    }

    #[test]
    fn recent_session_ids_returns_unique_sessions_in_order() -> Result<()> {
        let dir = tempdir()?;
        let history_path = dir.path().join("history.jsonl");
        fs::write(
            &history_path,
            r#"
{"session_id":"alpha"}
{"session_id":"beta"}
{"session_id":"alpha"}
{"session_id":"gamma"}
"#
            .trim_start(),
        )?;

        let ids = recent_session_ids(&history_path, 2)?;
        assert_eq!(ids, vec!["alpha".to_string(), "gamma".to_string()]);

        let ids_three = recent_session_ids(&history_path, 3)?;
        assert_eq!(
            ids_three,
            vec!["beta".to_string(), "alpha".to_string(), "gamma".to_string()]
        );
        Ok(())
    }

    #[test]
    fn select_newer_event_prefers_newer_timestamp() {
        let older = SessionEvent {
            session_id: "alpha".to_string(),
            event: RenderedEvent {
                payload: WaybarOutput {
                    text: "Old".to_string(),
                    tooltip: None,
                    alt: None,
                    class: vec![],
                },
                timestamp: Some("2025-10-29T10:00:00Z".to_string()),
            },
        };
        let newer = SessionEvent {
            session_id: "beta".to_string(),
            event: RenderedEvent {
                payload: WaybarOutput {
                    text: "New".to_string(),
                    tooltip: None,
                    alt: None,
                    class: vec![],
                },
                timestamp: Some("2025-10-29T11:00:00Z".to_string()),
            },
        };

        let picked = select_newer_event(Some(older.clone()), newer.clone()).unwrap();
        assert_eq!(picked.session_id, "beta");

        let unchanged = select_newer_event(Some(newer.clone()), older.clone()).unwrap();
        assert_eq!(unchanged.session_id, "beta");
    }
}
