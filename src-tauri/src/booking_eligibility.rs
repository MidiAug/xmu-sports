use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, TimeZone};
use crate::BookingTask;

pub fn check(task: &BookingTask, now: DateTime<FixedOffset>) -> Result<(), String> {
    let china = FixedOffset::east_opt(8 * 3600).unwrap();
    let now = now.with_timezone(&china);
    let date = NaiveDate::parse_from_str(&task.target_date, "%Y-%m-%d")
        .map_err(|_| "INVALID_SLOT: 预约日期无效")?;
    let delta = (date - now.date_naive()).num_days();
    if !(0..=1).contains(&delta) {
        return Err("DATE_NOT_OPEN: 立即预约只支持今天或明天，其他日期请等待对应开抢时间".into());
    }
    let start = NaiveTime::parse_from_str(&task.target_time_start, "%H:%M")
        .map_err(|_| "INVALID_SLOT: 预约开始时间无效")?;
    let end = NaiveTime::parse_from_str(&task.target_time_end, "%H:%M")
        .map_err(|_| "INVALID_SLOT: 预约结束时间无效")?;
    if end <= start { return Err("INVALID_SLOT: 预约结束时间必须晚于开始时间".into()); }
    if china.from_local_datetime(&date.and_time(start)).single().unwrap() <= now {
        return Err("BOOKING_CLOSED: 目标时段已经开始或已结束，请选择尚未开始的时段".into());
    }
    Ok(())
}

pub fn check_logged(task: &BookingTask, now: DateTime<FixedOffset>) -> Result<(), String> {
    check(task, now).map_err(|reason| {
        log::warn!(target: "booking", "[eligibility_rejected] task_id={} date={} start={} now={} reason={}",
            task.task_id, task.target_date, task.target_time_start, now, reason);
        reason
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task(date: &str, start: &str) -> BookingTask {
        serde_json::from_value(serde_json::json!({"task_id":"test", "sport":"健身房", "venue":"test",
            "target_date":date, "target_time_start":start, "target_time_end":"22:00", "fire_at":"2026-10-14T07:00:00+08:00"})).unwrap()
    }
    fn at(value: &str) -> DateTime<FixedOffset> { DateTime::parse_from_rfc3339(value).unwrap() }
    #[test]
    fn booking_boundaries() {
        let today = task("2026-10-13", "19:00");
        assert!(check(&today, at("2026-10-13T06:59:59+08:00")).is_ok());
        assert!(check(&today, at("2026-10-13T07:00:00+08:00")).is_ok());
        assert!(check(&today, at("2026-10-13T19:00:00+08:00")).unwrap_err().starts_with("BOOKING_CLOSED"));
        assert!(check(&task("2026-10-14", "19:00"), at("2026-10-13T23:59:59+08:00")).is_ok());
        assert!(check(&task("2026-10-14", "19:00"), at("2026-10-14T00:00:00+08:00")).is_ok());
        assert!(check(&task("2026-10-14", "19:00"), at("2026-10-13T06:59:59+08:00")).is_ok());
        assert!(check(&task("2026-10-15", "19:00"), at("2026-10-13T18:35:00+08:00")).unwrap_err().starts_with("DATE_NOT_OPEN"));
        assert!(check(&task("2026-10-15", "19:00"), at("2026-10-14T07:00:00+08:00")).is_ok());
        assert!(check(&today, at("2026-10-13T11:00:00Z")).is_err());
    }
}
