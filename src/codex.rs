//! Local Codex CLI source.
//!
//! Codex appends `event_msg` / `token_count` events to
//! `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. Each carries the server's
//! own `rate_limits` snapshot (`used_percent`, `window_minutes`, `resets_at`),
//! so unlike Claude we get a real percent with no calibration. The newest
//! snapshot across recent rollouts wins.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use crate::error::WidgetError;
use crate::usage::{Report, Window};

/// Rollouts older than this cannot hold a snapshot for a live weekly window.
const MAX_AGE: Duration = Duration::from_secs(7 * 86400);

fn sessions_dir() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
        .map(|d| d.join("sessions"))
        .filter(|p| p.is_dir())
}

fn rollouts(dir: &Path, cutoff: SystemTime, out: &mut Vec<(SystemTime, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rollouts(&path, cutoff, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            if let Ok(m) = entry.metadata().and_then(|m| m.modified()) {
                if m >= cutoff {
                    out.push((m, path));
                }
            }
        }
    }
}

/// `(event timestamp, rate_limits)` of the last snapshot in a rollout body.
fn last_snapshot(body: &str) -> Option<(String, Value)> {
    body.lines().rev().find_map(|line| {
        if !line.contains("\"rate_limits\"") {
            return None;
        }
        let v: Value = serde_json::from_str(line).ok()?;
        let rl = v.pointer("/payload/rate_limits")?;
        rl.get("primary")?.as_object()?;
        Some((v.get("timestamp")?.as_str()?.to_string(), rl.clone()))
    })
}

fn label(minutes: u64) -> String {
    match minutes {
        10080 => "Weekly (7d)".to_string(),
        m if m >= 1440 && m % 1440 == 0 => format!("{}d window", m / 1440),
        m if m % 60 == 0 => format!("{}h window", m / 60),
        m => format!("{m}m window"),
    }
}

fn window(w: &Value, now: DateTime<Utc>) -> Option<(String, Window, bool)> {
    let minutes = w.get("window_minutes")?.as_u64()?;
    let reset = w
        .get("resets_at")
        .and_then(Value::as_i64)
        .and_then(|s| Utc.timestamp_opt(s, 0).single());
    // A snapshot taken before the reset says nothing about the new window.
    let expired = reset.is_some_and(|r| r <= now);
    let pct = if expired {
        0
    } else {
        w.get("used_percent")?.as_f64()?.round() as u32
    };
    let window = Window {
        tokens: 0,
        pct,
        calibrated: true,
        resets_at: reset.filter(|_| !expired).map(|r| r.to_rfc3339()),
    };
    Some((label(minutes), window, minutes >= 1440))
}

/// Turn one `rate_limits` snapshot into a report. Pure, for tests.
fn report(rl: &Value, seen_at: &str, now: DateTime<Utc>) -> Report {
    let mut r = Report::default();
    for key in ["primary", "secondary"] {
        if let Some((label, w, weekday)) = rl.get(key).and_then(|w| window(w, now)) {
            r.row(&label, w, weekday);
        }
    }
    if let Some(plan) = rl.get("plan_type").and_then(Value::as_str) {
        r.notes.push(format!("Plan: {plan}"));
    }
    if let Ok(t) = DateTime::parse_from_rfc3339(seen_at) {
        let mins = (now - t.with_timezone(&Utc)).num_minutes().max(0);
        r.notes.push(format!("Last Codex activity: {}", ago(mins)));
    }
    r
}

pub fn ago(mins: i64) -> String {
    match mins {
        0 => "just now".to_string(),
        m if m < 60 => format!("{m}m ago"),
        m if m < 1440 => format!("{}h {}m ago", m / 60, m % 60),
        m => format!("{}d ago", m / 1440),
    }
}

pub fn collect(now: DateTime<Utc>) -> Result<Report, WidgetError> {
    let dir = sessions_dir().ok_or(WidgetError::LogsNotFound)?;
    let mut files = Vec::new();
    rollouts(&dir, SystemTime::now() - MAX_AGE, &mut files);
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    // Newest file with a snapshot wins; a fresh session may not have one yet.
    let (seen_at, rl) = files
        .iter()
        .find_map(|(_, p)| fs::read_to_string(p).ok().and_then(|b| last_snapshot(&b)))
        .ok_or(WidgetError::LogsNotFound)?;
    Ok(report(&rl, &seen_at, now))
}

/// `(sessions dir, recent rollout files)` for `--diagnose`.
pub fn diagnose() -> (Option<String>, usize) {
    let Some(dir) = sessions_dir() else {
        return (None, 0);
    };
    let mut files = Vec::new();
    rollouts(&dir, SystemTime::now() - MAX_AGE, &mut files);
    (Some(dir.display().to_string()), files.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real line from a Codex 0.154 rollout (session ids trimmed).
    const LINE: &str = r#"{"timestamp":"2026-09-24T12:06:13.290Z","ordinal":18,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":15220,"cached_input_tokens":12160,"output_tokens":173,"total_tokens":15393}},"rate_limits":{"limit_id":"codex","limit_name":null,"primary":{"used_percent":33.0,"window_minutes":300,"resets_at":1790269549},"secondary":{"used_percent":21.0,"window_minutes":10080,"resets_at":1790685734},"credits":{"has_credits":false,"unlimited":false,"balance":"0"},"plan_type":"plus"}}}"#;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn parses_real_snapshot() {
        let body = format!("{{\"type\":\"session_meta\"}}\n{LINE}\n{{\"type\":\"other\"}}\n");
        let (ts, rl) = last_snapshot(&body).unwrap();
        let r = report(&rl, &ts, at("2026-09-24T12:30:00Z"));
        assert_eq!(r.rows.len(), 2);
        assert_eq!(r.rows[0].label, "5h window");
        assert_eq!(r.rows[0].window.pct, 33);
        assert!(r.rows[0].window.calibrated && !r.rows[0].weekday);
        assert_eq!(r.rows[1].label, "Weekly (7d)");
        assert_eq!(r.rows[1].window.pct, 21);
        assert!(r.rows[1].weekday && r.rows[1].window.resets_at.is_some());
        assert_eq!(r.notes, vec!["Plan: plus", "Last Codex activity: 23m ago"]);
    }

    #[test]
    fn expired_window_reads_zero() {
        let (ts, rl) = last_snapshot(LINE).unwrap();
        // After the 5h reset (1790269549 = 2026-09-24T17:05:49Z) but before the weekly one.
        let r = report(&rl, &ts, at("2026-09-24T17:30:00Z"));
        assert_eq!(r.rows[0].window.pct, 0);
        assert!(r.rows[0].window.resets_at.is_none());
        assert_eq!(r.rows[1].window.pct, 21);
    }

    #[test]
    fn skips_null_rate_limits() {
        let body =
            format!("{LINE}\n{{\"timestamp\":\"x\",\"payload\":{{\"rate_limits\":null}}}}\n");
        assert!(last_snapshot(&body).is_some());
        assert!(last_snapshot("{\"payload\":{\"rate_limits\":null}}").is_none());
    }

    #[test]
    fn labels() {
        assert_eq!(label(300), "5h window");
        assert_eq!(label(10080), "Weekly (7d)");
        assert_eq!(label(1440), "1d window");
        assert_eq!(label(90), "90m window");
    }
}
