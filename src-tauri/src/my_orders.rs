//! Separate order worker; never launches booking or automatic login.
use serde_json::{json, Value};
use std::sync::{atomic::{AtomicBool, Ordering}, mpsc};
use std::time::{Duration, Instant};
use tauri::Manager;

static MUTATING: AtomicBool = AtomicBool::new(false);
pub(crate) fn mutating() -> bool { MUTATING.load(Ordering::SeqCst) }

struct Mutation;
impl Drop for Mutation { fn drop(&mut self) { MUTATING.store(false, Ordering::SeqCst); } }

fn readonly_allowed(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    if crate::child_running(&state) || crate::load_arm().armed || crate::availability::recovering() || mutating() {
        return Err("预约、登录或订单操作进行中，稍后刷新我的预约".into());
    }
    Ok(())
}

fn request(app: &tauri::AppHandle, payload: Value, mutation: bool) -> Result<Value, String> {
    if !mutation { readonly_allowed(app)?; }
    let mut worker = crate::worker_process::Worker::spawn(&crate::http_booking::worker_path(app)?, true, !mutation)?;
    let mut payload = payload;
    let rpc = worker.send(&mut payload)?;
    let started = Instant::now();
    loop {
        // A cancellation already sent must be allowed to finish its read-back.
        if !mutation {
            readonly_allowed(app)?;
        }
        match worker.receive(Duration::from_millis(100)) {
            Ok(value) if value["rpc_id"] == rpc && value["kind"] == "result" => return Ok(value),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(if mutation {"取消结果未知，请刷新或到小程序核对，勿重复取消"} else {"订单查询中断，请稍后刷新"}.into()),
            _ => {},
        }
        if started.elapsed() > Duration::from_secs(120) {
            return Err(if mutation {"取消结果未知，请刷新或到小程序核对，勿重复取消"} else {"订单查询超时，请稍后刷新"}.into());
        }
    }
}

#[tauri::command]
pub async fn get_my_orders(app: tauri::AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _query = crate::availability::QUERY.lock().map_err(|_| "订单查询状态不可用")?;
        request(&app, json!({"action":"my_orders"}), false)
    }).await.map_err(|_| "订单查询未完成".to_string())?
}

#[tauri::command]
pub async fn cancel_my_order(app: tauri::AppHandle, account_fingerprint: String, order_id: String, details_id: String) -> Result<Value, String> {
    // Reserve before yielding; booking/arming/login gates observe this reservation.
    let state = app.state::<crate::AppState>();
    let guard = {
        let _gate = state.task_gate.lock().map_err(|_| "任务状态不可用")?;
        if crate::child_running(&state) || crate::load_arm().armed || crate::availability::recovering() {
            return Err("请先结束预约或登录操作，再取消订单".into());
        }
        if MUTATING.swap(true, Ordering::SeqCst) { return Err("订单取消正在进行".into()); }
        Mutation
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let _query = crate::availability::QUERY.lock().map_err(|_| "订单状态不可用")?;
        request(&app, json!({"action":"cancel_order", "account_fingerprint":account_fingerprint,
                             "order_id":order_id, "details_id":details_id}), true)
    }).await.map_err(|_| "取消结果未知，请刷新或到小程序核对，勿重复取消".to_string())?
}
