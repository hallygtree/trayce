# Claude Usage Bar (Rust, cross-platform) Implementation Plan

> **Superseded in part (2026-06-01):** the `client.rs`/`token/` + `api/oauth/usage`
> endpoint described below were removed. The data source is now the parsing of the
> local JSONL logs (`src/logs.rs`), due to a violation of Anthropic's Usage Policy.
> See `docs/superpowers/adr-2026-06-01-local-logs-over-oauth-endpoint.md`.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a cross-platform (macOS/Linux/Windows) system-tray app in Rust that shows Claude plan (Pro/Max) usage against the 5h and weekly limits.

**Architecture:** One Cargo project. Portable modules (`error`, `usage`, `client`, `render`) carry all logic; only `token/` and `autostart.rs` have `#[cfg(target_os)]` code. The tray UI uses `tao` (event loop) + `tray-icon`. A background thread polls the `/api/oauth/usage` endpoint every 5 minutes and pushes results to the event loop via an `EventLoopProxy`.

**Tech Stack:** Rust 2021, `tao`, `tray-icon` 0.24, `ureq`, `serde`/`serde_json`, `chrono`, `dirs`.

---

## Notes for the implementer

- This plan is executed on a **macOS** machine. The macOS build, the portable
  modules (`cargo test`), and `--once` are fully verified here. The **Linux and
  Windows** token path and platform builds are written here but can only be
  verified on those machines — their verification steps say "(run on <OS>)".
- macOS-only `#[cfg]` code (`token/macos.rs`, `autostart.rs` macOS branch) is
  the only platform code that compiles on this machine. `token/file.rs` is
  written as **portable** Rust (plain file read) so it compiles and is unit-
  tested on macOS even though it serves Linux/Windows at runtime.
- After every task: `cargo build` must succeed and `cargo test` must pass.

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml` | Crate manifest + dependencies |
| `.gitignore` | Ignore `target/`, the `.app` bundle |
| `src/main.rs` | CLI dispatch (`--once`/`--selftest`/`--install`/`--uninstall`) or run the tray; module declarations |
| `src/error.rs` | `WidgetError` enum |
| `src/usage.rs` | `Usage`/`Window`/`ExtraUsage` structs + `decode_usage` |
| `src/render.rs` | Pure: level/colour, reset-time formatting, ASCII bar, credits, icon RGBA, title/tooltip strings |
| `src/token/mod.rs` | `fetch_token()` dispatched by `#[cfg]` |
| `src/token/macos.rs` | Keychain via `/usr/bin/security` (macOS only) |
| `src/token/file.rs` | `~/.claude/.credentials.json` (Linux/Windows; portable code) |
| `src/client.rs` | `fetch_usage()` — token + HTTP GET + decode |
| `src/tray.rs` | `run()` — tao event loop, tray icon + menu, poll thread wiring |
| `src/autostart.rs` | `install()`/`uninstall()` — per-OS auto-start |
| `macos/Info.plist` | `.app` bundle plist |
| `build-macos.sh` | Build + assemble `ClaudeUsageBar.app` |
| `README.md` | Build/install per OS |

All paths below are relative to `~/dev/claude-usage-bar-rs/`.

---

## Task 1: Toolchain and project scaffold

**Files:**
- Create: `~/dev/claude-usage-bar-rs/.gitignore`
- Create: `~/dev/claude-usage-bar-rs/Cargo.toml`
- Create: `~/dev/claude-usage-bar-rs/src/main.rs`
- Create: `~/dev/claude-usage-bar-rs/src/error.rs`

- [ ] **Step 1: Ensure the Rust toolchain is installed**

Run: `cargo --version`

If it prints a version, skip to Step 2. If `cargo` is not found, install Rust:

Run: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y`
Then: `source "$HOME/.cargo/env" && cargo --version`
Expected: `cargo 1.x.x`

- [ ] **Step 2: Initialise the Cargo project**

Run: `cd ~/dev/claude-usage-bar-rs && cargo init --name claude-usage-bar --vcs none`
Expected: `Creating binary (application) package`

- [ ] **Step 3: Add dependencies**

Run:
```bash
cd ~/dev/claude-usage-bar-rs
cargo add tao
cargo add tray-icon
cargo add ureq
cargo add serde --features derive
cargo add serde_json
cargo add chrono --no-default-features --features clock,std
cargo add dirs
cargo add --target 'cfg(target_os = "macos")' objc2-core-foundation
```
Expected: each prints `Adding <crate> ...` with no error.

- [ ] **Step 4: Create `.gitignore`**

```
/target
/ClaudeUsageBar.app
.DS_Store
```

- [ ] **Step 5: Create `src/error.rs`**

```rust
/// Every failure mode the widget can present to the user.
#[derive(Debug, Clone)]
pub enum WidgetError {
    /// Token store (Keychain / credentials file) absent or unreadable.
    TokenNotFound,
    /// Token store present but not in the expected shape.
    TokenMalformed,
    /// HTTP 401 — token rejected by the endpoint.
    Auth,
    /// Transport failure or non-401 non-200 HTTP status.
    Network(String),
    /// HTTP 200 but the JSON body did not match the expected schema.
    Format,
}
```

- [ ] **Step 6: Replace `src/main.rs` with the CLI dispatch stub**

```rust
#![allow(dead_code)] // removed in the final task once every module is wired

mod error;

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--once") => println!("once: not implemented"),
        Some("--selftest") => println!("selftest: not implemented"),
        Some("--install") => println!("install: not implemented"),
        Some("--uninstall") => println!("uninstall: not implemented"),
        _ => println!("tray: not implemented"),
    }
}
```

- [ ] **Step 7: Build and verify**

Run: `cd ~/dev/claude-usage-bar-rs && cargo build`
Expected: `Finished` with no errors.

Run: `cargo run -- --once`
Expected: `once: not implemented`

Run: `cargo run -- --selftest`
Expected: `selftest: not implemented`

Run: `cargo run`
Expected: `tray: not implemented`

- [ ] **Step 8: Commit**

```bash
cd ~/dev/claude-usage-bar-rs
git add .gitignore Cargo.toml Cargo.lock src/
git commit -m "Add Cargo project scaffold and WidgetError"
```

---

## Task 2: Usage model and JSON decoding

**Files:**
- Create: `~/dev/claude-usage-bar-rs/src/usage.rs`
- Modify: `~/dev/claude-usage-bar-rs/src/main.rs`

- [ ] **Step 1: Write the failing test**

Create `src/usage.rs`:

```rust
use serde::Deserialize;

