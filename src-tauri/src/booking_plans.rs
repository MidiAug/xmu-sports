use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Mutex, time::Duration as Wait};
use tauri::{Emitter, Manager};

#[derive(Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub sport: String,
    pub venue: String,
    pub start: String,
    pub end: String,
    pub weekdays: Vec<u32>,
    pub enabled: bool,
    pub account: String,
    #[serde(default)]
    pub first_date: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub task: crate::BookingTask,
    pub sport: String,
    pub account: String,
    pub status: String,
    pub message: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Plans {
    pub rules: Vec<Plan>,
    pub paused: bool,
    pub attempts: Vec<Attempt>,
    pub notified: Vec<String>,
}

pub struct Store(Mutex<Result<Plans, String>>);

#[derive(Serialize)]
pub struct PlanView {
    pub plans: Plans,
    pub next: Vec<Value>,
    wechat_running: bool,
}

pub fn china_now() -> DateTime<FixedOffset> {
    chrono::Utc::now().with_timezone(&FixedOffset::east_opt(28800).unwrap())
}

fn persist(plans: &Plans) -> Result<(), String> {
    crate::atomic_fs::write_atomic(
        &crate::data_root().join("booking-plans.json"),
        serde_json::to_vec_pretty(plans).map_err(|e| e.to_string())?,
    )
}

fn load() -> Result<Plans, String> {
    let path = crate::data_root().join("booking-plans.json");
    if !path.exists() {
        return Ok(Plans::default());
    }
    let plans: Plans = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("长期计划读取失败，原文件已保留：{e}"))?;
    validate(&plans.rules)?;
    Ok(plans)
}

fn catalog() -> Value {
    serde_json::from_str(include_str!("../../src/catalog/venues.json")).unwrap()
}

fn validate(rules: &[Plan]) -> Result<(), String> {
    let catalog = catalog();
    let mut ids = HashSet::new();
    let mut days = HashSet::new();
    for rule in rules {
        if uuid::Uuid::parse_str(&rule.id).is_err() || !ids.insert(&rule.id) {
            return Err("计划标识无效或重复".into());
        }
        if !crate::pending_orders::valid_reference(&rule.account) {
            return Err("请先检查登录，绑定本次计划的账号".into());
        }
        if rule.weekdays.is_empty() || rule.weekdays.iter().any(|day| !(1..=7).contains(day)) {
            return Err("请选择运动星期".into());
        }
        let sport = catalog["sports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == rule.sport && s["enabled"] != false)
            .ok_or("运动不存在")?;
        let venue = sport["venues"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == rule.venue)
            .ok_or("场馆不存在")?;
        if !venue["time_slots"]
            .as_array()
            .unwrap()
            .iter()
            .any(|slot| slot["start"] == rule.start && slot["end"] == rule.end)
        {
            return Err("请选择场馆提供的完整时段".into());
        }
        for day in rule.weekdays.iter().copied().collect::<HashSet<_>>() {
            if rule.enabled && !days.insert((&rule.account, &rule.sport, day)) {
                return Err("同一账号每项运动每天只能安排一次，请修改已有计划".into());
            }
        }
    }
    Ok(())
}

pub fn next_date(rule: &Plan, plans: &Plans, now: DateTime<FixedOffset>) -> NaiveDate {
    let first = NaiveDate::parse_from_str(&rule.first_date, "%Y-%m-%d").unwrap_or(now.date_naive());
    let start = (now.date_naive() + Duration::days(1)).max(first);
    (0..)
        .map(|offset| start + Duration::days(offset))
        .find(|date| {
            rule.weekdays.contains(&date.weekday().number_from_monday())
                && !plans.attempts.iter().any(|a| {
                    a.account == rule.account
                        && a.sport == rule.sport
                        && a.task.target_date == date.to_string()
                })
        })
        .unwrap()
}

fn fire_at(date: NaiveDate) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(28800)
        .unwrap()
        .from_local_datetime(
            &(date - Duration::days(1)).and_time(NaiveTime::from_hms_opt(7, 0, 0).unwrap()),
        )
        .unwrap()
}

fn skip_date(rule: &Plan, plans: &Plans, now: DateTime<FixedOffset>) -> NaiveDate {
    plans.attempts.iter().find(|a| a.status == "queued" && a.task.plan_id.as_deref() == Some(&rule.id)
        && a.task.target_date > now.date_naive().to_string())
        .and_then(|a| NaiveDate::parse_from_str(&a.task.target_date, "%Y-%m-%d").ok())
        .unwrap_or_else(|| next_date(rule, plans, now))
}

