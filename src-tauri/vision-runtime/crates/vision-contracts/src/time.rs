//! Wall-clock lease timestamps vs process-local monotonic capture time.
//!
//! - `issued_at` / `expires_at`: RFC3339 wall clock (UTC offset local).
//! - `Frame.monotonic_capture_time_ms`: ms since process monotonic epoch
//!   (`Instant`), **not** Unix wall time. Used for freshness / ordering only.

use std::sync::OnceLock;
use std::time::Instant;

use chrono::{DateTime, Local, TimeDelta};

/// Default lease lifetime before adapters must rebind.
pub const DEFAULT_LEASE_TTL_SECS: i64 = 30 * 60;

static MONOTONIC_EPOCH: OnceLock<Instant> = OnceLock::new();

fn epoch() -> Instant {
    *MONOTONIC_EPOCH.get_or_init(Instant::now)
}

/// Process-local monotonic milliseconds (starts near 0 after first call in process).
pub fn monotonic_now_ms() -> u64 {
    epoch().elapsed().as_millis() as u64
}

pub fn wall_now_rfc3339() -> String {
    Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn lease_issue_and_expiry(ttl_secs: i64) -> (String, String) {
    let issued = Local::now();
    let expires = issued + TimeDelta::seconds(ttl_secs.max(1));
    (
        issued.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        expires.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    )
}

pub fn parse_rfc3339(s: &str) -> Result<DateTime<Local>, String> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Local))
        .or_else(|_| {
            // Allow unix:{secs} used by older adapters during migration.
            if let Some(rest) = s.strip_prefix("unix:") {
                let secs: i64 = rest.parse().map_err(|e| format!("bad unix ts: {e}"))?;
                DateTime::from_timestamp(secs, 0)
                    .map(|dt| dt.with_timezone(&Local))
                    .ok_or_else(|| "unix ts out of range".into())
            } else {
                Err(format!("unparseable timestamp: {s}"))
            }
        })
}

pub fn lease_expired(expires_at: &str, now: DateTime<Local>) -> Result<bool, String> {
    let exp = parse_rfc3339(expires_at)?;
    Ok(now >= exp)
}

/// True when `newer` is at least `min_delta_ms` after `older` on the monotonic clock.
pub fn is_fresher(older_ms: u64, newer_ms: u64, min_delta_ms: u64) -> bool {
    newer_ms.saturating_sub(older_ms) >= min_delta_ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn lease_expiry_after_issued() {
        let (issued, expires) = lease_issue_and_expiry(1800);
        let i = parse_rfc3339(&issued).unwrap();
        let e = parse_rfc3339(&expires).unwrap();
        assert!(e > i);
        assert!((e - i).num_seconds() >= 1799);
    }

    #[test]
    fn monotonic_advances_not_wall_epoch() {
        let a = monotonic_now_ms();
        thread::sleep(Duration::from_millis(5));
        let b = monotonic_now_ms();
        assert!(b >= a + 4, "a={a} b={b}");
        // Must not look like Unix millis (~1.7e12 in 2026).
        assert!(b < 86_400_000, "monotonic should be small early in process, got {b}");
    }

    #[test]
    fn fresher_helper() {
        assert!(is_fresher(10, 25, 10));
        assert!(!is_fresher(10, 15, 10));
    }
}
