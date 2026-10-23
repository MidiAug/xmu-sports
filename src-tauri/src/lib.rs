use chrono::{DateTime, FixedOffset, Local};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::webview::PageLoadEvent;
use tauri::{Emitter, Manager, State, WindowEvent};

mod atomic_fs;
mod booking_eligibility;
mod task_config;
mod scheduled_run;
mod diag_export;
mod http_booking;
mod availability;
mod worker_process;
mod pending_orders;
mod worker_diagnostic;
mod normal_login;
mod semantic_login;
mod login_diagnostic;
mod run_control;
mod wechat_window;
mod run_trace;
mod tray;
mod wechat_probe;

pub const APP_DISPLAY_NAME: &str = "XMU Sports";

/// Runs before Tauri setup: cannot load or resume scheduled booking tasks.
pub fn login_check_only(ui: bool, output: &std::path::Path) -> Result<(), String> {
    login_diagnostic::run(ui, output)
}

pub fn worker_check_only(reference:Option<&str>,output:&std::path::Path)->Result<(),String> {
    worker_diagnostic::run(reference,output)
}

pub fn login_elements_only(output: &std::path::Path) -> Result<(), String> {
    let result=semantic_login::Session::open().and_then(|session|session.observe());
    let report=match result {
        Ok(observation)=>serde_json::json!({"ok":true,"observation":observation}),
        Err(code)=>serde_json::json!({"ok":false,"code":code}),
    };
    if let Some(parent)=output.parent() { std::fs::create_dir_all(parent).map_err(|e|e.to_string())?; }
    std::fs::write(output,serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())
}