use crate::error::WidgetError;

#[derive(Debug, Clone, Deserialize)]
pub struct Window {
    pub utilization: u32,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtraUsage {
    pub is_enabled: bool,
    pub used_credits: Option<f64>,
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub five_hour: Window,
    pub seven_day: Window,
    pub seven_day_opus: Option<Window>,
    pub seven_day_sonnet: Option<Window>,
    pub extra_usage: Option<ExtraUsage>,
}

/// Decode the `/api/oauth/usage` JSON body.
pub fn decode_usage(body: &[u8]) -> Result<Usage, WidgetError> {
    serde_json::from_slice(body).map_err(|_| WidgetError::Format)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "five_hour":{"utilization":17,"resets_at":"2026-05-21T20:50:00.508668+00:00"},
        "seven_day":{"utilization":18,"resets_at":"2026-05-24T07:00:00.508691+00:00"},
        "seven_day_opus":null,
        "seven_day_sonnet":{"utilization":5,"resets_at":"2026-05-24T07:00:00.508699+00:00"},
        "extra_usage":{"is_enabled":true,"monthly_limit":null,"used_credits":82,
                       "utilization":null,"currency":"BRL","disabled_reason":null}
    }"#;

    #[test]
    fn decodes_sample_payload() {
        let u = decode_usage(SAMPLE.as_bytes()).expect("should decode");
        assert_eq!(u.five_hour.utilization, 17);
        assert_eq!(u.seven_day.utilization, 18);
        assert_eq!(u.seven_day_sonnet.unwrap().utilization, 5);
        assert!(u.seven_day_opus.is_none());
        assert_eq!(u.extra_usage.unwrap().used_credits, Some(82.0));
        assert!(u.five_hour.resets_at.is_some());
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(decode_usage(b"not json"), Err(WidgetError::Format)));
    }
}
```

Add `mod usage;` to `src/main.rs` directly after `mod error;`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ~/dev/claude-usage-bar-rs && cargo test usage`
Expected: FAIL — the test module does not compile yet only if you skipped Step 1; if Step 1 is fully pasted the tests should already be runnable. Run it and confirm both tests are discovered. If they PASS immediately, that is acceptable here (the implementation and tests were pasted together). Record the actual result.

- [ ] **Step 3: Confirm the implementation**

The implementation (`decode_usage` + structs) is already in Step 1. No extra code needed — `serde` derives the decoding. Extra JSON fields (`monthly_limit`, `disabled_reason`, etc.) are ignored by default.

- [ ] **Step 4: Run the tests**

Run: `cd ~/dev/claude-usage-bar-rs && cargo test usage`
Expected: `test result: ok. 2 passed`.

- [ ] **Step 5: Commit**

```bash
cd ~/dev/claude-usage-bar-rs
git add src/usage.rs src/main.rs
git commit -m "Add Usage model and JSON decoding"
```

---

## Task 3: Pure render helpers

**Files:**
- Create: `~/dev/claude-usage-bar-rs/src/render.rs`
- Modify: `~/dev/claude-usage-bar-rs/src/main.rs`

- [ ] **Step 1: Write the failing tests and implementation**

Create `src/render.rs`:

