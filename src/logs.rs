//! Local Claude Code log source.
//!
//! Claude Code appends one JSON object per line to
//! `~/.claude/projects/<slug>/<session>.jsonl`. Assistant turns carry a
//! `message.usage` block with token counts; limit-hit turns carry
//! `isApiErrorMessage` with a 429 status and a human-readable reset note. We
//! aggregate usage into a 5h active block and a rolling 7d total, and detect
//! limit-hit events to calibrate the real plan limit, entirely offline.
//!
//! I/O (`scan`, `collect`) is kept separate from logic (`parse_all`,
//! `aggregate`, `blocks`, `learn_five_hour`) so the logic is unit-testable with
//! string fixtures, matching the pattern in `render.rs`.
//!
//! See `docs/design/limits-and-calibration.md` for the full rationale and the
//! known limitations (multi-device, shared bucket, model weighting, anchoring).

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration as StdDuration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, Timelike, Utc};

use crate::calibration::{self, Calibration};
use crate::error::WidgetError;
use crate::usage::{Usage, Window};

const FIVE_HOURS: i64 = 5 * 3600;
const SEVEN_DAYS: i64 = 7 * 86400;

/// A learned limit is trusted only this long after the hit that produced it. A
/// single old sample is noisy and limits are a moving target, so once the
/// calibration goes stale we revert to showing tokens until the next hit.
const CALIBRATION_TTL: i64 = SEVEN_DAYS;

/// Heuristic colour caps (tokens), used only when no learned limit exists. NOT
/// published plan limits; they only scale the tray colour and the ASCII bar
/// before calibration. The token number shown is always the real count.
const FIVE_HOUR_CAP: u64 = 5_000_000;
const SEVEN_DAY_CAP: u64 = 50_000_000;

/// Learned limits applied when computing percentages. `None` means fall back to
/// the heuristic cap and mark the window as not calibrated.
#[derive(Debug, Clone, Copy, Default)]
pub struct Limits {
    pub five_hour: Option<u64>,
    pub weekly: Option<u64>,
}

/// One assistant turn's token usage, parsed from a log line.
#[derive(Debug, Clone)]
struct Event {
    ts: DateTime<Utc>,
    model: String,
    tokens: u64,
    /// Dedup key parts: the same response can be logged twice (stream + retry).
    id: String,
    request_id: String,
}

/// Which limit a limit-hit event refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    Session,
    Weekly,
    ExtraUsage,
}

impl LimitKind {
    pub fn label(self) -> &'static str {
        match self {
            LimitKind::Session => "session (5h)",
            LimitKind::Weekly => "weekly (7d)",
            LimitKind::ExtraUsage => "extra usage",
        }
    }
}

/// A limit-hit event: the moment the account was throttled, plus the reset note
/// Claude Code printed (already in the user's local timezone).
#[derive(Debug, Clone)]
struct LimitEvent {
    ts: DateTime<Utc>,
    kind: LimitKind,
    reset_label: Option<String>,
}

/// Outcome of parsing a single line.
enum Parsed {
    Event(Box<Event>),
    Limit(Box<LimitEvent>),
    Skip,
    Bad,
}

