# Claude Usage Bar (Rust, cross-platform) - Design

**Date:** 2026-05-21
**Status:** Approved (brainstorming)

> **Superseded in part (2026-06-01):** the data source is no longer the OAuth
> token + `api/oauth/usage` endpoint and is now the parsing of the local JSONL
> logs (`~/.claude/projects/`), due to a violation of Anthropic's Usage Policy.
> See `docs/superpowers/adr-2026-06-01-local-logs-over-oauth-endpoint.md`.

## Objective

Rewrite the Claude Usage Bar in Rust as a single cross-platform codebase,
macOS, Linux, and Windows, retiring the macOS-only Swift app
(`~/dev/claude-usage-bar/`, which stays intact, paused).

A tray app (system tray / menu bar) that shows the consumption of the Claude
plan (Pro/Max) against the subscription limits: the 5h window and the weekly
limit.

## Prerequisites

- Rust toolchain (`rustup` / `cargo`) installed on every machine where it will
  be compiled. It is not installed on the current macOS machine, install it via
  `rustup`.
- Linux: GTK development libraries (the backend for `tray-icon`/`tao`).

## Data source

Undocumented endpoint, the same one used by Claude Code's `/usage`:

```
GET https://api.anthropic.com/api/oauth/usage
Authorization: Bearer <oauth-access-token>
anthropic-beta: oauth-2025-04-20
```

Response (HTTP 200), format confirmed on 2026-05-21:

```json
{
  "five_hour":        { "utilization": 17, "resets_at": "2026-05-21T20:50:00.508668+00:00" },
  "seven_day":        { "utilization": 18, "resets_at": "2026-05-24T07:00:00.508691+00:00" },
  "seven_day_opus":   null,
  "seven_day_sonnet": { "utilization": 5,  "resets_at": "2026-05-24T07:00:00.508699+00:00" },
  "extra_usage":      { "is_enabled": true, "used_credits": 82, "currency": "BRL",
                        "monthly_limit": null, "utilization": null, "disabled_reason": null }
}
```

Extra fields (`seven_day_cowork`, `tangelo`, etc.) are ignored.

## Stack

- `tao`: cross-platform event loop. On Linux it is based on GTK, which is what
  `tray-icon` needs. It is the event loop that Tauri uses.
- `tray-icon`: tray icon + native menu. Tauri's tray crate.
- `ureq`: blocking HTTP client, small, no async runtime.
- `serde` + `serde_json`: JSON.
- `#[cfg]`-gated: macOS needs no extra crate (shell out to `security`);
  Windows may need the `windows` crate (see "Risk: token on Windows").

Native compilation on each platform (`cargo build --release`). No
cross-compilation, tray apps cross-compile poorly.

## Known risks

### Token on Windows (viability gate)

Where Claude Code stores the OAuth token varies by OS:

- **macOS**: Keychain, item `Claude Code-credentials` (confirmed).
- **Linux**: probably `~/.claude/.credentials.json` in plain text.
- **Windows**: uncertain, it could be `%USERPROFILE%\.claude\.credentials.json`
  or the Credential Manager / DPAPI.

The first task of the implementation plan verifies this on real Linux and
Windows machines. If on Windows it is DPAPI, `token/windows.rs` uses the
`windows` crate to decrypt it. The risk is isolated in a single file; the rest
of the project does not depend on the answer.

### Undocumented endpoint

`/api/oauth/usage` may change format without notice. Decode failure leads to an
`⚠ fmt` error state, no crash.

### The tray on macOS requires a bundle

A raw binary does not reliably present an `NSStatusItem` (lesson from the Swift
app). The macOS build packages the binary into a `.app` (`Info.plist` with
`LSUIElement`). Windows and Linux run the plain executable.

### The tray on Linux is fragmented

Modern GNOME has no tray by default, it requires the AppIndicator extension.
KDE/XFCE have it. Solving this is out of scope; document it in the README.

## Architecture

Project: `~/dev/claude-usage-bar-rs/`, a Cargo project.