```rust
use chrono::{DateTime, Datelike, Local, Utc, Weekday};

use crate::usage::{Usage, Window};

/// Colour band for the tray icon, by the fullest window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Green,
    Orange,
    Red,
    Grey,
}

pub fn level_for(utilization: u32) -> Level {
    match utilization {
        0..=49 => Level::Green,
        50..=79 => Level::Orange,
        _ => Level::Red,
    }
}

pub fn worst_level(five: u32, seven: u32) -> Level {
    level_for(five.max(seven))
}

/// "em 3h 12m" / "em 2d 15h" / "em 5m" / "agora" / "—" (unparseable).
pub fn relative_reset(resets_at: &str, now: DateTime<Utc>) -> String {
    let Ok(dt) = DateTime::parse_from_rfc3339(resets_at) else {
        return "—".to_string();
    };
    let secs = (dt.with_timezone(&Utc) - now).num_seconds();
    if secs <= 0 {
        return "agora".to_string();
    }
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    if hours >= 24 {
        // Coarse copy by design: minutes omitted once a day or more away.
        format!("em {}d {}h", hours / 24, hours % 24)
    } else if hours > 0 {
        format!("em {hours}h {mins}m")
    } else {
        format!("em {mins}m")
    }
}

fn weekday_pt(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "seg",
        Weekday::Tue => "ter",
        Weekday::Wed => "qua",
        Weekday::Thu => "qui",
        Weekday::Fri => "sex",
        Weekday::Sat => "sáb",
        Weekday::Sun => "dom",
    }
}

/// "20:50" or "sáb 07:00" in local time. "—" if unparseable.
pub fn absolute_reset(resets_at: &str, include_weekday: bool) -> String {
    let Ok(dt) = DateTime::parse_from_rfc3339(resets_at) else {
        return "—".to_string();
    };
    let local = dt.with_timezone(&Local);
    if include_weekday {
        format!("{} {}", weekday_pt(local.weekday()), local.format("%H:%M"))
    } else {
        local.format("%H:%M").to_string()
    }
}

/// 10-char "▓▓░░░░░░░░" bar, clamped to 0..=100.
pub fn ascii_bar(utilization: u32) -> String {
    let width = 10u32;
    let filled = (((utilization.min(200) as f32) / 100.0) * width as f32).round() as u32;
    let filled = filled.min(width);
    "▓".repeat(filled as usize) + &"░".repeat((width - filled) as usize)
}

pub fn format_credits(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// Menu-bar / title text (used on macOS).
pub fn title_text(u: &Usage) -> String {
    format!(
        "5h {}% · 7d {}%",
        u.five_hour.utilization, u.seven_day.utilization
    )
}

/// Tooltip text (the at-a-glance line on Windows/Linux).
pub fn tooltip_text(u: &Usage) -> String {
    format!("Claude — {}", title_text(u))
}

/// One detail row: "Janela de 5h   17%   ▓▓░░░░░░░░".
pub fn window_row(label: &str, w: &Window) -> String {
    format!("{label}   {}%   {}", w.utilization, ascii_bar(w.utilization))
}

/// Reset row: "reseta em 3h 12m  ·  20:50". Empty string if no reset time.
pub fn reset_row(w: &Window, weekday: bool, now: DateTime<Utc>) -> String {
    match &w.resets_at {
        Some(r) => format!(
            "reseta {}  ·  {}",
            relative_reset(r, now),
            absolute_reset(r, weekday)
        ),
        None => String::new(),
    }
}

/// Error note + title-bar text for a failure.
pub fn error_text(e: &crate::error::WidgetError) -> (String, String) {
    use crate::error::WidgetError::*;
    match e {
        TokenNotFound | TokenMalformed => (
            "token não encontrado — veja o README".to_string(),
            "⚠ token".to_string(),
        ),
        Auth => (
            "token expirado — abra o Claude Code pra renovar".to_string(),
            "⚠ auth".to_string(),
        ),
        Network(msg) => (format!("sem conexão ({msg})"), "⚠".to_string()),
        Format => (
            "endpoint mudou de formato".to_string(),
            "⚠ fmt".to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn level_thresholds() {
        assert_eq!(level_for(49), Level::Green);
        assert_eq!(level_for(50), Level::Orange);
        assert_eq!(level_for(79), Level::Orange);
        assert_eq!(level_for(80), Level::Red);
        assert_eq!(worst_level(10, 85), Level::Red);
    }

    #[test]
    fn relative_reset_formats() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let plus = |secs: i64| (now + chrono::Duration::seconds(secs)).to_rfc3339();
        assert_eq!(relative_reset(&plus(3720), now), "em 1h 2m");
        assert_eq!(relative_reset(&plus(300), now), "em 5m");
        assert_eq!(relative_reset(&plus(140_000), now), "em 1d 14h");
        assert_eq!(relative_reset(&plus(-10), now), "agora");
        assert_eq!(relative_reset("garbage", now), "—");
    }

    #[test]
    fn ascii_bar_fills() {
        assert_eq!(ascii_bar(0), "░░░░░░░░░░");
        assert_eq!(ascii_bar(50).matches('▓').count(), 5);
        assert_eq!(ascii_bar(100), "▓▓▓▓▓▓▓▓▓▓");
        assert_eq!(ascii_bar(150), "▓▓▓▓▓▓▓▓▓▓");
    }

    #[test]
    fn credits_format() {
        assert_eq!(format_credits(82.0), "82");
        assert_eq!(format_credits(82.5), "82.50");
    }
}
```

Add `mod render;` to `src/main.rs` directly after `mod usage;`.

- [ ] **Step 2: Run the tests to verify they pass**

Run: `cd ~/dev/claude-usage-bar-rs && cargo test render`
Expected: `test result: ok. 4 passed`.

- [ ] **Step 3: Commit**

```bash
cd ~/dev/claude-usage-bar-rs
git add src/render.rs src/main.rs
git commit -m "Add pure render helpers"
```

---

## Task 4: Icon generation

**Files:**
- Modify: `~/dev/claude-usage-bar-rs/src/render.rs`

- [ ] **Step 1: Write the failing test**

Add this test to the `mod tests` block in `src/render.rs`, before the closing `}`:

```rust
    #[test]
    fn icon_rgba_shape() {
        let buf = icon_rgba(Level::Green);
        assert_eq!(buf.len(), ICON_SIZE * ICON_SIZE * 4);
        // Centre pixel is opaque.
        let c = (ICON_SIZE / 2 * ICON_SIZE + ICON_SIZE / 2) * 4;
        assert_eq!(buf[c + 3], 255);
        // Top-left corner is transparent (outside the circle).
        assert_eq!(buf[3], 0);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ~/dev/claude-usage-bar-rs && cargo test render::tests::icon_rgba_shape`
Expected: FAIL — `cannot find value 'ICON_SIZE'` / `cannot find function 'icon_rgba'`.

- [ ] **Step 3: Implement icon generation**

Add this to `src/render.rs`, immediately after the `format_credits` function:

```rust
/// Side length of the generated tray icon, in pixels.
pub const ICON_SIZE: usize = 32;

/// A filled circle in the level colour, as RGBA bytes (`ICON_SIZE` square).
pub fn icon_rgba(level: Level) -> Vec<u8> {
    let (r, g, b) = match level {
        Level::Green => (52u8, 199, 89),
        Level::Orange => (255, 149, 0),
        Level::Red => (255, 59, 48),
        Level::Grey => (142, 142, 147),
    };
    let mut buf = vec![0u8; ICON_SIZE * ICON_SIZE * 4];
    let centre = (ICON_SIZE as f32 - 1.0) / 2.0;
    let radius = ICON_SIZE as f32 / 2.0 - 2.0;
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let dx = x as f32 - centre;
            let dy = y as f32 - centre;
            let i = (y * ICON_SIZE + x) * 4;
            if dx * dx + dy * dy <= radius * radius {
                buf[i] = r;
                buf[i + 1] = g;
                buf[i + 2] = b;
                buf[i + 3] = 255;
            }
        }
    }
    buf
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd ~/dev/claude-usage-bar-rs && cargo test render`
Expected: `test result: ok. 5 passed`.

- [ ] **Step 5: Commit**

```bash
cd ~/dev/claude-usage-bar-rs
git add src/render.rs
git commit -m "Add tray icon RGBA generation"
```

---

## Task 5: Token providers