/// Extract a plain-text body from `message.content`, which may be a string or a
/// list of `{type, text}` parts.
fn message_text(msg: &serde_json::Value) -> String {
    match msg.get("content") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// Pull the reset note out of "... resets 3:20am (America/Fortaleza)".
fn parse_reset_label(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let pos = lower.find("resets ")?;
    let label = text[pos + "resets ".len()..].trim();
    (!label.is_empty()).then(|| label.to_string())
}

fn classify_limit(v: &serde_json::Value) -> Parsed {
    // Only genuine rate-limit errors (429). 401/500/529 are auth/server issues.
    if v.get("apiErrorStatus").and_then(|s| s.as_i64()) != Some(429) {
        return Parsed::Skip;
    }
    let Some(msg) = v.get("message") else {
        return Parsed::Skip;
    };
    let text = message_text(msg);
    let low = text.to_lowercase();
    let kind = if low.contains("session limit") {
        LimitKind::Session
    } else if low.contains("weekly limit") {
        // String not yet confirmed in real logs; speculative, see design doc.
        LimitKind::Weekly
    } else if low.contains("out of extra usage") {
        LimitKind::ExtraUsage
    } else {
        return Parsed::Skip;
    };
    let Some(ts) = v.get("timestamp").and_then(|t| t.as_str()) else {
        return Parsed::Skip;
    };
    let Ok(ts) = DateTime::parse_from_rfc3339(ts) else {
        return Parsed::Bad;
    };
    Parsed::Limit(Box::new(LimitEvent {
        ts: ts.with_timezone(&Utc),
        kind,
        reset_label: parse_reset_label(&text),
    }))
}

fn parse_line(line: &str) -> Parsed {
    let line = line.trim();
    if line.is_empty() {
        return Parsed::Skip;
    }
    let v: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return Parsed::Bad,
    };
    if v.get("isApiErrorMessage").and_then(|b| b.as_bool()) == Some(true) {
        return classify_limit(&v);
    }
    if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
        return Parsed::Skip;
    }
    let Some(msg) = v.get("message") else {
        return Parsed::Skip;
    };
    let Some(model) = msg.get("model").and_then(|m| m.as_str()) else {
        return Parsed::Skip;
    };
    if model.starts_with("<synthetic") {
        return Parsed::Skip;
    }
    let Some(usage) = msg.get("usage") else {
        return Parsed::Skip;
    };
    // Composition: input + output + cache creation. Cache *reads* are excluded
    // on purpose: they are cheap, automatic, and would dominate the total
    // (~97% of raw tokens), drowning out actual consumption.
    let field = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let tokens =
        field("input_tokens") + field("output_tokens") + field("cache_creation_input_tokens");
    let Some(ts) = v.get("timestamp").and_then(|t| t.as_str()) else {
        return Parsed::Skip;
    };
    let Ok(ts) = DateTime::parse_from_rfc3339(ts) else {
        return Parsed::Bad;
    };
    Parsed::Event(Box::new(Event {
        ts: ts.with_timezone(&Utc),
        model: model.to_string(),
        tokens,
        id: msg.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        request_id: v.get("requestId").and_then(|x| x.as_str()).unwrap_or("").to_string(),
    }))
}

/// Parse a whole file body. Returns usage events, limit events, and a count of
/// malformed lines (skipped, never fatal).
fn parse_all(text: &str) -> (Vec<Event>, Vec<LimitEvent>, usize) {
    let mut events = Vec::new();
    let mut limits = Vec::new();
    let mut bad = 0usize;
    for line in text.lines() {
        match parse_line(line) {
            Parsed::Event(e) => events.push(*e),
            Parsed::Limit(l) => limits.push(*l),
            Parsed::Bad => bad += 1,
            Parsed::Skip => {}
        }
    }
    (events, limits, bad)
}

fn floor_hour(t: DateTime<Utc>) -> DateTime<Utc> {
    t.with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(t)
}

/// A 5h session block: anchored to the floored hour of its first event. The
/// exact anchor is an approximation (see design doc); the authoritative reset
/// only appears in limit-event text.
#[derive(Debug, Clone)]
struct Block {
    start: DateTime<Utc>,
    tokens: u64,
}

/// Dedup by (id, request_id), then sort ascending by timestamp.
fn prepare(events: Vec<Event>) -> Vec<Event> {
    let mut seen = HashSet::new();
    let mut events: Vec<Event> = events
        .into_iter()
        .filter(|e| {
            if e.id.is_empty() && e.request_id.is_empty() {
                true
            } else {
                seen.insert((e.id.clone(), e.request_id.clone()))
            }
        })
        .collect();
    events.sort_by_key(|e| e.ts);
    events
}

/// Group prepared (deduped, sorted) events into 5h blocks. A new block starts
/// when an event is 5h past the block start, or 5h after the previous event.
fn blocks(events: &[Event]) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut last_ts: Option<DateTime<Utc>> = None;
    for e in events {
        let new_block = match (out.last(), last_ts) {
            (Some(b), Some(lt)) => {
                (e.ts - b.start).num_seconds() >= FIVE_HOURS
                    || (e.ts - lt).num_seconds() >= FIVE_HOURS
            }
            _ => true,
        };
        if new_block {
            out.push(Block {
                start: floor_hour(e.ts),
                tokens: e.tokens,
            });
        } else if let Some(b) = out.last_mut() {
            b.tokens += e.tokens;
        }
        last_ts = Some(e.ts);
    }
    out
}

