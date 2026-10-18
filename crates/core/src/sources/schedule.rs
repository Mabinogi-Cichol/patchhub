//! Automatic source-update scheduling semantics (p1-contract §10).
//!
//! Due means "should attempt an update", never "cache is invalid": a failed
//! attempt does not drop the last successful snapshot.

use super::model::SettingsFile;

pub const SECONDS_PER_HOUR: u64 = 3_600;

/// `checked_at + interval_hours * 3600`, only for a positive interval and a real
/// previous attempt. `None` means the source has no automatic update scheduled.
pub fn next_due_at(interval_hours: u64, last_attempt_at: u64) -> Option<u64> {
    if interval_hours == 0 || last_attempt_at == 0 {
        return None;
    }
    Some(last_attempt_at.saturating_add(interval_hours.saturating_mul(SECONDS_PER_HOUR)))
}

/// Whether a scheduled time has been reached; `None` (disabled/never attempted)
/// is never due.
pub fn is_due(now: u64, next_due_at: Option<u64>) -> bool {
    next_due_at.is_some_and(|due| now >= due)
}

/// Configured interval for one source; missing means automatic updates are off.
pub fn interval_hours(settings: &SettingsFile, source_id: &str) -> u64 {
    settings.intervals.get(source_id).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn next_due_at_requires_interval_and_attempt() {
        assert_eq!(next_due_at(0, 1_000), None);
        assert_eq!(next_due_at(6, 0), None);
        assert_eq!(next_due_at(6, 1_000), Some(1_000 + 6 * 3_600));
        assert_eq!(
            next_due_at(u64::MAX, u64::MAX),
            Some(u64::MAX),
            "saturates instead of overflowing"
        );
    }

    #[test]
    fn is_due_compares_against_the_scheduled_time() {
        assert!(!is_due(1_000, None));
        assert!(!is_due(999, Some(1_000)));
        assert!(is_due(1_000, Some(1_000)));
        assert!(is_due(1_001, Some(1_000)));
    }

    #[test]
    fn interval_hours_reads_settings_and_defaults_to_off() {
        let settings = SettingsFile {
            schema_version: 2,
            revision: 3,
            enabled: BTreeMap::new(),
            added: Vec::new(),
            games: BTreeMap::new(),
            intervals: BTreeMap::from([("patchmoe".to_string(), 24)]),
        };
        assert_eq!(interval_hours(&settings, "patchmoe"), 24);
        assert_eq!(interval_hours(&settings, "kagura-cn"), 0);
    }
}
