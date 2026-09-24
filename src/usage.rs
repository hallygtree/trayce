//! Usage model, filled by aggregating local Claude Code logs (see `logs.rs`).
//!
//! The legacy OAuth endpoint reported `utilization` as a percent of the plan
//! limit. Local logs only carry absolute token counts, so each window stores
//! the real token total plus `pct`, a token-vs-cap ratio used solely to drive
//! the tray colour and the ASCII bar (the cap is a heuristic, not a published
//! plan limit). See `render::format_tokens` for display.

/// One usage window (the active 5h block, or a rolling 7d total).
#[derive(Debug, Clone)]
pub struct Window {
    /// Total tokens in the window (input + output + cache creation; cache
    /// reads are excluded, see `logs.rs`).
    pub tokens: u64,
    /// Tokens as a percent of the limit. Drives colour and bar.
    pub pct: u32,
    /// True when `pct` is against a learned limit (real %); false when it is
    /// against the heuristic cap (show tokens, not a confident percent).
    pub calibrated: bool,
    /// RFC3339 reset time, when the window has a meaningful boundary (5h block).
    pub resets_at: Option<String>,
}

/// Aggregated usage across all locally-logged Claude Code sessions.
#[derive(Debug, Clone)]
pub struct Usage {
    pub five_hour: Window,
    pub seven_day: Window,
    pub seven_day_opus: Option<Window>,
    pub seven_day_sonnet: Option<Window>,
}

/// One labelled window in a provider-agnostic [`Report`].
#[derive(Debug, Clone)]
pub struct Row {
    pub label: String,
    pub window: Window,
    /// Show the weekday in the absolute reset time (multi-day windows).
    pub weekday: bool,
}

/// What the tray shows for one provider: headline windows (drive colour and
/// title) plus free-form detail lines.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub rows: Vec<Row>,
    pub notes: Vec<String>,
}

impl Report {
    pub fn row(&mut self, label: &str, window: Window, weekday: bool) {
        self.rows.push(Row {
            label: label.to_string(),
            window,
            weekday,
        });
    }
}

impl From<Usage> for Report {
    fn from(u: Usage) -> Self {
        let tok = |w: Option<Window>| {
            w.map(|w| format!("{} tok", crate::render::format_tokens(w.tokens)))
                .unwrap_or_else(|| "-".to_string())
        };
        let mut r = Report::default();
        r.row("5h window", u.five_hour, false);
        r.row("Weekly (7d)", u.seven_day, true);
        r.notes
            .push(format!("Weekly · Sonnet   {}", tok(u.seven_day_sonnet)));
        r.notes
            .push(format!("Weekly · Opus     {}", tok(u.seven_day_opus)));
        r
    }
}