```
src/
  main.rs        - CLI dispatch (--once/--selftest/--install/--uninstall) or run the tray
  error.rs       - WidgetError enum (4 variants)
  usage.rs       - Usage/Window/ExtraUsage structs + serde, decode
  client.rs      - fetch_usage(): GET on the endpoint, Result<Usage, WidgetError>
  render.rs      - pure: colour by level, format reset, ASCII bar, strings, generate icon RGBA
  tray.rs        - TrayApp: owns the TrayIcon + menu, event loop, polling thread
  autostart.rs   - install/remove auto-start per OS (#[cfg]-gated)
  token/
    mod.rs       - fetch_token() dispatched by #[cfg]
    macos.rs     - Keychain via /usr/bin/security
    linux.rs     - ~/.claude/.credentials.json
    windows.rs   - %USERPROFILE%\.claude\.credentials.json (provisional - see risk)
```

Only `token/*` and `autostart.rs` have OS-specific code. `usage`, `client`,
`render`, `error` are fully portable. `tray` is portable (`tao`/`tray-icon`
abstract away the platforms).

### Units

| Unit | What it does | Depends on |
|---|---|---|
| `error::WidgetError` | enum: `TokenNotFound`, `TokenMalformed`, `Auth`, `Network(String)`, `Format` | - |
| `usage` | structs + `decode_usage(&[u8]) -> Result<Usage, WidgetError>` | `serde` |
| `token::fetch_token` | gets the OAuth token from the current OS; read fresh on every poll, never cached | `#[cfg]` impls |
| `client::fetch_usage` | `fetch_token()` → GET → `decode_usage` → `Usage` | `token`, `usage`, `ureq` |
| `render` | pure functions: colour level, time formatting, RGBA icon generation, menu/tooltip/title strings | `usage` |
| `tray::TrayApp` | owns the `TrayIcon` + menu; `tao` event loop; receives polling results and updates the tray | `tray-icon`, `tao`, `render` |
| `autostart` | installs/removes auto-start for the current OS | `#[cfg]` |

### Data flow

```
polling thread: fetch_token() → fetch_usage() → Usage
   → mpsc channel → event loop thread (tao user-event)
   → render → TrayIcon (icon + tooltip + title + menu)
```

## Token per platform

`token::fetch_token() -> Result<String, WidgetError>`, dispatched by
`#[cfg(target_os)]`. Read fresh on every poll.

- **macOS**: `/usr/bin/security find-generic-password -s "Claude Code-credentials" -w`,
  parses the JSON, extracts `claudeAiOauth.accessToken`.
- **Linux**: reads `~/.claude/.credentials.json`, parses it, extracts
  `claudeAiOauth.accessToken`.
- **Windows**: provisional, reads `%USERPROFILE%\.claude\.credentials.json` the
  same way. Subject to verification (see "Risk: token on Windows").

Errors: missing file/Keychain leads to `TokenNotFound`; invalid JSON or no
`accessToken` leads to `TokenMalformed`.

## HTTP client

`client::fetch_usage() -> Result<Usage, WidgetError>`:

1. `token::fetch_token()`.
2. GET on the endpoint via `ureq`, headers `Authorization: Bearer <token>` and
   `anthropic-beta: oauth-2025-04-20`, 15s timeout.
3. Mapping: transport error leads to `Network(String)`; HTTP 401 leads to
   `Auth`; non-200 leads to `Network`; decode failure leads to `Format`.
4. HTTP 200 leads to `decode_usage` leads to `Usage`.

## Tray UI

### Icon

Generated at runtime as RGBA (no asset files). A rounded square filled with the
**level colour**, defined by the fullest window between 5h and 7d:

| Maximum utilization | Colour |
|---|---|
| `< 50%` | green |
| `50–80%` | orange |
| `≥ 80%` | red |

Error state leads to a grey icon. Regenerated when the level/state changes.
`tray_icon::Icon::from_rgba`.

### Title (macOS only)

`set_title("5h 17% · 7d 18%")`: plain text. On Windows/Linux `set_title` does
not apply; there the coloured icon is the at-a-glance signal. In an error state
the title shows the warning (`⚠ token`, etc.).

