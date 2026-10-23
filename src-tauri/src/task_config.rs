use crate::BookingTask;

pub fn normalize_http(mut task: BookingTask) -> BookingTask {
    task.executor="http".into();
    task.executor_impl=None;
    task.court_offset=0;
    task
}

/// A running/scheduled task is immutable. Older drafts may never overwrite a
/// newer disk version, even when the request comes from a previous page mount.
pub fn prepare_update(mut next: BookingTask, previous: Option<&BookingTask>, busy: bool) -> Result<BookingTask, String> {
    if busy { return Err("任务已锁定，请先取消定时预约或等待当前流程结束".into()); }
    if next.fire_delay_seconds > 300 { return Err("执行延迟须为 0–300 的整数秒".into()); }
    let revision = previous.filter(|old| old.task_id == next.task_id).map(|old| old.revision).unwrap_or(0);
    if next.revision != revision { return Err("预约配置已被更新，请重新载入配置后再修改".into()); }
    next.revision = revision.checked_add(1).ok_or("配置版本已达上限")?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task() -> BookingTask {
        serde_json::from_value(serde_json::json!({"task_id":"task", "sport":"乒乓球", "venue":"场馆",
            "target_date":"2026-10-17", "fire_at":"2026-10-16T07:00:00+08:00",
            "target_time_start":"19:30", "target_time_end":"21:00"})).unwrap()
    }
    #[test]
    fn legacy_tasks_gain_a_version_and_stale_pages_cannot_overwrite_it() {
        let old = task();
        assert_eq!(old.fire_delay_seconds, 0);
        assert_eq!(old.revision, 0);
        let saved = prepare_update(old.clone(), Some(&old), false).unwrap();
        assert_eq!(saved.revision, 1);
        assert!(prepare_update(old, Some(&saved), false).unwrap_err().contains("重新载入"));
        assert_eq!(prepare_update(saved.clone(), Some(&saved), false).unwrap().revision, 2);
    }
    #[test]
    fn delay_is_bounded_and_frozen_with_the_absolute_fire_time() {
        let old = task();
        let mut next = old.clone();
        next.fire_delay_seconds = 10;
        next.fire_at = "2026-10-16T07:00:10+08:00".into();
        let saved = prepare_update(next, Some(&old), false).unwrap();
        let restored: BookingTask = serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(restored.fire_delay_seconds, 10);
        assert_eq!(restored.fire_at, "2026-10-16T07:00:10+08:00");
        assert!(prepare_update(restored.clone(), Some(&saved), true).is_err());
        let mut invalid = restored; invalid.fire_delay_seconds = 301;
        assert!(prepare_update(invalid, Some(&saved), false).unwrap_err().contains("0–300"));
    }
    #[test]
    fn busy_tasks_reject_changes_to_venue_and_time_as_well_as_dates() {
        let old = task();
        let mut edited = old.clone();
        edited.venue = "另一个场馆".into();
        edited.target_time_start = "18:00".into();
        assert!(prepare_update(edited, Some(&old), true).unwrap_err().contains("锁定"));
    }
    #[test]
    fn migrating_legacy_visual_task_preserves_the_booking_goal() {
        let mut legacy=task();
        legacy.executor="visual".into();
        legacy.executor_impl=Some("native".into());
        legacy.court_offset=2;
        legacy.prefer_court=Some("04".into());
        let migrated=normalize_http(legacy.clone());
        assert_eq!(migrated.executor,"http");
        assert_eq!(migrated.executor_impl,None);
        assert_eq!(migrated.court_offset,0);
        assert_eq!(migrated.prefer_court,legacy.prefer_court);
        assert_eq!(migrated.target_date,legacy.target_date);
        assert_eq!(migrated.target_time_start,legacy.target_time_start);
        assert_eq!(migrated.task_id,legacy.task_id);
        assert_eq!(migrated.revision,legacy.revision);
    }
}
