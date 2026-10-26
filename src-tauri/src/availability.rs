//! Isolated availability and explicitly requested login recovery. No booking actions.
use serde_json::{json,Value};
use std::sync::{Arc,Mutex,mpsc,atomic::{AtomicBool,Ordering}};
use std::time::{Duration,Instant};
use tauri::Manager;
pub(crate) static QUERY:Mutex<()>=Mutex::new(());
static RECOVERY:Mutex<Option<(String,Arc<AtomicBool>)>>=Mutex::new(None);
static EARLY_CANCEL:Mutex<Vec<String>>=Mutex::new(Vec::new());

pub(crate) fn recovering()->bool { RECOVERY.lock().map(|v|v.is_some()).unwrap_or(true) }
struct RecoveryGuard(String);
impl Drop for RecoveryGuard {
    fn drop(&mut self) {
        if let Ok(mut current)=RECOVERY.lock() {
            if current.as_ref().is_some_and(|(id,_)|id==&self.0) { *current=None; }
        }
    }
}

#[tauri::command]
pub fn cancel_availability_login(operation_id:String) {
    if uuid::Uuid::parse_str(&operation_id).is_err() {return;}
    if let Ok(current)=RECOVERY.lock() {
        if let Some((id,active))=current.as_ref() {
            if *id==operation_id {active.store(false,Ordering::SeqCst);return;}
        }
    }
    // IPC cancellation can arrive before the async command is first polled.
    if let Ok(mut cancelled)=EARLY_CANCEL.lock() {
        if cancelled.len()>=64 {cancelled.remove(0);}
        cancelled.push(operation_id);
    }
}

fn readonly_check(app:&tauri::AppHandle,active:&impl Fn()->bool)->Result<Value,String> {
    if !active() {return Err("已取消".into());}
    let mut worker=crate::worker_process::Worker::spawn(&crate::http_booking::worker_path(app)?,true,true)?;
    let rpc=worker.send(&mut json!({"action":"check"}))?;
    let started=Instant::now();
    loop {
        if !active() {return Err("已取消".into());}
        match worker.receive(Duration::from_millis(100)) {
            Ok(value) if value["rpc_id"]==rpc && value["kind"]=="result"=>return Ok(value),
            Ok(value) if value["rpc_id"]==rpc && value["kind"]=="http_exchange"=>log::info!(target:"http_exchange","[availability_login_exchange] exchange={}",value),
            Err(mpsc::RecvTimeoutError::Disconnected)=>return Err("登录检查进程中断".into()),
            _=>{},
        }
        if started.elapsed()>Duration::from_secs(45) {return Err("登录检查超时".into());}
    }
}

fn login_only(checked:Value,expected:Option<&str>,active:impl Fn()->bool,recover:impl FnOnce(Option<&str>)->Result<Value,String>)->Result<Value,String> {
    if !active() {return Err("已取消".into());}
    let expected=expected.or_else(||checked["account_fingerprint"].as_str()).map(str::to_owned);
    let checked=crate::normal_login::validate(checked,expected.as_deref());
    let recoverable=crate::http_booking::needs_reauthorization(&checked)
        || matches!(checked["code"].as_str(),Some("SESSION_READ_FAILED"|"NO_SESSION_COOKIE"));
    if checked["ok"]==true || !recoverable {return Ok(checked);}
    let result=crate::normal_login::validate(recover(expected.as_deref())?,expected.as_deref());
    if !active() {return Err("已取消".into());}
    Ok(result)
}

#[tauri::command]
pub async fn reauthenticate_availability(app:tauri::AppHandle,operation_id:String,expected_account:Option<String>)->Result<Value,String> {
    if expected_account.as_deref().is_some_and(|value| !crate::pending_orders::valid_reference(value)) {return Err("原账号标识无效".into());}
    uuid::Uuid::parse_str(&operation_id).map_err(|_|"登录操作标识无效")?;
    // Reserve synchronously, so cancellation cannot race a queued blocking task.
    let state=app.state::<crate::AppState>();
    let active=Arc::new(AtomicBool::new(true));
    {
        let _gate=state.task_gate.lock().map_err(|_|"任务状态不可用")?;
        if crate::child_running(&state) || crate::load_arm().armed || crate::my_orders::mutating() {return Err("请先结束预约或订单操作".into());}
        let mut current=RECOVERY.lock().map_err(|_|"登录状态不可用")?;
        if current.is_some() {return Err("登录恢复正在进行".into());}
        let mut cancelled=EARLY_CANCEL.lock().map_err(|_|"登录状态不可用")?;
        if let Some(index)=cancelled.iter().position(|id|id==&operation_id) {cancelled.remove(index);return Err("已取消".into());}
        *current=Some((operation_id.clone(),active.clone()));
    }
    let guard=RecoveryGuard(operation_id);
    tauri::async_runtime::spawn_blocking(move|| {
        let _guard=guard;
        let _query=QUERY.lock().map_err(|_|"场次查询状态不可用")?;
        let state=app.state::<crate::AppState>();
        let enabled=||active.load(Ordering::SeqCst) && !crate::child_running(&state) && !crate::load_arm().armed;
        let checked=readonly_check(&app,&enabled)?;
        let result=login_only(checked,expected_account.as_deref(),&enabled,|expected|crate::normal_login::recover(
            &crate::data_root(),"availability-login",expected,&enabled,
            ||readonly_check(&app,&enabled),
            |phase|log::info!(target:"http","[availability_login_phase] phase={phase}"),
            |_|{},
        ))?;
        log::info!(target:"http","[availability_login_result] ok={} code={} booking_actions_available=false",result["ok"],result["code"]);
        Ok(result)
    }).await.map_err(|_|"登录恢复未完成".to_string())?
}

