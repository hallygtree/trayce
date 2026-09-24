//! Live Antigravity quota from the desktop app's local language server.
//!
//! The app starts a language server with `--csrf_token <t>` on its command
//! line and serves Connect-protocol JSON on loopback. `RetrieveUserQuotaSummary`
//! returns the same per-bucket quota the app shows (`remainingFraction`,
//! `resetTime`). Traffic never leaves 127.0.0.1 and no Google credential is
//! touched. Same approach as CodexBar (steipete/CodexBar, docs/antigravity.md).
//!
//! The `agy` CLI also runs a server, but it demands a CSRF token it does not
//! expose, so only the desktop app (or IDE) is usable here.

use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::antigravity::Bucket;

const PATH: &str = "/exa.language_server_pb.LanguageServerService/RetrieveUserQuotaSummary";

struct Server {
    pid: u32,
    csrf: String,
}

fn csrf_from_args<S: AsRef<str>>(args: &[S]) -> Option<String> {
    let mut it = args.iter().map(AsRef::as_ref);
    while let Some(a) = it.next() {
        if a == "--csrf_token" {
            return it.next().map(str::to_string);
        }
        if let Some(v) = a.strip_prefix("--csrf_token=") {
            return Some(v.to_string());
        }
    }
    None
}

fn servers() -> Vec<Server> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    sys.processes()
        .iter()
        .filter_map(|(pid, p)| {
            let args: Vec<String> = p
                .cmd()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            let ours = args
                .iter()
                .any(|a| a.to_lowercase().contains("antigravity"));
            let csrf = csrf_from_args(&args).filter(|t| !t.is_empty())?;
            ours.then(|| Server {
                pid: pid.as_u32(),
                csrf,
            })
        })
        .collect()
}

fn run(cmd: &mut Command) -> String {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Loopback ports `pid` listens on, from `netstat -ano` output. Matches on the
/// `0.0.0.0:0` remote column rather than the state word, which is localized.
fn netstat_ports(out: &str, pid: u32) -> Vec<u16> {
    let pid = pid.to_string();
    out.lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            let local = c.get(1)?.strip_prefix("127.0.0.1:")?;
            (c.first() == Some(&"TCP")
                && c.get(2) == Some(&"0.0.0.0:0")
                && c.last() == Some(&pid.as_str()))
            .then(|| local.parse().ok())
            .flatten()
        })
        .collect()
}

/// Ports from `lsof -Fn` output (`n127.0.0.1:61055` lines).
fn lsof_ports(out: &str) -> Vec<u16> {
    out.lines()
        .filter_map(|l| l.strip_prefix("n")?.rsplit(':').next()?.parse().ok())
        .collect()
}

fn ports(pid: u32) -> Vec<u16> {
    if cfg!(windows) {
        netstat_ports(
            &run(Command::new("netstat").args(["-ano", "-p", "TCP"])),
            pid,
        )
    } else {
        let pid = pid.to_string();
        lsof_ports(&run(Command::new("lsof").args([
            "-nP",
            "-iTCP",
            "-sTCP:LISTEN",
            "-a",
            "-p",
            &pid,
            "-Fn",
        ])))
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        // Self-signed loopback certificate; every URL below is 127.0.0.1.
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .disable_verification(true)
                .build(),
        )
        .timeout_global(Some(Duration::from_secs(3)))
        .build()
        .into()
}

fn post(agent: &ureq::Agent, url: &str, csrf: &str) -> Result<Value, String> {
    let body = json!({"metadata": {
        "ideName": "antigravity", "extensionName": "antigravity",
        "locale": "en", "ideVersion": "unknown"
    }});
    agent
        .post(url)
        .header("Connect-Protocol-Version", "1")
        .header("X-Codeium-Csrf-Token", csrf)
        .send_json(&body)
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_json()
        .map_err(|e| e.to_string())
}

fn window_label(id: &str, name: &str) -> String {
    let s = format!("{id} {name}").to_lowercase();
    if s.contains("week") {
        "Weekly (7d)".to_string()
    } else if s.contains("5h")
        || s.contains("5-hour")
        || s.contains("five")
        || s.contains("session")
    {
        "5h window".to_string()
    } else if name.is_empty() {
        id.to_string()
    } else {
        name.to_string()
    }
}

fn str_of<'a>(v: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .find_map(|k| v.get(k)?.as_str())
        .unwrap_or("")
        .trim()
}