pub fn login_minimized_check_only(output:&std::path::Path)->Result<(),String> {
    let report=semantic_login::probe_minimized().unwrap_or_else(|code|serde_json::json!({"ok":false,"code":code,"scheduler_started":false,"booking_actions_available":false}));
    if let Some(parent)=output.parent() {std::fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
    std::fs::write(output,serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BookingTask {
    pub task_id: String,
    #[serde(default)]
    pub revision: u64,
    pub sport: String,
    pub venue: String,
    pub target_date: String,
    pub fire_at: String,
    /// Absolute fire_at already includes this user-selected delay.
    #[serde(default)]
    pub fire_delay_seconds: u32,
    pub target_time_start: String,
    pub target_time_end: String,
    #[serde(default)]
    pub allow_time_fallback: bool,
    #[serde(default = "default_time_fallback_order")]
    pub time_fallback_order: String,
    #[serde(default = "default_executor")]
    pub executor: String,
    /// Compatibility field retained when reading older task files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor_impl: Option<String>,
    /// Multi-court only (e.g. "01"). Empty = first available court.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer_court: Option<String>,
    /// Legacy visual offset. HTTP tasks always use zero.
    #[serde(default = "default_court_offset")]
    pub court_offset: u32,
}

fn default_executor() -> String {
    "http".into()
}

fn default_time_fallback_order() -> String {
    "down_then_up".into()
}

fn default_court_offset() -> u32 {
    0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<Value>,
    pub ts: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    pub page_id: Option<String>,
    pub action: Option<String>,
    pub result: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_lease_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor_impl: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveStatus {
    pub task: Option<BookingTask>,
    pub last_event: Option<RunEvent>,
    pub recent_events: Vec<RunEvent>,
    pub running: bool,
    pub armed: bool,
    pub stopping: bool,
    pub operation: Option<String>,
    pub fire_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellPrefs {
    /// 关闭主窗时隐藏到托盘（需 close_pref_set）
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    /// 是否已确认过关闭偏好；false 时询问前端
    #[serde(default)]
    pub close_pref_set: bool,
    /// 「记住我的选择」是否已用过默认勾选（全生命周期仅首次默认勾）
    #[serde(default)]
    pub close_remember_default_used: bool,
    /// 常用运动 catalog id；空/缺省由前端视为全部 enabled
    #[serde(default)]
    pub favorite_sport_ids: Vec<String>,
    /// 场馆 ocr_label → "HH:MM|HH:MM" 上次所选时段
    #[serde(default)]
    pub venue_slot_prefs: std::collections::HashMap<String, String>,
    /// 详细诊断：observations 写入全量 ocr_hits / candidates（可能含界面文字）。
    /// 可用环境变量 `XMU_SPORTS_DETAILED_DIAG=0|1` 覆盖。
    #[serde(default)]
    pub detailed_diagnostics: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ShellPrefs {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            close_pref_set: false,
            close_remember_default_used: false,
            favorite_sport_ids: Vec::new(),
            venue_slot_prefs: std::collections::HashMap::new(),
            detailed_diagnostics: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArmFile {
    armed: bool,
    task_id: String,
    fire_at: String,
    #[serde(default)]
    task: Option<BookingTask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account_fingerprint: Option<String>,
}

impl Default for ArmFile {
    fn default() -> Self {
        Self {
            armed: false,
            task_id: String::new(),
            fire_at: String::new(),
            task: None,
            account_fingerprint: None,
        }
    }
}

#[derive(Clone, Serialize)]
struct CloseAskPayload {
    armed: bool,
}

pub struct AppState {
    /// 每次 arm / disarm 递增，作废旧等待线程
    pub arm_gen: AtomicU64,
    /// 每次启动 / 取消 native 跑递增，作废旧循环
    pub run_gen: AtomicU64,
    pub native_running: AtomicBool,
    pub stopping: AtomicBool,
    task_gate: Mutex<()>,
    active_task: Mutex<Option<BookingTask>>,
    active_operation: Mutex<Option<String>>,
}

fn data_root() -> PathBuf {
    std::env::var("XMU_SPORTS_DATA").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into())).join("xmu-sports")
    })
}

fn tasks_path() -> PathBuf {
    data_root().join("tasks").join("current.json")
}

fn runs_dir() -> PathBuf {
    data_root().join("runs")
}

fn shell_prefs_path() -> PathBuf {
    data_root().join("shell.json")
}

fn arm_path() -> PathBuf {
    data_root().join("arm.json")
}

fn ensure_dirs() -> Result<(), String> {
    fs::create_dir_all(data_root().join("tasks")).map_err(|e| e.to_string())?;
    fs::create_dir_all(runs_dir()).map_err(|e| e.to_string())?;
    Ok(())
}

fn load_shell_prefs() -> ShellPrefs {
    let p = shell_prefs_path();
    if !p.is_file() {
        return ShellPrefs::default();
    }
    match fs::read_to_string(&p) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => ShellPrefs::default(),
    }
}

fn save_shell_prefs_disk(prefs: &ShellPrefs) -> Result<(), String> {
    ensure_dirs()?;
    let raw = serde_json::to_string_pretty(prefs).map_err(|e| e.to_string())?;
    atomic_fs::write_atomic(&shell_prefs_path(), raw)
}

fn load_arm() -> ArmFile {
    let p = arm_path();
    if !p.is_file() {
        return ArmFile::default();
    }
    match fs::read_to_string(&p) {
        Ok(raw) => {
            parse_http_arm(&raw, fs::read_to_string(tasks_path()).ok().as_deref())
        },
        Err(_) => ArmFile::default(),
    }
}

fn parse_http_arm(raw: &str, legacy_task: Option<&str>) -> ArmFile {
    let mut arm: ArmFile=serde_json::from_str(raw).unwrap_or_default();
    let value: Value=serde_json::from_str(raw).unwrap_or_default();
    let task=if value["task"].is_object() {Some(value["task"].clone())}
        else {legacy_task.and_then(|text|serde_json::from_str::<Value>(text).ok())};
    // Missing executor meant vision in earlier versions. Require explicit HTTP
    // before resuming, including old arm files without a task snapshot.
    if !task.as_ref().is_some_and(|task|task["executor"]=="http") {arm.armed=false;}
    arm
}

#[cfg(test)]
mod lite_migration_tests {
    use super::*;
    #[test]
    fn only_explicit_http_schedules_resume_after_upgrade() {
        let base=serde_json::json!({"armed":true,"task_id":"task","fire_at":"2026-10-22T07:00:00+08:00"});
        for executor in [None,Some("vision"),Some("http")] {
            let mut task=serde_json::json!({"task_id":"task","sport":"乒乓球","venue":"场馆",
                "target_date":"2026-10-23","fire_at":"2026-10-22T07:00:00+08:00",
                "target_time_start":"19:30","target_time_end":"21:00"});
            if let Some(executor)=executor {task["executor"]=serde_json::json!(executor);}
            let mut snapshot=base.clone();snapshot["task"]=task.clone();
            assert_eq!(parse_http_arm(&snapshot.to_string(),None).armed,executor==Some("http"));
            assert_eq!(parse_http_arm(&base.to_string(),Some(&task.to_string())).armed,executor==Some("http"));
        }
        assert!(!parse_http_arm(&base.to_string(),None).armed);
    }
}

fn save_arm_disk(arm: &ArmFile) -> Result<(), String> {
    ensure_dirs()?;
    let raw = serde_json::to_string_pretty(arm).map_err(|e| e.to_string())?;
    atomic_fs::write_atomic(&arm_path(), raw)
}

fn read_task() -> Result<Option<BookingTask>, String> {
    let p = tasks_path();
    if !p.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let t: BookingTask = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(Some(task_config::normalize_http(t)))
}

fn child_running(state: &AppState) -> bool {
    state.native_running.load(Ordering::SeqCst)
}

pub(crate) fn kill_child(state: &AppState) {
    if child_running(state) { state.stopping.store(true, Ordering::SeqCst); }
    state.run_gen.fetch_add(1, Ordering::SeqCst);
    // Keep busy until the run actually exits; an in-flight HTTP send may still finish.
    log::info!(target: "vision", "[kill] native_run cancelled");
}

fn parse_fire_at(raw: &str) -> Result<DateTime<FixedOffset>, String> {
    DateTime::parse_from_rfc3339(raw)
        .or_else(|_| DateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%z"))
        .map_err(|e| format!("fire_at 无效: {e}"))
}

fn green_dot_rgba() -> Vec<u8> {
    let size = 16usize;
    let mut rgba = vec![0u8; size * size * 4];
    let cx = 12.0_f32;
    let cy = 4.0_f32;
    let r = 3.6_f32;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            if dx * dx + dy * dy <= r * r {
                let i = (y * size + x) * 4;
                rgba[i] = 0x3d;
                rgba[i + 1] = 0xd6;
                rgba[i + 2] = 0x8c;
                rgba[i + 3] = 0xff;
            }
        }
    }
    rgba
}

/// 在图标右上角叠绿色圆点（托盘用）。
fn icon_with_run_badge(base: &Image<'_>) -> Image<'static> {
    let w = base.width() as usize;
    let h = base.height() as usize;
    let mut rgba = base.rgba().to_vec();
    let cx = w as f32 - (w as f32 * 0.22);
    let cy = h as f32 * 0.22;
    let r = (w.min(h) as f32 * 0.16).max(3.0);
    let r2 = r * r;
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            if dx * dx + dy * dy <= r2 {
                let i = (y * w + x) * 4;
                rgba[i] = 0x3d;
                rgba[i + 1] = 0xd6;
                rgba[i + 2] = 0x8c;
                rgba[i + 3] = 0xff;
            }
        }
    }
    Image::new_owned(rgba, base.width(), base.height())
}

fn apply_run_badge(app: &tauri::AppHandle, on: bool) {
    if let Some(win) = app.get_webview_window("main") {
        if on {
            let img = Image::new_owned(green_dot_rgba(), 16, 16);
            let _ = win.set_overlay_icon(Some(img));
        } else {
            let _ = win.set_overlay_icon(None::<Image>);
        }
    }

    if let Some(tray) = app.tray_by_id(tray::tray_id()) {
        let _ = tray.set_tooltip(Some(APP_DISPLAY_NAME));
        if let Some(base) = app.default_window_icon() {
            if on {
                let _ = tray.set_icon(Some(icon_with_run_badge(base)));
            } else {
                let _ = tray.set_icon(Some(base.clone()));
            }
        }
    }
}

fn sync_run_badge(app: &tauri::AppHandle, state: &AppState) {
    let on = load_arm().armed || child_running(state);
    apply_run_badge(app, on);
}

