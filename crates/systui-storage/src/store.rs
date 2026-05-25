//! Local UI state store: a single versioned JSON document at
//! `~/.local/share/systui/state.json` holding data the TUI accumulates over
//! time — per-host health/finding **snapshots** (for the dashboard and security
//! **trend** lines), per-host **session notes**, and **saved log searches**.
//!
//! Best-effort by design: a missing or unreadable/corrupt file loads as the
//! default (empty) state, and a write failure is reported but never blocks the
//! UI. The document carries a `version` so the schema can evolve.

use std::path::PathBuf;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use systui_core::{CoreError, Result};

use crate::paths;

/// Current on-disk schema version.
pub const STATE_VERSION: u32 = 1;

/// How many daily health snapshots to keep per host (~3 months).
const MAX_SNAPSHOTS: usize = 90;
/// How many session notes to keep per host.
const MAX_NOTES: usize = 50;
/// How many saved searches to keep.
const MAX_SEARCHES: usize = 30;

/// A point-in-time health/finding sample, one per calendar day per host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthSnapshot {
    /// `YYYY-MM-DD` (host-local) the sample was taken.
    pub date: String,
    pub score: u8,
    pub critical: usize,
    pub high: usize,
    pub medium: usize,
}

impl HealthSnapshot {
    /// Total findings across the tracked severities.
    pub fn findings(&self) -> usize {
        self.critical + self.high + self.medium
    }
}

/// A free-text note the operator attached while reviewing a host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionNote {
    /// Timestamp string (host-local), e.g. `2026-05-25 14:18`.
    pub at: String,
    pub text: String,
}

/// A persisted log-search query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSearch {
    pub query: String,
}

/// The whole persisted document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentState {
    pub version: u32,
    /// host label → daily snapshots (oldest first).
    #[serde(default)]
    pub snapshots: std::collections::BTreeMap<String, Vec<HealthSnapshot>>,
    /// host label → notes (oldest first).
    #[serde(default)]
    pub notes: std::collections::BTreeMap<String, Vec<SessionNote>>,
    /// Saved log searches (global, most-recent first).
    #[serde(default)]
    pub saved_searches: Vec<SavedSearch>,
}

impl Default for PersistentState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            snapshots: Default::default(),
            notes: Default::default(),
            saved_searches: Vec::new(),
        }
    }
}

impl PersistentState {
    /// Record today's health snapshot for `host`, replacing any existing sample
    /// for the same date (so repeated refreshes in a day keep the latest), and
    /// capping the retained history.
    pub fn record_snapshot(
        &mut self,
        host: &str,
        date: &str,
        score: u8,
        critical: usize,
        high: usize,
        medium: usize,
    ) {
        let series = self.snapshots.entry(host.to_owned()).or_default();
        let snap = HealthSnapshot {
            date: date.to_owned(),
            score,
            critical,
            high,
            medium,
        };
        match series.iter_mut().find(|s| s.date == date) {
            Some(existing) => *existing = snap,
            None => series.push(snap),
        }
        let len = series.len();
        if len > MAX_SNAPSHOTS {
            series.drain(0..len - MAX_SNAPSHOTS);
        }
    }

    /// The snapshot whose date is nearest to `today - days`, considering only
    /// samples strictly before `today` (so a baseline is a genuine past value).
    /// `None` until enough history has accumulated.
    pub fn baseline(&self, host: &str, today: NaiveDate, days: i64) -> Option<&HealthSnapshot> {
        let target = today - chrono::Duration::days(days);
        self.snapshots
            .get(host)?
            .iter()
            .filter_map(|s| {
                NaiveDate::parse_from_str(&s.date, "%Y-%m-%d")
                    .ok()
                    .map(|d| (d, s))
            })
            .filter(|(d, _)| *d < today)
            .min_by_key(|(d, _)| (*d - target).num_days().abs())
            .map(|(_, s)| s)
    }

