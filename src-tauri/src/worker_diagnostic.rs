//! Early diagnostics: persistent worker transport, hard read-only worker mode.
use serde_json::{json,Value};
use std::path::{Path,PathBuf};
use std::time::{Duration,Instant};
use crate::worker_process::Worker;

fn worker_path()->Result<PathBuf,String> {
    let exe=std::env::current_exe().map_err(|_|"HTTP_COMPONENT_MISSING")?;
    [exe.parent().unwrap_or(Path::new(".")).join("http-booking/xmu-http-booking.exe"),
     PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/http-booking/xmu-http-booking.exe")]
        .into_iter().find(|path|path.is_file()).ok_or_else(||"HTTP_COMPONENT_MISSING".into())
}

fn rpc(worker:&mut Worker,mut payload:Value)->Result<Value,String> {
    let id=worker.send(&mut payload)?;
    let started=Instant::now();
    loop {
        match worker.receive(Duration::from_millis(100)) {
            Ok(value) if value["rpc_id"].as_str()==Some(id.as_str()) && value["kind"]=="result"=>return Ok(value),
            Ok(_)=>{},
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)=>return Err("HTTP_WORKER_ERROR".into()),
            Err(_)=>{},
        }
        if started.elapsed()>Duration::from_secs(60) {return Err("HTTP_WORKER_TIMEOUT".into());}
    }
}

pub(crate) fn run(reference:Option<&str>,output:&Path)->Result<(),String> {
    let result=(|| {
        let mut worker=Worker::spawn(&worker_path()?,true,true)?;
        if let Some(reference)=reference {
            if !crate::pending_orders::valid_reference(reference) {return Err("INVALID_REQUEST_REF".into());}
            return rpc(&mut worker,json!({"action":"recheck_pending","request_ref":reference}));
        }
        // Synthetic binding is only for a no-network model test. No booking
        // task, scheduler or user-supplied action can enter this diagnostic.
        let binding=json!({"task":{"sport":"本地模型诊断","venue":"本地模型诊断","target_date":"2099-01-02",
            "target_time_start":"10:00","target_time_end":"11:00"},"expected_account":"0".repeat(64)});
        let mut warm=binding.clone();warm["action"]=json!("warm");
        let first=rpc(&mut worker,warm)?;
        if first["ok"]!=true {return Ok(first);}
        let mut ping=binding;ping["action"]=json!("ping");
        let next=rpc(&mut worker,ping)?;
        let login=rpc(&mut worker,json!({"action":"check"}))?;
        // Also test the child-side boundary, independent from the caller.
        let denied=rpc(&mut worker,json!({"action":"book"}))?;
        Ok(json!({"ok":next["ok"]==true && first["model_load_count"]==1 && next["model_load_count"]==1
            && first["process_id"]==next["process_id"] && denied["code"]=="READONLY_WORKER",
            "warm":first,"ping":next,"login":login,"booking_rpc_denied":denied["code"]=="READONLY_WORKER"}))
    })().unwrap_or_else(|code:String|json!({"ok":false,"code":code}));
    let report=json!({"app_version":env!("CARGO_PKG_VERSION"),"mode":if reference.is_some() {"original_order_recheck_only"} else {"persistent_model_warm_only"},
        "scheduler_started":false,"booking_submitted":false,"result":result});
    if let Some(parent)=output.parent() {std::fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
    std::fs::write(output,serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())
}