fn date_offset_days(target_date: &str) -> Result<i64, String> {
    use chrono::NaiveDate;
    let target = NaiveDate::parse_from_str(target_date, "%Y-%m-%d")
        .map_err(|e| format!("target_date 无效: {e}"))?;
    let today = chrono::Utc::now().with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap()).date_naive();
    Ok((target - today).num_days())
}

pub(crate) fn trace_event(
    trace: &run_trace::RunTraceHandle,
    stage: &str,
    page_id: Option<&str>,
    reason_code: &str,
    step_id: Option<&str>,
    frame_id: Option<&str>,
) {
    let _ = trace.try_event(
        stage,
        page_id,
        reason_code,
        step_id,
        frame_id,
        None,
        serde_json::json!({}),
    );
}

fn history_to_run_event(h: run_trace::HistoryEvent) -> RunEvent {
    RunEvent {
        http: h.http,
        ts: h.ts,
        task_id: h.task_id,
        run_id: h.run_id,
        schema_version: h.schema_version,
        page_id: h.page_id,
        action: h.action,
        result: h.result,
        evidence: h.evidence,
        runtime_version: None,
        platform: h.platform,
        surface_lease_hash: None,
        capture_backend: None,
        reason_code: h.reason_code,
        action_plan_id: None,
        executor_impl: h.executor_impl,
    }
}

fn spawn_http_inner(app: tauri::AppHandle, state: &AppState, until: Option<String>) -> Result<String, String> {
    let _guard = state.task_gate.lock().map_err(|_| "任务状态不可用")?;
    if availability::recovering() {return Err("场次登录恢复中，请稍候或取消登录".into());}
    if load_arm().armed { return Err("已有定时任务，请先取消定时预约".into()); }
    ensure_dirs()?;
    let task = read_task()?.ok_or_else(|| "请先在任务页配置预约目标".to_string())?;
    spawn_task_inner(app, state, until, task)
}

fn spawn_task_inner(app: tauri::AppHandle, state: &AppState, until: Option<String>, task: BookingTask) -> Result<String, String> {
    spawn_task_with_schedule(app, state, until, task, None)
}