/// The block whose 5h span contains `t`, if any.
fn block_covering(blocks: &[Block], t: DateTime<Utc>) -> Option<&Block> {
    blocks
        .iter()
        .rev()
        .find(|b| t >= b.start && (t - b.start).num_seconds() < FIVE_HOURS)
}

/// Learn the 5h limit from session-limit events: the token sum of the block
/// active when the limit was hit, taking the most recent hit within the TTL.
/// Pure. Hits older than `CALIBRATION_TTL` are ignored as stale.
fn learn_five_hour(
    blocks: &[Block],
    limit_events: &[LimitEvent],
    now: DateTime<Utc>,
) -> Option<(u64, DateTime<Utc>)> {
    let cutoff = now - Duration::seconds(CALIBRATION_TTL);
    limit_events
        .iter()
        .filter(|e| e.kind == LimitKind::Session)
        .filter(|e| e.ts >= cutoff)
        .filter_map(|e| block_covering(blocks, e.ts).map(|b| (b.tokens, e.ts)))
        .filter(|(tokens, _)| *tokens > 0)
        .max_by_key(|(_, ts)| *ts)
}

/// The stored 5h limit, but only if it was learned within `CALIBRATION_TTL`.
fn fresh_five_hour(cal: &Calibration, now: DateTime<Utc>) -> Option<u64> {
    let updated = cal
        .five_hour_updated
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())?
        .with_timezone(&Utc);
    if (now - updated).num_seconds() <= CALIBRATION_TTL {
        cal.five_hour_limit
    } else {
        None
    }
}

fn make_window(tokens: u64, limit: Option<u64>, cap: u64, resets_at: Option<String>) -> Window {
    let denom = limit.unwrap_or(cap).max(1);
    let pct = ((tokens as f64 / denom as f64) * 100.0).round() as u32;
    Window {
        tokens,
        pct,
        calibrated: limit.is_some(),
        resets_at,
    }
}

fn aggregate_prepared(
    events: &[Event],
    blks: &[Block],
    now: DateTime<Utc>,
    limits: Limits,
) -> Usage {
    let active = block_covering(blks, now);
    let five_tokens = active.map(|b| b.tokens).unwrap_or(0);
    let five_reset =
        active.map(|b| (b.start + Duration::seconds(FIVE_HOURS)).to_rfc3339());
    let five_hour = make_window(five_tokens, limits.five_hour, FIVE_HOUR_CAP, five_reset);

    let cutoff = now - Duration::seconds(SEVEN_DAYS);
    let mut seven = 0u64;
    let mut opus = 0u64;
    let mut sonnet = 0u64;
    for e in events {
        if e.ts >= cutoff && e.ts <= now {
            seven += e.tokens;
            if e.model.contains("opus") {
                opus += e.tokens;
            } else if e.model.contains("sonnet") {
                sonnet += e.tokens;
            }
        }
    }

    Usage {
        five_hour,
        seven_day: make_window(seven, limits.weekly, SEVEN_DAY_CAP, None),
        seven_day_opus: (opus > 0).then(|| make_window(opus, None, SEVEN_DAY_CAP, None)),
        seven_day_sonnet: (sonnet > 0).then(|| make_window(sonnet, None, SEVEN_DAY_CAP, None)),
    }
}

/// Aggregate raw events into a [`Usage`]. Pure and deterministic given `now`.
/// Convenience wrapper that prepares and blocks the events; used by tests.
#[cfg(test)]
fn aggregate(events: Vec<Event>, now: DateTime<Utc>, limits: Limits) -> Usage {
    let events = prepare(events);
    let blks = blocks(&events);
    aggregate_prepared(&events, &blks, now, limits)
}

/// Resolve the `projects` log directory: `$CLAUDE_CONFIG_DIR/projects` if set,
/// otherwise `~/.claude/projects`. Same path on macOS, Linux, and Windows
/// (Claude Code uses `~/.claude` on all three). The non-macOS path is portable
/// via `dirs::home_dir`; verify on those machines.
fn logs_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        let p = PathBuf::from(dir).join("projects");
        if p.is_dir() {
            return Some(p);
        }
    }
    dirs::home_dir()
        .map(|h| h.join(".claude").join("projects"))
        .filter(|p| p.is_dir())
}

