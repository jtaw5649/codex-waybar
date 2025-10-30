# Codex Waybar

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

**Remote installer (default behaviour)**

```bash
curl -fsSL https://github.com/jtaw5649/codex-waybar/raw/main/install.sh | bash
```

If a matching release artifact exists, the script fetches the pre-built binary
and Waybar plugin, copies the examples and systemd unit, and performs the same
post-install steps (backup, systemd reload, Waybar restart). Falling back to a
source build happens automatically when no release archive is available. Use
`curl … | bash -s -- --prefix /opt/codex-waybar` (or pass `--no-systemd`) to
customise the installation.

Prerequisites: `curl` and `tar` for the release path. If the script needs to
build from source it will also require `git`, `cargo`, and the GTK/meson build
toolchain. When systemd support is enabled the installer reloads the user
daemon, enables+starts `codex-waybar.service`, and restarts `waybar` so the new
module is active immediately.

Prefer a manual download? Grab the latest tarball from the
[Releases](https://github.com/jtaw5649/codex-waybar/releases) page and unpack
it into your preferred prefix.

Every run also snapshots your existing Waybar configuration into
`$PREFIX/share/codex-waybar/backups/waybar-<timestamp>` (override with
`WAYBAR_BACKUP_ROOT`); the backup path is printed at the end of the install.

**Local checkout**

- `git clone https://github.com/jtaw5649/codex-waybar && cd codex-waybar`
  then run `./install.sh` to build and install from the current workspace.
- `cargo install --path .` installs just the binary into `~/.cargo/bin`; run the
  Meson steps in `cffi/codex_shimmer/` manually if you want the animated module.
- `cargo build --release` followed by copying
  `target/release/codex-waybar`, the example configs, and the systemd unit to
  your preferred locations.

### Uninstalling

```bash
./scripts/uninstall.sh
```

The uninstall script removes the binary and documentation from the same
locations used during installation.

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
| `--waybar-signal <n>` | Send `SIGRTMIN+n` to Waybar after each cache update (match the module’s `signal` field). |
| `--start-at-beginning` | Replay the entire log on startup (default: tail new entries only). |
| `--cache-file <path>` | Write the most recent payload to a JSON file (overwritten atomically each update). |
| `--print-cache <path>` | Print a cache file once and exit — ideal for Waybar polling. |

## Waybar integration

The daemon still tails Codex and writes the latest payload atomically to
`~/.cache/codex-waybar/latest.json`, but the shimmer effect is rendered inside
Waybar by a dedicated GTK CFFI module. The flow is:

1. `codex-waybar` keeps the cache fresh (one JSON object per line).
2. The `wb_codex_shimmer` plugin reads the cache, animates the label with Cairo
   and Pango, and exposes a `GtkDrawingArea` to Waybar.

### 1. Build (or rebuild) the CFFI module

The helper script now compiles the plugin whenever Meson is available, but you
can do it manually as well:

```bash
cd cffi/codex_shimmer
meson setup build --prefix="$HOME/.local"
meson compile -C build
meson install -C build   # installs wb_codex_shimmer.so into ~/.local/lib/waybar
```

Re-run the `meson compile` and `meson install` steps whenever you change the C
source so Waybar picks up the latest shared object.

### 2. Configure Waybar to load the plugin

Switch the old `custom/codex` block to the CFFI module and wire the Waybar RT
signal. A ready-to-use snippet lives in
[`examples/waybar-config-snippet.jsonc`](examples/waybar-config-snippet.jsonc);
copy it into your `~/.config/waybar/config.jsonc` and adjust paths or tunables
as needed. The module exposes animation parameters such as `period_ms`,
`pause_ms`, `width_chars`, `cycles`, `tick_ms`, and the highlight colours.
`base_alpha` controls the resting text opacity, while `highlight_alpha`
controls how translucent the shimmer sweep is.

### 3. Launch or restart the cache writer

The installer drops a ready-made user unit in
`~/.config/systemd/user/codex-waybar.service`:

```bash
systemctl --user daemon-reload
systemctl --user enable --now codex-waybar.service
systemctl --user add-wants graphical-session.target codex-waybar.service
```

Hyprland users should still export session variables into systemd:

```ini
# ~/.config/hypr/hyprland.conf
exec-once = dbus-update-activation-environment --systemd --all
exec-once = systemctl --user import-environment WAYLAND_DISPLAY XDG_RUNTIME_DIR XDG_SESSION_TYPE
```

Every time you change CLI flags or rebuild the plugin:

```bash
systemctl --user restart codex-waybar.service
systemctl --user status codex-waybar.service
```

Finally, set the Waybar module’s `signal` to match the daemon’s
`--waybar-signal` flag (default `15`) so each cache refresh triggers an immediate
redraw.

### Styling

[`examples/waybar-style.css`](examples/waybar-style.css) keeps the module
transparent so the GTK renderer can drive colours directly; copy or tweak it to
match your theme.

> **Note:** The Rust daemon now only writes plain-text payloads. The GTK
> `wb_codex_shimmer` module is required for the animated presentation—there is
> no built-in markup fallback.

## Known limitations

- Only `agent_reasoning` payloads are surfaced; other event types are ignored.
- Tooltip text is derived from the original Markdown, so very long reasoning
  strings may be unwieldy. Adjust `--max-chars` if you want longer inline text.

## Roadmap ideas

- Switch to inotify for more efficient tailing on Linux.
- Optional Markdown-to-Pango conversion for richer formatting.
- Configurable class mapping and theme presets.
