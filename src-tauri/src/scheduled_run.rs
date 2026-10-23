use chrono::{DateTime, FixedOffset};
use std::thread;
use std::time::Duration;

pub const PREPARE_SECONDS: i64 = 60;
pub const RECHECK_SECONDS: i64 = 15;

#[derive(Clone)]
pub struct ScheduledRun {
    pub arm_gen: u64,
    pub fire_at: DateTime<FixedOffset>,
    pub account_fingerprint: Option<String>,
}

pub fn start_at(fire_at: DateTime<FixedOffset>, http: bool) -> DateTime<FixedOffset> {
    if http { fire_at - chrono::Duration::seconds(PREPARE_SECONDS) } else { fire_at }
}

pub fn wait_until(deadline: DateTime<FixedOffset>, active: impl FnMut() -> bool) -> Result<(), String> {
    wait_with(deadline, active, || chrono::Local::now().fixed_offset(), thread::sleep)
}

fn wait_with(
    deadline: DateTime<FixedOffset>, mut active: impl FnMut() -> bool,
    mut now: impl FnMut() -> DateTime<FixedOffset>, mut sleep: impl FnMut(Duration),
) -> Result<(), String> {
    loop {
        if !active() { return Err("已取消".into()); }
        let remaining = (deadline - now()).num_milliseconds();
        if remaining <= 0 { return Ok(()); }
        sleep(Duration::from_millis(crate::run_control::fire_wait_ms(remaining)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn at(raw: &str) -> DateTime<FixedOffset> { DateTime::parse_from_rfc3339(raw).unwrap() }

    #[test]
    fn http_prepares_one_minute_early_with_fifteen_second_recheck() {
        let fire = at("2026-10-17T07:00:00+08:00");
        assert_eq!(start_at(fire, true), at("2026-10-17T06:59:00+08:00"));
        assert_eq!(start_at(fire, false), fire);
        assert_eq!(start_at(at("2026-10-17T00:00:30+08:00"), true), at("2026-10-16T23:59:30+08:00"));
        assert_eq!(start_at(fire, true), at("2026-10-16T22:59:00Z"));
        assert_eq!(fire-chrono::Duration::seconds(RECHECK_SECONDS),at("2026-10-17T06:59:45+08:00"));
    }
    #[test]
    fn delayed_absolute_deadline_drives_preparation_and_recheck() {
        let fire = at("2026-10-23T07:00:10+08:00");
        assert_eq!(start_at(fire, true), at("2026-10-23T06:59:10+08:00"));
        assert_eq!(fire-chrono::Duration::seconds(RECHECK_SECONDS), at("2026-10-23T06:59:55+08:00"));
        let now = Cell::new(at("2026-10-23T07:00:00+08:00"));
        let waited = Cell::new(0);
        wait_with(fire, ||true, ||now.get(), |duration| {
            waited.set(waited.get()+1);
            now.set(now.get()+chrono::Duration::milliseconds(duration.as_millis() as i64));
        }).unwrap();
        assert!(waited.get()>0 && now.get()>=fire);
    }

    #[test]
    fn readiness_never_fires_early_even_if_the_clock_moves_back() {
        let fire = at("2026-10-17T07:00:00+08:00");
        let remaining = Cell::new(120_i64);
        let sleeps = Cell::new(0);
        wait_with(fire, || true, || fire - chrono::Duration::milliseconds(remaining.get()), |duration| {
            sleeps.set(sleeps.get() + 1);
            remaining.set(if sleeps.get() == 1 { 220 } else { remaining.get() - duration.as_millis() as i64 });
        }).unwrap();
        assert!(remaining.get() <= 0 && sleeps.get() > 2);
    }

    #[test]
    fn cancellation_prevents_fire_even_after_deadline() {
        let fire = at("2026-10-17T07:00:00+08:00");
        assert_eq!(wait_with(fire, || false, || fire, |_| panic!("must not wait")), Err("已取消".into()));
        let active = Cell::new(true);
        assert_eq!(wait_with(fire, || active.get(), || fire - chrono::Duration::seconds(100), |_| active.set(false)), Err("已取消".into()));
    }

    #[test]
    fn late_start_runs_immediately_instead_of_waiting_again() {
        let fire = at("2026-10-17T07:00:00+08:00");
        wait_with(fire, || true, || fire + chrono::Duration::seconds(30), |_| panic!("already due")).unwrap();
    }
}