/// Collect `.jsonl` files under `dir` (recursively), skipping any whose mtime
/// predates the 7d cutoff since they cannot affect either window.
fn jsonl_files(dir: &PathBuf, cutoff: SystemTime) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(jsonl_files(&path, cutoff));
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let fresh = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|m| m >= cutoff)
                .unwrap_or(true);
            if fresh {
                files.push(path);
            }
        }
    }
    files
}

/// Result of reading the log directory.
struct Scan {
    dir: PathBuf,
    files: usize,
    events: Vec<Event>,
    limit_events: Vec<LimitEvent>,
    malformed: usize,
}

fn scan(now: DateTime<Utc>) -> Result<Scan, WidgetError> {
    let dir = logs_dir().ok_or(WidgetError::LogsNotFound)?;
    let cutoff = UNIX_EPOCH
        + StdDuration::from_secs((now - Duration::seconds(SEVEN_DAYS)).timestamp().max(0) as u64);

    let mut events = Vec::new();
    let mut limit_events = Vec::new();
    let mut malformed = 0usize;
    let paths = jsonl_files(&dir, cutoff);
    let files = paths.len();
    for path in paths {
        if let Ok(body) = fs::read_to_string(&path) {
            let (mut evs, mut lim, bad) = parse_all(&body);
            events.append(&mut evs);
            limit_events.append(&mut lim);
            malformed += bad;
        }
    }
    Ok(Scan {
        dir,
        files,
        events,
        limit_events,
        malformed,
    })
}

/// Apply newly observed session-limit hits to the calibration, persisting if a
/// more recent hit was found. Returns the (possibly updated) calibration.
fn update_calibration(
    mut cal: Calibration,
    blks: &[Block],
    limit_events: &[LimitEvent],
    now: DateTime<Utc>,
) -> Calibration {
    if let Some((limit, ts)) = learn_five_hour(blks, limit_events, now) {
        let newer = cal
            .five_hour_updated
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|prev| ts > prev.with_timezone(&Utc))
            .unwrap_or(true);
        if newer {
            cal.five_hour_limit = Some(limit);
            cal.five_hour_updated = Some(ts.to_rfc3339());
            calibration::save(&cal);
        }
    }
    cal
}

/// Read every recent log file, calibrate, aggregate, and return the usage.
/// `Err` when the directory is missing or holds no usage events.
pub fn collect(now: DateTime<Utc>) -> Result<Usage, WidgetError> {
    let scan = scan(now)?;
    if scan.malformed > 0 {
        eprintln!("logs: skipped {} malformed line(s)", scan.malformed);
    }
    if scan.events.is_empty() {
        return Err(WidgetError::LogsNotFound);
    }

    let prepared = prepare(scan.events);
    let blks = blocks(&prepared);
    let cal = update_calibration(calibration::load(), &blks, &scan.limit_events, now);
    let limits = Limits {
        five_hour: fresh_five_hour(&cal, now),
        weekly: cal.weekly_limit,
    };
    Ok(aggregate_prepared(&prepared, &blks, now, limits))
}

/// Human-readable diagnostics for the `--diagnose` command and OSS support.
pub struct Diagnostics {
    pub dir: Option<String>,
    pub files: usize,
    pub usage_events: usize,
    /// (kind label, RFC3339 timestamp, reset note) per detected limit event.
    pub limit_events: Vec<(String, String, Option<String>)>,
    pub malformed: usize,
    pub calibration: Calibration,
    /// Whether the stored 5h calibration is still within its TTL (drives the UI).
    pub five_hour_fresh: bool,
}

