//! Standalone read-only diagnostic, before any app/scheduler setup.
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use serde_json::{json, Value};

fn worker() -> Result<PathBuf,String> {
    let exe = std::env::current_exe().map_err(|_| "HTTP_COMPONENT_MISSING")?;
    [exe.parent().unwrap_or(Path::new(".")).join("http-booking/xmu-http-booking.exe"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/http-booking/xmu-http-booking.exe")]
        .into_iter().find(|path|path.is_file()).ok_or_else(||"HTTP_COMPONENT_MISSING".into())
}

fn check() -> Result<Value,String> {
    let mut cmd=Command::new(worker()?);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child=cmd.spawn().map_err(|_|"HTTP_WORKER_START_FAILED")?;
    // Fixed action: no CLI-supplied payload, booking goal or submission mode.
    if let Some(mut input)=child.stdin.take() {
        if writeln!(input,"{}",json!({"action":"check"})).is_err() {
            let _=child.kill(); let _=child.wait(); return Err("HTTP_WORKER_START_FAILED".into());
        }
    }
    let stdout=child.stdout.take().ok_or("HTTP_WORKER_START_FAILED")?;
    let (tx,rx)=mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(body)=line.strip_prefix("@@XMU_HTTP@@") {
                if let Ok(value)=serde_json::from_str::<Value>(body) {
                    if value["kind"]=="result" { let _=tx.send(value); break; }
                }
            }
        }
    });
    let result=rx.recv_timeout(Duration::from_secs(45)).map_err(|_|"HTTP_READONLY_CHECK_TIMEOUT".into());
    let _=child.kill(); let _=child.wait();
    result
}

pub(crate) fn run(ui: bool, output: &Path) -> Result<(),String> {
    if let Some(parent)=output.parent() { std::fs::create_dir_all(parent).map_err(|e|e.to_string())?; }
    let event_file=std::cell::RefCell::new(std::fs::File::create(output.with_extension("events.jsonl")).map_err(|e|e.to_string())?);
    let emit=|value:Value| {
        let mut file=event_file.borrow_mut();
        let _=writeln!(&mut *file,"{}",value);
        let _=file.flush();
    };
    let mut phases=Vec::new();
    let mut initial_report=Value::Null;
    let mut observations=Vec::new();
    let mut window_states=Vec::new();
    let result=(|| {
        let initial=crate::normal_login::validate(check()?,None);
        initial_report=initial.clone();
        emit(json!({"kind":"initial_check","result":initial}));
        let expected=initial["account_fingerprint"].as_str().map(str::to_owned);
        if !ui || (initial["ok"]!=true && !crate::http_booking::needs_reauthorization(&initial)) { return Ok(initial); }
        crate::normal_login::recover(&crate::data_root(), &format!("login-only-{}",uuid::Uuid::new_v4()),
            expected.as_deref(), ||true, check,
            |phase| { phases.push(phase.to_owned()); let state=json!({"phase":phase,"window":crate::wechat_window::background_status()}); window_states.push(state.clone());emit(state); },
            |observation| { emit(json!({"kind":"elements","observation":observation})); observations.push(observation); })
    })().unwrap_or_else(|code:String|json!({"ok":false,"code":code}));
    // No tokens, signed URLs, cookies or order contents in the saved report.
    let report=json!({"mode":"login_and_existing_orders_only","app_version":env!("CARGO_PKG_VERSION"),
        "scheduler_started":false,"booking_actions_available":false,"kernel_hook_used":false,
        "normal_ui_requested":ui,"initial_check":initial_report,
        "visual_fallback_used":phases.iter().any(|phase|phase=="visual_login_fallback"),"phases":phases,
        "login_elements":observations,"window_states":window_states,"result":result});
    if let Some(parent)=output.parent() { std::fs::create_dir_all(parent).map_err(|e|e.to_string())?; }
    std::fs::write(output,serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())
}