fn spawn_task_with_schedule(
    app: tauri::AppHandle, state: &AppState, until: Option<String>, task: BookingTask,
    schedule: Option<scheduled_run::ScheduledRun>,
) -> Result<String, String> {
    let auth_only = task.executor == "http" && until.as_deref() == Some("auth");
    if !auth_only {
        let now = Local::now().fixed_offset();
        booking_eligibility::check_logged(&task, schedule.as_ref().map(|s| s.fire_at.max(now)).unwrap_or(now))?;
    }
    if child_running(state) {
        return Err("已有 HTTP 任务在执行".into());
    }

    let offset = date_offset_days(&task.target_date)?;
    if offset < 0 && !auth_only {
        return Err("预约日期早于今天，请回任务页调整".into());
    }

    if task.executor!="http" {return Err("轻量版仅支持 HTTP，请重新保存预约配置".into());}
    let prepare_http=until.as_deref()==Some("trial_slot") || until.as_deref()==Some("prepare");
    let until_label=if auth_only {"http_auth_check"} else if prepare_http {"http_prepare"} else {"http_submit"};

    if state.native_running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return Err("已有预约任务在执行或正在停止".into());
    }
    state.stopping.store(false, Ordering::SeqCst);

    let run_gen = state.run_gen.fetch_add(1, Ordering::SeqCst) + 1;
    // Freeze run_id before spawn event / miniprogram open / session (AC-RS / Task 0.3).
    let run_id = format!(
        "{}-g{}",
        Local::now().format("%Y%m%d-%H%M%S%.3f"),
        run_gen
    );

    let prefs = load_shell_prefs();
    let mut trace_cfg = run_trace::RunTraceConfig::default();
    trace_cfg.detailed_diagnostics =
        run_trace::detailed_diagnostics_from_env(prefs.detailed_diagnostics);
    let task_summary = serde_json::json!({
        "revision": task.revision,
        "sport": task.sport,
        "venue": task.venue,
        "target_date": task.target_date,
        "target_time_start": task.target_time_start,
        "target_time_end": task.target_time_end,
        "fire_at": task.fire_at,
        "executor": task.executor,
        "prefer_court": task.prefer_court,
        "court_offset": task.court_offset,
        "allow_time_fallback": task.allow_time_fallback,
        "time_fallback_order": task.time_fallback_order,
    });
    let trace = run_trace::RunTraceHandle::open(
        &runs_dir(),
        &task.task_id,
        &run_id,
        task_summary,
        Some(until_label),
        trace_cfg,
    ).map_err(|error| {
        state.native_running.store(false, Ordering::SeqCst);
        error
    })?;
    trace_event(&trace, "spawn", None, if auth_only { "auth_check_started" } else if prepare_http { "prepare_started" } else { "started" }, None, None);
    *state.active_task.lock().unwrap_or_else(|error| error.into_inner()) = Some(task.clone());
    *state.active_operation.lock().unwrap_or_else(|error| error.into_inner()) = Some(if schedule.is_some() { "scheduled" } else if auth_only { "auth" } else if prepare_http { "prepare" } else { "booking" }.into());


    let task_id = task.task_id.clone();
    let log_task_id = task_id.clone();
    let log_target_date = task.target_date.clone();

    thread::spawn(move || {
        let outcome = (|| {
            {
                let expected = schedule.as_ref().and_then(|s| s.account_fingerprint.clone());
                let auth = http_booking::ensure_auth(&app, run_gen, &trace, expected.as_deref(), schedule.is_some())?;
                if auth["ok"] != true {
                    return Err(http_booking::auth_failure(&trace, &auth));
                }
                let prepared_account = http_booking::verified_account(&auth, expected.as_deref())
                    .map_err(|code| http_booking::auth_failure(&trace, &serde_json::json!({"code":code})))?;
                if auth_only {
                    let _ = trace.try_event("done", Some("http_authenticating"), "http_auth_ready", None, None, None,
                        serde_json::json!({"http":{"phase":"authenticating","message":"体育登录已就绪，未提交预约"}}));
                    return Ok("http_auth_ready".into());
                }
                let mut warmed=None;
                if let Some(schedule) = &schedule {
                    {
                        let state = app.state::<AppState>();
                        let _guard = state.task_gate.lock().map_err(|_| "任务状态不可用")?;
                        if !scheduled_run_active(&app, run_gen, schedule.arm_gen) { return Err("已取消".into()); }
                        let mut arm = load_arm();
                        if !arm.armed || arm.task_id != task.task_id { return Err("已取消".into()); }
                        arm.account_fingerprint = Some(prepared_account.clone());
                        save_arm_disk(&arm)?;
                    }
                    // Persist the bound account before allocating a warm process;
                    // a crash during model loading must not lose the account fence.
                    warmed=Some(http_booking::PreparedWorker::prepare(&app,run_gen,&trace,&task,&prepared_account)?);
                    log::info!(target: "arm", "[h5_prepared] run_id={} fire_at={} seconds_to_fire={}", run_id, schedule.fire_at,
                        (schedule.fire_at - Local::now().fixed_offset()).num_seconds());
                    let _ = trace.try_event("http_phase", Some("http_scheduled_h5_ready"), "scheduled_h5_ready", None, None, None,
                        serde_json::json!({"http":{"phase":"scheduled_h5_ready","fire_at":schedule.fire_at.to_rfc3339()}}));
                    let _ = app.emit("run-changed", ());
                    scheduled_run::wait_until(schedule.fire_at - chrono::Duration::seconds(scheduled_run::RECHECK_SECONDS),
                        || scheduled_run_active(&app, run_gen, schedule.arm_gen))?;
                    // Check the cached H5 just before opening; only an expired/missing
                    // entry triggers another native login. Keep the prepared account.
                    let current = http_booking::ensure_auth(&app, run_gen, &trace, Some(&prepared_account), true)?;
                    if current["ok"] != true { return Err(http_booking::auth_failure(&trace, &current)); }
                    if let Some(warmed)=warmed.as_mut() {warmed.recheck(&app,run_gen,&trace,&task,&prepared_account)?;}
                    let _ = trace.try_event("http_phase", Some("http_scheduled_waiting_fire"), "scheduled_waiting_fire", None, None, None,
                        serde_json::json!({"http":{"phase":"scheduled_waiting_fire","fire_at":schedule.fire_at.to_rfc3339()}}));
                    let _ = app.emit("run-changed", ());
                    scheduled_run::wait_until(schedule.fire_at, || scheduled_run_active(&app, run_gen, schedule.arm_gen))?;
                    {
                        let state = app.state::<AppState>();
                        let _guard = state.task_gate.lock().map_err(|_| "任务状态不可用")?;
                        if !scheduled_run_active(&app, run_gen, schedule.arm_gen) { return Err("已取消".into()); }
                        clear_arm_file()?;
                    }
                    log::info!(target: "arm", "[fire_prepared] run_id={} task_id={} fire_at={}", run_id, task_id, schedule.fire_at);
                    let _ = app.emit("arm-changed", ());
                    booking_eligibility::check_logged(&task, Local::now().fixed_offset())?;
                }
                return http_booking::run(&app, run_gen, &task, &trace, prepare_http, Some(&prepared_account),warmed.as_mut());
            }
        })();
        // A cancellation during the scheduled wait has no worker result to write it.
        if schedule.is_some() {
            if let Err(error) = &outcome {
                let cancelled = error == "已取消";
                let already_reported = run_trace::read_recent_run_events(&runs_dir(), &task_id, 1).ok()
                    .and_then(|events| events.last().cloned()).is_some_and(|event|
                        event.run_id.as_deref() == Some(run_id.as_str()) &&
                        matches!(event.action.as_deref(), Some("fail" | "cancelled")));
                if !already_reported {
                    let _ = trace.try_event(if cancelled { "cancelled" } else { "fail" }, None,
                        if cancelled { "cancelled" } else { error }, None, None, None, serde_json::json!({}));
                }
            }
            let state = app.state::<AppState>();
            if let Ok(_guard) = state.task_gate.lock() {
                if schedule.as_ref().is_some_and(|s| s.arm_gen == state.arm_gen.load(Ordering::SeqCst)) {
                    let _ = clear_arm_file();
                }
            }
            let _ = app.emit("arm-changed", ());
        }
        let (status, code) = match &outcome {
            Ok(c) => ("succeeded", Some(c.as_str())),
            Err(e) if e == "已取消" => ("cancelled", Some("cancelled")),
            Err(e) if matches!(e.as_str(), "SUBMIT_UNKNOWN" | "RESULT_UNCONFIRMED") => ("unconfirmed", Some(e.as_str())),
            Err(e) => ("failed", Some(e.as_str())),
        };
        let finish = trace.finish(status, code);
        if finish.trace_degraded {
            log::warn!(
                target: "vision",
                "[trace_degraded] task_id={} run_id={} dropped_debug={} writer_errors={} flush_ok={}",
                task_id,
                run_id,
                finish.metrics.dropped_debug_count,
                finish.metrics.writer_error_count,
                finish.flush_ok
            );
        }
        if let Some(state) = app.try_state::<AppState>() {
            state.stopping.store(false, Ordering::SeqCst);
            state.native_running.store(false, Ordering::SeqCst);
            sync_run_badge(&app, &state);
        }
        let _ = app.emit("run-changed", ());
        match outcome {
            Ok(code) => log::info!(target: "vision", "[native_done] task_id={task_id} result={code}"),
            Err(e) if matches!(e.as_str(), "SUBMIT_UNKNOWN" | "RESULT_UNCONFIRMED") =>
                log::warn!(target: "http", "[booking_unconfirmed] task_id={task_id} result={e}"),
            Err(e) => log::error!(target: "vision", "[native_failed] task_id={task_id} err={e}"),
        }
    });

    log::info!(
        target: "vision",
        "[spawn_native] task_id={} until={} target_date={} today={} offset={}",
        log_task_id,
        until_label,
        log_target_date,
        Local::now().date_naive(),
        offset
    );
    Ok(format!(
        "已启动 Native 视觉执行 task={} until={} offset={}",
        log_task_id, until_label, offset
    ))
}