**Files:**
- Create: `~/dev/claude-usage-bar-rs/src/token/mod.rs`
- Create: `~/dev/claude-usage-bar-rs/src/token/macos.rs`
- Create: `~/dev/claude-usage-bar-rs/src/token/file.rs`
- Modify: `~/dev/claude-usage-bar-rs/src/main.rs`

- [ ] **Step 1: Create `src/token/file.rs` with its test**

This module is portable Rust (a plain file read) — it compiles and is tested on
macOS even though at runtime it serves Linux and Windows.

```rust
//! Token from `~/.claude/.credentials.json` (Linux and Windows).

use std::path::PathBuf;

use crate::error::WidgetError;

fn credentials_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join(".credentials.json"))
}

/// Extract `claudeAiOauth.accessToken` from a credentials-file body.
fn token_from_json(body: &str) -> Result<String, WidgetError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| WidgetError::TokenMalformed)?;
    value
        .get("claudeAiOauth")
        .and_then(|o| o.get("accessToken"))
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .ok_or(WidgetError::TokenMalformed)
}

pub fn fetch_token() -> Result<String, WidgetError> {
    let path = credentials_path().ok_or(WidgetError::TokenNotFound)?;
    let body = std::fs::read_to_string(&path).map_err(|_| WidgetError::TokenNotFound)?;
    token_from_json(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_credentials() {
        let body = r#"{"claudeAiOauth":{"accessToken":"abc123"}}"#;
        assert_eq!(token_from_json(body).unwrap(), "abc123");
    }

    #[test]
    fn rejects_missing_token() {
        assert!(matches!(
            token_from_json(r#"{"claudeAiOauth":{}}"#),
            Err(WidgetError::TokenMalformed)
        ));
        assert!(matches!(
            token_from_json("not json"),
            Err(WidgetError::TokenMalformed)
        ));
    }
}
```

- [ ] **Step 2: Create `src/token/macos.rs`**

```rust
//! Token from the macOS Keychain item `Claude Code-credentials`.

use std::process::Command;

use crate::error::WidgetError;

pub fn fetch_token() -> Result<String, WidgetError> {
    let output = Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()
        .map_err(|_| WidgetError::TokenNotFound)?;

    if !output.status.success() {
        return Err(WidgetError::TokenNotFound);
    }
    let body = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(body.trim()).map_err(|_| WidgetError::TokenMalformed)?;
    value
        .get("claudeAiOauth")
        .and_then(|o| o.get("accessToken"))
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .ok_or(WidgetError::TokenMalformed)
}
```

- [ ] **Step 3: Create `src/token/mod.rs`**

```rust
//! OAuth token retrieval, dispatched by platform. Read fresh on every poll —
//! Claude Code rotates the token, so a cached value would go stale.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod file;

use crate::error::WidgetError;

pub fn fetch_token() -> Result<String, WidgetError> {
    #[cfg(target_os = "macos")]
    {
        macos::fetch_token()
    }
    #[cfg(not(target_os = "macos"))]
    {
        file::fetch_token()
    }
}
```

Add `mod token;` to `src/main.rs` directly after `mod render;`.

- [ ] **Step 4: Build and test**

Run: `cd ~/dev/claude-usage-bar-rs && cargo build && cargo test token`
Expected: build `Finished`; `test result: ok. 2 passed` (the `file` module tests).

> Note: on macOS, `cargo build` compiles `token/macos.rs` only; `token/file.rs`
> is still compiled because it is declared portable and its tests run. The
> Windows/Linux runtime path is verified later on those machines.

- [ ] **Step 5: Commit**

```bash
cd ~/dev/claude-usage-bar-rs
git add src/token/ src/main.rs
git commit -m "Add per-platform token providers"
```

---

## Task 6: Usage client

**Files:**
- Create: `~/dev/claude-usage-bar-rs/src/client.rs`
- Modify: `~/dev/claude-usage-bar-rs/src/main.rs`

- [ ] **Step 1: Create `src/client.rs`**

```rust
//! Fetches the usage payload from the undocumented `/api/oauth/usage` endpoint.

use std::time::Duration;

use crate::error::WidgetError;
use crate::token;
use crate::usage::{decode_usage, Usage};

const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";

pub fn fetch_usage() -> Result<Usage, WidgetError> {
    let token = token::fetch_token()?;

    let response = ureq::get(ENDPOINT)
        .set("Authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", "oauth-2025-04-20")
        .timeout(Duration::from_secs(15))
        .call();

    match response {
        Ok(resp) => {
            let mut bytes = Vec::new();
            resp.into_reader()
                .read_to_end(&mut bytes)
                .map_err(|e| WidgetError::Network(e.to_string()))?;
            decode_usage(&bytes)
        }
        Err(ureq::Error::Status(401, _)) => Err(WidgetError::Auth),
        Err(ureq::Error::Status(code, _)) => {
            Err(WidgetError::Network(format!("HTTP {code}")))
        }
        Err(ureq::Error::Transport(t)) => Err(WidgetError::Network(t.to_string())),
    }
}
```

> `into_reader().read_to_end` needs `std::io::Read` in scope — add `use std::io::Read;`
> at the top of the file if `cargo build` reports the trait is not in scope.

- [ ] **Step 2: Wire a temporary `--once` into `src/main.rs`**

Add `mod client;` after `mod token;`. Then replace the `Some("--once") => ...`
arm of the `match` in `main()` with:

```rust
        Some("--once") => match client::fetch_usage() {
            Ok(u) => {
                println!("5h: {}%", u.five_hour.utilization);
                println!("7d: {}%", u.seven_day.utilization);
                println!(
                    "7d sonnet: {}",
                    u.seven_day_sonnet
                        .map(|w| format!("{}%", w.utilization))
                        .unwrap_or_else(|| "—".to_string())
                );
                println!(
                    "7d opus:   {}",
                    u.seven_day_opus
                        .map(|w| format!("{}%", w.utilization))
                        .unwrap_or_else(|| "—".to_string())
                );
                if let Some(ex) = u.extra_usage {
                    println!(
                        "extra: {} {}",
                        ex.used_credits.map(render::format_credits).unwrap_or_default(),
                        ex.currency.unwrap_or_default()
                    );
                }
            }
            Err(e) => {
                eprintln!("error: {e:?}");
                std::process::exit(1);
            }
        },
```

