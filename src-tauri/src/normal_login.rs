//! Login via Windows UI Automation, followed by verified HTTP handoff.
use std::path::Path;
use std::time::{Duration,Instant};
use serde_json::{json,Value};
pub(crate) fn manual(reason: &str) -> Value {
    json!({"ok":false,"code":"LOGIN_CONFIRM_REQUIRED","state":"needs_login",
        "recovery_mode":"normal_ui_http","reason":reason,
        "message":"请在电脑微信的厦大体育完成页面要求的登录确认，再进入场馆预约，随后重新检测登录；未提交预约"})
}

pub(crate) fn validate(result: Value, expected: Option<&str>) -> Value {
    let account = result["account_fingerprint"].as_str().filter(|value| !value.is_empty());
    // Check identity even when the server rejects a stale entry.
    if account.is_some_and(|account| expected.is_some_and(|expected| account != expected)) {
        return json!({"ok":false,"code":"ACCOUNT_CHANGED"});
    }
    if result["ok"] == true && (result["authenticated"] != true ||
        result["account_checked"] != true || account.is_none()) {
        return json!({"ok":false,"code":"HTTP_AUTH_UNVERIFIED"});
    }
    result
}

fn retryable(result: &Value) -> bool {
    matches!(result["code"].as_str(), Some("NO_ENTRY" | "INVALID_ENTRY" | "SESSION_EXPIRED" | "SESSION_READ_FAILED")) ||
        (result["code"] == "API_REJECTED" && result["phase"] == "authenticating")
}