pub(crate) fn clear_arm_file() -> Result<(), String> {
    save_arm_disk(&ArmFile::default())
}

fn start_arm_waiter(app: tauri::AppHandle, gen: u64, fire_at: String) {
    thread::spawn(move || {
        loop {
            let state = app.state::<AppState>();
            if state.arm_gen.load(Ordering::SeqCst) != gen {
                return;
            }
            let arm = load_arm();
            if !arm.armed {
                return;
            }
            let Ok(deadline) = parse_fire_at(&fire_at) else {
                let _ = clear_arm_file();
                log::warn!(target: "arm", "[resume_cleared] reason=bad_fire_at");
                sync_run_badge(&app, &state);
                return;
            };
            let now = Local::now().fixed_offset();
            let snapshot = arm.task.clone().or_else(|| read_task().ok().flatten());
            let http = snapshot.as_ref().is_some_and(|task| task.executor == "http");
            let start_at = scheduled_run::start_at(deadline, http);
            if now >= start_at {
                let Ok(_guard) = state.task_gate.lock() else { return; };
                if state.arm_gen.load(Ordering::SeqCst) != gen { return; }
                // HTTP keeps its persisted arm until fire, so a restart in the
                // one-minute preparation window can resume the same snapshot.
                let task_id = arm.task_id.clone();
                if !http { let _ = clear_arm_file(); }
                let result = snapshot.ok_or_else(|| "定时任务配置丢失".to_string()).and_then(|task|
                    spawn_task_with_schedule(app.clone(), &state, None, task,
                        http.then_some(scheduled_run::ScheduledRun { arm_gen: gen, fire_at: deadline,
                            account_fingerprint: arm.account_fingerprint.clone() })));
                match result {
                    Ok(_) => log::info!(target: "arm", "[{}] task_id={} fire_at={}", if http { "prepare_start" } else { "fire" }, task_id, deadline),
                    Err(e) => { let _ = clear_arm_file(); log::error!(target: "arm", "[fire_failed] err={e}"); },
                }
                sync_run_badge(&app, &state);
                let _ = app.emit("arm-changed", ());
                return;
            }
            thread::sleep(Duration::from_millis(run_control::fire_wait_ms(
                (start_at - now).num_milliseconds(),
            )));
        }
    });
}

fn scheduled_run_active(app: &tauri::AppHandle, run_gen: u64, arm_gen: u64) -> bool {
    let state = app.state::<AppState>();
    state.arm_gen.load(Ordering::SeqCst) == arm_gen && run_control::run_still_active(app, run_gen)
}

fn resume_arm_if_needed(app: &tauri::AppHandle) {
    let arm = load_arm();
    if !arm.armed {
        return;
    }
    let Ok(Some(task)) = read_task() else {
        let _ = clear_arm_file();
        log::warn!(target: "arm", "[resume_cleared] reason=no_task");
        return;
    };
    if task.task_id != arm.task_id {
        let _ = clear_arm_file();
        log::warn!(target: "arm", "[resume_cleared] reason=task_mismatch");
        return;
    }
    let state = app.state::<AppState>();
    let gen = state.arm_gen.fetch_add(1, Ordering::SeqCst) + 1;
    start_arm_waiter(app.clone(), gen, arm.fire_at.clone());
    sync_run_badge(app, &state);
    log::info!(
        target: "arm",
        "[resume] fire_at={} task_id={}",
        arm.fire_at,
        arm.task_id
    );
}

#[tauri::command]
fn save_task(
    _app: tauri::AppHandle,
    state: State<'_, AppState>,
    task: BookingTask,
) -> Result<BookingTask, String> {
    let task=task_config::normalize_http(task);
    let _guard = state.task_gate.lock().map_err(|_| "任务状态不可用")?;
    ensure_dirs()?;
    let prev_arm = load_arm();
    let running = child_running(&state);
    let previous = read_task()?;
    let task = task_config::prepare_update(task, previous.as_ref(), prev_arm.armed || running)?;

    let p = tasks_path();
    let raw = serde_json::to_string_pretty(&task).map_err(|e| e.to_string())?;
    atomic_fs::write_atomic(&p, raw)?;
    let run = runs_dir().join(&task.task_id);
    fs::create_dir_all(&run).map_err(|e| e.to_string())?;
    Ok(task)
}

#[tauri::command]
fn get_live_status(state: State<'_, AppState>) -> Result<LiveStatus, String> {
    let _guard = state.task_gate.lock().map_err(|_| "任务状态不可用")?;
    let running = child_running(&state);
    let arm = load_arm();
    let active = state.active_task.lock().map_err(|_| "任务状态不可用")?.clone();
    let snapshot = if running { active } else if arm.armed { arm.task.clone() } else { None };
    let task = match snapshot { Some(task) => Some(task), None => read_task()? };
    let mut recent_events: Vec<RunEvent> = match &task {
        Some(t) => run_trace::read_recent_run_events(&runs_dir(), &t.task_id, 120)?.into_iter().map(history_to_run_event).collect(),
        None => Vec::new(),
    };
    recent_events.extend(run_trace::read_recent_run_events(&runs_dir(),pending_orders::TRACE_TASK,120)?.into_iter().map(history_to_run_event));
    recent_events.sort_by(|a,b|a.ts.cmp(&b.ts));
    if recent_events.len()>120 {recent_events.drain(..recent_events.len()-120);}
    let last_event = recent_events.last().cloned();
    let armed = arm.armed
        && task
            .as_ref()
            .map(|t| t.task_id == arm.task_id)
            .unwrap_or(false);
    Ok(LiveStatus {
        task,
        last_event,
        recent_events,
        running,
        armed,
        stopping: state.stopping.load(Ordering::SeqCst) && running,
        operation: if running { state.active_operation.lock().unwrap_or_else(|error| error.into_inner()).clone() } else { None },
        fire_at: if armed {
            Some(arm.fire_at)
        } else {
            None
        },
    })
}

#[tauri::command]
fn list_recent_events(limit: usize) -> Result<Vec<RunEvent>, String> {
    ensure_dirs()?;
    let mut all: Vec<RunEvent> = run_trace::collect_history_events(&runs_dir())?
        .into_iter()
        .map(history_to_run_event)
        .collect();
    // limit == 0：不截断（历史页需整次 run 全量事件，否则步数超限刷屏会顶掉前缀）
    if limit > 0 {
        all.truncate(limit);
    }
    Ok(all)
}

