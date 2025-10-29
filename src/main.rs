use std::{
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
use serde::Serialize;
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

#[derive(Serialize)]
struct WaybarOutput {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tooltip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alt: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    class: Vec<String>,
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

    let mut current_session_id = match (&args.session_id, &args.session_file) {
        (Some(id), _) => id.clone(),
        (None, Some(path)) => infer_session_id_from_path(path)
            .context("Failed to infer session id from --session-file")?,
        (None, None) => latest_session_id(&history_path)?
            .context("Could not locate any recent Codex session in history.jsonl")?,
    };

    let mut current_session_path =
        resolve_session_path(&sessions_root, &args.session_file, &current_session_id)?;

    let poll_interval = Duration::from_millis(args.poll_ms);
    let session_refresh_interval = Duration::from_secs(args.session_refresh_secs);

    let mut last_session_refresh = Instant::now() - session_refresh_interval;
    let mut offset: u64 = 0;

    if let Some(payload) = prime_session(
        &current_session_path,
        &mut offset,
        args.max_chars,
        cache_path,
        args.start_at_beginning,
    )? {
        emit_payload(&payload, cache_path)?;
    }

    loop {
        if args.session_file.is_none() && last_session_refresh.elapsed() >= session_refresh_interval
        {
            if let Some(latest_id) = latest_session_id(&history_path)? {
                if latest_id != current_session_id {
                    if let Some(new_path) = locate_session_file(&sessions_root, &latest_id)? {
                        current_session_id = latest_id;
                        current_session_path = new_path;
                        if let Some(payload) = prime_session(
                            &current_session_path,
                            &mut offset,
                            args.max_chars,
                            cache_path,
                            args.start_at_beginning,
                        )? {
                            emit_payload(&payload, cache_path)?;
                        }
                    } else {
                        eprintln!("Session {} announced but file not yet available", latest_id);
                    }
                }
            }
            last_session_refresh = Instant::now();
        }

        match read_new_lines(&current_session_path, &mut offset) {
            Ok(lines) => {
                for line in lines {
                    match process_log_line(&line, args.max_chars) {
                        Ok(Some(payload)) => emit_payload(&payload, cache_path)?,
                        Ok(None) => {}
                        Err(err) => {
                            eprintln!("Failed to process log entry: {err:?}");
                        }
                    }
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                if args.session_file.is_none() {
                    if let Some(new_path) =
                        locate_session_file(&sessions_root, &current_session_id)?
                    {
                        current_session_path = new_path;
                        if let Some(payload) = prime_session(
                            &current_session_path,
                            &mut offset,
                            args.max_chars,
                            cache_path,
                            args.start_at_beginning,
                        )? {
                            emit_payload(&payload, cache_path)?;
                        }
                    }
                }
            }
            Err(err) => {
                eprintln!("Error reading {}: {err}", current_session_path.display());
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

fn resolve_session_path(
    sessions_root: &Path,
    explicit: &Option<PathBuf>,
    session_id: &str,
) -> Result<PathBuf> {
    let path = match explicit {
        Some(path) => path.clone(),
        None => locate_session_file(sessions_root, session_id)?
            .with_context(|| format!("Unable to find log file for session {}", session_id))?,
    };

    eprintln!(
        "Streaming Codex session {} from {}",
        session_id,
        path.display()
    );

    Ok(path)
}

fn infer_session_id_from_path(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split('-').last())
        .and_then(|segment| segment.strip_suffix(".jsonl"))
        .map(|s| s.to_string())
}

fn latest_session_id(history_path: &Path) -> Result<Option<String>> {
    let file = match File::open(history_path) {
        Ok(f) => f,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let reader = BufReader::new(file);
    let mut latest: Option<String> = None;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(val) => {
                if let Some(id) = val.get("session_id").and_then(|v| v.as_str()) {
                    latest = Some(id.to_string());
                }
            }
            Err(err) => {
                eprintln!("Skipping malformed history entry: {err}");
            }
        }
    }
    Ok(latest)
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

fn read_new_lines(path: &Path, offset: &mut u64) -> io::Result<Vec<String>> {
    let file = File::open(path)?;
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
    cache_path: &Path,
    start_at_beginning: bool,
) -> Result<Option<WaybarOutput>> {
    if !path.exists() {
        *offset = 0;
        return Ok(None);
    }

    if start_at_beginning {
        *offset = 0;
    } else {
        *offset = fs::metadata(path)?.len();
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut last_payload: Option<WaybarOutput> = None;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(payload) = process_log_line(&line, max_chars)? {
            last_payload = Some(payload);
        }
    }

    if let Some(payload) = &last_payload {
        write_payload_to_cache(payload, cache_path)?;
    }

    Ok(last_payload)
}

fn process_log_line(line: &str, max_chars: usize) -> Result<Option<WaybarOutput>> {
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

    Ok(Some(WaybarOutput {
        text: truncated,
        tooltip,
        alt: phase,
        class: classes,
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
