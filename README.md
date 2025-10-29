# Codex Waybar Streamer

A small Rust utility that tails the local Codex CLI rollout logs and emits
Waybar-compatible JSON so you can surface Codex's live reasoning directly in
your status bar. The binary watches `~/.codex/history.jsonl` to follow the
active session, streams `agent_reasoning` events in real time, and can now
persist the latest payload to a cache file so Waybar (or any other consumer)
can poll it safely.

## Features

- Auto-discovers the newest Codex session by parsing `history.jsonl`.
- Tails the session log like `tail -F`, gracefully handling rotations.
- Scrubs Markdown emphasis, collapses whitespace, and truncates text for the
  Waybar label while preserving the original reasoning in a tooltip.
- Emits optional phase-based CSS classes (e.g., `phase-inspecting-jsonl-log-format`)
  derived from the bold heading when present.
- Optionally writes the latest payload to a cache file so multiple consumers
  can poll without keeping a stream running.

## Building

```bash
cargo build --release
```

The optimized binary will be written to `target/release/codex-waybar`.

## Installation

Use the bundled script to build and install into `~/.local/bin` (override with `PREFIX`, `BIN_DIR`, or `SHARE_DIR` as needed):

```bash
./scripts/install.sh
systemctl --user daemon-reload
```

### Uninstalling

```bash
./scripts/uninstall.sh
```

The uninstall script removes the binary and documentation from the same locations used during installation.

## Runtime options

Run `codex-waybar --help` for the full set of flags. Key arguments:

| Flag | Description |
| --- | --- |
| `--session-file <path>` | Stream a specific rollout file (skip auto discovery). |
| `--session-id <id>` | Force a session id and auto-resolve its file. |
| `--history-path <path>` | Override the default `~/.codex/history.jsonl`. |
| `--sessions-root <path>` | Override the default `~/.codex/sessions`. |
| `--poll-ms <ms>` | Tail poll interval (default 250 ms). |
| `--session-refresh-secs <s>` | How often to re-check history for a new session (default 5 s). |
| `--max-chars <n>` | Truncate the rendered label to _n_ characters (default 120). |
| `--start-at-beginning` | Replay the entire log on startup (default: tail new entries only). |
| `--cache-file <path>` | Write the most recent payload to a JSON file (overwritten atomically each update). |
| `--print-cache <path>` | Print a cache file once and exit — ideal for Waybar polling. |

## Waybar integration

This project ships in the cache-polling configuration that is proven to work reliably today. `codex-waybar` tails the active Codex session, writes the latest payload atomically to a cache file, and Waybar polls that file once per second.

### 1. Launch the cache writer (systemd user service)

The installer copies a ready-made user unit to `~/.config/systemd/user/codex-waybar.service`.
Enable it once and bind it to the graphical session target:

```bash
systemctl --user daemon-reload
systemctl --user enable --now codex-waybar.service
systemctl --user add-wants graphical-session.target codex-waybar.service
```

Hyprland users should ensure session variables are imported into systemd so the
service inherits `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`, etc. Add the following
to your Hyprland config:

```ini
# ~/.config/hypr/hyprland.conf
exec-once = dbus-update-activation-environment --systemd --all
exec-once = systemctl --user import-environment WAYLAND_DISPLAY XDG_RUNTIME_DIR XDG_SESSION_TYPE
```

Restart the service after changing any CLI flags or upgrading the binary:

```bash
systemctl --user restart codex-waybar.service
systemctl --user status codex-waybar.service
```

### 2. Configure Waybar to poll the cache

See the ready-to-use JSON in `examples/waybar-config-snippet.jsonc`; adjust paths or intervals as needed.

### Styling

`examples/waybar-style.css` includes the shimmer gradient and phase tint from the reference screenshot—copy or tweak it to match your theme.

## Known limitations

- Only `agent_reasoning` payloads are surfaced; other event types are ignored.
- Tooltip text is derived from the original Markdown, so very long reasoning
  strings may be unwieldy. Adjust `--max-chars` if you want longer inline text.

## Roadmap ideas

- Switch to inotify for more efficient tailing on Linux.
- Optional Markdown-to-Pango conversion for richer formatting.
- Configurable class mapping and theme presets.
