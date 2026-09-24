# Trayce

[![CI](https://img.shields.io/github/actions/workflow/status/hallygtree/trayce/ci.yml?branch=main&label=CI)](https://github.com/hallygtree/trayce/actions)
[![License](https://img.shields.io/github/license/hallygtree/trayce)](LICENSE)

**Trayce is a system-tray app for macOS, Linux and Windows. It shows how much
of your AI coding plans you have used (Claude Code, Codex CLI and
Antigravity) without opening any of them.**

It reads the local data each tool already writes to disk. It makes no internet
calls and never touches your tokens or credentials.

Trayce started as a fork of
[sammyjdev/claude-usage-bar](https://github.com/sammyjdev/claude-usage-bar),
which covered Claude only.

## Contents

- [Features](#features)
- [Supported tools](#supported-tools)
- [Quick start](#quick-start)
- [Using the tray](#using-the-tray)
- [Install](#install)
- [CLI reference](#cli-reference)
- [Configuration](#configuration)
- [How each tool is read](#how-each-tool-is-read)
- [Privacy](#privacy)
- [Limitations](#limitations)
- [Development](#development)
- [Credits and license](#credits-and-license)

## Features

- **Several tools in one icon.** Claude Code, Codex CLI and Antigravity side by side.
- **You choose what shows.** Turn each tool on or off, and show all of them or just one, from the tray menu itself.
- **Real percentages where they exist.**
  - **Codex:** the server's own numbers.
  - **Antigravity:** the real quota while its desktop app is open.
  - **Claude:** Claude Code's own numbers, via its status line.
- **No fake precision.** A window with no known limit shows a token count instead of a guessed percentage.
- **Offline and read-only.** No credentials read, no calls to any provider, log files opened read-only.
- **A colour you can read at a glance.** The icon dot turns green, orange or red with the fullest window on screen.
- **A single small Rust binary.**

## Supported tools

| Tool | Where Trayce reads from | What you see |
|------|-------------------------|--------------|
| **Claude Code** | Claude Code's status-line data, plus `~/.claude/projects/**/*.jsonl` | real **%** of the 5h and weekly windows once the status line is set up (`trayce --setup-claude`); otherwise 5h / 7d tokens with an estimated 5h % |
| **Codex CLI** | `~/.codex/sessions/**/rollout-*.jsonl` | real **%** of the 5h and weekly windows, with reset times and plan |
| **Antigravity** | `~/.gemini/antigravity-cli/conversations/*.db`, plus the desktop app's local server | 5h / 7d tokens and requests per model; real **%** per quota bucket while the desktop app is open (the last reading is kept after you close it) |

## Quick start

**Windows:** one line in PowerShell, no admin rights, no toolchain:

```powershell
irm https://raw.githubusercontent.com/hallygtree/trayce/main/install.ps1 | iex
```

This command:
- installs Trayce in `%LOCALAPPDATA%\Programs\trayce`
- sets it to start on login
- starts it

Run the same line again to update.

**macOS / Linux, or from source:** you need the
[Rust toolchain](https://rustup.rs). Linux also needs the tray libraries; see
[Install](#install).

```bash
git clone https://github.com/hallygtree/trayce.git
cd trayce
cargo build --release

# Check what Trayce finds on this machine, for every supported tool
./target/release/trayce --once all

# Start the tray app
./target/release/trayce
```

On Windows the binary is `target\release\trayce.exe`. On macOS, build the
`.app` bundle instead (see [macOS](#macos)), because a bare binary does not
show a menu-bar item.

Out of the box only Claude is enabled. Open the tray menu, go to **Enabled AIs**
and tick the other tools you use. For Claude's real percentages, also run
`trayce --setup-claude` once (see [Claude Code](#claude-code)).

## Using the tray

Hover the icon for a one-line summary per tool. Click it to open the menu.

**Show → All enabled** gives each tool its own submenu. Hover a tool's submenu
to see its details:

```text
● Trayce
├─ Claude        5h 43% · 7d 8.1M tok      ▸  5h window   43%   ▓▓▓▓░░░░░░
│                                             resets in 2h 10m  ·  18:50
│                                             Weekly (7d)   8.1M tok   ▓░░░░░░░░░
│                                             Weekly · Sonnet   1.8M tok
│                                             Weekly · Opus     6.3M tok
├─ Codex         5h 67% · 7d 31%           ▸
├─ Antigravity   5h 195k tok · 7d 195k tok ▸
├─ Show          ▸  ☑ All enabled / ☐ Only Claude / ☐ Only Codex / …
├─ Enabled AIs   ▸  ☑ Claude  ☑ Codex  ☑ Antigravity
├─ Refresh now
└─ Quit
```

**Show → Only &lt;tool&gt;** switches to a flat menu with just that tool, like the
original single-tool app. The **Show** and **Enabled AIs** submenus and
**Refresh now** / **Quit** stay at the bottom in both modes. Changes are saved
immediately and trigger a refresh.

- **Icon colour:** the fullest window among the tools on screen sets the
  colour. Green below 50%, orange from 50% to 79%, red at 80% or more, and grey
  on an error (`⚠ logs`).
- **Refresh:** every 60 seconds, or right away with **Refresh now**.
- **macOS:** the menu bar also shows the summary as text next to the icon.
- **Errors:** if a tool's data cannot be read, its entry shows `⚠ logs` with a
  note, and its last good reading stays visible.

## Install

### Prebuilt binaries

Every release ships macOS, Linux and Windows builds on the
[Releases](https://github.com/hallygtree/trayce/releases) page:
- macOS: `trayce-macos.zip`, which contains `Trayce.app`
- Linux: `trayce-linux.tar.gz`
- Windows: `trayce-windows.zip`

> [!NOTE]
> The binaries are not code-signed. On the first launch on **macOS**, right-click
> the app, then choose *Open* → *Open*. On **Windows**, click *More info* → *Run anyway*.

### macOS

```bash
./build-macos.sh                                   # builds Trayce.app
./Trayce.app/Contents/MacOS/trayce --install       # start on login (LaunchAgent)
```

Run `--install` from inside the bundle so the LaunchAgent points at the
`.app`. If you move the app, for example to `/Applications`, run it again.
To remove it: `./Trayce.app/Contents/MacOS/trayce --uninstall`.

### Linux

```bash
# Debian/Ubuntu; other distros: the gtk3, xdo and ayatana-appindicator3 dev packages
sudo apt install libgtk-3-dev libxdo-dev libayatana-appindicator3-dev

cargo build --release
install -Dm755 target/release/trayce ~/.local/bin/trayce
~/.local/bin/trayce --install      # writes ~/.config/autostart/trayce.desktop
```

GNOME has no system tray by default. Install the
[AppIndicator](https://extensions.gnome.org/extension/615/appindicator-support/)
extension. KDE, XFCE and most other desktops work out of the box.

### Windows

Install or update, as the current user with no admin rights:

```powershell
irm https://raw.githubusercontent.com/hallygtree/trayce/main/install.ps1 | iex
```

[`install.ps1`](install.ps1) takes these steps:
1. Downloads the latest `trayce-windows.zip` release.
2. Stops a running copy, if any.
3. Unpacks it to `%LOCALAPPDATA%\Programs\trayce`.
4. Registers start-on-login (an `HKCU\...\Run` value).
5. Starts the app.

Uninstall:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/hallygtree/trayce/main/install.ps1))) -Uninstall
```

Your settings in `%APPDATA%\trayce` are kept on update and uninstall.

<details>
<summary>Build from source instead</summary>

You need [Rust](https://rustup.rs) and the Visual Studio **Build Tools** with
the *Desktop development with C++* workload:

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
cargo build --release
.\target\release\trayce.exe --install    # start on login from this path
```

</details>

## CLI reference

| Command | What it does |
|---------|--------------|
| `trayce` | Runs the tray app (default). |
| `trayce --once` | Prints current usage of the **enabled** tools and exits. |
| `trayce --once all` | Same, for **every** supported tool. The quickest check on a new machine. |
| `trayce --diagnose` | Prints what was found for each tool, for support. For Claude: log dir, event counts, limit events and calibration. For Codex: sessions dir and rollouts. For Antigravity: databases, readable and unreadable rows, the desktop app's server, and the cached quota. It prints no prompts, code or tokens. |
| `trayce --selftest` | Runs internal asserts and exits 0 on pass. |
| `trayce --install` | Starts Trayce on login. |
| `trayce --uninstall` | Stops starting Trayce on login. |
| `trayce --setup-claude` | Registers Trayce as Claude Code's status line, which gives it Claude's real percentages. |
| `trayce --claude-statusline` | The status-line command itself. Claude Code calls it with JSON on stdin. |

On Windows, Trayce is a windowed app, so PowerShell does not wait for its
output. Add `| Out-Host` to see it in order, for example
`trayce --once all | Out-Host`. The installed binary is
`%LOCALAPPDATA%\Programs\trayce\trayce.exe`.

Example:

```text
$ trayce --once all
[Claude]
  5h window      43%
  Weekly (7d)    8.1M tok
  Weekly · Sonnet   1.8M tok
  Weekly · Opus     6.3M tok
[Codex]
  5h window      67%
  Weekly (7d)    31%
  Plan: plus
  Last Codex activity: 6m ago
[Antigravity]
  5h window      195k tok
  Weekly (7d)    195k tok
  Quota %: open the Antigravity app once
  7d · gemini-3.8-flash   22 req · 195k tok
```

## Configuration

The tray menu manages the settings for you. They live in
`<data_dir>/trayce/config.json`:

```json
{
  "enabled": ["Claude", "Codex", "Antigravity"],
  "mode": { "Single": "Codex" }
}
```

- **`enabled`:** the tools Trayce reads.
- **`mode`:** either `"All"` or `{ "Single": "<tool>" }`. If the single tool is
  disabled, the menu falls back to showing all enabled tools.
- **No file, or an unreadable one:** Trayce uses the defaults, which are Claude
  only, shown alone.

Environment variables:

| Variable | Default | Effect |
|----------|---------|--------|
| `CLAUDE_CONFIG_DIR` | `~/.claude` | Where Claude Code keeps its data; Trayce reads `projects/` inside it |
| `CODEX_HOME` | `~/.codex` | Where Codex keeps its data; Trayce reads `sessions/` inside it |

Files Trayce writes, and nothing else:

| File | Content |
|------|---------|
| `<data_dir>/trayce/config.json` | enabled tools and display mode |
| `<data_dir>/trayce/antigravity_quota.json` | last Antigravity quota reading |
| `<data_dir>/trayce/claude_rate_limits.json` | last Claude status-line snapshot |
| `~/.claude/settings.json` | only with `--setup-claude`: the `statusLine` entry (backup kept alongside) |
| `<data_dir>/claude-usage-bar/calibration.json` | learned Claude limit. It keeps the original app's path, so an existing calibration carries over |

`<data_dir>` depends on the OS:
- macOS: `~/Library/Application Support`
- Linux: `~/.local/share`
- Windows: `%APPDATA%`

## How each tool is read

### Claude Code

**Real percentages (recommended): run `trayce --setup-claude` once.**

- **Where the numbers come from:** Claude Code passes its own plan usage to
  its [status line](https://code.claude.com/docs/en/statusline) command:
  `rate_limits.five_hour` and `rate_limits.seven_day`, each with
  `used_percentage` and `resets_at`.
- **What `--setup-claude` does:** it sets `trayce --claude-statusline` as that
  command in `~/.claude/settings.json`, after backing the file up to
  `settings.json.trayce-backup`.
  - On each Claude reply, Trayce saves the numbers for the tray. Claude Code's
    status bar also shows a short `5h 42% · 7d 18%`.
  - These are the same percentages Claude Code shows, for Pro and Max plans.
- **If you already have a status line:** Trayce leaves it alone. Pipe the same
  JSON into `trayce --claude-statusline` from your own script instead.
- **To undo:** run `/statusline delete` in Claude Code.

**Without the status line, Trayce falls back to the logs:**

- **Tokens:** Trayce parses the JSONL session logs Claude Code writes (the same
  source as `ccusage`). It sums tokens into the active **5h** block and a
  rolling **7d** total.
- **What counts:** `input + output + cache_creation`. Cache *reads* are
  excluded: they are cheap and automatic, and would be about 97% of the
  number otherwise.
- **The 5h percentage is estimated from your limit hits.** When you hit your
  5h limit, Claude Code logs a `429 · resets …` event, and your token count at
  that moment becomes the learned limit.
  - A learned limit is trusted for 7 days.
  - A later 5h block that goes over it without a hit proves it too low, so it
    is dropped. The window then shows tokens again instead of a percentage
    over 100%.
  - Details:
    [docs/design/limits-and-calibration.md](docs/design/limits-and-calibration.md).
- **Why local logs:** the original app called Anthropic's undocumented usage
  endpoint with Claude Code's OAuth token. Anthropic's policy reserves those
  tokens for Claude Code and Claude.ai, and it enforces that on the server
  side, so Trayce only reads local logs.

### Codex CLI

Every Codex turn writes the server's own `rate_limits` snapshot to the rollout
log: `used_percent`, `window_minutes` and `resets_at`. Trayce shows the newest
snapshot from the last 7 days, so the percentages are the real ones.
If a window's reset time has passed since that snapshot, it shows `0%` until
Codex runs again.

### Antigravity

Trayce reads Antigravity in two layers:

1. **Always, from the CLI's data.**
   - Trayce reads each request's token counts from the `agy` CLI's
     conversation databases, opened read-only.
   - It sums them into rolling 5h / 7d windows (input + output + thinking;
     cache reads excluded) and adds per-model request counts.
   - The data is undocumented protobuf, so a future Antigravity release may
     break it. If that happens, Antigravity alone shows `logs unreadable`.
2. **The real %, while the desktop app is open.**
   - The Antigravity desktop app runs a local language server. Trayce asks it
     for the same quota buckets the app shows, on `127.0.0.1` only.
   - Buckets outside the main group (for example third-party models) appear as
     notes.
   - The answer is cached, so after you close the app you still see it
     ("last seen 2h ago"). A bucket whose reset time has passed shows `0%`.

Trayce doesn't always show a real % because the quota lives only on Google's
servers:
- The `agy` CLI's own local server requires a token the CLI does not expose.
- `agy -p /usage` is sent to the model as a normal prompt, which spends quota.
- Calling Google directly with the Antigravity OAuth token is what got
  accounts banned in 2026.

**Trayce never reads `oauth_creds.json`.**

## Privacy

- **What it reads:** local logs and databases that contain your prompts and
  code. From them Trayce extracts only token counts, limit state and model names.
- **Credentials:** it never reads or sends tokens or credentials.
- **Files:** it opens the tools' logs and databases read-only. The one
  exception is `trayce --setup-claude`, which edits Claude Code's
  `settings.json`, and only when you run it.
- **Network:** the one exception is a request to `127.0.0.1`, the Antigravity
  desktop app's own local server, and only while that app runs. Nothing leaves
  your machine.

## Limitations

- **Single device.** Each machine sees only its own local data. Usage from
  other machines, or from the web apps (claude.ai, ChatGPT, Gemini), is not
  counted, so the token totals are a lower bound.
- **Claude:**
  - Real percentages need the status line (`trayce --setup-claude`) and a
    Pro/Max plan. They are as fresh as Claude Code's last reply.
  - The log fallback is an estimate: tokens are summed flat, although Opus
    weighs more than Sonnet against the limit, and the 7d window stays in
    tokens.
- **Codex:** the numbers are as fresh as your last Codex turn.
- **Antigravity:**
  - A real % needs the desktop app, not just the CLI.
  - The data formats are undocumented and may change.
  - Some users report the backend returning a fixed 100% remaining.

Tested with Claude Code 2.1.x, Codex CLI 0.154, and Antigravity CLI (`agy`)
1.2.10. CI builds and tests on macOS, Linux and Windows. Log formats change
between releases. If a tool stops updating, run `trayce --diagnose` and
[open an issue](https://github.com/hallygtree/trayce/issues) with the output.

## Development

```bash
cargo test                    # unit tests (parsers use real-format fixtures)
cargo clippy --all-targets
cargo run -- --once all
```

```text
src/
  main.rs              CLI dispatch, launches the tray
  providers.rs         the supported tools and their collectors
  config.rs            enabled tools + display mode, persisted
  usage.rs             usage model (Window, Report)
  tray.rs              tray icon, menu, 60s poll loop
  render.rs            colours, formatting, icon generation
  logs.rs              Claude: JSONL parsing, 5h/7d aggregation
  calibration.rs       Claude: learned plan limit
  codex.rs             Codex: rate_limits snapshots
  antigravity.rs       Antigravity: conversation DBs, quota cache
  antigravity_live.rs  Antigravity: desktop app's local quota server
  autostart.rs         per-OS login auto-start
```

Built with [`tao`](https://crates.io/crates/tao) and
[`tray-icon`](https://crates.io/crates/tray-icon). The logic sits apart from
the I/O, so each parser can be tested with string fixtures.

## Credits and license

Trayce builds on [claude-usage-bar](https://github.com/sammyjdev/claude-usage-bar)
by [@sammyjdev](https://github.com/sammyjdev), which provided the tray app, the
Claude log parsing and auto-calibration. The Antigravity local-server approach
follows [CodexBar](https://github.com/steipete/CodexBar).

[MIT](LICENSE)
