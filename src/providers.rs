//! The AI tools Trayce knows how to read, all from local data.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::WidgetError;
use crate::usage::Report;
use crate::{antigravity, codex, logs};

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
            Provider::Claude => logs::collect(now).map(Report::from),
            Provider::Codex => codex::collect(now),
            Provider::Antigravity => antigravity::collect(now),
        }
    }
}
