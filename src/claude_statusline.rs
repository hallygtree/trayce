//! Real Claude plan percentages via Claude Code's status line.
//!
//! Claude Code pipes a JSON blob into the configured `statusLine` command on
//! every assistant message. For Pro/Max subscribers it carries
//! `rate_limits.{five_hour,seven_day}.{used_percentage,resets_at}`: the same
//! numbers Claude Code itself shows. `trayce --claude-statusline` is that
//! command: it saves the snapshot for the tray and prints a short summary as
//! the status line. `trayce --setup-claude` registers it in Claude's settings.
//! Documented at https://code.claude.com/docs/en/statusline.

use std::io::Read;
use std::path::PathBuf;

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::{json, Value};

use crate::codex::ago;
use crate::usage::{Report, Window};

/// Older snapshots are ignored: both windows have reset since.
const MAX_AGE_DAYS: i64 = 7;

fn snapshot_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("trayce").join("claude_rate_limits.json"))
}

/// One window from the status-line JSON, or `None` if absent or garbled
/// (Claude Code has shipped an epoch timestamp in `used_percentage`).
fn window(w: &Value, now: DateTime<Utc>) -> Option<Window> {
    let used = w.get("used_percentage")?.as_f64()?;
    if !(0.0..=1000.0).contains(&used) {
        return None;
    }
    let reset = w
        .get("resets_at")
        .and_then(Value::as_i64)
        .and_then(|s| Utc.timestamp_opt(s, 0).single());
    let expired = reset.is_some_and(|r| r <= now);
    Some(Window {
        tokens: 0,
        pct: if expired { 0 } else { used.round() as u32 },
        calibrated: true,
        resets_at: reset.filter(|_| !expired).map(|r| r.to_rfc3339()),
    })
}

/// Rows from a saved snapshot `{seen_at, rate_limits}`. Pure.
fn rows(snapshot: &Value, now: DateTime<Utc>) -> Option<Report> {
    let seen = DateTime::parse_from_rfc3339(snapshot.get("seen_at")?.as_str()?)
        .ok()?
        .with_timezone(&Utc);
    if now - seen > Duration::days(MAX_AGE_DAYS) {
        return None;
    }
    let rl = snapshot.get("rate_limits")?;
    let mut r = Report::default();
    for (key, label, weekday) in [
        ("five_hour", "5h window", false),
        ("seven_day", "Weekly (7d)", true),
    ] {
        if let Some(w) = rl.get(key).and_then(|w| window(w, now)) {
            r.row(label, w, weekday);
        }
    }
    if r.rows.is_empty() {
        return None;
    }
    r.notes.push(format!(
        "Real % from Claude Code, {}",
        ago((now - seen).num_minutes().max(0))
    ));
    Some(r)
}

/// The saved snapshot as a report, if there is a usable one.
pub fn report(now: DateTime<Utc>) -> Option<Report> {
    let body = std::fs::read_to_string(snapshot_path()?).ok()?;
    rows(&serde_json::from_str(&body).ok()?, now)
}

/// Status-line text: "5h 42% · 7d 18%", or empty without rate limits. Pure.
fn status_text(rl: Option<&Value>) -> String {
    let pct = |k: &str| {
        rl?.get(k)?
            .get("used_percentage")?
            .as_f64()
            .filter(|p| (0.0..=1000.0).contains(p))
    };
    [("5h", pct("five_hour")), ("7d", pct("seven_day"))]
        .into_iter()
        .filter_map(|(l, p)| p.map(|p| format!("{l} {p:.0}%")))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// `trayce --claude-statusline`: called by Claude Code with JSON on stdin.
pub fn record() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let v: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    let rl = v.get("rate_limits").filter(|r| r.is_object());
    if let (Some(rl), Some(path)) = (rl, snapshot_path()) {
        let snap = json!({"seen_at": Utc::now().to_rfc3339(), "rate_limits": rl});
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, snap.to_string());
    }
    println!("{}", status_text(rl));
}

fn settings_path() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))
        .map(|d| d.join("settings.json"))
}

/// The `statusLine.command` value pointing at this binary. Forward slashes
/// work in both the bash and cmd shells Claude Code may use on Windows.
fn our_command(exe: &str) -> String {
    format!("\"{}\" --claude-statusline", exe.replace('\\', "/"))
}