- [ ] **Step 3: Build and verify against a live curl**

Run: `cd ~/dev/claude-usage-bar-rs && cargo build && cargo run -- --once`
Expected: 4 lines (`5h:`, `7d:`, `7d sonnet:`, `7d opus:`) plus an `extra:` line, with plausible percentages.

Cross-check the numbers:
```bash
TOKEN=$(security find-generic-password -s "Claude Code-credentials" -w \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["claudeAiOauth"]["accessToken"])')
curl -s https://api.anthropic.com/api/oauth/usage \
  -H "Authorization: Bearer $TOKEN" -H "anthropic-beta: oauth-2025-04-20"
```
Expected: the `utilization` values in the curl JSON match the `--once` output.

- [ ] **Step 4: Commit**

```bash
cd ~/dev/claude-usage-bar-rs
git add src/client.rs src/main.rs
git commit -m "Add usage client and --once output"
```

---

## Task 7: Tray icon and menu

**Files:**
- Create: `~/dev/claude-usage-bar-rs/src/tray.rs`
- Modify: `~/dev/claude-usage-bar-rs/src/main.rs`

- [ ] **Step 1: Create `src/tray.rs`**

This task builds the tray with a **static** initial state (no polling yet —
that is Task 8). `build_menu` and the event loop are the focus.

```rust
//! System-tray UI: tao event loop + tray-icon. Polling is wired in Task 8.

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::render::{self, Level};
use crate::usage::Usage;

/// Events delivered into the tao event loop.
enum UserEvent {
    Menu(MenuEvent),
}

fn icon(level: Level) -> Icon {
    Icon::from_rgba(
        render::icon_rgba(level),
        render::ICON_SIZE as u32,
        render::ICON_SIZE as u32,
    )
    .expect("icon_rgba always yields a valid RGBA buffer")
}

/// Wake the macOS run loop so tray changes draw immediately. No-op elsewhere.
fn wake_macos() {
    #[cfg(target_os = "macos")]
    unsafe {
        use objc2_core_foundation::{CFRunLoopGetMain, CFRunLoopWakeUp};
        if let Some(rl) = CFRunLoopGetMain() {
            CFRunLoopWakeUp(&rl);
        }
    }
}

fn disabled(text: &str) -> MenuItem {
    MenuItem::new(text, false, None)
}

/// Build the dropdown menu. Returns the menu plus the ids of the two
/// actionable items so the event loop can match clicks against them.
fn build_menu(usage: Option<&Usage>, error_note: Option<&str>) -> (Menu, MenuId, MenuId) {
    let menu = Menu::new();
    let now = chrono::Utc::now();

    if let Some(u) = usage {
        let _ = menu.append(&disabled(&render::window_row("Janela de 5h", &u.five_hour)));
        let reset5 = render::reset_row(&u.five_hour, false, now);
        if !reset5.is_empty() {
            let _ = menu.append(&disabled(&reset5));
        }
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&disabled(&render::window_row("Semanal (7d)", &u.seven_day)));
        let reset7 = render::reset_row(&u.seven_day, true, now);
        if !reset7.is_empty() {
            let _ = menu.append(&disabled(&reset7));
        }
        let _ = menu.append(&PredefinedMenuItem::separator());

        let sonnet = u
            .seven_day_sonnet
            .as_ref()
            .map(|w| format!("{}%", w.utilization))
            .unwrap_or_else(|| "—".to_string());
        let opus = u
            .seven_day_opus
            .as_ref()
            .map(|w| format!("{}%", w.utilization))
            .unwrap_or_else(|| "—".to_string());
        let _ = menu.append(&disabled(&format!("Semanal · Sonnet   {sonnet}")));
        let _ = menu.append(&disabled(&format!("Semanal · Opus     {opus}")));

        if let Some(ex) = &u.extra_usage {
            if ex.is_enabled {
                let credits = ex
                    .used_credits
                    .map(render::format_credits)
                    .unwrap_or_else(|| "—".to_string());
                let currency = ex.currency.clone().unwrap_or_default();
                let _ = menu.append(&disabled(&format!(
                    "Uso extra          {credits} créditos ({currency})"
                )));
            }
        }
        let _ = menu.append(&PredefinedMenuItem::separator());
    }

    if let Some(note) = error_note {
        let _ = menu.append(&disabled(note));
    }

    let refresh = MenuItem::new("Atualizar agora", true, None);
    let quit = MenuItem::new("Sair", true, None);
    let refresh_id = refresh.id().clone();
    let quit_id = quit.id().clone();
    let _ = menu.append(&refresh);
    let _ = menu.append(&quit);

    (menu, refresh_id, quit_id)
}

pub fn run() -> ! {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    let mut tray: Option<TrayIcon> = None;
    let mut refresh_id: Option<MenuId> = None;
    let mut quit_id: Option<MenuId> = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                let (menu, r_id, q_id) = build_menu(None, Some("carregando…"));
                refresh_id = Some(r_id);
                quit_id = Some(q_id);
                tray = Some(
                    TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_tooltip("Claude — carregando…")
                        .with_icon(icon(Level::Grey))
                        .build()
                        .expect("failed to build tray icon"),
                );
                wake_macos();
            }
            Event::UserEvent(UserEvent::Menu(ev)) => {
                if Some(&ev.id) == quit_id.as_ref() {
                    tray.take();
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => {}
        }
    })
}
```

> If `cargo build` reports that `Menu::append` does not exist, use
> `menu.append_items(&[&item])` per item, or collect items into a `Vec` and
> pass them to `append_items` — both are valid `muda` APIs for tray-icon 0.24.

- [ ] **Step 2: Wire `tray::run()` into `src/main.rs`**

Add `mod tray;` after `mod client;`. Replace the `_ => println!("tray: not implemented"),`
arm with:

```rust
        _ => tray::run(),
```

- [ ] **Step 3: Build and verify launch**

Run: `cd ~/dev/claude-usage-bar-rs && cargo build`
Expected: `Finished` with no errors.

Run (headless check — you cannot see the macOS menu bar):
```bash
cd ~/dev/claude-usage-bar-rs
cargo run & PID=$!
sleep 4
kill -0 $PID && echo "alive (no crash)" || echo "CRASHED"
kill $PID 2>/dev/null
```
Expected: `alive (no crash)`. Visual confirmation of the tray item is deferred to a human.

- [ ] **Step 4: Commit**

```bash
cd ~/dev/claude-usage-bar-rs
git add src/tray.rs src/main.rs
git commit -m "Add tray icon and dropdown menu"
```

---

## Task 8: Polling and live updates

**Files:**
- Modify: `~/dev/claude-usage-bar-rs/src/tray.rs`

- [ ] **Step 1: Extend `UserEvent` and add the poll wiring**

In `src/tray.rs`, replace the `UserEvent` enum with:

```rust
/// Events delivered into the tao event loop.
enum UserEvent {
    Menu(MenuEvent),
    Poll(Result<Usage, crate::error::WidgetError>),
}
```

Add these imports to the top of `src/tray.rs`:

```rust
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use crate::client::fetch_usage;
use crate::error::WidgetError;
```

Add this constant after the imports:

```rust
const POLL_INTERVAL: Duration = Duration::from_secs(300);
```

- [ ] **Step 2: Add the `apply` function**

Add this function to `src/tray.rs`, immediately before `pub fn run()`:

```rust
/// Update the tray from a poll result.
fn apply(
    tray: &Option<TrayIcon>,
    last_usage: &mut Option<Usage>,
    refresh_id: &mut Option<MenuId>,
    quit_id: &mut Option<MenuId>,
    result: Result<Usage, WidgetError>,
) {
    let Some(tray) = tray else { return };

    match result {
        Ok(u) => {
            let level = render::worst_level(u.five_hour.utilization, u.seven_day.utilization);
            let _ = tray.set_icon(Some(icon(level)));
            let _ = tray.set_tooltip(Some(render::tooltip_text(&u)));
            tray.set_title(Some(render::title_text(&u)));
            let (menu, r, q) = build_menu(Some(&u), None);
            tray.set_menu(Some(Box::new(menu)));
            *refresh_id = Some(r);
            *quit_id = Some(q);
            *last_usage = Some(u);
        }
        Err(e) => {
            let (note, title) = render::error_text(&e);
            let keep_last = matches!(e, WidgetError::Network(_)) && last_usage.is_some();
            if keep_last {
                // Transient network failure: keep the last good data, just flag it.
                let _ = tray.set_tooltip(Some(format!("⚠ {note}")));
                let (menu, r, q) = build_menu(last_usage.as_ref(), Some(&note));
                tray.set_menu(Some(Box::new(menu)));
                *refresh_id = Some(r);
                *quit_id = Some(q);
            } else {
                let _ = tray.set_icon(Some(icon(Level::Grey)));
                let _ = tray.set_tooltip(Some(note.clone()));
                tray.set_title(Some(title));
                let (menu, r, q) = build_menu(last_usage.as_ref(), Some(&note));
                tray.set_menu(Some(Box::new(menu)));
                *refresh_id = Some(r);
                *quit_id = Some(q);
            }
        }
    }
}
```

- [ ] **Step 3: Replace `pub fn run()` with the polling version**

```rust
pub fn run() -> ! {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    // Forward menu events into the event loop.
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    // Background poll thread: fetch, push result, then wait 5 min OR until a
    // manual-refresh signal arrives on `refresh_rx`.
    let (refresh_tx, refresh_rx) = mpsc::channel::<()>();
    let poll_proxy = event_loop.create_proxy();
    thread::spawn(move || loop {
        let result = fetch_usage();
        if poll_proxy.send_event(UserEvent::Poll(result)).is_err() {
            return; // event loop has shut down
        }
        match refresh_rx.recv_timeout(POLL_INTERVAL) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        }
    });

    let mut tray: Option<TrayIcon> = None;
    let mut last_usage: Option<Usage> = None;
    let mut refresh_id: Option<MenuId> = None;
    let mut quit_id: Option<MenuId> = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                let (menu, r_id, q_id) = build_menu(None, Some("carregando…"));
                refresh_id = Some(r_id);
                quit_id = Some(q_id);
                tray = Some(
                    TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_tooltip("Claude — carregando…")
                        .with_icon(icon(Level::Grey))
                        .build()
                        .expect("failed to build tray icon"),
                );
                wake_macos();
            }
            Event::UserEvent(UserEvent::Poll(result)) => {
                apply(&tray, &mut last_usage, &mut refresh_id, &mut quit_id, result);
                wake_macos();
            }
            Event::UserEvent(UserEvent::Menu(ev)) => {
                if Some(&ev.id) == quit_id.as_ref() {
                    tray.take();
                    *control_flow = ControlFlow::Exit;
                } else if Some(&ev.id) == refresh_id.as_ref() {
                    let _ = refresh_tx.send(());
                }
            }
            _ => {}
        }
    })
}
```

- [ ] **Step 4: Build and verify live updates**

Run: `cd ~/dev/claude-usage-bar-rs && cargo build`
Expected: `Finished` with no errors.

Run (headless check):
```bash
cd ~/dev/claude-usage-bar-rs
cargo run & PID=$!
sleep 10
kill -0 $PID && echo "alive after first poll+render" || echo "CRASHED"
kill $PID 2>/dev/null
```
Expected: `alive after first poll+render` — the app survives the launch poll and the `apply` render path. Visual confirmation (icon turns colour, menu shows numbers) is deferred to a human.

- [ ] **Step 5: Confirm `cargo test` still passes**

Run: `cd ~/dev/claude-usage-bar-rs && cargo test`
Expected: all tests pass (`usage`, `render`, `token::file`).

- [ ] **Step 6: Commit**

