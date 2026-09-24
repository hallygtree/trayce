//! Learned usage limits, persisted across runs.
//!
//! The plan limit is not published, so we learn it from the limit-hit events
//! Claude Code records in its logs (see `logs.rs`). The learned value is stored
//! here because the originating event can age out of the 7d log scan window.
//!
//! Location: `<data_dir>/claude-usage-bar/calibration.json` (portable via
//! `dirs::data_dir`). Missing or unreadable file simply means "not calibrated".

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Calibration {
    /// Learned 5h session limit in tokens, from the most recent session-limit hit.
    pub five_hour_limit: Option<u64>,
    /// RFC3339 timestamp of the hit that produced `five_hour_limit`.
    pub five_hour_updated: Option<String>,
    /// Learned weekly limit (string not yet confirmed in logs; see design doc).
    pub weekly_limit: Option<u64>,
    pub weekly_updated: Option<String>,
}

fn path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("claude-usage-bar").join("calibration.json"))
}

pub fn load() -> Calibration {
    let Some(p) = path() else {
        return Calibration::default();
    };
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(c: &Calibration) {
    let Some(p) = path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = serde_json::to_string_pretty(c) {
        let _ = std::fs::write(&p, s);
    }
}
