# Codex Waybar

A small Rust utility that tails the local Codex CLI rollout logs and emits
Waybar-compatible JSON so you can surface Codex's live reasoning directly in
your status bar. The binary watches `~/.codex/history.jsonl` to follow the
active session, streams `agent_reasoning` events in real time, and can now
persist the latest payload to a cache file so Waybar (or any other consumer)
can poll it safely.

> **Rename notice:** the repository remains **codex-waybar**, but the installed
> daemon and Waybar module ship as **codex-shimmer** to avoid conflicts with
> `pkill waybar`. Legacy `CODEX_WAYBAR_*` environment variables still work; new
> configuration uses the `codex-shimmer` name for binaries, cache paths, and
> units.

## Features

- Auto-discovers the newest Codex session by parsing `history.jsonl`.
- Tails the session log like `tail -F`, gracefully handling rotations.
- Scrubs Markdown emphasis, collapses whitespace, and truncates text for the
  Waybar label while preserving the original reasoning in a tooltip.
- Emits optional phase-based CSS classes (e.g., `phase-inspecting-jsonl-log-format`)
  derived from the bold heading when present.
- Optionally writes the latest payload to a cache file so multiple consumers
  can poll without keeping a stream running.

## Installation

### Remote installer (recommended)

```bash
curl -fsSL https://github.com/jtaw5649/codex-waybar/raw/master/install.sh | bash
```

The script first searches for a release asset that matches your architecture.
When one exists, it downloads the pre-built `codex-shimmer` binary together with
the GTK shimmer plugin, installs the examples and systemd unit, and then runs
the usual post-install steps (Waybar backup, systemd reload, Waybar restart). If
no release archive is available it automatically clones the repository and
builds everything from source instead. Pass `--prefix /opt/codex-shimmer`
(or `--no-systemd`) via `curl … | bash -s -- <flags>` to customise the
deployment.

Prerequisites: `curl` and `tar` for the release path. When the script needs to
compile from source you will also need `git`, `cargo`, `meson`, and the GTK
development headers. Every run snapshots your existing Waybar configuration and
prints the backup location that was created.

#### Apply the example Waybar configuration

The installer never overwrites your Waybar files. Merge the samples it installs
under `$PREFIX/share/codex-shimmer/`:

1. Copy or merge `$PREFIX/share/codex-shimmer/examples/waybar-config-snippet.jsonc`
   into `~/.config/waybar/config.jsonc` so Waybar loads the
   `cffi/codex_shimmer` module (adjust paths if you installed to a non-default
   prefix).
2. Append the contents of `$PREFIX/share/codex-shimmer/examples/waybar-style.css`
   to your Waybar stylesheet; the GTK renderer relies on those rules to keep the
   shimmer transparent and aligned.
3. If you skipped the automatic systemd setup, copy
   `$PREFIX/share/codex-shimmer/systemd/codex-shimmer.service` into
   `~/.config/systemd/user/` and enable it manually.

### Manual release download

Prefer not to pipe to `bash`? Grab the latest tarball from the
[Releases](https://github.com/jtaw5649/codex-waybar/releases) page, extract it
into your preferred prefix, and then follow the steps above to merge the config,
CSS, and systemd unit.

### Local checkout (development)

Clone the repository when you want to hack on the sources:

```bash
git clone https://github.com/jtaw5649/codex-waybar
cd codex-waybar
```

#### Use the install script

Running `./install.sh` inside the checkout performs the same workflow as the
remote installer but uses your local sources. It honours `--prefix` and the
`CODEX_SHIMMER_*` environment overrides (legacy `CODEX_WAYBAR_*` names remain
supported for compatibility), rebuilds the Rust daemon and CFFI
module, backs up your Waybar directory, and restarts Waybar by default. After it
finishes, follow [Apply the example Waybar configuration](#apply-the-example-waybar-configuration)
to merge the sample config and CSS.

#### Manual build (advanced)

Install just the binary into your Cargo bin directory:

```bash
cargo install --path .
```

Or build the optimized binary in place without installing:

```bash
cargo build --release
```

The resulting executable lives at `target/release/codex-shimmer`. To (re)build
and install the GTK shimmer plugin manually:

```bash
cd cffi/codex_shimmer
meson setup build --prefix="$HOME/.local"
meson compile -C build
meson install -C build
```

When you install manually, copy the files under `examples/` into your Waybar
configuration and drop `systemd/codex-shimmer.service` into
`~/.config/systemd/user/` if you want systemd to manage the daemon.

### Uninstalling

```bash
./scripts/uninstall.sh
```

The uninstall script removes the binary and documentation from the same
locations used during installation.

## Runtime options

Run `codex-shimmer --help` for the full set of flags. Key arguments:

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
`~/.cache/codex-shimmer/latest.json`, but the shimmer effect is rendered inside
Waybar by a dedicated GTK CFFI module. The flow is:

1. `codex-shimmer` keeps the cache fresh (one JSON object per line).
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
controls the strength of the optional highlight. Two additional knobs tune the
transparency effect itself:

- `mask_scale` (default `3.0`) multiplies the Gaussian sweep when we punch a
  hole through the glyphs. Increase it to reveal more of the bar background;
  decrease it if you prefer a subtler cut-out.
- `overlay_scale` (default `0.0`) re-enables the bright screen-blend overlay. A
  value of zero leaves the shimmer fully transparent; try `0.3`–`0.6` for a
  soft glow.

When `base_color` is omitted the plugin inherits Waybar’s foreground colour, so
your theme continues to drive the resting text tone while the mask/overlay
settings dictate the shimmer.

### 3. Launch or restart the cache writer

The installer drops a ready-made user unit in
`~/.config/systemd/user/codex-shimmer.service`:

```bash
systemctl --user daemon-reload
systemctl --user enable --now codex-shimmer.service
systemctl --user add-wants graphical-session.target codex-shimmer.service
```

Hyprland users should still export session variables into systemd:

```ini
# ~/.config/hypr/hyprland.conf
exec-once = dbus-update-activation-environment --systemd --all
exec-once = systemctl --user import-environment WAYLAND_DISPLAY XDG_RUNTIME_DIR XDG_SESSION_TYPE
```

Every time you change CLI flags or rebuild the plugin:

```bash
systemctl --user restart codex-shimmer.service
systemctl --user status codex-shimmer.service
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