/// 导出诊断 zip 到系统 Downloads（日志 + 近期 runs 事件）。
#[tauri::command]
fn export_diagnostic_logs(app: tauri::AppHandle) -> Result<String, String> {
    let download = app
        .path()
        .download_dir()
        .map_err(|e| format!("无法解析下载目录: {e}"))?;
    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|e| format!("无法解析日志目录: {e}"))?;
    let version = app.package_info().version.to_string();
    let out = diag_export::write_diagnostic_zip(
        &download,
        &log_dir,
        &runs_dir(),
        &version,
        Local::now(),
    )?;
    if let Err(e) = diag_export::reveal_in_folder(&out) {
        log::warn!(target: "diag", "[reveal_failed] err={e}");
    }
    log::info!(
        target: "diag",
        "[export_ok] path={}",
        out.display()
    );
    Ok(out.to_string_lossy().into_owned())
}

/// 在资源管理器中打开 App LogDir。
#[tauri::command]
fn open_app_log_dir(app: tauri::AppHandle) -> Result<String, String> {
    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|e| format!("无法解析日志目录: {e}"))?;
    fs::create_dir_all(&log_dir).map_err(|e| format!("无法创建日志目录: {e}"))?;
    diag_export::open_dir(&log_dir)?;
    Ok(log_dir.to_string_lossy().into_owned())
}

/// 清空历史页所用的运行事件（新旧布局下的 `events.jsonl`）。
/// 执行中禁止清空，避免与正在写入的 trace writer 冲突。
#[tauri::command]
fn clear_run_history(state: State<'_, AppState>) -> Result<String, String> {
    if child_running(&state) {
        return Err("HTTP 任务仍在运行，请先结束后再清空".into());
    }
    ensure_dirs()?;
    let n = run_trace::clear_all_events(&runs_dir())?;
    Ok(if n == 0 {
        "没有可清空的记录".into()
    } else {
        "已清空运行记录".into()
    })
}

#[tauri::command]
fn start_http_run(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    until: Option<String>,
) -> Result<String, String> {
    http_booking::validate_operation(until.as_deref())?;
    let out = spawn_http_inner(app.clone(), &state, until)?;
    sync_run_badge(&app, &state);
    Ok(out)
}

#[tauri::command]
fn list_pending_orders()->Vec<Value> {pending_orders::list(&data_root())}

#[tauri::command]
async fn recheck_order(app:tauri::AppHandle,request_ref:String)->Result<Value,String> {
    if !pending_orders::valid_reference(&request_ref) {return Err("原请求标识不合法".into());}
    let run_gen;
    {
        let state=app.state::<AppState>();
        let _guard=state.task_gate.lock().map_err(|_|"任务状态不可用")?;
        if load_arm().armed || child_running(&state) {return Err("请等待当前操作结束或先取消定时预约".into());}
        if state.native_running.compare_exchange(false,true,Ordering::SeqCst,Ordering::SeqCst).is_err() {return Err("已有操作在执行".into());}
        state.stopping.store(false,Ordering::SeqCst);
        run_gen=state.run_gen.fetch_add(1,Ordering::SeqCst)+1;
        *state.active_operation.lock().unwrap_or_else(|error|error.into_inner())=Some("recheck".into());
        *state.active_task.lock().unwrap_or_else(|error|error.into_inner())=None;
    }
    tauri::async_runtime::spawn_blocking(move || {
        // Guard also releases the busy state if any fallible trace/worker step fails.
        struct BusyGuard(tauri::AppHandle);
        impl Drop for BusyGuard {fn drop(&mut self) {
            let state=self.0.state::<AppState>();state.stopping.store(false,Ordering::SeqCst);state.native_running.store(false,Ordering::SeqCst);
            sync_run_badge(&self.0,&state);let _=self.0.emit("run-changed",());
        }}
        let _busy=BusyGuard(app.clone());
        let run_id=format!("{}-recheck-g{}",Local::now().format("%Y%m%d-%H%M%S%.3f"),run_gen);
        let trace=run_trace::RunTraceHandle::open(&runs_dir(),pending_orders::TRACE_TASK,&run_id,
            serde_json::json!({"request_ref":request_ref,"read_only":true}),Some("order_recheck"),run_trace::RunTraceConfig::default())?;
        trace_event(&trace,"spawn",Some("http_recheck"),"recheck_started",None,None);
        let _=app.emit("run-changed",());
        let mut result=http_booking::recheck_order(&app,run_gen,&trace,&request_ref)
            .unwrap_or_else(|code|serde_json::json!({"ok":false,"code":code}));
        result["read_only"]=serde_json::json!(true);result["booking_submitted"]=serde_json::json!(false);
        let confirmed=result["ok"]==true && result["verified"]==true;
        if !confirmed && result["message"].as_str().is_none() {result["message"]=serde_json::json!("再次查单没有完成，原预约结果保持待确认；未提交预约");}
        let _=trace.try_event(if confirmed {"done"} else {"fail"},Some("http_recheck"),
            if confirmed {"http_order_rechecked"} else {"ORDER_RECHECK_UNCONFIRMED"},None,None,None,serde_json::json!({"http":result}));
        trace.finish(if confirmed {"succeeded"} else {"unconfirmed"},Some(if confirmed {"http_order_rechecked"} else {"ORDER_RECHECK_UNCONFIRMED"}));
        Ok(result)
    }).await.map_err(|_|"查单组件异常，原预约结果保持待确认".to_string())?
}

