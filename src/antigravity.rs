//! Local Antigravity source.
//!
//! Two layers:
//! 1. Always: per-request token counts from the CLI's conversation databases
//!    (`~/.gemini/antigravity-cli/conversations/<id>.db`, SQLite). The rows are
//!    undocumented protobuf, read with a minimal wire-format walker. Field map
//!    (verified against `agy --output-format json` usage, agy 1.2.10):
//!    `gen_metadata.data` → 1 { 4 { 2: input, 3: output, 5: cache read,
//!    9: thinking }, 19: model, 20: map<string,string> incl. `last_step_index` };
//!    `steps.metadata` → 1 { 1: unix seconds }.
//! 2. When the Antigravity desktop app is running: the real per-bucket quota
//!    from its local language server (`antigravity_live.rs`), cached on disk so
//!    the last reading survives the app closing.
//!
//! Never reads `oauth_creds.json` and never talks to Google directly: using the
//! Antigravity OAuth token from third-party tools got accounts banned in 2026.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration as StdDuration, SystemTime};

use chrono::{DateTime, Duration, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::antigravity_live;
use crate::codex::ago;
use crate::error::WidgetError;
use crate::render::format_tokens;
use crate::usage::{Report, Window};

const FIVE_HOURS: i64 = 5 * 3600;
const SEVEN_DAYS: i64 = 7 * 86400;
// ponytail: heuristic caps copied from the Claude source, only colour the tray
// until a live quota reading exists; the real limits are not published.
const FIVE_HOUR_CAP: u64 = 5_000_000;
const SEVEN_DAY_CAP: u64 = 50_000_000;

// ---- protobuf wire format -------------------------------------------------

enum Val<'a> {
    Int(u64),
    Bytes(&'a [u8]),
}

fn varint(b: &[u8], i: &mut usize) -> Option<u64> {
    let mut r = 0u64;
    for shift in (0..64).step_by(7) {
        let x = *b.get(*i)?;
        *i += 1;
        r |= u64::from(x & 0x7f) << shift;
        if x < 0x80 {
            return Some(r);
        }
    }
    None
}

/// Top-level fields of one message. `None` on malformed input.
fn fields(b: &[u8]) -> Option<Vec<(u64, Val<'_>)>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let key = varint(b, &mut i)?;
        let val = match key & 7 {
            0 => Val::Int(varint(b, &mut i)?),
            2 => {
                let n = varint(b, &mut i)? as usize;
                let v = b.get(i..i.checked_add(n)?)?;
                i += n;
                Val::Bytes(v)
            }
            1 => {
                i += 8;
                continue;
            }
            5 => {
                i += 4;
                continue;
            }
            _ => return None,
        };
        out.push((key >> 3, val));
    }
    Some(out)
}

fn sub(b: &[u8], field: u64) -> Option<&[u8]> {
    fields(b)?.into_iter().find_map(|(f, v)| match v {
        Val::Bytes(x) if f == field => Some(x),
        _ => None,
    })
}

fn int(b: &[u8], field: u64) -> u64 {
    fields(b)
        .and_then(|fs| {
            fs.into_iter().find_map(|(f, v)| match v {
                Val::Int(x) if f == field => Some(x),
                _ => None,
            })
        })
        .unwrap_or(0)
}

#[derive(Debug, PartialEq)]
struct Gen {
    model: String,
    tokens: u64,
    step: Option<i64>,
}

fn parse_gen(data: &[u8]) -> Option<Gen> {
    let root = sub(data, 1)?;
    let usage = sub(root, 4)?;
    // Input + output + thinking. Cache reads are excluded, matching the Claude
    // source (they dominate the total and are not what the quota meters).
    let tokens = int(usage, 2) + int(usage, 3) + int(usage, 9);
    let model = sub(root, 19)
        .map(|m| String::from_utf8_lossy(m).into_owned())
        .unwrap_or_else(|| "unknown".to_string());
    let step = fields(root)?.into_iter().find_map(|(f, v)| match v {
        Val::Bytes(entry) if f == 20 => {
            let key = sub(entry, 1)?;
            (key == b"last_step_index")
                .then(|| std::str::from_utf8(sub(entry, 2)?).ok()?.parse().ok())
                .flatten()
        }
        _ => None,
    });
    Some(Gen {
        model,
        tokens,
        step,
    })
}

fn step_time(metadata: &[u8]) -> Option<DateTime<Utc>> {
    let secs = int(sub(metadata, 1)?, 1);
    Utc.timestamp_opt(secs as i64, 0)
        .single()
        .filter(|_| secs > 0)
}