fn task(rule: &Plan, date: NaiveDate) -> crate::BookingTask {
    let catalog = catalog();
    let sport = catalog["sports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == rule.sport)
        .unwrap();
    let venue = sport["venues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == rule.venue)
        .unwrap();
    serde_json::from_value(json!({"task_id":format!("plan-{}-{date}",rule.id),"plan_id":rule.id,
        "sport":sport["ocr_label"],"venue":venue["ocr_label"],"target_date":date.to_string(),
        "fire_at":fire_at(date).to_rfc3339(),"target_time_start":rule.start,"target_time_end":rule.end,
        "executor":"http","allow_time_fallback":false})).unwrap()
}

fn changed(app: &tauri::AppHandle) {
    crate::tray::refresh_plans(app);
    crate::sync_run_badge(app, &app.state::<crate::AppState>());
    let _ = app.emit("plans-changed", ());
}

fn has_upcoming(plans: &Plans, now: DateTime<FixedOffset>) -> bool {
    if plans.paused || plans.attempts.iter().any(|a| a.status == "unconfirmed") {
        return false;
    }
    if plans.attempts.iter().any(|a| a.status == "queued" || a.status == "running") {
        return true;
    }
    let tomorrow = now.date_naive() + Duration::days(1);
    plans.rules.iter().filter(|p| p.enabled).any(|rule| {
        let fire = fire_at(skip_date(rule, plans, now));
        fire >= now && fire.date_naive() <= tomorrow
    })
}

pub fn upcoming(app: &tauri::AppHandle) -> bool {
    let Some(state) = app.try_state::<Store>() else { return false; };
    let Ok(locked) = state.0.lock() else { return false; };
    locked.as_ref().is_ok_and(|plans| has_upcoming(plans, china_now()))
}

#[tauri::command]
pub fn get_booking_plans(app: tauri::AppHandle) -> Result<PlanView, String> {
    let state = app.state::<Store>();
    let locked = state.0.lock().map_err(|_| "计划状态不可用")?;
    let plans = locked.as_ref().map_err(Clone::clone)?;
    let now = china_now();
    let next = plans
        .rules
        .iter()
        .filter(|p| p.enabled)
        .map(|p| {
            let date = skip_date(p, plans, now);
            json!({"id":p.id,"date":date.to_string(),"fire_at":fire_at(date).to_rfc3339()})
        })
        .collect();
    Ok(PlanView {
        plans: plans.clone(),
        next,
        wechat_running: crate::wechat_probe::probe().running,
    })
}

#[tauri::command]
pub fn save_booking_plans(
    app: tauri::AppHandle,
    mut rules: Vec<Plan>,
    paused: bool,
) -> Result<(), String> {
    validate(&rules)?;
    let state = app.state::<Store>();
    let mut locked = state.0.lock().map_err(|_| "计划状态不可用")?;
    let plans = locked.as_mut().map_err(|e| e.clone())?;
    if !paused && plans.attempts.iter().any(|a| a.status == "unconfirmed") {
        return Err("仍有待确认的预约，请先查单，再将该项标记为已核对并跳过，之后恢复计划".into());
    }
    let now = china_now();
    for rule in &mut rules {
        rule.weekdays.sort_unstable();
        rule.weekdays.dedup();
        rule.first_date = plans
            .rules
            .iter()
            .find(|old| old.id == rule.id)
            .map(|old| old.first_date.clone())
            .unwrap_or_else(|| {
                (now.date_naive()
                    + Duration::days(if now.time() < NaiveTime::from_hms_opt(7, 0, 0).unwrap() {
                        1
                    } else {
                        2
                    }))
                .to_string()
            });
    }
    let mut next = plans.clone();
    next.rules = rules;
    next.paused = paused;
    for attempt in next.attempts.iter_mut().filter(|a| a.status == "queued") {
        if !next
            .rules
            .iter()
            .any(|r| r.enabled && attempt.task.plan_id.as_deref() == Some(&r.id))
        {
            attempt.status = "skipped".into();
            attempt.message = "计划已停用或删除".into();
        }
    }
    let cutoff = (now.date_naive() - Duration::days(30)).to_string();
    next.attempts.retain(|a| a.task.target_date >= cutoff || a.status == "unconfirmed");
    next.notified.retain(|d| d.as_str() >= cutoff.as_str());
    let enabled_ids: Vec<_> = next
        .rules
        .iter()
        .filter(|r| r.enabled)
        .map(|r| r.id.clone())
        .collect();
    persist(&next)?;
    *plans = next;
    drop(locked);
    let runtime = app.state::<crate::AppState>();
    let active = runtime
        .active_task
        .lock()
        .map_err(|_| "任务状态不可用")?
        .clone();
    let armed = crate::load_arm().task;
    if active
        .filter(|_| crate::child_running(&runtime))
        .or(armed)
        .and_then(|t| t.plan_id)
        .is_some_and(|id| paused || !enabled_ids.contains(&id))
    {
        crate::disarm_hang(app.clone(), runtime)?;
    }
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn skip_booking_plan(app: tauri::AppHandle, id: String) -> Result<String, String> {
    let state = app.state::<Store>();
    let mut locked = state.0.lock().map_err(|_| "计划状态不可用")?;
    let plans = locked.as_mut().map_err(|e| e.clone())?;
    let rule = plans
        .rules
        .iter()
        .find(|p| p.id == id)
        .ok_or("计划不存在")?;
    let date = skip_date(rule, plans, china_now());
    let attempt = Attempt {
        task: task(rule, date),
        sport: rule.sport.clone(),
        account: rule.account.clone(),
        status: "skipped".into(),
        message: "已跳过这一次".into(),
    };
    let mut next = plans.clone();
    if let Some(queued) = next
        .attempts
        .iter_mut()
        .find(|a| a.status == "queued" && a.task.plan_id.as_deref() == Some(&id) && a.task.target_date == date.to_string())
    {
        queued.status = "skipped".into();
        queued.message = "已跳过这一次".into();
    } else {
        next.attempts.push(attempt);
    }
    persist(&next)?;
    *plans = next;
    drop(locked);
    changed(&app);
    Ok(date.to_string())
}

#[tauri::command]
pub fn resolve_booking_plan(app: tauri::AppHandle, task_id: String) -> Result<(), String> {
    let state = app.state::<Store>();
    let mut locked = state.0.lock().map_err(|_| "计划状态不可用")?;
    let plans = locked.as_mut().map_err(|e| e.clone())?;
    let mut next = plans.clone();
    let attempt = next
        .attempts
        .iter_mut()
        .find(|a| a.task.task_id == task_id && a.status == "unconfirmed")
        .ok_or("没有对应的待确认记录")?;
    attempt.status = "skipped".into();
    attempt.message = "已人工核对，本项不再自动补约".into();
    persist(&next)?;
    *plans = next;
    drop(locked);
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn booking_plan_account(app: tauri::AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _query = crate::availability::QUERY
            .lock()
            .map_err(|_| "查询状态不可用")?;
        if crate::child_running(&app.state::<crate::AppState>())
            || crate::availability::recovering()
            || crate::my_orders::mutating()
        {
            return Err("请等待当前预约或登录结束".into());
        }
        let mut worker = crate::worker_process::Worker::spawn(
            &crate::http_booking::worker_path(&app)?,
            true,
            true,
        )?;
        let rpc = worker.send(&mut json!({"action":"check"}))?;
        let deadline = std::time::Instant::now() + Wait::from_secs(45);
        while std::time::Instant::now() < deadline {
            match worker.receive(Wait::from_millis(100)) {
                Ok(v) if v["rpc_id"] == rpc && v["kind"] == "result" => {
                    return crate::http_booking::verified_account(&v, None).map_err(|_| {
                        v["message"]
                            .as_str()
                            .unwrap_or("请先在单次预约页检查登录，再启用计划")
                            .to_string()
                    })
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("登录检查中断".into())
                }
                _ => {}
            }
        }
        Err("登录检查超时，请稍后重试".into())
    })
    .await
    .map_err(|_| "登录检查未完成".to_string())?
}

pub fn enabled<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    app.try_state::<Store>().is_some_and(|state| {
        state.0.lock().ok().is_some_and(|v| {
            v.as_ref()
                .is_ok_and(|p| !p.paused && p.rules.iter().any(|r| r.enabled))
        })
    })
}

fn apply_outcome(plans: &mut Plans, task: &crate::BookingTask, outcome: &Result<String, String>) {
    if let Some(attempt) = plans
        .attempts
        .iter_mut()
        .find(|a| a.task.task_id == task.task_id)
    {
        let (status, message) = match outcome {
            Ok(code) if code == "http_submitted" => (
                "unconfirmed",
                "学校已接受，查单尚未核实，请先查看我的预约".into(),
            ),
            Ok(code) => ("succeeded", code.clone()),
            Err(code) if code == "SPORT_DAY_ALREADY_BOOKED" => {
                ("existing", "当天这项运动已有预约".into())
            }
            Err(code) if matches!(code.as_str(), "SUBMIT_UNKNOWN" | "RESULT_UNCONFIRMED") => {
                ("unconfirmed", "提交结果待确认，请先查单；未重复提交".into())
            }
            Err(code) if code == "已取消" => ("cancelled", "已停止本次预约".into()),
            Err(code) => ("failed", code.clone()),
        };
        attempt.status = status.into();
        attempt.message = message;
        // Only known slot-level failures can advance to another sport without intervention.
        if outcome.as_ref().is_ok_and(|code| code == "http_submitted")
            || outcome.as_ref().is_err_and(|code| {
                !matches!(
                    code.as_str(),
                    "已取消"
                        | "SPORT_DAY_ALREADY_BOOKED"
                        | "SLOT_OCCUPIED"
                        | "SLOT_LOCKED"
                        | "SLOT_UNAVAILABLE"
                        | "NO_SLOT"
                        | "BOOKING_CLOSED"
                        | "FALLBACKS_EXHAUSTED"
                        | "CAPTCHA_REJECTED"
                        | "CAPTCHA_UNREADABLE"
                )
            })
        {
            plans.paused = true;
        }
    }
}

pub fn finish(app: &tauri::AppHandle, task: &crate::BookingTask, outcome: &Result<String, String>) {
    if task.plan_id.is_none() {
        return;
    }
    let state = app.state::<Store>();
    let Ok(mut locked) = state.0.lock() else {
        return;
    };
    let Ok(plans) = locked.as_mut() else {
        return;
    };
    apply_outcome(plans, task, outcome);
    let date = &task.target_date;
    let notify_key = format!("{date}-done");
    let pending = plans
        .attempts
        .iter()
        .any(|a| a.task.target_date == *date && a.status == "queued");
    if (plans.paused || !pending) && !plans.notified.contains(&notify_key) {
        let round: Vec<_> = plans
            .attempts
            .iter()
            .filter(|a| a.task.target_date == *date)
            .collect();
        let success = round
            .iter()
            .filter(|a| matches!(a.status.as_str(), "succeeded" | "existing"))
            .count();
        crate::startup::notify(
            app,
            "本轮自动预约结果",
            &format!(
                "{date}：{success} 项已约到或已有预约。{}",
                if plans.paused {
                    "计划已暂停，请打开应用处理未确认或失败项。"
                } else {
                    "打开应用查看明细。"
                }
            ),
        );
        plans.notified.push(notify_key);
    }
    if let Err(error) = persist(plans) {
        plans.paused = true;
        log::error!(target:"plans","[save_failed] {error}");
    }
    drop(locked);
    changed(app);
}

pub fn stop_round(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<Store>();
    let mut locked = state.0.lock().map_err(|_| "计划状态不可用")?;
    let plans = locked.as_mut().map_err(|e| e.clone())?;
    let now = china_now();
    let date = now.date_naive() + Duration::days(1);
    let mut next = plans.clone();
    for attempt in next
        .attempts
        .iter_mut()
        .filter(|a| a.task.target_date == date.to_string() && a.status == "queued")
    {
        attempt.status = "skipped".into();
        attempt.message = "已停止本轮".into();
    }
    for rule in &plans.rules {
        if rule.enabled && next_date(rule, plans, now) == date {
            next.attempts.push(Attempt {
                task: task(rule, date),
                sport: rule.sport.clone(),
                account: rule.account.clone(),
                status: "skipped".into(),
                message: "已停止本轮".into(),
            });
        }
    }
    persist(&next)?;
    *plans = next;
    drop(locked);
    changed(app);
    Ok(())
}

#[tauri::command]
pub fn stop_booking_round(app: tauri::AppHandle) -> Result<(), String> {
    stop_round(&app)?;
    let state = app.state::<crate::AppState>();
    let active = state
        .active_task
        .lock()
        .map_err(|_| "任务状态不可用")?
        .clone();
    if active
        .filter(|_| crate::child_running(&state))
        .or(crate::load_arm().task)
        .is_some_and(|t| t.plan_id.is_some())
    {
        crate::disarm_hang(app.clone(), state)?;
    }
    Ok(())
}

fn due_plan(
    plans: &Plans,
    now: DateTime<FixedOffset>,
) -> Option<(Plan, NaiveDate, DateTime<FixedOffset>)> {
    if plans.paused {
        return None;
    }
    plans.rules.iter().filter(|p| p.enabled).find_map(|p| {
        let date = next_date(p, plans, now);
        let fire = fire_at(date);
        (now >= fire - Duration::seconds(60)).then(|| (p.clone(), date, fire))
    })
}

fn freeze_round(plans: &mut Plans, now: DateTime<FixedOffset>) {
    if let Some((_, date, _)) = due_plan(plans, now) {
        let round: Vec<_> = plans
            .rules
            .iter()
            .filter(|p| p.enabled && next_date(p, plans, now) == date)
            .map(|p| Attempt {
                task: task(p, date),
                sport: p.sport.clone(),
                account: p.account.clone(),
                status: "queued".into(),
                message: "本轮目标已冻结，修改从下一轮生效".into(),
            })
            .collect();
        plans.attempts.extend(round);
    }
}

fn expire_queued(plans: &mut Plans, now: DateTime<FixedOffset>) -> bool {
    let mut changed = false;
    for attempt in plans
        .attempts
        .iter_mut()
        .filter(|a| a.status == "queued" && a.task.target_date <= now.date_naive().to_string())
    {
        attempt.status = "missed".into();
        attempt.message = "已错过当天开抢，未追溯提交".into();
        changed = true;
    }
    changed
}

fn tick(app: &tauri::AppHandle) -> Result<(), String> {
    let runtime = app.state::<crate::AppState>();
    let _gate = runtime.task_gate.lock().map_err(|_| "任务状态不可用")?;
    if crate::child_running(&runtime)
        || crate::availability::recovering()
        || crate::my_orders::mutating()
    {
        return Ok(());
    }
    let state = app.state::<Store>();
    let mut locked = state.0.lock().map_err(|_| "计划状态不可用")?;
    let plans = locked.as_mut().map_err(|e| e.clone())?;
    let now = china_now();
    if expire_queued(plans, now) {
        persist(plans)?;
    }
    if plans.paused {
        return Ok(());
    }
    let mut next = plans.clone();
    freeze_round(&mut next, now);
    let date = (now.date_naive() + Duration::days(1)).to_string();
    let due = next
        .attempts
        .iter()
        .position(|a| a.status == "queued" && a.task.target_date == date);
    if let Some(index) = due {
        if crate::load_arm().armed {
            return Ok(());
        }
        let frozen = next.attempts[index].task.clone();
        let account = next.attempts[index].account.clone();
        let fire = crate::parse_fire_at(&frozen.fire_at)?;
        if now < fire - Duration::seconds(60) {
            return Ok(());
        }
        crate::booking_eligibility::check(&frozen, fire.max(now))?;
        next.attempts[index].status = "running".into();
        next.attempts[index].message = "本轮已准备，修改规则从下一次生效".into();
        let cutoff = (now.date_naive() - Duration::days(30)).to_string();
        next.attempts
            .retain(|a| a.task.target_date >= cutoff || a.status == "unconfirmed");
        next.notified.retain(|d| d.as_str() >= cutoff.as_str());
        persist(&next)?;
        *plans = next;
        let arm = crate::ArmFile {
            armed: true,
            task_id: frozen.task_id.clone(),
            fire_at: fire.to_rfc3339(),
            task: Some(frozen),
            account_fingerprint: Some(account),
        };
        if let Err(error) = crate::save_arm_disk(&arm) {
            plans.paused = true;
            persist(plans)?;
            return Err(error);
        }
        let generation = runtime
            .arm_gen
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        crate::start_arm_waiter(app.clone(), generation, arm.fire_at);
        drop(locked);
        crate::sync_run_badge(app, &runtime);
        let _ = app.emit("arm-changed", ());
        changed(app);
    }
    Ok(())
}

fn remind(app: &tauri::AppHandle) {
    let state = app.state::<Store>();
    let Ok(mut locked) = state.0.lock() else {
        return;
    };
    let Ok(plans) = locked.as_mut() else {
        return;
    };
    if plans.paused || !plans.rules.iter().any(|p| p.enabled) {
        return;
    }
    let wechat_running = crate::wechat_probe::probe().running;
    let now = china_now();
    let date = now.date_naive() + Duration::days(1);
    let fire = fire_at(date);
    let key = format!("{}-wechat", now.date_naive());
    if now >= fire - Duration::minutes(5)
        && now < fire
        && !plans.notified.contains(&key)
        && plans
            .rules
            .iter()
            .any(|p| p.enabled && next_date(p, plans, now) == date)
        && !wechat_running
    {
        plans.notified.push(key);
        if persist(plans).is_ok() {
            crate::startup::notify(
                app,
                "自动预约即将开始",
                "微信尚未运行，请打开并确认登录；现有预约会话会在执行前另行核验。",
            );
        }
    }
}

fn recover_interrupted(plans: &mut Plans, arm: &crate::ArmFile) -> bool {
    let mut changed = false;
    let was_paused = plans.paused;
    // The persisted arm is cleared before a send. Only that pre-send snapshot may resume.
    for attempt in plans.attempts.iter_mut().filter(|a| a.status == "running") {
        if was_paused || !arm.armed || arm.task_id != attempt.task.task_id {
            attempt.status = "unconfirmed".into();
            attempt.message = "上次运行中断，请先查单；未重复提交".into();
            changed = true;
        }
    }
    if changed {
        plans.paused = true;
    }
    changed
}

pub fn start(app: &tauri::AppHandle) {
    let mut loaded = load();
    if let Ok(plans) = &mut loaded {
        let arm = crate::load_arm();
        if recover_interrupted(plans, &arm) | expire_queued(plans, china_now()) {
            if let Err(error) = persist(plans) {
                loaded = Err(error);
            }
        }
    }
    app.manage(Store(Mutex::new(loaded)));
    crate::tray::refresh_plans(app);
    crate::sync_run_badge(app, &app.state::<crate::AppState>());
    let app = app.clone();
    std::thread::spawn(move || {
        let mut counter = 0;
        loop {
            if counter % 60 == 0 {
                crate::sync_run_badge(&app, &app.state::<crate::AppState>());
            }
            if counter % 60 == 0 && enabled(&app) {
                remind(&app);
                let _ = app.emit("plans-health", ());
            }
            if let Err(error) = tick(&app) {
                log::warn!(target:"plans","[scheduler] {error}");
                std::thread::sleep(Wait::from_secs(59));
            }
            counter += 1;
            std::thread::sleep(Wait::from_secs(1));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(value: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(value).unwrap()
    }
    fn rule() -> Plan {
        Plan {
            id: uuid::Uuid::new_v4().to_string(),
            sport: "gym".into(),
            venue: "test".into(),
            start: "16:30".into(),
            end: "18:00".into(),
            weekdays: vec![1, 2, 3, 4, 5, 6, 7],
            enabled: true,
            account: "a".repeat(64),
            first_date: "2026-10-30".into(),
        }
    }
    #[test]
    fn upcoming_badge_tracks_opening_day_and_plan_state() {
        let mut p = catalog_rule();
        p.first_date = "2026-11-01".into();
        p.weekdays = vec![1, 2, 3, 4, 5, 6, 7];
        let mut plans = Plans { rules: vec![p.clone()], ..Plans::default() };
        assert!(has_upcoming(&plans, at("2026-10-30T12:00:00+08:00")), "tomorrow's opening lights the badge");
        assert!(!has_upcoming(&plans, at("2026-10-29T23:59:59+08:00")), "distant opening stays quiet");
        assert!(has_upcoming(&plans, at("2026-10-31T06:00:00+08:00")), "today's future opening lights the badge");
        plans.paused = true;
        assert!(!has_upcoming(&plans, at("2026-10-30T12:00:00+08:00")));
        plans.paused = false;
        plans.rules[0].enabled = false;
        assert!(!has_upcoming(&plans, at("2026-10-30T12:00:00+08:00")));
        plans.rules[0].enabled = true;
        plans.attempts.push(Attempt { task: task(&p, NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()), sport:p.sport.clone(), account:p.account.clone(), status:"unconfirmed".into(), message:String::new() });
        assert!(!has_upcoming(&plans, at("2026-10-30T12:00:00+08:00")));
        plans.attempts[0].status = "queued".into();
        plans.rules.clear();
        assert!(has_upcoming(&plans, at("2026-10-30T12:00:00+08:00")), "frozen queue remains active without rules");
        plans.attempts[0].status = "skipped".into();
        assert!(!has_upcoming(&plans, at("2026-10-30T12:00:00+08:00")));
    }

    #[test]
    fn dates_follow_sport_day_and_roll_over_year() {
        let mut p = rule();
        let plans = Plans::default();
        assert_eq!(
            next_date(&p, &plans, at("2026-10-29T00:30:00+08:00")).to_string(),
            "2026-10-30"
        );
        assert_eq!(
            fire_at(next_date(&p, &plans, at("2026-10-29T00:30:00+08:00"))),
            at("2026-10-29T07:00:00+08:00")
        );
        p.weekdays = vec![1];
        assert_eq!(
            next_date(&p, &plans, at("2026-12-31T12:00:00+08:00")).to_string(),
            "2027-01-04"
        );
    }
    #[test]
    fn outcomes_skip_same_sport_across_rule_ids_but_not_accounts() {
        let p = rule();
        let now = at("2026-10-29T10:00:00+08:00");
        let task=serde_json::from_value(json!({"task_id":"old","sport":"健身","venue":"另一场馆","target_date":"2026-10-30","fire_at":"2026-10-29T07:00:00+08:00","target_time_start":"16:30","target_time_end":"18:00"})).unwrap();
        let mut plans = Plans::default();
        plans.attempts.push(Attempt {
            task,
            sport: p.sport.clone(),
            account: p.account.clone(),
            status: "unconfirmed".into(),
            message: "".into(),
        });
        assert_eq!(next_date(&p, &plans, now).to_string(), "2026-10-31");
        plans.attempts[0].account = "b".repeat(64);
        assert_eq!(next_date(&p, &plans, now).to_string(), "2026-10-30");
    }
    fn catalog_rule() -> Plan {
        let catalog = catalog();
        let sport = catalog["sports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["enabled"] != false)
            .unwrap();
        let venue = &sport["venues"][0];
        let slot = &venue["time_slots"][0];
        Plan {
            sport: sport["id"].as_str().unwrap().into(),
            venue: venue["id"].as_str().unwrap().into(),
            start: slot["start"].as_str().unwrap().into(),
            end: slot["end"].as_str().unwrap().into(),
            ..rule()
        }
    }
    #[test]
    fn validates_catalog_and_weekday_overlap_at_the_boundary() {
        let first = catalog_rule();
        assert!(validate(&[first.clone()]).is_ok());
        let mut copy = first.clone();
        copy.id = uuid::Uuid::new_v4().to_string();
        assert!(validate(&[first.clone(), copy.clone()])
            .unwrap_err()
            .contains("每天只能"));
        copy.enabled = false;
        assert!(validate(&[first.clone(), copy.clone()]).is_ok());
        copy.enabled = true;
        copy.account = "b".repeat(64);
        assert!(validate(&[first.clone(), copy.clone()]).is_ok());
        copy.venue = "unknown".into();
        assert!(validate(&[copy]).is_err());
        let mut bad = first;
        bad.weekdays = vec![];
        assert!(validate(&[bad]).is_err());
    }
    #[test]
    fn queue_uses_priority_and_respects_pause_and_completed_items() {
        let first = catalog_rule();
        let mut second = first.clone();
        second.id = uuid::Uuid::new_v4().to_string();
        second.account = "b".repeat(64);
        let mut plans = Plans {
            rules: vec![first.clone(), second.clone()],
            ..Plans::default()
        };
        let now = at("2026-10-29T06:59:00+08:00");
        assert!(due_plan(&plans, at("2026-10-29T06:58:59+08:00")).is_none());
        assert_eq!(due_plan(&plans, now).unwrap().0.id, first.id);
        plans.attempts.push(Attempt {
            task: task(&first, next_date(&first, &plans, now)),
            sport: first.sport.clone(),
            account: first.account.clone(),
            status: "succeeded".into(),
            message: "".into(),
        });
        assert_eq!(
            due_plan(&plans, at("2026-10-29T10:00:00+08:00"))
                .unwrap()
                .0
                .id,
            second.id
        );
        plans.paused = true;
        assert!(due_plan(&plans, now).is_none());
        plans.paused = false;
        plans.rules[1].enabled = false;
        assert!(due_plan(&plans, now).is_none());
    }
    #[test]
    fn unknown_or_unverified_submission_pauses_queue_but_slot_failure_can_continue() {
        let p = catalog_rule();
        let now = at("2026-10-29T07:00:00+08:00");
        let task = task(&p, next_date(&p, &Plans::default(), now));
        for (outcome, paused, status) in [
            (Err("SUBMIT_UNKNOWN".into()), true, "unconfirmed"),
            (Ok("http_submitted".into()), true, "unconfirmed"),
            (Err("ACCOUNT_CHANGED".into()), true, "failed"),
            (Err("SLOT_OCCUPIED".into()), false, "failed"),
            (Err("SPORT_DAY_ALREADY_BOOKED".into()), false, "existing"),
            (Ok("http_verified".into()), false, "succeeded"),
            (Err("已取消".into()), false, "cancelled"),
        ] {
            let mut plans = Plans::default();
            plans.attempts.push(Attempt {
                task: task.clone(),
                sport: p.sport.clone(),
                account: p.account.clone(),
                status: "running".into(),
                message: "".into(),
            });
            apply_outcome(&mut plans, &task, &outcome);
            assert_eq!(plans.paused, paused);
            assert_eq!(plans.attempts[0].status, status);
        }
    }

    #[test]
    fn frozen_queue_keeps_targets_and_order_and_expires_without_replaying() {
        let first = catalog_rule();
        let mut second = first.clone();
        second.id = uuid::Uuid::new_v4().to_string();
        second.account = "b".repeat(64);
        let mut plans = Plans {
            rules: vec![first.clone(), second.clone()],
            ..Plans::default()
        };
        let now = at("2026-10-29T06:59:00+08:00");
        freeze_round(&mut plans, now);
        assert_eq!(plans.attempts.len(), 2);
        assert_eq!(plans.attempts[0].task.plan_id.as_ref(), Some(&first.id));
        plans.rules.reverse();
        plans.rules[0].start = "22:00".into();
        freeze_round(&mut plans, now);
        assert_eq!(plans.attempts.len(), 2);
        assert_eq!(plans.attempts[1].task.target_time_start, second.start);
        assert!(!expire_queued(&mut plans, at("2026-10-29T23:59:59+08:00")));
        assert!(expire_queued(&mut plans, at("2026-10-30T00:00:00+08:00")));
        assert!(plans.attempts.iter().all(|a| a.status == "missed"));
        assert_eq!(
            next_date(&first, &plans, at("2026-10-30T00:00:00+08:00")).to_string(),
            "2026-10-31"
        );
    }
    #[test]
    fn restart_resumes_only_the_persisted_pre_send_arm() {
        let p = catalog_rule();
        let now = at("2026-10-29T06:59:00+08:00");
        let mut plans = Plans {
            rules: vec![p],
            ..Plans::default()
        };
        freeze_round(&mut plans, now);
        plans.attempts[0].status = "running".into();
        let frozen = plans.attempts[0].task.clone();
        let arm = crate::ArmFile {
            armed: true,
            task_id: frozen.task_id.clone(),
            fire_at: frozen.fire_at.clone(),
            task: Some(frozen),
            account_fingerprint: Some("a".repeat(64)),
        };
        assert!(!recover_interrupted(&mut plans, &arm));
        assert!(!plans.paused);
        assert!(recover_interrupted(&mut plans, &crate::ArmFile::default()));
        assert!(plans.paused);
        assert_eq!(plans.attempts[0].status, "unconfirmed");
        let restored: Plans =
            serde_json::from_str(&serde_json::to_string(&plans).unwrap()).unwrap();
        assert!(due_plan(&restored, now).is_none());
    }

    #[test]
    fn restart_keeps_enabled_rules_and_pause_preference() {
        let plans = Plans { rules: vec![catalog_rule()], ..Plans::default() };
        let restored: Plans = serde_json::from_slice(&serde_json::to_vec(&plans).unwrap()).unwrap();
        let now = at("2026-10-29T06:59:00+08:00");
        assert!(due_plan(&restored, now).is_some());
        let paused = Plans { paused: true, ..restored };
        let restored: Plans = serde_json::from_slice(&serde_json::to_vec(&paused).unwrap()).unwrap();
        assert!(due_plan(&restored, now).is_none());
    }

    #[test]
    fn skip_date_matches_the_queued_target_then_moves_past_running() {
        let rule = catalog_rule();
        let mut plans = Plans { rules: vec![rule.clone()], ..Plans::default() };
        let now = at("2026-10-29T06:59:00+08:00");
        freeze_round(&mut plans, now);
        assert_eq!(skip_date(&rule, &plans, now).to_string(), "2026-10-30");
        plans.attempts[0].status = "running".into();
        assert_eq!(skip_date(&rule, &plans, now).to_string(), "2026-10-31");
    }
}