    /// Append a note for `host`, capping the retained count.
    pub fn add_note(&mut self, host: &str, at: &str, text: &str) {
        let notes = self.notes.entry(host.to_owned()).or_default();
        notes.push(SessionNote {
            at: at.to_owned(),
            text: text.to_owned(),
        });
        let len = notes.len();
        if len > MAX_NOTES {
            notes.drain(0..len - MAX_NOTES);
        }
    }

    /// Notes for `host`, oldest first.
    pub fn notes(&self, host: &str) -> &[SessionNote] {
        self.notes.get(host).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Save a search query (most-recent first, de-duplicated, capped).
    pub fn add_search(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.saved_searches.retain(|s| s.query != query);
        self.saved_searches.insert(
            0,
            SavedSearch {
                query: query.to_owned(),
            },
        );
        self.saved_searches.truncate(MAX_SEARCHES);
    }
}

/// Reader/writer for the local state document.
#[derive(Debug, Clone)]
pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    /// Open the store at the default data-directory location.
    pub fn at_default_location() -> Result<Self> {
        Ok(Self {
            path: paths::data_dir()?.join("state.json"),
        })
    }

    /// Use a specific path (useful for tests).
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Load the state; a missing or corrupt file yields the default state, so a
    /// first run (or a bad write) never errors.
    pub fn load(&self) -> PersistentState {
        let Ok(bytes) = std::fs::read(&self.path) else {
            return PersistentState::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    /// Persist the state as pretty JSON, creating the parent directory if needed.
    pub fn save(&self, state: &PersistentState) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(state)
            .map_err(|e| CoreError::Config(format!("state serialization: {e}")))?;
        std::fs::write(&self.path, json)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("systui-state-{}-{nanos}.json", std::process::id()))
    }

    #[test]
    fn snapshot_dedupes_by_date_and_caps() {
        let mut state = PersistentState::default();
        state.record_snapshot("h", "2026-05-25", 90, 1, 2, 3);
        state.record_snapshot("h", "2026-05-25", 88, 0, 1, 1); // same day → replace
        assert_eq!(state.snapshots["h"].len(), 1);
        assert_eq!(state.snapshots["h"][0].score, 88);
    }

    #[test]
    fn baseline_picks_nearest_past_day() {
        let mut state = PersistentState::default();
        let today = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
        state.record_snapshot("h", "2026-05-18", 91, 0, 3, 3); // 7 days ago
        state.record_snapshot("h", "2026-05-24", 80, 1, 2, 4); // yesterday
        state.record_snapshot("h", "2026-05-25", 78, 1, 1, 2); // today (excluded)

        let week = state.baseline("h", today, 7).unwrap();
        assert_eq!(week.score, 91); // the 7-days-ago sample
        // No baseline before any history exists.
        assert!(state.baseline("other", today, 7).is_none());
    }

    #[test]
    fn notes_and_searches_round_trip_through_disk() {
        let path = tmp_path();
        let store = StateStore::with_path(&path);

        let mut state = store.load(); // default on first run
        state.add_note("prod-01", "2026-05-25 14:18", "reviewed nginx errors");
        state.add_search("level:err nginx");
        state.add_search("level:err nginx"); // dedup
        state.add_search("auth failures");
        store.save(&state).unwrap();

        let reloaded = store.load();
        assert_eq!(reloaded.notes("prod-01").len(), 1);
        assert_eq!(reloaded.notes("prod-01")[0].text, "reviewed nginx errors");
        assert_eq!(reloaded.saved_searches.len(), 2);
        assert_eq!(reloaded.saved_searches[0].query, "auth failures"); // most recent first

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn corrupt_file_loads_as_default() {
        let path = tmp_path();
        std::fs::write(&path, b"not json").unwrap();
        let state = StateStore::with_path(&path).load();
        assert_eq!(state, PersistentState::default());
        std::fs::remove_file(&path).ok();
    }
}
