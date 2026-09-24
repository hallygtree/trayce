mod antigravity;
mod antigravity_live;
mod autostart;
mod calibration;
mod codex;
mod config;
mod error;
mod logs;
mod providers;
mod render;
mod tray;
mod usage;

use providers::Provider;

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--once") => run_once(),
        Some("--diagnose") => run_diagnose(),
        Some("--selftest") => run_selftest(),
        Some("--install") => autostart::install(),
        Some("--uninstall") => autostart::uninstall(),
        _ => tray::run(),
    }
}

/// Print every enabled provider once (all of them with `--once all`).
fn run_once() {
    let all = std::env::args().nth(2).as_deref() == Some("all");
    let list = if all {
        Provider::ALL.to_vec()
    } else {
        config::load().enabled
    };
    let now = chrono::Utc::now();
    let mut failed = false;
    for p in list {
        println!("[{}]", p.name());
        match p.collect(now) {
            Ok(r) => {
                for row in &r.rows {
                    println!("  {:<14} {}", row.label, render::window_value(&row.window));
                }
                for note in &r.notes {
                    println!("  {note}");
                }
            }
            Err(e) => {
                failed = true;
                println!("  error: {}", render::error_text(&e).0);
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}

/// Report what was found in the logs, for support and for contributors sending
/// limit-event samples from other plans, locales, and Claude Code versions.
fn run_diagnose() {
    println!("[Claude]");
    let d = logs::diagnose(chrono::Utc::now());
    println!("logs dir:      {}", d.dir.as_deref().unwrap_or("NOT FOUND"));
    println!("files (<=7d):  {}", d.files);
    println!("usage events:  {}", d.usage_events);
    println!("malformed:     {}", d.malformed);
    println!("limit events:  {}", d.limit_events.len());
    for (kind, ts, reset) in &d.limit_events {
        println!(
            "  - {ts}  {kind}  resets {}",
            reset.as_deref().unwrap_or("?")
        );
    }
    match d.calibration.five_hour_limit {
        Some(limit) => println!(
            "5h calibration: {} tok (learned {}){}",
            render::format_tokens(limit),
            d.calibration.five_hour_updated.as_deref().unwrap_or("?"),
            if d.five_hour_fresh {
                ""
            } else {
                " [STALE, showing tokens until the next hit]"
            }
        ),
        None => println!("5h calibration: not calibrated (no session-limit hit seen yet)"),
    }

    println!(
        "
[Codex]"
    );
    let (dir, files) = codex::diagnose();
    println!("sessions dir:  {}", dir.as_deref().unwrap_or("NOT FOUND"));
    println!("rollouts (<=7d): {files}");

    println!(
        "
[Antigravity]"
    );
    for line in antigravity::diagnose() {
        println!("{line}");
    }
    println!(
        "
config: {:?}",
        config::load()
    );
}

fn run_selftest() {
    use render::Level;
    assert_eq!(render::level_for(49), Level::Green);
    assert_eq!(render::level_for(80), Level::Red);
    assert_eq!(render::ascii_bar(100), "▓▓▓▓▓▓▓▓▓▓");
    assert_eq!(
        render::icon_rgba(Level::Green).len(),
        render::ICON_SIZE * render::ICON_SIZE * 4
    );
    println!("selftest: PASS");
}