#[tauri::command]
fn arm_hang(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let _guard = state.task_gate.lock().map_err(|_| "任务状态不可用")?;
    if availability::recovering() {return Err("场次登录恢复中，请稍候或取消登录".into());}
    let task = read_task()?.ok_or_else(|| "请先在任务页配置预约目标".to_string())?;
    let fire = parse_fire_at(&task.fire_at)?;
    let now = Local::now().fixed_offset();
    booking_eligibility::check_logged(&task, if fire > now { fire } else { now })?;
    if child_running(&state) {
        return Err("已有 HTTP 任务在执行，请先取消".into());
    }
    let arm = ArmFile {
        armed: true,
        task_id: task.task_id.clone(),
        fire_at: task.fire_at.clone(),
        task: Some(task.clone()),
        account_fingerprint: None,
    };
    save_arm_disk(&arm)?;
    let gen = state.arm_gen.fetch_add(1, Ordering::SeqCst) + 1;
    start_arm_waiter(app.clone(), gen, arm.fire_at.clone());
    sync_run_badge(&app, &state);
    let _ = app.emit("arm-changed", ());
    log::info!(
        target: "arm",
        "[arm_hang] fire_at={} task_id={}",
        arm.fire_at,
        arm.task_id
    );
    Ok(format!("已开始挂机，等待 {}", arm.fire_at))
}

#[tauri::command]
fn disarm_hang(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let _guard = state.task_gate.lock().map_err(|_| "任务状态不可用")?;
    state.arm_gen.fetch_add(1, Ordering::SeqCst);
    clear_arm_file()?;
    kill_child(&state);
    sync_run_badge(&app, &state);
    let _ = app.emit("arm-changed", ());
    log::info!(target: "arm", "[disarm_hang]");
    Ok(if child_running(&state) { "正在停止；已发送的提交将先确认结果".into() } else { "已取消定时预约".into() })
}

#[tauri::command]
fn set_run_badge(app: tauri::AppHandle, state: State<'_, AppState>, on: Option<bool>) -> Result<(), String> {
    let flag = on.unwrap_or_else(|| load_arm().armed || child_running(&state));
    apply_run_badge(&app, flag);
    Ok(())
}

#[tauri::command]
fn read_task_raw() -> Result<Value, String> {
    match read_task()? {
        Some(t) => Ok(serde_json::to_value(t).unwrap()),
        None => Ok(Value::Null),
    }
}

#[tauri::command]
fn probe_wechat() -> Result<wechat_probe::WechatProbe, String> {
    let p = wechat_probe::probe();
    log::debug!(
        target: "live",
        "[probe_wechat] running={} miniprogram={} title={}",
        p.running,
        p.miniprogram,
        p.title.as_deref().unwrap_or("-")
    );
    Ok(p)
}

#[tauri::command]
async fn launch_xmu_miniprogram()->Result<Value,String> {
    tauri::async_runtime::spawn_blocking(|| {
        wechat_window::launch_xmu_miniprogram(None)?;
        Ok(serde_json::json!({"launched":true}))
    }).await.map_err(|_|"启动线程中断".to_string())?
}

#[tauri::command]
fn get_shell_prefs() -> Result<ShellPrefs, String> {
    Ok(load_shell_prefs())
}

#[tauri::command]
fn save_shell_prefs(prefs: ShellPrefs) -> Result<(), String> {
    save_shell_prefs_disk(&prefs)
}

#[tauri::command]
fn hide_to_tray(app: tauri::AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("main")
        .ok_or_else(|| "无 main 窗口".to_string())?;
    win.hide().map_err(|e| e.to_string())
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    tray::quit_app(&app);
}

fn busy_now(app: &tauri::AppHandle) -> bool {
    let armed = load_arm().armed;
    let running = app
        .try_state::<AppState>()
        .map(|s| child_running(&s))
        .unwrap_or(false);
    armed || running
}