// ---- local scan -------------------------------------------------------------

struct Req {
    ts: DateTime<Utc>,
    model: String,
    tokens: u64,
}

fn conversations_dir() -> Option<PathBuf> {
    dirs::home_dir()
        .map(|h| {
            h.join(".gemini")
                .join("antigravity-cli")
                .join("conversations")
        })
        .filter(|p| p.is_dir())
}

fn read_db(path: &Path, bad: &mut usize) -> rusqlite::Result<Vec<Req>> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut steps = BTreeMap::new();
    let mut q = conn.prepare("SELECT idx, metadata FROM steps")?;
    let mut rows = q.query([])?;
    while let Some(row) = rows.next()? {
        let idx: i64 = row.get(0)?;
        let meta: Option<Vec<u8>> = row.get(1)?;
        if let Some(ts) = meta.as_deref().and_then(step_time) {
            steps.insert(idx, ts);
        }
    }
    let last = steps.values().max().copied();

    let mut out = Vec::new();
    let mut q = conn.prepare("SELECT data FROM gen_metadata")?;
    let mut rows = q.query([])?;
    while let Some(row) = rows.next()? {
        let data: Vec<u8> = row.get(0)?;
        let Some(g) = parse_gen(&data) else {
            *bad += 1;
            continue;
        };
        // Missing step link: fall back to the conversation's last activity.
        let Some(ts) = g.step.and_then(|s| steps.get(&s).copied()).or(last) else {
            *bad += 1;
            continue;
        };
        out.push(Req {
            ts,
            model: g.model,
            tokens: g.tokens,
        });
    }
    Ok(out)
}

struct Scan {
    dir: PathBuf,
    dbs: usize,
    reqs: Vec<Req>,
    bad: usize,
}

fn scan() -> Result<Scan, WidgetError> {
    let dir = conversations_dir().ok_or(WidgetError::LogsNotFound)?;
    let cutoff = SystemTime::now() - StdDuration::from_secs(SEVEN_DAYS as u64);
    let (mut dbs, mut reqs, mut bad) = (0, Vec::new(), 0);
    for entry in fs::read_dir(&dir)
        .map_err(|_| WidgetError::LogsNotFound)?
        .flatten()
    {
        let path = entry.path();
        let fresh = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|m| m >= cutoff);
        // The -wal sidecar holds the newest writes, so check its mtime too.
        let wal_fresh = fs::metadata(path.with_extension("db-wal"))
            .and_then(|m| m.modified())
            .map(|m| m >= cutoff);
        if path.extension().and_then(|e| e.to_str()) != Some("db")
            || !(fresh.unwrap_or(true) || wal_fresh.unwrap_or(false))
        {
            continue;
        }
        dbs += 1;
        match read_db(&path, &mut bad) {
            Ok(mut r) => reqs.append(&mut r),
            Err(e) => {
                eprintln!("antigravity: {}: {e}", path.display());
                bad += 1;
            }
        }
    }
    Ok(Scan {
        dir,
        dbs,
        reqs,
        bad,
    })
}

fn window(tokens: u64, cap: u64) -> Window {
    Window {
        tokens,
        pct: ((tokens as f64 / cap as f64) * 100.0).round() as u32,
        calibrated: false,
        resets_at: None,
    }
}

/// Local-only report: rolling 5h / 7d token totals plus per-model notes. Pure.
fn local_report(reqs: &[Req], now: DateTime<Utc>) -> Report {
    let in_window = |secs: i64| {
        let from = now - Duration::seconds(secs);
        reqs.iter().filter(move |r| r.ts > from && r.ts <= now)
    };
    let mut r = Report::default();
    r.row(
        "5h window",
        window(in_window(FIVE_HOURS).map(|q| q.tokens).sum(), FIVE_HOUR_CAP),
        false,
    );
    r.row(
        "Weekly (7d)",
        window(in_window(SEVEN_DAYS).map(|q| q.tokens).sum(), SEVEN_DAY_CAP),
        true,
    );
    let mut by_model: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
    for q in in_window(SEVEN_DAYS) {
        let e = by_model.entry(q.model.as_str()).or_default();
        e.0 += 1;
        e.1 += q.tokens;
    }
    for (model, (n, tok)) in by_model {
        r.notes.push(format!(
            "7d · {model}   {n} req · {} tok",
            format_tokens(tok)
        ));
    }
    r
}