### Tooltip (all platforms)

`"Claude — 5h 17% · 7d 18%"` on hover. It is the exact "at-a-glance" on
Windows/Linux.

### Menu (native dropdown, all platforms)

```
5h window        17%   ▓▓░░░░░░░░
resets in 3h 12m  ·  20:50
─────────────────────────────────
Weekly (7d)      18%   ▓▓░░░░░░░░
resets in 2d 15h  ·  Sat 07:00
─────────────────────────────────
Weekly · Sonnet   5%
Weekly · Opus     -
Extra usage      82 credits (BRL)
─────────────────────────────────
Updated at 16:42
Refresh now
Quit
```

- Per-model rows (`seven_day_sonnet`/`seven_day_opus`): `-` when the field is
  `null`.
- `extra_usage`: the row disappears when `is_enabled` is `false`.
- Items are plain text, native cross-platform menus do not reliably support
  per-item colour; the colour signal lives in the icon.
- "Refresh now" triggers an immediate poll; "Quit" exits.

## Behaviour

### Polling

- Background thread: `fetch` → sends the result through the `mpsc` channel →
  sleeps 300s.
- Immediate poll on launch.
- "Refresh now" triggers an extra fetch.
- The event loop receives the results via a `tao` user-event and updates the
  tray.
- Token re-read from the OS on every poll (never cached).

### Wake-from-sleep

Removed on purpose. Cross-platform wake detection is complex and the 5-minute
timer already bounds the age of the data. A deliberate simplification (YAGNI),
it is a change from the Swift app, which had a refresh on wake.

### Error states

The same 4 as the Swift app. The widget never erases the last good data on a
transient failure.

| Situation | Icon | Title (macOS) / Tooltip / Menu |
|---|---|---|
| No network / timeout | keeps colour, marks ⚠ | keeps the last values; menu: "no connection" |
| HTTP 401 | grey | `⚠ auth`; "token expired, open Claude Code" |
| Token missing / inaccessible | grey | `⚠ token`; per-OS instruction |
| JSON in an unexpected format | grey | `⚠ fmt`; "endpoint format changed" |

## CLI and auto-start

`main.rs` dispatches by argument:

- `--once`: fetches and prints the usage, exits. Portable check.
- `--selftest`: runs internal asserts, exits 0/1.
- `--install` / `--uninstall`: installs/removes auto-start for the current OS.
- no args: runs the tray.

`autostart` (`#[cfg]`-gated):

- **macOS**: writes `~/Library/LaunchAgents/com.samdev.claude-usage-bar.plist`,
  pointing to the executable inside the `.app`; `launchctl bootstrap`.
- **Linux**: writes `~/.config/autostart/claude-usage-bar.desktop` (XDG autostart).
- **Windows**: adds a value under
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

## Build

`cargo build --release` native on each OS. On macOS, a build script packages the
resulting binary into a `ClaudeUsageBar.app` (`Info.plist` with `LSUIElement`),
required for the tray to appear. Windows and Linux use the executable directly.

Build artifacts (binary, `.app`, `target/`) stay out of git.

## Tests / success criteria

- `cargo test`: unit tests of the portable modules:
  - `usage`: `decode_usage` against a sample JSON (utilizations, `opus` null,
    `extra_usage`, dates with fractional seconds).
  - `render`: colour by level at the 49/50/79/80 thresholds; reset formatting
    (relative/absolute); icon colour selection by the fullest window.
- `--once`: verification of the live data path, run on each OS.
- `--selftest`: quick smoke test on the final binary.
- Tray UI: manual verification per platform.

**Done when:** `cargo test` passes; `--once` returns live data on the 3
platforms; coloured icon + tooltip + menu work on each OS; auto-start installs
and works on each OS.

## Out of scope (YAGNI)

- Refresh on wake-from-sleep.
- Usage history / charts.
- System notifications when a limit is reached.
- Configuration via UI (poll interval, thresholds are constants).
- Per-item colour in the menu.
- Cross-compilation / distribution of prebuilt binaries, native build on each OS.
- Solving the absence of a tray on GNOME, just document it.