/// Add or refresh our status line in a settings object. `Err` with the
/// current command when another status line is configured. Pure.
fn install_into(settings: &mut Value, command: &str) -> Result<(), String> {
    let obj = settings
        .as_object_mut()
        .ok_or_else(|| "settings.json is not a JSON object".to_string())?;
    if let Some(existing) = obj
        .get("statusLine")
        .and_then(|s| s.get("command"))
        .and_then(Value::as_str)
    {
        if !existing.contains("--claude-statusline") {
            return Err(existing.to_string());
        }
    }
    obj.insert(
        "statusLine".to_string(),
        json!({"type": "command", "command": command}),
    );
    Ok(())
}

/// `trayce --setup-claude`: register `--claude-statusline` in Claude Code.
pub fn setup() {
    let (Some(path), Ok(exe)) = (settings_path(), std::env::current_exe()) else {
        eprintln!("could not locate Claude Code settings or this executable");
        return;
    };
    let body = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let mut settings: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "{} is not valid JSON ({e}); not touching it",
                path.display()
            );
            return;
        }
    };
    match install_into(&mut settings, &our_command(&exe.display().to_string())) {
        Ok(()) => {
            if path.exists() {
                let _ = std::fs::copy(&path, path.with_extension("json.trayce-backup"));
            }
            match serde_json::to_string_pretty(&settings).map(|s| std::fs::write(&path, s + "\n")) {
                Ok(Ok(())) => println!(
                    "Claude Code status line set in {}. Real percentages appear after Claude's next reply.",
                    path.display()
                ),
                _ => eprintln!("could not write {}", path.display()),
            }
        }
        Err(existing) => eprintln!(
            "Claude Code already has a status line ({existing}); left unchanged.\n\
             To feed Trayce too, pipe the same JSON into `trayce --claude-statusline` from your script."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    // Shape from the Claude Code status-line docs.
    fn snap(seen: &str) -> Value {
        json!({"seen_at": seen, "rate_limits": {
            "five_hour": {"used_percentage": 23.5, "resets_at": 1790276400},
            "seven_day": {"used_percentage": 41.2, "resets_at": 1790700000}
        }})
    }

    #[test]
    fn snapshot_rows() {
        let r = rows(&snap("2026-09-24T12:00:00Z"), at("2026-09-24T12:10:00Z")).unwrap();
        assert_eq!(r.rows.len(), 2);
        assert_eq!(
            (r.rows[0].label.as_str(), r.rows[0].window.pct),
            ("5h window", 24)
        );
        assert!(r.rows[0].window.calibrated && r.rows[0].window.resets_at.is_some());
        assert_eq!(
            (r.rows[1].label.as_str(), r.rows[1].window.pct),
            ("Weekly (7d)", 41)
        );
        assert_eq!(r.notes, vec!["Real % from Claude Code, 10m ago"]);
    }

    #[test]
    fn expired_and_stale() {
        // 1790276400 = 2026-09-24T19:00:00Z: the 5h window has reset by 20:00.
        let r = rows(&snap("2026-09-24T12:00:00Z"), at("2026-09-24T20:00:00Z")).unwrap();
        assert_eq!(r.rows[0].window.pct, 0);
        assert_eq!(r.rows[1].window.pct, 41);
        assert!(rows(&snap("2026-09-01T12:00:00Z"), at("2026-09-24T12:00:00Z")).is_none());
    }

    #[test]
    fn rejects_garbled_percent() {
        let bad = json!({"used_percentage": 1_790_276_400.0, "resets_at": 1790276400});
        assert!(window(&bad, at("2026-09-24T12:00:00Z")).is_none());
        assert_eq!(status_text(Some(&json!({"five_hour": bad}))), "");
    }

    #[test]
    fn status_line_text() {
        let rl = snap("x")["rate_limits"].clone();
        assert_eq!(status_text(Some(&rl)), "5h 24% · 7d 41%");
        assert_eq!(status_text(None), "");
    }

    #[test]
    fn install_respects_other_status_lines() {
        let cmd = our_command("C:\\Programs\\trayce\\trayce.exe");
        assert_eq!(cmd, "\"C:/Programs/trayce/trayce.exe\" --claude-statusline");
        let mut s = json!({"model": "opus"});
        install_into(&mut s, &cmd).unwrap();
        assert_eq!(s["statusLine"]["command"], cmd.as_str());
        assert_eq!(s["model"], "opus");
        // Re-running updates our own entry.
        install_into(&mut s, "\"/new/trayce\" --claude-statusline").unwrap();
        assert_eq!(
            s["statusLine"]["command"],
            "\"/new/trayce\" --claude-statusline"
        );
        // Someone else's status line is left alone.
        let mut other = json!({"statusLine": {"type": "command", "command": "~/my-line.sh"}});
        assert_eq!(install_into(&mut other, &cmd).unwrap_err(), "~/my-line.sh");
    }
}
