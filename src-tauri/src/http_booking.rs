//! Bundled worker transport. The school requests remain inside the user's local worker.
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::{Emitter, Manager};

use crate::{run_control, run_trace, BookingTask};
use crate::worker_process::Worker;

pub(crate) fn validate_operation(until: Option<&str>) -> Result<(), String> {
    if matches!(until, None | Some("auth" | "prepare" | "trial_slot" | "submit")) { Ok(()) }
    else { Err("未知操作，未开始预约".into()) }
}

pub(crate) fn worker_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(root) = app.path().resource_dir() {
        candidates.push(root.join("http-booking/xmu-http-booking.exe"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/http-booking/xmu-http-booking.exe"));
    candidates.into_iter().find(|path| path.is_file())
        .ok_or_else(|| "HTTP_COMPONENT_MISSING".to_string())
}

fn request(
    app: &tauri::AppHandle, run_gen: u64, payload: Value,
    trace: &run_trace::RunTraceHandle,
) -> Result<Value, String> {
    let mut worker=Worker::spawn(&worker_path(app)?,false,false)?;
    request_with_worker(app,run_gen,payload,trace,&mut worker)
}

pub(crate) struct PreparedWorker {
    worker: Worker,
    binding: Value,
    restarts: u32,
}

impl PreparedWorker {
    pub fn prepare(app:&tauri::AppHandle,run_gen:u64,trace:&run_trace::RunTraceHandle,
                   task:&BookingTask,account:&str)->Result<Self,String> {
        let mut worker=Worker::spawn(&worker_path(app)?,true,false)?;
        let binding=json!({"task":task,"expected_account":account});
        let mut payload=binding.clone(); payload["action"]=json!("warm");
        let result=request_with_worker(app,run_gen,payload,trace,&mut worker)?;
        if result["code"]=="CANCELLED" {return Err("已取消".into());}
        if result["ok"]!=true || result["model_ready"]!=true || result["inference_preloaded"]!=true {
            return Err(result["code"].as_str().unwrap_or("WORKER_WARM_FAILED").into());
        }
        let _=trace.try_event("http_phase",Some("http_worker_warmed"),"worker_warmed",None,None,None,
            json!({"http":{"phase":"worker_warmed","model_load_count":result["model_load_count"],"process_id":result["process_id"],"booking_submitted":false}}));
        let _=app.emit("run-changed",());
        Ok(Self{worker,binding,restarts:0})
    }

    pub fn recheck(&mut self,app:&tauri::AppHandle,run_gen:u64,trace:&run_trace::RunTraceHandle,
                   task:&BookingTask,account:&str)->Result<(),String> {
        if self.worker.booking_dispatched {return Err("SUBMIT_UNKNOWN".into());}
        if self.binding!=json!({"task":task,"expected_account":account}) {return Err("WARM_CONTEXT_CHANGED".into());}
        if !self.worker.alive() {
            // Only before a booking RPC has ever been written; no reconnect once
            // there is any chance that this process has processed a submission.
            if self.worker.booking_dispatched || self.restarts>=1 {return Err("HTTP_WARM_WORKER_LOST".into());}
            let mut replacement=Self::prepare(app,run_gen,trace,task,account)?;
            replacement.restarts=self.restarts+1;
            *self=replacement;
        }
        let mut payload=self.binding.clone();payload["action"]=json!("ping");
        let result=request_with_worker(app,run_gen,payload,trace,&mut self.worker)?;
        if result["code"]=="CANCELLED" {return Err("已取消".into());}
        if result["ok"]!=true || result["model_ready"]!=true {return Err(result["code"].as_str().unwrap_or("WORKER_WARM_FAILED").into());}
        let _=trace.try_event("http_phase",Some("http_worker_warm_rechecked"),"worker_warm_rechecked",None,None,None,
            json!({"http":{"phase":"worker_warm_rechecked","model_load_count":result["model_load_count"],"process_id":result["process_id"],"booking_submitted":false}}));
        Ok(())
    }
}

fn request_with_worker(
    app:&tauri::AppHandle, run_gen:u64, mut payload:Value,
    trace:&run_trace::RunTraceHandle, worker:&mut Worker,
)->Result<Value,String> {
    if !run_control::run_still_active(app,run_gen) {return Err("已取消".into());}
    let root=crate::data_root().join("http-booking");
    std::fs::create_dir_all(&root).map_err(|error|error.to_string())?;
    let cancel_file=root.join(format!("{}.cancel",trace.run_id()));
    payload["run_id"]=json!(trace.run_id());
    payload["cancel_file"]=json!(cancel_file);
    let started=Instant::now();
    let rpc_id=match worker.send(&mut payload) {
        Ok(id)=>id,
        Err(code)=>return Ok(json!({"ok":false,"code":code})),
    };
    log::info!(target: "http", "[worker_start] run_id={} action={} app_version={} persistent={} process_id={}",
        trace.run_id(), payload["action"].as_str().unwrap_or("unknown"), env!("CARGO_PKG_VERSION"),worker.persistent,worker.process_id());
    let mut submit_started = false;
    let mut cancel_sent = false;
    let result = loop {
        if !run_control::run_still_active(app, run_gen) && !cancel_sent {
            if std::fs::write(&cancel_file, b"cancel").is_err() {
                break json!({"ok":false,"code":if submit_started || worker.booking_dispatched {"SUBMIT_UNKNOWN"} else {"CANCELLED"}});
            }
            cancel_sent = true;
        }
        match worker.receive(Duration::from_millis(100)) {
            Ok(value) if value["kind"]!="ready" && value["rpc_id"].as_str()!=Some(rpc_id.as_str()) => continue,
            Ok(value) if value["kind"] == "ready" => {
                log::info!(target: "http", "[worker_ready] run_id={} startup_ms={}", trace.run_id(), worker.born.elapsed().as_millis());
            }
            Ok(value) if value["kind"] == "http_exchange" => {
                log::info!(target: "http_exchange", "[http_exchange] run_id={} action={} exchange={}",
                    trace.run_id(), payload["action"], value);
                let _ = trace.try_event("http_exchange", Some("http_exchange"), "upstream_reply",
                    None, None, None, json!({"http": value}));
            }
            Ok(value) if value["kind"] == "progress" => {
                let phase = value["phase"].as_str().unwrap_or("running");
                log::info!(target: "http", "[progress] run_id={} phase={} elapsed_ms={} candidate={} court={}",
                    trace.run_id(), phase, started.elapsed().as_millis(),
                    value["candidate_index"].as_u64().map(|index| (index + 1).to_string()).unwrap_or_default(),
                    value["court"].as_str().unwrap_or(""));
                if let Some(diagnostics) = value.get("diagnostics") {
                    log::info!(target: "http", "[compatibility] run_id={} phase={} diagnostics={}",
                        trace.run_id(), phase, diagnostics);
                }
                submit_started |= phase == "submitting";
                let _ = trace.try_event("http_phase", Some(&format!("http_{phase}")), phase,
                    None, None, None, json!({"http": value, "elapsed_ms": started.elapsed().as_millis()}));
                let _ = app.emit("run-changed", ());
            }
            Ok(value) if value["kind"] == "result" => break value,
            Ok(_) => {},
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break json!({"ok":false,"code":worker.failure_code("HTTP_WORKER_ERROR")});
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {},
        }
        if started.elapsed() > Duration::from_secs(300) {
            break json!({"ok":false,"code":worker.failure_code("HTTP_WORKER_TIMEOUT")});
        }
    };
    log::info!(target: "http", "[worker_result] run_id={} action={} elapsed_ms={} ok={} code={} state={} native_login={} fresh_entry={} mini_program_closed={} details={}",
        trace.run_id(), payload["action"].as_str().unwrap_or("unknown"), started.elapsed().as_millis(),
        result["ok"], result["code"], result["state"], result["native_login_performed"],
        result["fresh_entry_observed"], result["mini_program_closed"], result["details"]);
    if let Some(diagnostics) = result.get("diagnostics") {
        log::info!(target: "http", "[worker_metrics] run_id={} action={} diagnostics={}", trace.run_id(), payload["action"], diagnostics);
    }
    let _ = std::fs::remove_file(&cancel_file);
    Ok(result)
}

pub(crate) fn check_auth(app: &tauri::AppHandle, run_gen: u64, trace: &run_trace::RunTraceHandle) -> Result<Value, String> {
    crate::trace_event(trace, "http_phase", Some("http_authenticating"), "authenticating", None, None);
    let result = request(app, run_gen, json!({"action":"check"}), trace)?;
    let _ = trace.try_event("auth_check", Some("http_authenticating"), result["state"].as_str().unwrap_or("failed"),
                           None, None, None, json!({"http": result}));
    let _ = app.emit("run-changed", ());
    Ok(result)
}

pub(crate) fn recover_auth(app: &tauri::AppHandle, run_gen: u64, trace: &run_trace::RunTraceHandle,
                          expected_account: Option<&str>) -> Result<Value, String> {
    let observation_step=std::cell::Cell::new(0u32);
    let result = crate::normal_login::recover(&crate::data_root(), trace.run_id(), expected_account,
        || run_control::run_still_active(app, run_gen),
        || check_auth(app, run_gen, trace),
        |phase| {
            let _ = trace.try_event("http_phase", Some("http_normal_ui_login"), phase,
                None, None, None, json!({"http":{"phase":phase,"recovery_mode":"normal_ui_http"}}));
            let _ = app.emit("run-changed", ());
        },
        |observation| {
            observation_step.set(observation_step.get()+1);
            let _=trace.try_observation(observation_step.get(),"login_elements",&json!({"uia":observation}));
        })?;
    let _ = trace.try_event("auth_check", Some("http_fresh_entry"), result["state"].as_str().unwrap_or("failed"),
                           None, None, None, json!({"http":result}));
    let _ = app.emit("run-changed", ());
    Ok(result)
}

/// Reuse only an entry just verified by the school, recovering only a missing or
/// expired login. Preparation, scheduled recheck and proven pre-send failures use this gate.
pub(crate) fn ensure_auth(
    app: &tauri::AppHandle, run_gen: u64, trace: &run_trace::RunTraceHandle,
    expected_account: Option<&str>, scheduled: bool,
) -> Result<Value, String> {
    let checked = check_auth(app, run_gen, trace)?;
    let reused = checked["ok"] == true;
    let result = reuse_or_recover_auth(checked, expected_account, |bound_account| {
        if !run_control::run_still_active(app, run_gen) { return Err("已取消".into()); }
        crate::trace_event(trace, "login_recovery", Some("xmu_login_gate"),
                           "refreshing_h5_entry", None, None);
        recover_scheduled_auth(app, run_gen, trace, bound_account, scheduled)
    })?;
    if !run_control::run_still_active(app, run_gen) { return Err("已取消".into()); }
    if reused && result["ok"] == true {
        log::info!(target: "http", "[h5_entry_reused] run_id={} account_checked=true", trace.run_id());
        let _ = trace.try_event("http_phase", Some("http_h5_entry_reused"), "h5_entry_reused",
                               None, None, None, json!({"http":{"phase":"h5_entry_reused"}}));
        let _ = app.emit("run-changed", ());
    }
    Ok(result)
}

fn reuse_or_recover_auth(
    checked: Value, expected: Option<&str>,
    recover: impl FnOnce(Option<&str>) -> Result<Value, String>,
) -> Result<Value, String> {
    let observed = checked["account_fingerprint"].as_str().filter(|account| !account.is_empty());
    if observed.is_some_and(|account| expected.is_some_and(|expected| account != expected)) {
        return Ok(json!({"ok":false,"code":"ACCOUNT_CHANGED"}));
    }
    // Even an expired ticket identifies the account we must recover. Do not let
    // recovery silently switch an unscheduled run to a different account.
    let bound_account = expected.or(observed).map(str::to_owned);
    if checked["ok"] == true {
        return Ok(validate_auth_result(checked, bound_account.as_deref()));
    }
    if !needs_reauthorization(&checked) { return Ok(checked); }
    Ok(validate_auth_result(recover(bound_account.as_deref())?, bound_account.as_deref()))
}

fn validate_auth_result(result: Value, expected: Option<&str>) -> Value {
    if result["ok"] != true { return result; }
    if result["authenticated"] != true || result["account_checked"] != true {
        return json!({"ok":false,"code":"HTTP_AUTH_UNVERIFIED"});
    }
    if let Err(code) = verified_account(&result, expected) {
        return json!({"ok":false,"code":code});
    }
    result
}

/// Keep a scheduled run alive only after a new server check of the bound account.
pub(crate) fn recover_scheduled_auth(
    app: &tauri::AppHandle, run_gen: u64, trace: &run_trace::RunTraceHandle,
    expected_account: Option<&str>, scheduled: bool,
) -> Result<Value, String> {
    let refreshed = recover_auth(app, run_gen, trace, expected_account)?;
    if !scheduled { return Ok(refreshed); }
    recheck_after_refresh_failure(refreshed, expected_account, || {
        crate::trace_event(trace, "http_phase", Some("http_scheduled_h5_rechecking"),
                           "scheduled_h5_rechecking", None, None);
        let checked = check_auth(app, run_gen, trace)?;
        if checked["ok"] == true && checked["authenticated"] == true && checked["account_checked"] == true
            && verified_account(&checked, expected_account).is_ok() {
            log::warn!(target: "arm", "[h5_refresh_fallback] run_id={} cached_entry_revalidated=true", trace.run_id());
            crate::trace_event(trace, "http_phase", Some("http_scheduled_h5_reused"),
                               "scheduled_h5_reused", None, None);
        }
        Ok(checked)
    })
}

fn recheck_after_refresh_failure(
    refreshed: Value, expected: Option<&str>, recheck: impl FnOnce() -> Result<Value, String>,
) -> Result<Value, String> {
    // Never bypass cancellation, a changed account, rejected credentials, or an
    // unsupported runtime. A previous successful check alone is insufficient.
    if refreshed["ok"] == true || expected.is_none_or(str::is_empty)
        || !matches!(refreshed["code"].as_str(),
            Some("RUNTIME_BRIDGE_FAILED" | "RUNTIME_UNAVAILABLE" | "LOGIN_CONFIRM_REQUIRED")) {
        return Ok(refreshed);
    }
    let checked = recheck()?;
    if checked["ok"] == true {
        if checked["authenticated"] != true || checked["account_checked"] != true {
            return Ok(json!({"ok": false, "code": "HTTP_AUTH_UNVERIFIED"}));
        }
        if let Err(code) = verified_account(&checked, expected) {
            return Ok(json!({"ok": false, "code": code}));
        }
    }
    Ok(checked)
}

pub(crate) fn run(
    app: &tauri::AppHandle, run_gen: u64, task: &BookingTask,
    trace: &run_trace::RunTraceHandle, prepare: bool, expected_account: Option<&str>, warmed:Option<&mut PreparedWorker>,
) -> Result<String, String> {
    let payload=json!({"action": if prepare {"prepare"} else {"book"}, "task": task, "expected_account": expected_account});
    let result=match warmed {
        Some(warmed)=>{
            if !warmed.worker.alive() {
                warmed.recheck(app,run_gen,trace,task,expected_account.ok_or("HTTP_AUTH_UNVERIFIED")?)?;
            }
            request_with_worker(app,run_gen,payload.clone(),trace,&mut warmed.worker)?
        },
        None=>request(app,run_gen,payload.clone(),trace)?,
    };
    let initial_failure=json!({"code":result["code"],"phase":result["phase"],
        "api":result["details"]["api"],"diagnostics":result["diagnostics"]});
    let result=recover_pre_submit(result,expected_account,
        |account| {
            let _=trace.try_event("http_phase",Some("http_pre_submit_auth_recovering"),"pre_submit_auth_recovering",None,None,None,
                json!({"http":{"phase":"pre_submit_auth_recovering","submission_not_sent":true,"initial_failure":initial_failure}}));
            let _=app.emit("run-changed",());
            ensure_auth(app,run_gen,trace,Some(account),false)
        },
        || {
            let _=trace.try_event("http_phase",Some("http_pre_submit_auth_restored"),"pre_submit_auth_restored",None,None,None,
                json!({"http":{"phase":"pre_submit_auth_restored","submission_not_sent":true}}));
            // Same run ID/idempotency key and immutable task. The consumed warm
            // process is never reset; use a fresh process after explicit proof.
            request(app,run_gen,payload,trace)
        })?;
    let root = crate::data_root().join("http-booking");
    let _ = crate::atomic_fs::write_atomic(&root.join(format!("{}.json", trace.run_id())),
                                          serde_json::to_string_pretty(&result).unwrap_or_default().as_bytes());
    let code = if result["ok"].as_bool() == Some(true) {
        if prepare { "http_prepared" } else if result["state"] == "already_reserved" { "http_already_reserved" }
        else if result["verified"].as_bool() == Some(true) { "http_verified" }
        else { "http_submitted" }
    } else { result["code"].as_str().unwrap_or("HTTP_FAILED") };
    let _ = trace.try_event(if result["ok"] == true {"done"} else if code == "CANCELLED" {"cancelled"} else {"fail"},
                           Some("http_result"), if code == "CANCELLED" {"cancelled"} else {code},
                           None, None, None, json!({"http": result}));
    if result["ok"] == true { Ok(code.into()) }
    else if code == "CANCELLED" { Err("已取消".into()) } else { Err(code.into()) }
}

fn recover_pre_submit(
    original:Value,expected:Option<&str>,
    recover:impl FnOnce(&str)->Result<Value,String>,
    retry:impl FnOnce()->Result<Value,String>,
)->Result<Value,String> {
    let Some(account)=expected.filter(|value|crate::pending_orders::valid_reference(value)) else {return Ok(original);};
    let proof=&original["execution_proof"];
    let allowed=original["ok"]!=true && matches!(original["code"].as_str(),Some("NO_ENTRY"|"INVALID_ENTRY"|"SESSION_EXPIRED"))
        && !matches!(original["state"].as_str(),Some("unknown"|"submitted"))
        && original["account_fingerprint"].as_str()==Some(account)
        && proof["version"]==1 && proof["stage"]=="pre_submission_auth"
        && proof["add_order_calls"]==0 && proof["captcha_calls"]==0 && proof["journal_created"]==false
        && original["request_ref"].is_null();
    if !allowed {return Ok(original);}
    let recovered=validate_auth_result(recover(account)?,Some(account));
    if recovered["ok"]!=true {return Ok(recovered);}
    // Deliberately one retry, with no recursion even if authentication expires
    // again. All unknown/accepted results retain their original non-replay rule.
    let mut result=retry()?;
    result["auth_recovery_before_submit"]=json!(true);
    Ok(result)
}

pub(crate) fn recheck_order(app:&tauri::AppHandle,run_gen:u64,trace:&run_trace::RunTraceHandle,reference:&str)->Result<Value,String> {
    // Never accept a caller-provided account, task, target or submission mode.
    let payload=json!({"action":"recheck_pending","request_ref":reference});
    let result=request(app,run_gen,payload.clone(),trace)?;
    if needs_reauthorization(&result) {
        // This fingerprint came from the validated durable journal in the
        // worker, not from the browser or current editable task.
        if let Some(account)=result["account_fingerprint"].as_str().filter(|value|crate::pending_orders::valid_reference(value)) {
            let auth=ensure_auth(app,run_gen,trace,Some(account),false)?;
            if auth["ok"]==true {return request(app,run_gen,payload,trace);}
            return Ok(auth);
        }
    }
    Ok(result)
}

pub(crate) fn needs_reauthorization(result: &Value) -> bool {
    matches!(result["code"].as_str(), Some("NO_ENTRY" | "INVALID_ENTRY" | "SESSION_EXPIRED"))
        || (result["code"] == "API_REJECTED" && result["phase"] == "authenticating")
}

pub(crate) fn verified_account(result: &Value, expected: Option<&str>) -> Result<String, String> {
    let account = result["account_fingerprint"].as_str().filter(|value| !value.is_empty())
        .ok_or_else(|| "HTTP_AUTH_UNVERIFIED".to_string())?;
    if expected.is_some_and(|expected| expected != account) { return Err("ACCOUNT_CHANGED".into()); }
    Ok(account.to_owned())
}

pub(crate) fn auth_failure(trace: &run_trace::RunTraceHandle, result: &Value) -> String {
    let code = if needs_reauthorization(result) { "LOGIN_CONFIRM_REQUIRED" }
               else { result["code"].as_str().unwrap_or("HTTP_WORKER_ERROR") };
    let _ = trace.try_event(if code == "CANCELLED" {"cancelled"} else {"fail"}, Some("http_authenticating"),
                           if code == "CANCELLED" {"cancelled"} else {code}, None, None, None,
                           json!({"http":result}));
    if code == "CANCELLED" {"已取消".into()} else {code.into()}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn before_send_auth_failure()->Value {
        json!({"ok":false,"code":"SESSION_EXPIRED","state":"needs_login","account_fingerprint":"a".repeat(64),
            "execution_proof":{"version":1,"stage":"pre_submission_auth","add_order_calls":0,"captcha_calls":0,"journal_created":false}})
    }

    #[test]
    fn execution_time_expiry_recovers_original_account_once_only_after_positive_no_send_proof() {
        let account="a".repeat(64);
        let recovered=ready(&account);
        let calls=std::cell::Cell::new(0);
        let result=recover_pre_submit(before_send_auth_failure(),Some(&account),|bound| {
            assert_eq!(bound,account);calls.set(calls.get()+1);Ok(recovered)
        },|| {calls.set(calls.get()+1);Ok(json!({"ok":true,"verified":true}))}).unwrap();
        assert_eq!(calls.get(),2);assert_eq!(result["auth_recovery_before_submit"],true);
        assert_eq!(result["verified"],true);
        let repeated=recover_pre_submit(before_send_auth_failure(),Some(&account),|_|Ok(ready(&account)),||Ok(before_send_auth_failure())).unwrap();
        assert_eq!(repeated["code"],"SESSION_EXPIRED");
    }

    #[test]
    fn missing_ambiguous_sent_or_different_account_evidence_never_restarts_a_worker() {
        let account="a".repeat(64);
        for change in [json!({"code":"SUBMIT_UNKNOWN"}),json!({"code":"HTTP_WORKER_ERROR"}),
            json!({"state":"unknown"}),json!({"account_fingerprint":"b".repeat(64)}),
            json!({"execution_proof":null}),json!({"request_ref":"c".repeat(64)}),
            json!({"execution_proof":{"version":1,"stage":"pre_submission_auth","add_order_calls":1,"captcha_calls":0,"journal_created":false}})] {
            let mut value=before_send_auth_failure();value.as_object_mut().unwrap().extend(change.as_object().unwrap().clone());
            assert_eq!(recover_pre_submit(value.clone(),Some(&account),|_|panic!("unsafe recovery"),||panic!("unsafe resend")).unwrap(),value);
        }
        for auth in [json!({"ok":false,"code":"LOGIN_CONFIRM_REQUIRED"}),ready(&"b".repeat(64)),json!({"ok":true})] {
            let result=recover_pre_submit(before_send_auth_failure(),Some(&account),|_|Ok(auth),||panic!("unverified account retried")).unwrap();
            assert_ne!(result["ok"],true);
        }
          assert_eq!(recover_pre_submit(before_send_auth_failure(),Some(&account),|_|Err("已取消".into()),||panic!("cancelled recovery retried")).unwrap_err(),"已取消");
          assert_eq!(recover_pre_submit(before_send_auth_failure(),Some(&account),|_|Ok(ready(&account)),||Err("SUBMIT_UNKNOWN".into())).unwrap_err(),"SUBMIT_UNKNOWN");
    }

    #[test]
    fn unknown_operation_never_defaults_to_submission() {
        for until in [None, Some("auth"), Some("prepare"), Some("trial_slot"), Some("submit")] {
            assert!(validate_operation(until).is_ok());
        }
        for until in ["", "check_existing", "preapre", "cancel"] {
            assert!(validate_operation(Some(until)).is_err());
        }
    }

    fn ready(account: &str) -> Value {
        json!({"ok":true,"state":"ready","authenticated":true,
               "account_checked":true,"account_fingerprint":account})
    }

    #[test]
    fn valid_entry_never_opens_the_runtime_even_when_recovery_is_unavailable() {
        for expected in [None, Some("a")] {
            let checked = ready("a");
            let result = reuse_or_recover_auth(checked.clone(), expected,
                |_| panic!("a valid HTTP entry must not require a supported kernel")).unwrap();
            assert_eq!(result, checked);
        }
    }

    #[test]
    fn only_recoverable_auth_failures_open_recovery_and_bind_the_original_account() {
        for code in ["NO_ENTRY", "INVALID_ENTRY", "SESSION_EXPIRED", "API_REJECTED"] {
            for expected in [None, Some("a")] {
                let result = reuse_or_recover_auth(
                    json!({"ok":false,"code":code,"phase":"authenticating","account_fingerprint":"a"}),
                    expected, |bound| {
                        assert_eq!(bound, Some("a"));
                        Ok(ready("a"))
                    }).unwrap();
                assert_eq!(result, ready("a"));
            }
        }
        let result = reuse_or_recover_auth(json!({"ok":false,"code":"NO_ENTRY"}), None, |bound| {
            assert_eq!(bound, None);
            Ok(ready("first-account"))
        }).unwrap();
        assert_eq!(result, ready("first-account"));
    }

    #[test]
    fn network_profile_cancellation_and_non_auth_failures_do_not_trigger_login() {
        for code in ["HTTP_FAILED", "SERVER_BUSY", "RATE_LIMITED", "UNEXPECTED_REDIRECT",
            "ACCOUNT_CHANGED", "AMBIGUOUS_PROFILE", "NATIVE_TOKEN_REQUIRED", "CANCELLED", "HTTP_WORKER_ERROR"] {
            let checked = json!({"ok":false,"code":code});
            assert_eq!(reuse_or_recover_auth(checked.clone(), Some("a"),
                |_| panic!("must preserve the original failure")).unwrap(), checked);
        }
        let rejected = json!({"ok":false,"code":"API_REJECTED","phase":"server_precheck"});
        assert_eq!(reuse_or_recover_auth(rejected.clone(), Some("a"),
            |_| panic!("not an authentication rejection")).unwrap(), rejected);
    }

    #[test]
    fn account_changes_are_rejected_before_and_after_recovery() {
        for checked in [ready("b"), json!({"ok":false,"code":"SESSION_EXPIRED","account_fingerprint":"b"})] {
            let result = reuse_or_recover_auth(checked, Some("a"), |_| panic!("account changed")).unwrap();
            assert_eq!(result["code"], "ACCOUNT_CHANGED");
            assert_ne!(result["ok"], true);
        }
        for expected in [None, Some("a")] {
            let result = reuse_or_recover_auth(
                json!({"ok":false,"code":"SESSION_EXPIRED","account_fingerprint":"a"}),
                expected, |_| Ok(ready("b"))).unwrap();
            assert_eq!(result["code"], "ACCOUNT_CHANGED");
        }
    }

    #[test]
    fn incomplete_identity_checks_are_not_reused_or_accepted_after_recovery() {
        for incomplete in [
            json!({"ok":true,"account_fingerprint":"a"}),
            json!({"ok":true,"authenticated":false,"account_checked":true,"account_fingerprint":"a"}),
            json!({"ok":true,"authenticated":true,"account_checked":false,"account_fingerprint":"a"}),
            json!({"ok":true,"authenticated":true,"account_checked":true}),
            json!({"ok":true,"authenticated":true,"account_checked":true,"account_fingerprint":""}),
        ] {
            let result = reuse_or_recover_auth(incomplete.clone(), Some("a"),
                |_| panic!("unverified success is not an expired login")).unwrap();
            assert_eq!(result["code"], "HTTP_AUTH_UNVERIFIED");
            let result = reuse_or_recover_auth(json!({"ok":false,"code":"NO_ENTRY"}), Some("a"),
                |_| Ok(incomplete)).unwrap();
            assert_eq!(result["code"], "HTTP_AUTH_UNVERIFIED");
        }
    }

    #[test]
    fn scheduled_recheck_recovers_only_if_the_bound_entry_has_expired() {
        let mut recoveries = 0;
        let prepared = reuse_or_recover_auth(ready("a"), None, |_| {
            recoveries += 1;
            Ok(ready("a"))
        }).unwrap();
        let account = verified_account(&prepared, None).unwrap();
        assert_eq!(recoveries, 0);
        let rechecked = reuse_or_recover_auth(ready("a"), Some(&account), |_| {
            recoveries += 1;
            Ok(ready("a"))
        }).unwrap();
        assert_eq!(rechecked, ready("a"));
        assert_eq!(recoveries, 0);
        let recovered = reuse_or_recover_auth(json!({"ok":false,"code":"SESSION_EXPIRED"}), Some(&account), |bound| {
            assert_eq!(bound, Some("a"));
            recoveries += 1;
            Ok(ready("a"))
        }).unwrap();
        assert_eq!(recovered, ready("a"));
        assert_eq!(recoveries, 1);
    }

    #[test]
    fn recovery_failures_and_cancellation_remain_terminal() {
        for code in ["RUNTIME_UNSUPPORTED", "LOGIN_CONFIRM_REQUIRED", "CANCELLED", "SESSION_EXPIRED"] {
            let failed = json!({"ok":false,"code":code});
            let result = reuse_or_recover_auth(json!({"ok":false,"code":"SESSION_EXPIRED"}), Some("a"),
                |_| Ok(failed.clone())).unwrap();
            assert_eq!(result, failed);
        }
        let result = reuse_or_recover_auth(json!({"ok":false,"code":"SESSION_EXPIRED"}), Some("a"),
            |_| Err("已取消".into()));
        assert_eq!(result.unwrap_err(), "已取消");
    }

    #[test]
    fn transient_refresh_failure_rechecks_the_server_before_reusing_entry() {
        let refreshed = json!({"ok":false,"code":"RUNTIME_BRIDGE_FAILED"});
        let checked = json!({"ok":true,"authenticated":true,"account_checked":true,"account_fingerprint":"a"});
        let result = recheck_after_refresh_failure(refreshed, Some("a"), || Ok(checked.clone())).unwrap();
        assert_eq!(result, checked);
    }
    #[test]
    fn refreshed_entry_fallback_never_accepts_expiry_changed_account_or_unverified_result() {
        for (checked, expected_code) in [
            (json!({"ok":false,"code":"SESSION_EXPIRED"}), "SESSION_EXPIRED"),
            (json!({"ok":false,"code":"NETWORK_ERROR"}), "NETWORK_ERROR"),
            (json!({"ok":true,"authenticated":true,"account_checked":true,"account_fingerprint":"b"}), "ACCOUNT_CHANGED"),
            (json!({"ok":true,"authenticated":true,"account_checked":true}), "HTTP_AUTH_UNVERIFIED"),
            (json!({"ok":true,"account_fingerprint":"a"}), "HTTP_AUTH_UNVERIFIED"),
        ] {
            let result = recheck_after_refresh_failure(json!({"code":"RUNTIME_BRIDGE_FAILED"}), Some("a"), || Ok(checked)).unwrap();
            assert_eq!(result["code"], expected_code);
            assert_ne!(result["ok"], true);
        }
        assert_eq!(recheck_after_refresh_failure(json!({"code":"RUNTIME_BRIDGE_FAILED"}), Some("a"),
            || Err("已取消".into())).unwrap_err(), "已取消");
    }
    #[test]
    fn terminal_refresh_errors_and_unbound_accounts_never_use_fallback() {
        for code in ["CANCELLED", "ACCOUNT_CHANGED", "SESSION_EXPIRED", "RUNTIME_UNSUPPORTED",
            "RUNTIME_CLOSE_FAILED", "API_REJECTED", "SUBMIT_UNKNOWN"] {
            let result = json!({"ok":false,"code":code});
            assert_eq!(recheck_after_refresh_failure(result.clone(), Some("a"), || panic!("must stop")).unwrap(), result);
        }
        for expected in [None, Some("")] {
            recheck_after_refresh_failure(json!({"code":"RUNTIME_BRIDGE_FAILED"}), expected, || panic!("unbound account")).unwrap();
        }
        let ready = json!({"ok":true});
        assert_eq!(recheck_after_refresh_failure(ready.clone(), Some("a"), || panic!("already refreshed")).unwrap(), ready);
    }
    #[test]
    fn prepared_and_rechecked_h5_must_belong_to_the_same_account() {
        assert_eq!(verified_account(&json!({"account_fingerprint":"account-a"}), None).unwrap(), "account-a");
        assert!(verified_account(&json!({"account_fingerprint":"account-a"}), Some("account-a")).is_ok());
        assert_eq!(verified_account(&json!({"account_fingerprint":"account-b"}), Some("account-a")).unwrap_err(), "ACCOUNT_CHANGED");
        assert_eq!(verified_account(&json!({"ok":true}), Some("account-a")).unwrap_err(), "HTTP_AUTH_UNVERIFIED");
        assert!(verified_account(&json!({"account_fingerprint":""}), None).is_err());
    }
    #[test]
    fn only_missing_or_expired_entries_start_native_login_recovery() {
        for code in ["NO_ENTRY", "INVALID_ENTRY", "SESSION_EXPIRED"] {
            assert!(needs_reauthorization(&json!({"code": code})));
        }
        for code in ["ACCOUNT_CHANGED", "AMBIGUOUS_PROFILE", "NATIVE_TOKEN_REQUIRED", "HTTP_WORKER_ERROR", "SUBMIT_UNKNOWN"] {
            assert!(!needs_reauthorization(&json!({"code": code})));
        }
        assert!(!needs_reauthorization(&json!({"ok": true, "state":"ready"})));
        assert!(needs_reauthorization(&json!({"code":"API_REJECTED", "phase":"authenticating"})));
        assert!(!needs_reauthorization(&json!({"code":"API_REJECTED", "phase":"server_precheck"})));
    }
}
