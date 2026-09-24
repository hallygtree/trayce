# Limits and auto-calibration

Design document for contributors. Why the app measures what it measures, what it
cannot know, and how auto-calibration recovers the percentage the old version got
from the OAuth endpoint (retired for violating Anthropic's Usage Policy, see
`../superpowers/adr-2026-06-01-local-logs-over-oauth-endpoint.md`).

## The core problem

Usage percentage = `consumption / limit`. Local logs give the **consumption**
(token sum). They do not give the **limit**: Anthropic does not publish the
subscription plan limits in token counts, and the real limit:

- is counted in weighted "messages", not tokens;
- weights by model (Opus counts ~12x Sonnet);
- varies by message size, attachments, and feature;
- changes over time (e.g. May 2026 doubled the Max 5h window; March 2026 reduced
  it during peak hours 5-11am PT);
- is shared across Claude Code, Claude.ai, and Cowork.

So we **do not hardcode limits**. The only approach that scales across the many
plans is auto-calibration: learn each user's own limit from the limit-hit events
Claude Code already records in its logs.

## Plans (June 2026)

| Plan | 5h session | Weekly | Note |
|------|-----------|--------|------|
| Free | yes | no | 5h only |
| Pro | yes | yes | ~5x Free per session |
| Max 5x ($100) | yes | yes | 5x Pro |
| Max 20x ($200) | yes | yes | 20x Pro |
| API/Console | no | no | credit-based, no 5h session |
| Team/Enterprise | varies | varies | treat as not calibrated |

## What is exact vs inferred

Only the denominator is inferred. Everything else is read directly:

| Value | How we get it | Precision |
|-------|---------------|-----------|
| Your consumption (tokens per window) | read from the log | exact |
| The moment you were throttled | `429` event in the log | exact |
| Official reset ("3:20am") | limit-event text | exact (when an event exists) |
| The plan limit (the denominator) | inferred from consumption at hit time | estimated |

## The calibration signal

When you hit the cap, Claude Code writes a line to the JSONL:

```
type: assistant   isApiErrorMessage: true   apiErrorStatus: 429
message.content[].text: "You've hit your session limit . resets 3:20am (America/Fortaleza)"
```

Observed variants:

- `You've hit your session limit . resets <time>` -> **5h** limit.
- `You're out of extra usage . resets <time>` -> **extra credits** pool (Max only).
- **weekly** limit: string not yet observed in the test logs. Detection is
  written speculatively and marked unverified.

Reset formats seen: `3:20am`, `4am`, `1:20pm`, `5:20pm`, always with
`(America/Fortaleza)`. The time is already in the user's timezone, so the app
does not convert: it shows the text as-is.

## How calibration works

1. While scanning the logs, besides usage events, we detect limit events
   (`isApiErrorMessage` + `apiErrorStatus == 429` + text classification).
2. For each **session limit** event at time T, we sum the tokens of the 5h block
   that contains T. That value is a sample of the real 5h limit.
3. We keep the **most recent** sample as `five_hour_limit`, persisted in
   `<data_dir>/claude-usage-bar/calibration.json`, because the event can age out
   of the 7-day window we scan.
4. From then on, `5h % = current_block_tokens / five_hour_limit`. Without
   calibration we fall back to the heuristic cap and mark the window as not
   calibrated.
5. A learned limit has a **7-day TTL** (`CALIBRATION_TTL`). Samples older than
   that are ignored when learning, and a stored limit older than that is not
   used. A single old sample is noisy and limits move, so a stale calibration
   reverts to the tokens view rather than driving a misleading percent. The
   percent is therefore freshest right after a throttle, and fades to tokens
   until the next hit.

Denominator resolution, in order: user manual override (future) -> learned
limit -> heuristic cap (marked "not calibrated"). The absolute token number is
always available; we never show a fake precise percentage.

## Learning is passive (this is what keeps it within the guidelines)

The app never probes. It does not call any endpoint, never spends quota to
"test", and never touches the token. It only reads events that already happened
naturally while you worked. Observing and never probing is precisely why this
stays within Anthropic's rules. There is no legal "active" version of this.

## It converges, it does not "validate"

This does not become the exact official number the endpoint gave. It **converges
toward a good estimate**, with a residual error that cannot be eliminated:

- **model weighting:** a flat token sum never maps perfectly to the weighted limit;
- **shared bucket:** usage on claude.ai/Cowork counts against the same limit but
  is not in the local logs, so measured consumption is always a lower bound;
- **moving target:** Anthropic adjusts limits and reduces them at peak hours.

Each hit refines the estimate, but there is a noise floor. It gets "good enough",
not "correct".

## Irreducible limitations (document for the user)

- **Needs one hit to calibrate.** Before that, tokens only.
- **Shared bucket:** see above.
- **Multi-device:** each machine sees only its own logs; calibration is per-machine.
- **Model weighting:** tokens are summed flat; the percentage is faithful only
  when the model mix is stable between calibration and current use. Per-model
  weighting is a v2 item.
- **5h block anchoring:** the exact reset is only reliable when it comes from a
  limit-event text. Between events the block is approximate (the "new block after
  a 5h gap" rule does not exactly reproduce the observed reset).
- **Limit is a moving target:** Anthropic adjusts limits and reduces them at peak
  hours. Calibration re-learns on the next hit, but lags until then.
- **Plan change:** old calibration (e.g. Pro) is too low after an upgrade (Max)
  until the next hit; the percentage is clamped and re-learns.

## Parsing resilience (the #1 OSS risk)

Everything that depends on the log format is centralized and tolerant:

- a malformed line is skipped with a counter, never aborts;
- the JSONL schema can change between Claude Code versions; missing fields
  degrade instead of breaking;
- `--diagnose` reports what was found (logs, usage events, limit events,
  calibration state) for support and for contributors to send samples from other
  plans, locales, and versions;
- the README lists the tested Claude Code versions.

## Privacy

The app reads all project logs (which contain code and prompts), but extracts
only token counts and limit state. **Nothing leaves the machine**, there is no
network, access is read-only. This is part of the trust contract of an OSS
project and must be stated in the README.

## Roadmap

- v1 (this delivery): limit-event detection, 5h calibration, real percentage or
  tokens with a "not calibrated" marker, `--diagnose`, persistence.
- v2: manual override via config + community defaults per plan; weekly
  calibration once the string is confirmed; per-model weighting; use the official
  reset from the event in the UI; incremental read cache.