fn handoff(expected: Option<&str>, active: &impl Fn()->bool,
    verify: &mut impl FnMut()->Result<Value,String>, progress: &mut impl FnMut(&str),
    adapter: &str,
) -> Result<Value,String> {
    progress("normal_ui_http_verifying");
    for attempt in 0..3 {
        if !active() { return Err("已取消".into()); }
        let mut result=validate(verify()?,expected);
        if !active() { return Err("已取消".into()); }
        if result["ok"]==true {
            result["recovery_mode"]=json!("normal_ui_http");
            result["login_adapter"]=json!(adapter);
            result["kernel_hook_used"]=json!(false);
            progress("normal_ui_http_ready");
            return Ok(result);
        }
        if !retryable(&result) { return Ok(result); }
        if attempt<2 {
            for _ in 0..15 {
                if !active() { return Err("已取消".into()); }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Ok(manual("H5_ENTRY_STILL_EXPIRED"))
}

/// HTTP is the authority; reaching an H5-looking page alone never proves login.
/// Callbacks make cancellation and host progress work without starting Tauri in
/// the diagnostic command (which must never resume an armed booking).
pub(crate) fn recover(
    root: &Path, run_id: &str, expected: Option<&str>,
    active: impl Fn() -> bool,
    mut verify: impl FnMut() -> Result<Value, String>,
    mut progress: impl FnMut(&str),
    mut element_evidence: impl FnMut(Value),
) -> Result<Value,String> {
    if !active() {return Err("已取消".into());}
    let overlay=crate::login_overlay::LoginOverlay::start();
    let mut report=|phase:&str| {overlay.phase(phase);progress(phase);};
    let mut result=recover_once(root,run_id,expected,&active,&mut verify,&mut report,&mut element_evidence)?;
    if should_reenter(&result) {
        if !active() { return Err("已取消".into()); }
        report("normal_ui_reenter");
        if let Err(code)=reopen_miniprogram(&active) {
            return if code=="已取消" {Err(code)} else {Ok(manual(&code))};
        }
        result=recover_once(root,run_id,expected,&active,&mut verify,&mut report,&mut element_evidence)?;
    }
    if result["ok"]==true {
        if !active() {return Err("已取消".into());}
        let account=result["account_fingerprint"].as_str().map(str::to_owned);
        report("closing_miniprogram");
        match close_miniprogram(&active) {
            Ok(())=> {
                // Closing the mini-program must not be mistaken for a logout.
                let previous=result;
                result=validate(verify()?,account.as_deref());
                for key in ["recovery_mode","login_adapter","kernel_hook_used"] {result[key]=previous[key].clone();}
                result["mini_program_closed"]=json!(true);
                report("normal_ui_closed");
            },
            Err(code) if code=="已取消"=>return Err(code),
            Err(_)=>{result["mini_program_closed"]=json!(false);report("normal_ui_close_pending");},
        }
    }
    Ok(result)
}

fn close_miniprogram(active:&impl Fn()->bool)->Result<(),String> {
    use windows::Win32::Foundation::{WPARAM,LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW,WM_CLOSE,IsWindow,IsWindowVisible};
    let targets=crate::wechat_window::discover_wechat_candidates()?;
    if targets.is_empty() {return Ok(());}
    if targets.len()!=1 {return Err("UIA_TARGET_AMBIGUOUS".into());}
    let target=&targets[0];
    if !active() {return Err("已取消".into());}
    unsafe{PostMessageW(Some(target.hwnd()),WM_CLOSE,WPARAM(0),LPARAM(0))}.map_err(|_|"MINIPROGRAM_CLOSE_FAILED")?;
    for _ in 0..30 {
        if !active() {return Err("已取消".into());}
        if !unsafe{IsWindow(Some(target.hwnd())).as_bool() && IsWindowVisible(target.hwnd()).as_bool()} {return Ok(());}
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("MINIPROGRAM_CLOSE_PENDING".into())
}

fn should_reenter(result:&Value)->bool {
    result["ok"]!=true && result["code"]=="LOGIN_CONFIRM_REQUIRED" && result["reason"]=="H5_ENTRY_STILL_EXPIRED"
}

fn reopen_miniprogram(active:&impl Fn()->bool)->Result<(),String> {
    use windows::Win32::Foundation::{WPARAM,LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW,WM_CLOSE};
    if !active() {return Err("已取消".into());}
    let candidates=crate::wechat_window::discover_wechat_candidates().map_err(|_|"MINIPROGRAM_DISCOVERY_FAILED")?;
    let targets=candidates.into_iter().filter(|s|s.visible && s.title.contains("厦大体育")).collect::<Vec<_>>();
    if targets.len()>1 { return Err("UIA_TARGET_AMBIGUOUS".into()); }
    if let Some(target)=targets.first() {
        unsafe{PostMessageW(Some(target.hwnd()),WM_CLOSE,WPARAM(0),LPARAM(0))}.map_err(|_|"MINIPROGRAM_CLOSE_FAILED")?;
        let started=Instant::now();
        loop {
            if !active() {return Err("已取消".into());}
            let surfaces=crate::wechat_window::discover_wechat_candidates().map_err(|_|"MINIPROGRAM_DISCOVERY_FAILED")?;
            if !surfaces.iter().any(|s|s.visible && s.hwnd_u64==target.hwnd_u64 && s.owner==target.owner) {break;}
            if started.elapsed()>Duration::from_secs(3) {return Err("MINIPROGRAM_CLOSE_PENDING".into());}
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if !active() {return Err("已取消".into());}
    crate::wechat_window::launch_xmu_miniprogram(None).map_err(|_|"MINIPROGRAM_LAUNCH_FAILED")?;
    let started=Instant::now();
    loop {
        if !active() {return Err("已取消".into());}
        let surfaces=crate::wechat_window::discover_wechat_candidates().map_err(|_|"MINIPROGRAM_DISCOVERY_FAILED")?;
        if surfaces.iter().any(|s|s.visible && s.title.contains("厦大体育")) {return Ok(());}
        if started.elapsed()>Duration::from_secs(15) {return Err("MINIPROGRAM_NOT_VISIBLE".into());}
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn recover_once(
    _root:&Path,_run_id:&str,expected:Option<&str>,active:impl Fn()->bool,
    mut verify:impl FnMut()->Result<Value,String>,mut progress:impl FnMut(&str),
    mut element_evidence:impl FnMut(Value),
)->Result<Value,String> {
    let started=Instant::now();
    let pause=|ms:u64|->Result<(),String> {
        for _ in 0..ms.div_ceil(100) {
            if !active() {return Err("已取消".into());}
            std::thread::sleep(Duration::from_millis(100));
        }
        if active() {Ok(())} else {Err("已取消".into())}
    };
    if !active() {return Err("已取消".into());}
    progress("normal_ui_opening");
    let present=||crate::wechat_window::discover_wechat_candidates().map(|s|!s.is_empty()).unwrap_or(false);
    if !present() {
        crate::wechat_window::launch_xmu_miniprogram(None)?;
        while !present() && started.elapsed()<Duration::from_secs(15) {pause(300)?;}
    }
    if !present() {return Ok(manual("MINIPROGRAM_NOT_VISIBLE"));}
    if let Err(code)=crate::wechat_window::foreground_miniprogram_window() {return Ok(manual(&code));}
    progress("normal_ui_foreground");
    let semantic=match crate::semantic_login::Session::open() {
        Ok(session)=>session,Err(code)=>return Ok(manual(&code)),
    };
    let mut pending:Option<(crate::semantic_login::Control,Instant)>=None;
    let mut empty_since:Option<Instant>=None;
    while started.elapsed()<Duration::from_secs(180) {
        if !active() {return Err("已取消".into());}
        let report=match semantic.observe() {
            Ok(report)=>report,Err(code)=>return Ok(manual(&code)),
        };
        if report.has_controls() || report.decision==crate::semantic_login::Decision::Handoff {empty_since=None;}
        else if empty_since.get_or_insert_with(Instant::now).elapsed()>Duration::from_secs(12) {return Ok(manual("UIA_LOGIN_UNAVAILABLE"));}
        element_evidence(json!(report));
        use crate::semantic_login::{Decision,Control};
        match report.decision {
            Decision::Handoff=>return handoff(expected,&active,&mut verify,&mut progress,"uia"),
            Decision::Activate(control)=> {
                if let Some((previous,sent_at))=&pending {
                    if *previous==control {
                        if sent_at.elapsed()>Duration::from_secs(15) {return Ok(manual("UIA_NAVIGATION_TIMEOUT"));}
                        pause(500)?;continue;
                    }
                }
                progress("semantic_login_control");
                progress(match control {Control::Login=>"native_login_opening",Control::Agreement=>"native_agreement",Control::Authorize=>"native_authorizing",Control::Stadium=>"entering_stadium"});
                if !active() {return Err("已取消".into());}
                match semantic.activate(control) {
                    Ok(Some(method))=> {progress(method);pending=Some((control,Instant::now()));pause(600)?;},
                    Ok(None)=>pause(300)?,
                    Err(code)=>return Ok(manual(&code)),
                }
            }
            Decision::Wait|Decision::Unsupported=>pause(500)?,
        }
    }
    Ok(manual("UIA_LOGIN_TIMEOUT"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_and_server_verification_are_required() {
        assert_eq!(validate(json!({"ok":true}),None)["code"],"HTTP_AUTH_UNVERIFIED");
        assert_eq!(validate(json!({"ok":false,"account_fingerprint":"other"}),Some("original"))["code"],"ACCOUNT_CHANGED");
        let verified=json!({"ok":true,"authenticated":true,"account_checked":true,"account_fingerprint":"original"});
        assert_eq!(validate(verified.clone(),Some("original")),verified);
        assert_eq!(validate(verified,Some("other"))["code"],"ACCOUNT_CHANGED");
    }
    #[test]
    fn cancelled_recovery_never_opens_ui_or_calls_http() {
        let result=recover(Path::new("."),"test",None,||false,||panic!("http called"),|_|panic!("ui opened"),|_|panic!("ui observed"));
        assert_eq!(result.unwrap_err(),"已取消");
    }
    #[test]
    fn only_server_rejected_stadium_entries_get_one_reentry() {
        assert!(should_reenter(&manual("H5_ENTRY_STILL_EXPIRED")));
        for result in [manual("NORMAL_UI_TIMEOUT"),manual("UIA_INPUT_OUTCOME_UNKNOWN"),
            json!({"ok":false,"code":"ACCOUNT_CHANGED"}),json!({"ok":false,"code":"NETWORK_ERROR"}),
            json!({"ok":true,"reason":"H5_ENTRY_STILL_EXPIRED"})] {
            assert!(!should_reenter(&result));
        }
    }
}