/// 启动诊断：TCP 探活 Vite（区分 IPv4 / IPv6 不通 vs WebView 慢）。
fn boot_probe_dev_url(label: &str, addr: std::net::SocketAddr) {
    let t0 = Instant::now();
    match TcpStream::connect_timeout(&addr, Duration::from_millis(800)) {
        Ok(_) => log::info!(
            target: "boot",
            "[dev_url_tcp] label={label} addr={addr} ok=true elapsed_ms={}",
            t0.elapsed().as_millis()
        ),
        Err(e) => log::warn!(
            target: "boot",
            "[dev_url_tcp] label={label} addr={addr} ok=false elapsed_ms={} err={e}",
            t0.elapsed().as_millis()
        ),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(debug_assertions)]
    let log_level = log::LevelFilter::Debug;
    #[cfg(not(debug_assertions))]
    let log_level = log::LevelFilter::Info;

    // 贯穿 host → page_load → window 的启动计时原点
    let boot_t0 = Arc::new(Instant::now());
    let boot_t0_setup = Arc::clone(&boot_t0);
    let boot_t0_page = Arc::clone(&boot_t0);
    let boot_t0_win = Arc::clone(&boot_t0);
    let first_focus = Arc::new(AtomicBool::new(false));
    let first_focus_win = Arc::clone(&first_focus);

    let log_stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let log_file_stem = format!("xmu-sports-{log_stamp}");
    let log_session_file = format!("{log_file_stem}.log");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        // timezone_strategy 会覆盖 format，故 format 必须写在其后
        .plugin(
            tauri_plugin_log::Builder::new()
                .max_file_size(8 * 1024 * 1024)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(4))
                .level(log_level)
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .format(|out, message, record| {
                    let ts = chrono::Local::now().format("%H:%M:%S%.3f");
                    let level = record.level();
                    let target = record.target();
                    // 前端 message 已是 [pkg] [event] …；丢掉 webview:toPlugin@URL
                    if target == tauri_plugin_log::WEBVIEW_TARGET
                        || target.starts_with("webview:")
                    {
                        out.finish(format_args!("[{ts}] [{level}] {message}"))
                    } else {
                        out.finish(format_args!("[{ts}] [{level}] [{target}] {message}"))
                    }
                })
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview)
                        .filter(|metadata| metadata.target() != "http_exchange"),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some(log_file_stem),
                    }),
                ])
                .build(),
        )
        .manage(AppState {
            arm_gen: AtomicU64::new(0),
            run_gen: AtomicU64::new(0),
            native_running: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            task_gate: Mutex::new(()),
            active_task: Mutex::new(None),
            active_operation: Mutex::new(None),
        })
        .setup(move |app| {
            if let Ok(log_dir) = app.path().app_log_dir() {
                match diag_export::prepare_session_logs(&log_dir, &log_session_file) {
                    Ok(pruned) => log::info!(
                        target: "diag",
                        "[session_logs] file={log_session_file} pruned={pruned} dir={}",
                        log_dir.display()
                    ),
                    Err(e) => log::warn!(
                        target: "diag",
                        "[session_logs_failed] file={log_session_file} err={e}"
                    ),
                }
            } else {
                log::warn!(target: "diag", "[session_logs_failed] err=app_log_dir unavailable");
            }

            let interrupted = run_trace::mark_interrupted_stale_runs(&runs_dir());
            if interrupted > 0 {
                log::warn!(
                    target: "trace",
                    "[stale_runs_interrupted] count={interrupted}"
                );
            }

            log::info!(
                target: "boot",
                "[host_start] since_process_ms={}",
                boot_t0_setup.elapsed().as_millis()
            );
            let t_tray = Instant::now();
            if let Err(e) = tray::setup_tray(app.handle()) {
                log::error!(target: "tray", "[setup_failed] err={e}");
            } else {
                log::info!(
                    target: "tray",
                    "[ready] elapsed_ms={}",
                    t_tray.elapsed().as_millis()
                );
            }
            let t_arm = Instant::now();
            resume_arm_if_needed(app.handle());
            log::debug!(
                target: "boot",
                "[resume_arm_done] elapsed_ms={}",
                t_arm.elapsed().as_millis()
            );

            if let Some(win) = app.get_webview_window("main") {
                let visible = win.is_visible().unwrap_or(false);
                let url = win.url().map(|u| u.to_string()).unwrap_or_else(|_| "?".into());
                log::info!(
                    target: "boot",
                    "[main_window] visible={visible} url={url} since_host_ms={}",
                    boot_t0_setup.elapsed().as_millis()
                );
            } else {
                log::warn!(
                    target: "boot",
                    "[main_window] missing since_host_ms={}",
                    boot_t0_setup.elapsed().as_millis()
                );
            }

            // 开发态：只探 IPv4（devUrl 已固定 127.0.0.1；探 IPv6 只会白白超时 ~800ms）
            #[cfg(debug_assertions)]
            {
                use std::net::{Ipv4Addr, SocketAddr};
                boot_probe_dev_url("ipv4", SocketAddr::from((Ipv4Addr::LOCALHOST, 1420)));
            }

            log::info!(
                target: "boot",
                "[setup_done] since_host_ms={}",
                boot_t0_setup.elapsed().as_millis()
            );
            Ok(())
        })
        .on_page_load(move |_webview, payload| {
            let event = match payload.event() {
                PageLoadEvent::Started => "started",
                PageLoadEvent::Finished => "finished",
            };
            log::info!(
                target: "boot",
                "[page_load] event={event} url={} since_host_ms={}",
                payload.url(),
                boot_t0_page.elapsed().as_millis()
            );
        })
        .on_window_event(move |window, event| {
            if let WindowEvent::Focused(true) = event {
                if !first_focus_win.swap(true, Ordering::SeqCst) {
                    log::info!(
                        target: "boot",
                        "[window_focused] label={} since_host_ms={}",
                        window.label(),
                        boot_t0_win.elapsed().as_millis()
                    );
                }
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let prefs = load_shell_prefs();
                let busy = busy_now(window.app_handle());

                if busy {
                    if prefs.close_pref_set && prefs.close_to_tray {
                        api.prevent_close();
                        let _ = window.hide();
                        return;
                    }
                    // 询问 / 退出偏好：挂机中一律先问
                    api.prevent_close();
                    let _ = window.app_handle().emit(
                        "shell-ask-close",
                        CloseAskPayload { armed: true },
                    );
                    return;
                }

                if !prefs.close_pref_set {
                    api.prevent_close();
                    let _ = window.app_handle().emit(
                        "shell-ask-close",
                        CloseAskPayload { armed: false },
                    );
                    return;
                }
                if prefs.close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // else: allow close → exit
            }
        })
        .invoke_handler(tauri::generate_handler![
            save_task,
            availability::get_venue_availability,
            availability::reauthenticate_availability,
            availability::cancel_availability_login,
            get_live_status,
            list_recent_events,
            clear_run_history,
            export_diagnostic_logs,
            open_app_log_dir,
            start_http_run,
            list_pending_orders,
            recheck_order,
            arm_hang,
            disarm_hang,
            set_run_badge,
            read_task_raw,
            probe_wechat,
            launch_xmu_miniprogram,
            get_shell_prefs,
            save_shell_prefs,
            hide_to_tray,
            quit_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod task_snapshot_tests {
    use super::*;
    #[test]
    fn tray_and_bundle_share_the_display_name_while_upgrade_identity_stays_stable() {
        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["productName"], APP_DISPLAY_NAME);
        assert_eq!(config["identifier"], "local.xmu.sports");
        assert_eq!(config["mainBinaryName"], "xmu-sports");
    }
    #[test]
    fn scheduled_snapshot_survives_serialization_and_later_draft_edits() {
        let mut draft: BookingTask = serde_json::from_value(serde_json::json!({
            "task_id":"task", "sport":"乒乓球", "venue":"原场馆", "target_date":"2026-10-17",
            "fire_at":"2026-10-16T07:00:00+08:00", "target_time_start":"19:30", "target_time_end":"21:00"
        })).unwrap();
        let arm = ArmFile { armed: true, task_id: draft.task_id.clone(), fire_at: draft.fire_at.clone(), task: Some(draft.clone()),
            account_fingerprint: Some("prepared-account".into()) };
        draft.venue = "新场馆".into();
        draft.target_time_start = "18:00".into();
        let restored: ArmFile = serde_json::from_str(&serde_json::to_string(&arm).unwrap()).unwrap();
        assert_eq!(restored.task.as_ref().unwrap().venue, "原场馆");
        assert_eq!(restored.task.as_ref().unwrap().target_time_start, "19:30");
        assert_eq!(restored.account_fingerprint.as_deref(), Some("prepared-account"));
        let legacy: ArmFile = serde_json::from_value(serde_json::json!({"armed":true,"task_id":"task","fire_at":"2026-10-16T07:00:00+08:00"})).unwrap();
        assert!(legacy.task.is_none());
        assert!(legacy.account_fingerprint.is_none());
    }
}