// ---- live quota + cache -----------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bucket {
    /// Quota group, e.g. "Gemini Models" or "Claude and GPT models".
    pub group: String,
    /// "5h window" / "Weekly (7d)" / the server's display name.
    pub label: String,
    pub remaining_fraction: f64,
    /// RFC3339.
    pub reset_time: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Cached {
    fetched_at: String,
    buckets: Vec<Bucket>,
}

fn cache_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("trayce").join("antigravity_quota.json"))
}

fn load_cache() -> Option<Cached> {
    serde_json::from_str(&fs::read_to_string(cache_path()?).ok()?).ok()
}

fn save_cache(c: &Cached) {
    let Some(p) = cache_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string_pretty(c) {
        let _ = fs::write(p, s);
    }
}

/// Replace the local token rows with real quota rows. The first group (Gemini)
/// drives the headline; other groups become notes. Pure.
fn apply_quota(r: &mut Report, buckets: &[Bucket], seen: DateTime<Utc>, now: DateTime<Utc>) {
    let Some(first) = buckets.first().map(|b| b.group.clone()) else {
        return;
    };
    let mut rows = Report::default();
    let mut notes = Vec::new();
    let mut reset_since = false;
    for b in buckets {
        let reset = b
            .reset_time
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc));
        // The reading predates a reset we have since passed: the bucket is full again.
        let expired = reset.is_some_and(|t| t <= now && t > seen);
        reset_since |= expired;
        let pct = if expired {
            0
        } else {
            ((1.0 - b.remaining_fraction.clamp(0.0, 1.0)) * 100.0).round() as u32
        };
        let w = Window {
            tokens: 0,
            pct,
            calibrated: true,
            resets_at: reset.filter(|_| !expired).map(|t| t.to_rfc3339()),
        };
        if b.group == first {
            let weekday = b.label.contains("7d");
            rows.row(&b.label, w, weekday);
        } else {
            notes.push(format!("{} · {}   {pct}%", b.group, b.label));
        }
    }
    let mins = (now - seen).num_minutes();
    let mut head = vec![if mins < 2 {
        format!("Quota: live from Antigravity ({first})")
    } else {
        format!("Quota: last seen {} ({first})", ago(mins))
    }];
    if reset_since {
        head.push("Some windows reset since the last reading".to_string());
    }
    head.extend(notes);
    head.append(&mut r.notes);
    r.rows = rows.rows;
    r.notes = head;
}

pub fn collect(now: DateTime<Utc>) -> Result<Report, WidgetError> {
    let local = scan();
    let live = antigravity_live::fetch();
    let cached = match live {
        Ok(buckets) => {
            let c = Cached {
                fetched_at: now.to_rfc3339(),
                buckets,
            };
            save_cache(&c);
            Some(c)
        }
        Err(_) => load_cache(),
    };
    let mut report = match &local {
        Ok(s) => local_report(&s.reqs, now),
        // No CLI data but a quota reading exists: still worth showing.
        Err(_) if cached.is_some() => Report::default(),
        Err(e) => return Err(e.clone()),
    };
    if let Ok(s) = &local {
        if s.reqs.is_empty() && s.bad > 0 && cached.is_none() {
            return Err(WidgetError::Unreadable(format!(
                "{} unreadable row(s)",
                s.bad
            )));
        }
    }
    if let Some(c) = cached {
        let seen = DateTime::parse_from_rfc3339(&c.fetched_at)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or(now);
        apply_quota(&mut report, &c.buckets, seen, now);
    } else {
        report
            .notes
            .insert(0, "Quota %: open the Antigravity app once".to_string());
    }
    Ok(report)
}