/// Parse a `RetrieveUserQuotaSummary` response. The payload may sit under
/// `response`, `summary`, or at the root; `remainingFraction` may be flat, under
/// `remaining`, or a protobuf oneof (`{"case": "remainingFraction", "value": x}`).
/// Buckets without a known fraction or marked disabled are dropped. Pure.
fn parse_summary(v: &Value) -> Option<Vec<Bucket>> {
    let payload = v.get("response").or(v.get("summary")).unwrap_or(v);
    let mut out = Vec::new();
    for g in payload.get("groups")?.as_array()? {
        let group = match str_of(g, &["displayName", "name"]) {
            "" => "Quota",
            s => s,
        };
        for b in g
            .get("buckets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let id = str_of(b, &["bucketId", "id"]);
            let rem = b.get("remaining");
            let fraction = b
                .get("remainingFraction")
                .or(rem.and_then(|r| r.get("remainingFraction")))
                .or(rem
                    .filter(|r| r.get("case").and_then(Value::as_str) == Some("remainingFraction"))
                    .and_then(|r| r.get("value")))
                .and_then(Value::as_f64);
            let disabled = b.get("disabled").and_then(Value::as_bool).unwrap_or(false);
            let (Some(fraction), false, false) = (fraction, disabled, id.is_empty()) else {
                continue;
            };
            out.push(Bucket {
                group: group.to_string(),
                label: window_label(id, str_of(b, &["displayName", "name"])),
                remaining_fraction: fraction,
                reset_time: b
                    .get("resetTime")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Current quota buckets, or why none could be read.
pub fn fetch() -> Result<Vec<Bucket>, String> {
    let servers = servers();
    if servers.is_empty() {
        return Err("Antigravity app not running".to_string());
    }
    let agent = agent();
    let mut last_err = String::new();
    for s in &servers {
        for port in ports(s.pid) {
            for scheme in ["https", "http"] {
                match post(
                    &agent,
                    &format!("{scheme}://127.0.0.1:{port}{PATH}"),
                    &s.csrf,
                ) {
                    Ok(v) => match parse_summary(&v) {
                        Some(b) => return Ok(b),
                        None => last_err = format!("port {port}: no quota in response"),
                    },
                    Err(e) => last_err = format!("port {port}: {e}"),
                }
            }
        }
    }
    Err(last_err)
}

/// Lines for `--diagnose`. Never prints the CSRF token.
pub fn diagnose() -> Vec<String> {
    let servers = servers();
    let mut out = vec![format!("app language servers: {}", servers.len())];
    for s in &servers {
        out.push(format!("  - pid {} ports {:?}", s.pid, ports(s.pid)));
    }
    match fetch() {
        Ok(b) => {
            out.push(format!("live quota: {} bucket(s)", b.len()));
            for x in b {
                out.push(format!(
                    "  - {} · {}  {:.0}% left, resets {}",
                    x.group,
                    x.label,
                    x.remaining_fraction * 100.0,
                    x.reset_time.as_deref().unwrap_or("?")
                ));
            }
        }
        Err(e) => out.push(format!("live quota: unavailable ({e})")),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csrf_extraction() {
        let args = [
            "C:\\Antigravity\\language_server.exe",
            "--app_data_dir",
            "antigravity",
            "--csrf_token",
            "abc",
        ];
        assert_eq!(csrf_from_args(&args).as_deref(), Some("abc"));
        assert_eq!(
            csrf_from_args(&["x", "--csrf_token=def"]).as_deref(),
            Some("def")
        );
        assert_eq!(csrf_from_args(&["agy.exe"]), None);
    }

    #[test]
    fn netstat_parsing() {
        let out = "\n  Proto  Endereço local  Endereço externo  Estado  PID\n  \
            TCP    127.0.0.1:61055    0.0.0.0:0    OUVINDO    32860\n  \
            TCP    127.0.0.1:61056    0.0.0.0:0    LISTENING    32860\n  \
            TCP    127.0.0.1:61057    127.0.0.1:443    ESTABLISHED    32860\n  \
            TCP    0.0.0.0:135    0.0.0.0:0    LISTENING    32860\n  \
            TCP    127.0.0.1:9000    0.0.0.0:0    LISTENING    1234\n";
        assert_eq!(netstat_ports(out, 32860), vec![61055, 61056]);
    }

    #[test]
    fn lsof_parsing() {
        assert_eq!(
            lsof_ports("p123\nf7\nn127.0.0.1:61055\nf8\nn127.0.0.1:61056\n"),
            vec![61055, 61056]
        );
    }

    #[test]
    fn summary_shapes() {
        // Shape used by CodexBar's fixtures for Antigravity 2.x.
        let v: Value = serde_json::from_str(
            r#"{"response":{"groups":[
                {"displayName":"Gemini Models","buckets":[
                    {"bucketId":"gemini-5h","displayName":"Session Limit","remaining":{"remainingFraction":0.86},"resetTime":"2026-09-24T17:00:00Z"},
                    {"bucketId":"gemini-weekly","displayName":"Weekly Limit","remainingFraction":0.55}]},
                {"displayName":"Claude and GPT models","buckets":[
                    {"bucketId":"3p-5h","remaining":{"case":"remainingFraction","value":1}},
                    {"bucketId":"3p-weekly","disabled":true,"remainingFraction":1},
                    {"bucketId":"3p-other"}]}]}}"#,
        )
        .unwrap();
        let b = parse_summary(&v).unwrap();
        assert_eq!(b.len(), 3);
        assert_eq!(
            (b[0].label.as_str(), b[0].remaining_fraction),
            ("5h window", 0.86)
        );
        assert_eq!(b[0].reset_time.as_deref(), Some("2026-09-24T17:00:00Z"));
        assert_eq!(
            (b[1].label.as_str(), b[1].remaining_fraction),
            ("Weekly (7d)", 0.55)
        );
        assert_eq!(
            (b[2].group.as_str(), b[2].label.as_str()),
            ("Claude and GPT models", "5h window")
        );

        let root: Value = serde_json::from_str(
            r#"{"groups":[{"buckets":[{"id":"x","remainingFraction":0.5}]}]}"#,
        )
        .unwrap();
        assert_eq!(parse_summary(&root).unwrap()[0].group, "Quota");
        assert!(parse_summary(&json!({"code":"unauthenticated"})).is_none());
    }
}