pub fn diagnose(now: DateTime<Utc>) -> Diagnostics {
    match scan(now) {
        Ok(scan) => {
            let prepared = prepare(scan.events);
            let blks = blocks(&prepared);
            let cal = update_calibration(calibration::load(), &blks, &scan.limit_events, now);
            let mut limit_events: Vec<_> = scan
                .limit_events
                .iter()
                .map(|e| (e.kind.label().to_string(), e.ts.to_rfc3339(), e.reset_label.clone()))
                .collect();
            limit_events.sort_by(|a, b| a.1.cmp(&b.1));
            Diagnostics {
                dir: Some(scan.dir.display().to_string()),
                files: scan.files,
                usage_events: prepared.len(),
                limit_events,
                malformed: scan.malformed,
                five_hour_fresh: fresh_five_hour(&cal, now).is_some(),
                calibration: cal,
            }
        }
        Err(_) => {
            let cal = calibration::load();
            Diagnostics {
                dir: None,
                files: 0,
                usage_events: 0,
                limit_events: Vec::new(),
                malformed: 0,
                five_hour_fresh: fresh_five_hour(&cal, now).is_some(),
                calibration: cal,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn ev(ts: &str, model: &str, tokens: u64, id: &str, req: &str) -> Event {
        Event {
            ts: at(ts),
            model: model.to_string(),
            tokens,
            id: id.to_string(),
            request_id: req.to_string(),
        }
    }

    #[test]
    fn parses_real_assistant_line() {
        let line = r#"{"type":"assistant","timestamp":"2026-05-30T17:27:53.120Z","requestId":"req_1","message":{"model":"claude-opus-4-8","id":"msg_1","usage":{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":10,"cache_read_input_tokens":5}}}"#;
        match parse_line(line) {
            Parsed::Event(e) => {
                // 100 + 50 + 10; the cache_read 5 is excluded by design.
                assert_eq!(e.tokens, 160);
                assert_eq!(e.model, "claude-opus-4-8");
                assert_eq!(e.id, "msg_1");
                assert_eq!(e.request_id, "req_1");
            }
            _ => panic!("expected an event"),
        }
    }

    #[test]
    fn skips_user_and_synthetic_lines() {
        assert!(matches!(
            parse_line(r#"{"type":"user","message":{"role":"user"}}"#),
            Parsed::Skip
        ));
        assert!(matches!(
            parse_line(r#"{"type":"assistant","timestamp":"2026-05-30T17:27:53Z","message":{"model":"<synthetic>","usage":{"output_tokens":1}}}"#),
            Parsed::Skip
        ));
    }

    #[test]
    fn parses_session_limit_event() {
        let line = r#"{"type":"assistant","timestamp":"2026-05-23T04:25:12.557Z","isApiErrorMessage":true,"apiErrorStatus":429,"message":{"role":"assistant","content":[{"type":"text","text":"You've hit your session limit . resets 3:20am (America/Fortaleza)"}]}}"#;
        match parse_line(line) {
            Parsed::Limit(l) => {
                assert_eq!(l.kind, LimitKind::Session);
                assert_eq!(l.reset_label.as_deref(), Some("3:20am (America/Fortaleza)"));
            }
            _ => panic!("expected a limit event"),
        }
    }

    #[test]
    fn ignores_non_429_api_errors() {
        let line = r#"{"isApiErrorMessage":true,"apiErrorStatus":401,"timestamp":"2026-05-23T04:25:12Z","message":{"content":[{"type":"text","text":"Please run /login . API Error: 401"}]}}"#;
        assert!(matches!(parse_line(line), Parsed::Skip));
    }

    #[test]
    fn counts_malformed_lines_without_aborting() {
        let body = concat!(
            "{not json}\n",
            r#"{"type":"assistant","timestamp":"2026-05-30T17:00:00Z","requestId":"r","message":{"model":"claude-opus-4-8","id":"a","usage":{"output_tokens":7}}}"#,
            "\n",
            "\n",
            "garbage line\n",
        );
        let (events, limits, bad) = parse_all(body);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tokens, 7);
        assert!(limits.is_empty());
        assert_eq!(bad, 2);
    }

    #[test]
    fn five_hour_block_anchors_to_floored_hour() {
        let now = at("2026-05-30T19:00:00Z");
        let events = vec![
            ev("2026-05-30T17:27:00Z", "claude-opus-4-8", 100, "a", "1"),
            ev("2026-05-30T18:15:00Z", "claude-opus-4-8", 50, "b", "2"),
        ];
        let u = aggregate(events, now, Limits::default());
        assert_eq!(u.five_hour.tokens, 150);
        assert!(!u.five_hour.calibrated);
        assert_eq!(u.five_hour.resets_at.as_deref(), Some("2026-05-30T22:00:00+00:00"));
    }

    #[test]
    fn inactive_block_reports_zero_five_hour() {
        let now = at("2026-05-31T02:00:00Z");
        let events = vec![ev("2026-05-30T17:00:00Z", "claude-opus-4-8", 100, "a", "1")];
        let u = aggregate(events, now, Limits::default());
        assert_eq!(u.five_hour.tokens, 0);
        assert!(u.five_hour.resets_at.is_none());
    }

    #[test]
    fn gap_over_five_hours_starts_new_block() {
        let now = at("2026-05-30T18:30:00Z");
        let events = vec![
            ev("2026-05-30T08:00:00Z", "claude-opus-4-8", 999, "a", "1"),
            ev("2026-05-30T18:00:00Z", "claude-opus-4-8", 42, "b", "2"),
        ];
        let u = aggregate(events, now, Limits::default());
        assert_eq!(u.five_hour.tokens, 42);
    }

    #[test]
    fn seven_day_rolling_splits_by_model() {
        let now = at("2026-05-30T12:00:00Z");
        let events = vec![
            ev("2026-05-29T12:00:00Z", "claude-opus-4-8", 100, "a", "1"),
            ev("2026-05-28T12:00:00Z", "claude-sonnet-4-6", 30, "b", "2"),
            ev("2026-05-20T12:00:00Z", "claude-opus-4-8", 500, "c", "3"),
        ];
        let u = aggregate(events, now, Limits::default());
        assert_eq!(u.seven_day.tokens, 130);
        assert_eq!(u.seven_day_opus.unwrap().tokens, 100);
        assert_eq!(u.seven_day_sonnet.unwrap().tokens, 30);
    }

    #[test]
    fn deduplicates_repeated_events() {
        let now = at("2026-05-30T12:30:00Z");
        let dup = ev("2026-05-30T12:00:00Z", "claude-opus-4-8", 100, "msg_x", "req_x");
        let u = aggregate(vec![dup.clone(), dup], now, Limits::default());
        assert_eq!(u.five_hour.tokens, 100);
        assert_eq!(u.seven_day.tokens, 100);
    }

    #[test]
    fn uncalibrated_pct_scales_against_cap() {
        let now = at("2026-05-30T12:30:00Z");
        let events = vec![ev(
            "2026-05-30T12:00:00Z",
            "claude-opus-4-8",
            FIVE_HOUR_CAP / 2,
            "a",
            "1",
        )];
        let u = aggregate(events, now, Limits::default());
        assert_eq!(u.five_hour.pct, 50);
        assert!(!u.five_hour.calibrated);
    }

    #[test]
    fn calibrated_pct_scales_against_learned_limit() {
        let now = at("2026-05-30T12:30:00Z");
        let events = vec![ev("2026-05-30T12:00:00Z", "claude-opus-4-8", 600_000, "a", "1")];
        let limits = Limits {
            five_hour: Some(1_000_000),
            weekly: None,
        };
        let u = aggregate(events, now, limits);
        assert_eq!(u.five_hour.pct, 60);
        assert!(u.five_hour.calibrated);
    }

    #[test]
    fn learns_five_hour_limit_from_session_hit() {
        let events = prepare(vec![
            ev("2026-05-23T00:30:00Z", "claude-opus-4-8", 700_000, "a", "1"),
            ev("2026-05-23T03:00:00Z", "claude-opus-4-8", 300_000, "b", "2"),
        ]);
        let blks = blocks(&events);
        let hit = LimitEvent {
            ts: at("2026-05-23T04:25:00Z"),
            kind: LimitKind::Session,
            reset_label: Some("3:20am".into()),
        };
        // Fresh hit: both events are in the same block (anchored 00:00).
        let learned =
            learn_five_hour(&blks, std::slice::from_ref(&hit), at("2026-05-23T05:00:00Z"));
        assert_eq!(learned.map(|(t, _)| t), Some(1_000_000));
        // Same hit seen 8 days later is stale and ignored.
        let stale = learn_five_hour(&blks, &[hit], at("2026-05-31T05:00:00Z"));
        assert!(stale.is_none());
    }

    #[test]
    fn fresh_five_hour_respects_ttl() {
        let cal = Calibration {
            five_hour_limit: Some(900_000),
            five_hour_updated: Some(at("2026-05-23T04:25:00Z").to_rfc3339()),
            ..Default::default()
        };
        assert_eq!(fresh_five_hour(&cal, at("2026-05-25T00:00:00Z")), Some(900_000));
        assert_eq!(fresh_five_hour(&cal, at("2026-06-01T00:00:00Z")), None);
    }

    #[test]
    fn empty_input_is_all_zero() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let u = aggregate(vec![], now, Limits::default());
        assert_eq!(u.five_hour.tokens, 0);
        assert_eq!(u.seven_day.tokens, 0);
        assert!(u.seven_day_opus.is_none());
        assert!(u.seven_day_sonnet.is_none());
    }
}