/// Lines for `--diagnose`.
pub fn diagnose() -> Vec<String> {
    let mut out = Vec::new();
    match scan() {
        Ok(s) => {
            out.push(format!("conversations: {}", s.dir.display()));
            out.push(format!("databases (<=7d): {}", s.dbs));
            out.push(format!(
                "requests: {}   unreadable rows: {}",
                s.reqs.len(),
                s.bad
            ));
        }
        Err(_) => out.push("conversations: NOT FOUND".to_string()),
    }
    out.extend(antigravity_live::diagnose());
    match load_cache() {
        Some(c) => out.push(format!(
            "cached quota: {} bucket(s), fetched {}",
            c.buckets.len(),
            c.fetched_at
        )),
        None => out.push("cached quota: none".to_string()),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn put_varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }
    fn int_field(out: &mut Vec<u8>, f: u64, v: u64) {
        put_varint(out, f << 3);
        put_varint(out, v);
    }
    fn bytes_field(out: &mut Vec<u8>, f: u64, v: &[u8]) {
        put_varint(out, (f << 3) | 2);
        put_varint(out, v.len() as u64);
        out.extend_from_slice(v);
    }

    /// Mirrors a real `gen_metadata` row (agy 1.2.10): input 12610, output
    /// 121, thinking 35, cache read 16239 (excluded); the other fields and the
    /// fixed64 are noise the walker must skip.
    fn gen_row(step: &str) -> Vec<u8> {
        let mut usage = Vec::new();
        int_field(&mut usage, 1, 1318);
        int_field(&mut usage, 2, 12610);
        int_field(&mut usage, 3, 121);
        int_field(&mut usage, 5, 16239);
        int_field(&mut usage, 6, 24);
        bytes_field(&mut usage, 7, b"bot-5239492f");
        int_field(&mut usage, 9, 35);
        int_field(&mut usage, 10, 86);
        let mut root = Vec::new();
        int_field(&mut root, 3, 1318);
        bytes_field(&mut root, 4, &usage);
        root.extend_from_slice(&[0x09, 1, 2, 3, 4, 5, 6, 7, 8]); // fixed64 field 1
        bytes_field(&mut root, 19, b"gemini-3.8-flash");
        for (k, v) in [
            ("model_enum", "MODEL_PLACEHOLDER_M318"),
            ("last_step_index", step),
        ] {
            let mut e = Vec::new();
            bytes_field(&mut e, 1, k.as_bytes());
            bytes_field(&mut e, 2, v.as_bytes());
            bytes_field(&mut root, 20, &e);
        }
        let mut out = Vec::new();
        bytes_field(&mut out, 1, &root);
        out
    }

    #[test]
    fn parses_gen_metadata() {
        let g = parse_gen(&gen_row("6")).unwrap();
        assert_eq!(g.model, "gemini-3.8-flash");
        assert_eq!(g.tokens, 12610 + 121 + 35);
        assert_eq!(g.step, Some(6));
        assert!(parse_gen(&[0xff, 0xff]).is_none());
        assert!(parse_gen(&[]).is_none());
    }

    #[test]
    fn parses_step_time() {
        let mut ts = Vec::new();
        int_field(&mut ts, 1, 1_790_252_872);
        let mut meta = Vec::new();
        bytes_field(&mut meta, 1, &ts);
        assert_eq!(step_time(&meta), Some(at("2026-09-24T12:27:52Z")));
    }

    #[test]
    fn local_windows() {
        let now = at("2026-09-24T12:00:00Z");
        let req = |h: i64, tokens| Req {
            ts: now - Duration::hours(h),
            model: "gemini-3.8-flash".into(),
            tokens,
        };
        let r = local_report(&[req(1, 100), req(6, 1000), req(200, 5)], now);
        assert_eq!(r.rows[0].window.tokens, 100);
        assert_eq!(r.rows[1].window.tokens, 1100);
        assert!(!r.rows[0].window.calibrated);
        assert_eq!(r.notes, vec!["7d · gemini-3.8-flash   2 req · 1k tok"]);
    }

    #[test]
    fn quota_replaces_rows() {
        let now = at("2026-09-24T12:00:00Z");
        let b = |group: &str, label: &str, f, reset: &str| Bucket {
            group: group.into(),
            label: label.into(),
            remaining_fraction: f,
            reset_time: Some(reset.into()),
        };
        let buckets = [
            b("Gemini Models", "5h window", 0.86, "2026-09-24T11:00:00Z"),
            b("Gemini Models", "Weekly (7d)", 0.55, "2026-09-28T00:00:00Z"),
            b(
                "Claude and GPT models",
                "5h window",
                1.0,
                "2026-09-24T15:00:00Z",
            ),
        ];
        let mut r = local_report(&[], now);
        apply_quota(&mut r, &buckets, at("2026-09-24T10:00:00Z"), now);
        assert_eq!(r.rows.len(), 2);
        // 5h bucket reset at 11:00, after the 10:00 reading: full again.
        assert_eq!(r.rows[0].window.pct, 0);
        assert_eq!(r.rows[1].window.pct, 45);
        assert!(r.rows[1].window.calibrated && r.rows[1].weekday);
        assert_eq!(r.notes[0], "Quota: last seen 2h 0m ago (Gemini Models)");
        assert_eq!(r.notes[1], "Some windows reset since the last reading");
        assert_eq!(r.notes[2], "Claude and GPT models · 5h window   0%");
    }
}