```bash
cd ~/dev/claude-usage-bar-rs
git add src/tray.rs
git commit -m "Add polling thread and live tray updates"
```

---

## Task 9: macOS .app bundle

**Files:**
- Create: `~/dev/claude-usage-bar-rs/macos/Info.plist`
- Create: `~/dev/claude-usage-bar-rs/build-macos.sh`

- [ ] **Step 1: Create `macos/Info.plist`**

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>claude-usage-bar</string>
    <key>CFBundleIdentifier</key>
    <string>com.samdev.claude-usage-bar</string>
    <key>CFBundleName</key>
    <string>Claude Usage Bar</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
```

- [ ] **Step 2: Create `build-macos.sh`**

```bash
#!/bin/bash
set -e
cd "$(dirname "$0")"

cargo build --release

APP="ClaudeUsageBar.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/claude-usage-bar "$APP/Contents/MacOS/claude-usage-bar"
cp macos/Info.plist "$APP/Contents/Info.plist"

echo "built: $(pwd)/$APP"
```

> A bare Mach-O binary does not reliably present an `NSStatusItem` on macOS;
> the `.app` bundle with `LSUIElement` is required. Linux and Windows run the
> plain `target/release/claude-usage-bar` executable directly.

- [ ] **Step 3: Build the bundle and verify it launches**

```bash
cd ~/dev/claude-usage-bar-rs
chmod +x build-macos.sh
./build-macos.sh
open ClaudeUsageBar.app
sleep 8
pgrep -f ClaudeUsageBar.app/Contents/MacOS/claude-usage-bar && echo "running" || echo "NOT running"
pkill -f ClaudeUsageBar.app/Contents/MacOS/claude-usage-bar
```
Expected: `built: ...ClaudeUsageBar.app` then `running`.

- [ ] **Step 4: Commit**

```bash
cd ~/dev/claude-usage-bar-rs
git add macos/Info.plist build-macos.sh
git commit -m "Add macOS .app bundle packaging"
```

---

## Task 10: Auto-start (install / uninstall)

**Files:**
- Create: `~/dev/claude-usage-bar-rs/src/autostart.rs`
- Modify: `~/dev/claude-usage-bar-rs/src/main.rs`

- [ ] **Step 1: Create `src/autostart.rs`**

```rust
//! Per-platform auto-start on login. `install()` / `uninstall()` are invoked
//! by the `--install` / `--uninstall` CLI flags.

use std::path::PathBuf;

const LABEL: &str = "com.samdev.claude-usage-bar";

/// Absolute path to the executable that auto-start should launch.
fn exe_path() -> PathBuf {
    std::env::current_exe().expect("current exe path is available")
}

#[cfg(target_os = "macos")]
pub fn install() {
    use std::process::Command;

    let plist_dir = dirs::home_dir().unwrap().join("Library/LaunchAgents");
    let _ = std::fs::create_dir_all(&plist_dir);
    let plist_path = plist_dir.join(format!("{LABEL}.plist"));
    let exe = exe_path();
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{LABEL}</string>
    <key>ProgramArguments</key><array><string>{}</string></array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
</dict>
</plist>
"#,
        exe.display()
    );
    std::fs::write(&plist_path, plist).expect("write LaunchAgent plist");
    let uid = format!("gui/{}", unsafe { libc_getuid() });
    let _ = Command::new("launchctl")
        .args(["bootstrap", &uid, plist_path.to_str().unwrap()])
        .status();
    println!("installed: {}", plist_path.display());
}

#[cfg(target_os = "macos")]
fn libc_getuid() -> u32 {
    // `id -u` avoids a libc dependency just for getuid().
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
pub fn uninstall() {
    let plist_path = dirs::home_dir()
        .unwrap()
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"));
    let uid = format!("gui/{}", libc_getuid());
    let _ = std::process::Command::new("launchctl")
        .args(["bootout", &uid, plist_path.to_str().unwrap()])
        .status();
    let _ = std::fs::remove_file(&plist_path);
    println!("uninstalled");
}

#[cfg(target_os = "linux")]
pub fn install() {
    let dir = dirs::home_dir().unwrap().join(".config/autostart");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("claude-usage-bar.desktop");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Claude Usage Bar\nExec={}\nX-GNOME-Autostart-enabled=true\n",
        exe_path().display()
    );
    std::fs::write(&path, entry).expect("write autostart .desktop");
    println!("installed: {}", path.display());
}

#[cfg(target_os = "linux")]
pub fn uninstall() {
    let path = dirs::home_dir()
        .unwrap()
        .join(".config/autostart/claude-usage-bar.desktop");
    let _ = std::fs::remove_file(&path);
    println!("uninstalled");
}

#[cfg(target_os = "windows")]
pub fn install() {
    // HKCU Run key — value name = LABEL, value data = exe path.
    let status = std::process::Command::new("reg")
        .args([
            "add",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            LABEL,
            "/t",
            "REG_SZ",
            "/d",
            &exe_path().display().to_string(),
            "/f",
        ])
        .status();
    match status {
        Ok(s) if s.success() => println!("installed: HKCU Run\\{LABEL}"),
        _ => eprintln!("install failed — could not write the registry Run key"),
    }
}

#[cfg(target_os = "windows")]
pub fn uninstall() {
    let _ = std::process::Command::new("reg")
        .args([
            "delete",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            LABEL,
            "/f",
        ])
        .status();
    println!("uninstalled");
}
```

- [ ] **Step 2: Wire `--install` / `--uninstall` into `src/main.rs`**

Add `mod autostart;` after `mod tray;`. Replace the `Some("--install")` and
`Some("--uninstall")` match arms with:

```rust
        Some("--install") => autostart::install(),
        Some("--uninstall") => autostart::uninstall(),
```

- [ ] **Step 3: Build and verify (macOS)**

Run: `cd ~/dev/claude-usage-bar-rs && cargo build`
Expected: `Finished` with no errors.

Run:
```bash
cd ~/dev/claude-usage-bar-rs
cargo run -- --install
launchctl list | grep claude-usage-bar && echo "registered"
cargo run -- --uninstall
```
Expected: `installed: ...plist`, then `registered`, then `uninstalled`.

> The Linux (`.desktop`) and Windows (registry) branches do not compile on
> macOS — they are verified later on those machines. `cargo build` on macOS
> only compiles the `#[cfg(target_os = "macos")]` branch.

