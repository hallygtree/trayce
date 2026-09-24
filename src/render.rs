use chrono::{DateTime, Datelike, Local, Utc, Weekday};

use crate::usage::{Report, Window};

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

/// "in 3h 12m" / "in 2d 15h" / "in 5m" / "now" / "-" (unparseable).
pub fn relative_reset(resets_at: &str, now: DateTime<Utc>) -> String {
    let Ok(dt) = DateTime::parse_from_rfc3339(resets_at) else {
        return "-".to_string();
    };
    let secs = (dt.with_timezone(&Utc) - now).num_seconds();
    if secs <= 0 {
        return "now".to_string();
    }
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    if hours >= 24 {
        // Coarse copy by design: minutes omitted once a day or more away.
        format!("in {}d {}h", hours / 24, hours % 24)
    } else if hours > 0 {
        format!("in {hours}h {mins}m")
    } else {
        format!("in {mins}m")
    }
}

fn weekday_en(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

/// "20:50" or "Sat 07:00" in local time. "-" if unparseable.
pub fn absolute_reset(resets_at: &str, include_weekday: bool) -> String {
    let Ok(dt) = DateTime::parse_from_rfc3339(resets_at) else {
        return "-".to_string();
    };
    let local = dt.with_timezone(&Local);
    if include_weekday {
        format!("{} {}", weekday_en(local.weekday()), local.format("%H:%M"))
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

/// "1.2M" / "18.3M" / "523k" / "742". Compact token count for the tray.
pub fn format_tokens(t: u64) -> String {
    if t >= 1_000_000 {
        format!("{:.1}M", t as f64 / 1_000_000.0)
    } else if t >= 1_000 {
        format!("{:.0}k", t as f64 / 1_000.0)
    } else {
        t.to_string()
    }
}

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

/// A window's headline value: a real percent when calibrated, otherwise the
/// raw token count (no fake confident percent before calibration).
pub fn window_value(w: &Window) -> String {
    if w.calibrated {
        format!("{}%", w.pct)
    } else {
        format!("{} tok", format_tokens(w.tokens))
    }
}

/// Short label for a window row: "5h window" → "5h", "Weekly (7d)" → "7d".
fn short_label(label: &str) -> &str {
    match label.rfind('(') {
        Some(i) => label[i + 1..].trim_end_matches(')'),
        None => label.split_whitespace().next().unwrap_or(label),
    }
}

/// Menu-bar / title text (used on macOS): "5h 42% · 7d 1.2M tok".
pub fn title_text(r: &Report) -> String {
    r.rows
        .iter()
        .map(|row| format!("{} {}", short_label(&row.label), window_value(&row.window)))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// One detail row: "5h window   86%   ▓▓░░░░░░░░" (or "1.2M tok" uncalibrated).
pub fn window_row(label: &str, w: &Window) -> String {
    format!("{label}   {}   {}", window_value(w), ascii_bar(w.pct))
}

/// Reset row: "resets in 3h 12m  ·  20:50". Empty string if no reset time.
pub fn reset_row(w: &Window, weekday: bool, now: DateTime<Utc>) -> String {
    match &w.resets_at {
        Some(r) => format!(
            "resets {}  ·  {}",
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
        LogsNotFound => (
            "logs not found, see the README".to_string(),
            "⚠ logs".to_string(),
        ),
        Unreadable(why) => (format!("logs unreadable: {why}"), "⚠ logs".to_string()),
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
    }

    #[test]
    fn title_from_rows() {
        let w = |pct, calibrated| Window {
            tokens: 1_200_000,
            pct,
            calibrated,
            resets_at: None,
        };
        let mut r = Report::default();
        r.row("5h window", w(42, true), false);
        r.row("Weekly (7d)", w(90, false), true);
        assert_eq!(title_text(&r), "5h 42% · 7d 1.2M tok");
    }

    #[test]
    fn relative_reset_formats() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let plus = |secs: i64| (now + chrono::Duration::seconds(secs)).to_rfc3339();
        assert_eq!(relative_reset(&plus(3720), now), "in 1h 2m");
        assert_eq!(relative_reset(&plus(300), now), "in 5m");
        assert_eq!(relative_reset(&plus(140_000), now), "in 1d 14h");
        assert_eq!(relative_reset(&plus(-10), now), "now");
        assert_eq!(relative_reset("garbage", now), "-");
    }

    #[test]
    fn ascii_bar_fills() {
        assert_eq!(ascii_bar(0), "░░░░░░░░░░");
        assert_eq!(ascii_bar(50).matches('▓').count(), 5);
        assert_eq!(ascii_bar(100), "▓▓▓▓▓▓▓▓▓▓");
        assert_eq!(ascii_bar(150), "▓▓▓▓▓▓▓▓▓▓");
    }

    #[test]
    fn tokens_format() {
        assert_eq!(format_tokens(742), "742");
        assert_eq!(format_tokens(12_400), "12k");
        assert_eq!(format_tokens(1_240_000), "1.2M");
        assert_eq!(format_tokens(18_300_000), "18.3M");
    }

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
}
