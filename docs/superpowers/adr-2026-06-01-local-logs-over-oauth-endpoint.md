# ADR: move from the OAuth endpoint to local log parsing

**Date:** 2026-06-01
**Status:** Accepted
**Supersedes:** the data source described in the design and plan from 2026-05-21

## Context

The first version read Claude Code's OAuth token (Keychain on macOS, the
`.credentials.json` file on Linux/Windows) and called the undocumented
`api.anthropic.com/api/oauth/usage` endpoint. Anthropic's Usage Policy reserves
the Free/Pro/Max OAuth tokens for Claude Code and Claude.ai; using them from any
other tool violates the Consumer Terms, with server-side enforcement active
since January 2026. Distributing the app in that form put users' accounts at
risk of a ban.

## Decision

Replace the data source with parsing of the local JSONL logs that Claude Code
already writes under `~/.claude/projects/**/*.jsonl` (the same approach as
`ccusage`). The app stops making any network call and stops touching the token
or credentials.

Details resolved in the refactor:

- **Usage expression:** absolute tokens per window (the endpoint gave a percent
  of the plan limit; the logs only give tokens, and the plan limit is not
  published). Colour and bar use heuristic caps configurable in `src/logs.rs`,
  refined by auto-calibration (see `../design/limits-and-calibration.md`).
- **Token composition:** `input + output + cache_creation`. Cache reads are
  excluded: in real data they are ~97% of the total and would drown out actual
  consumption.
- **5h window:** `ccusage`-style anchored block (start floored to the hour of
  the block's first event, 5h duration), which gives a real `resets_at`.
- **7d window:** a simple rolling sum, with no log-derivable reset.

## Consequences

- Removed `src/client.rs`, `src/token/`, the `ureq` dependency, and the OAuth header.
- `objc2-core-foundation` stays: still used by `wake_macos`.
- `WidgetError` reduced to `LogsNotFound` (UI state `⚠ logs`).
- Fault-tolerant parsing: a malformed line is skipped with a counter, never aborts.
- **Single-device limitation:** the result is accurate for one machine. With use
  across several machines the totals cover only the local logs, and the 5h block
  boundary (floored to the hour of the first local event) can diverge from what
  the account actually metered. Documented in the README.
- Added auto-calibration (`src/calibration.rs`): the plan limit is learned from
  the limit-hit events Claude Code logs, recovering a real percentage without
  the endpoint. See the design document.