#[tauri::command]
pub async fn get_venue_availability(app:tauri::AppHandle,sport:String,venue:String,date:String,force:bool)->Result<Value,String> {
    tauri::async_runtime::spawn_blocking(move|| {
        let _guard=QUERY.lock().map_err(|_|"场次查询状态不可用")?;
        let state=app.state::<crate::AppState>();
        if recovering() || crate::my_orders::mutating() {return Err("登录或订单操作进行中，请稍候刷新".into());}
        if crate::child_running(&state) || crate::load_arm().armed {
            return Err("定时预约或预约流程正在运行，暂不查询场次，请结束后刷新".into());
        }
        let mut worker=crate::worker_process::Worker::spawn(&crate::http_booking::worker_path(&app)?,true,true)?;
        let rpc=worker.send(&mut json!({"action":"availability","sport":sport,"venue":venue,"date":date,"force":force}))?;
        let started=Instant::now();
        loop {
            if crate::child_running(&state) || crate::load_arm().armed {
                return Err("预约任务已启动，已停止后台场次查询".into());
            }
            match worker.receive(Duration::from_millis(100)) {
                Ok(value) if value["rpc_id"]==rpc && value["kind"]=="result"=> {
                    log::info!(target:"http","[availability_result] rpc_id={} ok={} code={} cached={}",rpc,value["ok"],value["code"],value["cached"]);
                    return Ok(value);
                },
                Ok(value) if value["rpc_id"]==rpc && value["kind"]=="http_exchange"=> {
                    log::info!(target:"http_exchange","[availability_exchange] exchange={}",value);
                },
                Ok(_)=>{},
                Err(mpsc::RecvTimeoutError::Disconnected)=>return Err("场次查询进程中断，请刷新重试".into()),
                Err(_)=>{},
            }
            if started.elapsed()>Duration::from_secs(60) {return Err("场次查询超时，请稍后刷新".into());}
        }
    }).await.map_err(|_|"场次查询未完成".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_only_never_recovers_network_failures_or_valid_sessions() {
        for checked in [json!({"ok":false,"code":"NETWORK_ERROR"}),json!({"ok":true,"authenticated":true,"account_checked":true,"account_fingerprint":"a"})] {
            let _=login_only(checked,None,||true,|_|panic!("unexpected UI recovery")).unwrap();
        }
        assert!(login_only(json!({"ok":false,"code":"SESSION_EXPIRED"}),None,||false,|_|panic!("cancelled recovery")).is_err());
    }
    #[test]
    fn recovery_requires_same_verified_account() {
        let checked=json!({"ok":false,"code":"SESSION_EXPIRED","account_fingerprint":"a"});
        let value=login_only(checked,None,||true,|expected| {
            assert_eq!(expected,Some("a"));
            Ok(json!({"ok":true,"authenticated":true,"account_checked":true,"account_fingerprint":"b"}))
        }).unwrap();
        assert_eq!(value["code"],"ACCOUNT_CHANGED");
        let result=login_only(json!({"ok":false,"code":"NO_ENTRY"}),None,||true,|_|Ok(json!({"ok":true}))).unwrap();
        assert_eq!(result["code"],"HTTP_AUTH_UNVERIFIED");
    }
    #[test]
    fn displayed_account_cannot_change_during_recovery() {
        let result=login_only(json!({"ok":false,"code":"SESSION_EXPIRED","account_fingerprint":"b"}),Some("a"),||true,|_|panic!("changed account must not recover")).unwrap();
        assert_eq!(result["code"],"ACCOUNT_CHANGED");
    }

}