- [ ] **Step 4: Commit**

```bash
cd ~/dev/claude-usage-bar-rs
git add src/autostart.rs src/main.rs
git commit -m "Add per-platform auto-start install/uninstall"
```

---

## Task 11: Self-test, README, and warning cleanup

**Files:**
- Modify: `~/dev/claude-usage-bar-rs/src/main.rs`
- Create: `~/dev/claude-usage-bar-rs/README.md`

- [ ] **Step 1: Implement `--selftest` in `src/main.rs`**

Replace the `Some("--selftest") => ...` match arm with:

```rust
        Some("--selftest") => {
            use render::Level;
            assert_eq!(render::level_for(49), Level::Green);
            assert_eq!(render::level_for(80), Level::Red);
            assert_eq!(render::ascii_bar(100), "▓▓▓▓▓▓▓▓▓▓");
            assert_eq!(render::icon_rgba(Level::Green).len(), render::ICON_SIZE * render::ICON_SIZE * 4);
            println!("selftest: PASS");
        }
```

- [ ] **Step 2: Remove the crate-level `#![allow(dead_code)]`**

Delete the line `#![allow(dead_code)]` from the top of `src/main.rs`.

- [ ] **Step 3: Build warning-clean and run the self-test**

Run: `cd ~/dev/claude-usage-bar-rs && cargo build 2>&1 | tee /tmp/build.log`
Expected: `Finished` with **no `warning:` lines**. If any dead-code warnings
appear, they indicate genuinely unused code — report them rather than
re-adding the blanket `allow`.

Run: `cargo run -- --selftest`
Expected: `selftest: PASS`

- [ ] **Step 4: Create `README.md`**

```markdown
# Claude Usage Bar

Cross-platform (macOS / Linux / Windows) system-tray app showing Claude plan
(Pro/Max) usage against the 5h and weekly limits. Data comes from the
`/api/oauth/usage` endpoint, authenticated with the OAuth token that Claude
Code stores on the local machine.

## Build

Requires the Rust toolchain (`rustup`).

    cargo build --release        # Linux / Windows — run target/release/claude-usage-bar
    ./build-macos.sh             # macOS — produces ClaudeUsageBar.app

macOS needs the `.app` bundle: a bare binary does not show a menu bar item.

## Run

    claude-usage-bar             # runs the tray app
    claude-usage-bar --once      # prints current usage and exits
    claude-usage-bar --selftest  # runs internal asserts, exits 0 on pass
    claude-usage-bar --install   # enable auto-start on login
    claude-usage-bar --uninstall # disable auto-start

## Token source

- **macOS** — Keychain item `Claude Code-credentials`.
- **Linux / Windows** — `~/.claude/.credentials.json`.

If the icon shows `⚠ token`, the token store was not found — confirm Claude
Code is signed in on this machine.

## Platform notes

- **Linux** — the tray uses `StatusNotifierItem`. GNOME has no tray by default;
  install the *AppIndicator* extension. KDE / XFCE work out of the box.
- **Windows** — `--install` writes an `HKCU\...\Run` registry value.
- The `/api/oauth/usage` endpoint is undocumented and may change. `⚠ fmt` means
  the response shape changed and `src/usage.rs` needs updating.
```

- [ ] **Step 5: Commit**

```bash
cd ~/dev/claude-usage-bar-rs
git add src/main.rs README.md
git commit -m "Add --selftest, README, and clean up warnings"
```

---

## Self-Review Notes

- **Spec coverage:** stack `tao`/`tray-icon`/`ureq`/`serde`/`chrono`/`dirs`
  (Task 1); `Usage` model + decode (Task 2); render helpers + icon (Tasks 3-4);
  per-platform token incl. the macOS Keychain and the shared
  `.credentials.json` reader (Task 5); HTTP client + error mapping (Task 6);
  tray icon, colour-by-level, tooltip, macOS title, dropdown menu (Task 7);
  5-min polling, manual refresh, error states, last-good-data retention
  (Task 8); macOS `.app` bundle (Task 9); per-OS auto-start (Task 10);
  `--once`/`--selftest`/`--install`/`--uninstall`, README incl. GNOME caveat
  and undocumented-endpoint note (Tasks 6, 10, 11). All spec sections map to a
  task.
- **Refinement from the spec:** the spec listed `token/{macos,linux,windows}.rs`;
  the plan uses `token/{macos,file}.rs` because the Linux and Windows logic is
  identical (read `.credentials.json`) — DRY, and `file.rs` is portable so it is
  unit-tested on macOS. The Windows-DPAPI contingency from the spec still
  applies: if verification on a Windows machine shows the token is not in a
  plain file, a `token/windows.rs` is added then.
- **Spec simplification:** wake-from-sleep refresh was already declared out of
  scope in the spec — not implemented.
- **Type consistency:** `WidgetError` (Task 1) used unchanged in Tasks 2/5/6/8;
  `Usage`/`Window`/`ExtraUsage` (Task 2) used in Tasks 3/6/7/8; `render::Level`,
  `ICON_SIZE`, `icon_rgba`, `worst_level`, `title_text`, `tooltip_text`,
  `window_row`, `reset_row`, `error_text`, `format_credits` (Tasks 3-4) used in
  Tasks 7/8/11; `fetch_token` (Task 5) → `fetch_usage` (Task 6) → `UserEvent::Poll`
  (Task 8); `build_menu` signature `(Option<&Usage>, Option<&str>) -> (Menu, MenuId, MenuId)`
  stable across Tasks 7-8.
- **Cross-platform limitation:** Tasks executed on macOS verify the macOS build,
  the portable core (`cargo test`), and `--once`. Linux/Windows token runtime
  and the `.desktop`/registry auto-start are written here and verified on those
  machines (steps note this explicitly).
