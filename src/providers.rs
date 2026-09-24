//! The AI tools Trayce knows how to read, all from local data.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::WidgetError;
use crate::usage::Report;
use crate::{antigravity, claude_statusline, codex, logs};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    Claude,
    Codex,
    Antigravity,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Claude, Provider::Codex, Provider::Antigravity];

    pub fn name(self) -> &'static str {
        match self {
            Provider::Claude => "Claude",
            Provider::Codex => "Codex",
            Provider::Antigravity => "Antigravity",
        }
    }

    pub fn collect(self, now: DateTime<Utc>) -> Result<Report, WidgetError> {
        match self {
            Provider::Claude => claude(now),
            Provider::Codex => codex::collect(now),
            Provider::Antigravity => antigravity::collect(now),
        }
    }
}

/// Claude: the real percentages from the status-line snapshot when there is
/// one, with the per-model token notes from the logs; otherwise the logs alone.
fn claude(now: DateTime<Utc>) -> Result<Report, WidgetError> {
    let logs = logs::collect(now).map(Report::from);
    match claude_statusline::report(now) {
        Some(mut live) => {
            if let Ok(r) = logs {
                live.notes.extend(r.notes);
            }
            Ok(live)
        }
        None => logs.map(|mut r| {
            r.notes
                .push("Real %: run `trayce --setup-claude`".to_string());
            r
        }),
    }
}
