//! User settings, persisted across runs.
//!
//! Location: `<data_dir>/trayce/config.json`. Missing or unreadable file means
//! defaults: Claude only, shown alone, matching the original claude-usage-bar.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::providers::Provider;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// Every enabled provider, one submenu each.
    All,
    /// One provider, flat menu.
    Single(Provider),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub enabled: Vec<Provider>,
    pub mode: Mode,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: vec![Provider::Claude],
            mode: Mode::Single(Provider::Claude),
        }
    }
}

impl Config {
    /// Providers on screen, in canonical order. A `Single` pick that was since
    /// disabled falls back to everything enabled.
    pub fn shown(&self) -> Vec<Provider> {
        match self.mode {
            Mode::Single(p) if self.enabled.contains(&p) => vec![p],
            _ => self.enabled.clone(),
        }
    }

    pub fn toggle(&mut self, p: Provider) {
        if self.enabled.contains(&p) {
            self.enabled.retain(|&e| e != p);
        } else {
            self.enabled.push(p);
            self.enabled
                .sort_by_key(|e| Provider::ALL.iter().position(|a| a == e));
        }
    }
}

fn path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("trayce").join("config.json"))
}

pub fn load() -> Config {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(c: &Config) {
    let Some(p) = path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = serde_json::to_string_pretty(c) {
        let _ = std::fs::write(&p, s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_shown() {
        let mut c = Config::default();
        assert_eq!(c.shown(), vec![Provider::Claude]);
        c.toggle(Provider::Antigravity);
        c.toggle(Provider::Codex);
        assert_eq!(c.enabled, Provider::ALL.to_vec());
        c.mode = Mode::Single(Provider::Codex);
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        assert_eq!(c.shown(), vec![Provider::Codex]);
        c.toggle(Provider::Codex);
        assert_eq!(c.shown(), vec![Provider::Claude, Provider::Antigravity]);
        c.mode = Mode::All;
        assert_eq!(c.shown().len(), 2);
    }
}
