//! Login via Windows UI Automation, followed by verified HTTP handoff.
use std::path::Path;
use std::time::{Duration,Instant};
use serde_json::{json,Value};
pub(crate) fn manual(reason: &str) -> Value {
    let message=match reason {
        "UIA_STADIUM_ENTRY_NOT_FOUND"=>"已识别厦大体育首页，但自动滚动查找后仍未找到可操作的“场馆预约”入口；可能是页面布局、缩放或控件未暴露。请手动进入场馆预约后重试；预约会话尚未核验，未提交预约",
        "UIA_HOME_SCROLL_UNAVAILABLE"=>"已识别厦大体育首页，但未找到可见的“场馆预约”入口，且当前窗口或页面未能执行自动滚动。请保持小程序可见，手动向下滚动并进入场馆预约后重试；未提交预约",
        "UIA_STADIUM_NAVIGATION_TIMEOUT"=>"已点击“场馆预约”，但未检测到预约页面；可能是页面加载或跳转异常。请查看小程序页面后重试；未提交预约",
        "MINIPROGRAM_NOT_VISIBLE"|"UIA_TARGET_MISSING"=>"未检测到可见的厦大体育小程序窗口，无法继续自动导航。请确认电脑微信已登录并打开厦大体育后重试；未提交预约",
        "H5_ENTRY_STILL_EXPIRED"=>"已进入场馆预约，但学校接口仍返回预约登录凭证失效；小程序首页登录不代表预约会话有效。请在小程序重新进入场馆预约后重试；未提交预约",
        "H5_ENTRY_UNAVAILABLE"=>"已检测到场馆预约页面，但未能从本机读取有效的预约登录凭证。请等待页面加载完成，重新进入场馆预约后重试；未提交预约",
        "UIA_LOGIN_UNAVAILABLE"|"UIA_LOGIN_TIMEOUT"=>"无法识别当前小程序页面的可操作入口，尚不能确定是登录、授权还是页面加载问题。请查看小程序当前页面，必要时手动进入场馆预约后重试；未提交预约",
        "UIA_NAVIGATION_TIMEOUT"=>"已操作登录或授权控件，但页面未继续跳转。请查看小程序是否仍需协议、授权或身份确认后重试；未提交预约",
        "UIA_INPUT_OUTCOME_UNKNOWN"=>"小程序操作结果未能确认，已停止自动操作，避免重复点击。请查看当前页面后重试；未提交预约",
        "UIA_TARGET_AMBIGUOUS"=>"检测到多个厦大体育窗口，无法确定操作目标。请只保留一个小程序窗口后重试；未提交预约",
        _=>"小程序自动导航未完成，预约登录状态尚未核验。请查看当前页面，手动进入场馆预约后重试；未提交预约",
    };
    json!({"ok":false,"code":"LOGIN_CONFIRM_REQUIRED","state":"needs_login",
        "recovery_mode":"normal_ui_http","reason":reason,
        "message":message})
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
        if attempt==2 && result["code"]!="SESSION_EXPIRED" && result["code"]!="API_REJECTED" {
            let mut failure=manual("H5_ENTRY_UNAVAILABLE");
            failure["entry_error_code"]=result["code"].clone();
            return Ok(failure);
        }
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
    verify: impl FnMut() -> Result<Value, String>,
    progress: impl FnMut(&str),
    element_evidence: impl FnMut(Value),
) -> Result<Value,String> {
    recover_mode(root,run_id,expected,false,active,verify,progress,element_evidence)
}

pub(crate) fn recover_fresh(
    root: &Path, run_id: &str, expected: Option<&str>,
    active: impl Fn() -> bool, verify: impl FnMut() -> Result<Value,String>,
    progress: impl FnMut(&str),element_evidence:impl FnMut(Value),
) -> Result<Value,String> {
    recover_mode(root,run_id,expected,true,active,verify,progress,element_evidence)
}

pub(crate) fn recover_mode(
    root:&Path,run_id:&str,expected:Option<&str>,fresh:bool,
    active:impl Fn()->bool,mut verify:impl FnMut()->Result<Value,String>,
    mut progress:impl FnMut(&str),mut element_evidence:impl FnMut(Value),
) -> Result<Value,String> {
    if !active() {return Err("已取消".into());}
    let evidence=std::cell::RefCell::new(crate::login_evidence::LoginEvidence::new(root,run_id));
    let overlay=crate::login_overlay::LoginOverlay::start();
    let mut report=|phase:&str| {evidence.borrow_mut().record(phase,json!({}));overlay.phase(phase);progress(phase);};
    let mut observe=|value:Value| {
        if active() && value["diagnostic_capture"]==true {
            evidence.borrow_mut().snapshot(value["reason"].as_str().unwrap_or("unknown"),&overlay,&active);
        }
        evidence.borrow_mut().record("login_elements",value.clone());element_evidence(value);
    };
    let outcome=(|| {
        let mut attempts=0;
        let mut result=recovery_attempts(fresh,&active,|reopen| {
            if reopen {
                if !active() {return Err("已取消".into());}
                if attempts>0 {observe(json!({"diagnostic_capture":true,"reason":"before_reopen"}));}
                report("normal_ui_reenter");
                if let Err(code)=reopen_miniprogram(&active) {
                    return if code=="已取消" {Err(code)} else {Ok(manual(&code))};
                }
            }
            attempts+=1;
            let result=recover_once(root,run_id,expected,&active,&mut verify,&mut report,&mut observe)?;
            observe(json!({"attempt_finished":true,"result":result}));
            Ok(result)
        })?;
        if result["ok"]==true {
            if !active() {return Err("已取消".into());}
            let account=result["account_fingerprint"].as_str().map(str::to_owned);
            report("closing_miniprogram");
            match close_miniprogram(&active) {
                Ok(())=> {
                    // Closing the mini-program must not be mistaken for a logout.
                    let previous=result;
                    if !active() {return Err("已取消".into());}
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
    })();
    if active() && !matches!(&outcome,Ok(result) if result["ok"]==true) {
        evidence.borrow_mut().snapshot("final_failure",&overlay,&active);
    }
    evidence.borrow_mut().record("finished",match &outcome {
        Ok(result)=>result.clone(),Err(code)=>json!({"ok":false,"code":code}),
    });
    outcome
}

fn recovery_attempts(fresh:bool,active:&impl Fn()->bool,mut attempt:impl FnMut(bool)->Result<Value,String>)->Result<Value,String> {
    if !active() {return Err("已取消".into());}
    let result=attempt(fresh)?;
    if !should_reenter(&result) {return Ok(result);}
    if !active() {return Err("已取消".into());}
    attempt(true)
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
    result["ok"]!=true && result["code"]=="LOGIN_CONFIRM_REQUIRED" && matches!(result["reason"].as_str(),
        Some("H5_ENTRY_STILL_EXPIRED"|"H5_ENTRY_UNAVAILABLE"|"UIA_LOGIN_UNAVAILABLE"|"UIA_NAVIGATION_TIMEOUT"|"UIA_LOGIN_TIMEOUT"))
}

fn reopen_miniprogram(active:&impl Fn()->bool)->Result<(),String> {
    use windows::Win32::Foundation::{WPARAM,LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW,WM_CLOSE};
    if !active() {return Err("已取消".into());}
    let candidates=crate::wechat_window::discover_wechat_candidates().map_err(|_|"MINIPROGRAM_DISCOVERY_FAILED")?;
    let targets=candidates.into_iter().filter(|s|s.title.contains("厦大体育")).collect::<Vec<_>>();
    if targets.len()>1 { return Err("UIA_TARGET_AMBIGUOUS".into()); }
    if let Some(target)=targets.first() {
        unsafe{PostMessageW(Some(target.hwnd()),WM_CLOSE,WPARAM(0),LPARAM(0))}.map_err(|_|"MINIPROGRAM_CLOSE_FAILED")?;
        let started=Instant::now();
        loop {
            if !active() {return Err("已取消".into());}
            let surfaces=crate::wechat_window::discover_wechat_candidates().map_err(|_|"MINIPROGRAM_DISCOVERY_FAILED")?;
            if !surfaces.iter().any(|s|s.hwnd_u64==target.hwnd_u64 && s.owner==target.owner) {break;}
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
    let mut semantic=match crate::semantic_login::Session::open() {
        Ok(session)=>session,Err(code)=>return Ok(manual(&code)),
    };
    navigate(&mut semantic,expected,&active,&mut verify,&mut progress,&mut element_evidence,
        ||started.elapsed(),pause)
}

trait LoginUi {
    fn observe(&mut self)->Result<crate::semantic_login::Observation,String>;
    fn activate(&mut self,control:crate::semantic_login::Control)->Result<Option<&'static str>,String>;
    fn refresh(&mut self)->Result<(),String>;
    fn scroll_home(&mut self,active:&dyn Fn()->bool)->Result<Option<&'static str>,String>;
}
impl LoginUi for crate::semantic_login::Session {
    fn observe(&mut self)->Result<crate::semantic_login::Observation,String> {Self::observe(self)}
    fn activate(&mut self,control:crate::semantic_login::Control)->Result<Option<&'static str>,String> {Self::activate(self,control)}
    fn refresh(&mut self)->Result<(),String> {Self::refresh(self)}
    fn scroll_home(&mut self,active:&dyn Fn()->bool)->Result<Option<&'static str>,String> {Self::scroll_home(self,active)}
}

#[derive(Default)]
struct NavigationState {
    pending:Option<(crate::semantic_login::Control,Duration)>,
    empty_since:Option<Duration>,refreshed:bool,
    home_seen:bool,scrolls:u32,scrolls_sent:u32,last_scroll:Option<Duration>,
}
#[derive(Debug,PartialEq)]
enum NavigationAction { Handoff,Activate(crate::semantic_login::Control),Scroll,Refresh,Wait,Fail(&'static str) }
impl NavigationState {
    fn next(&mut self,report:&crate::semantic_login::Observation,now:Duration)->NavigationAction {
        use crate::semantic_login::Decision;
        self.home_seen|=report.home_page;
        if report.has_controls() || report.decision==Decision::Handoff {self.empty_since=None;}
        let stalled=match report.decision {
            Decision::Handoff=>return NavigationAction::Handoff,
            Decision::Activate(control)=> {
                if let Some((previous,sent))=self.pending {
                    if previous==control {Some((now.saturating_sub(sent),if control==crate::semantic_login::Control::Stadium {"UIA_STADIUM_NAVIGATION_TIMEOUT"} else {"UIA_NAVIGATION_TIMEOUT"}))}
                    else {self.refreshed=false;return NavigationAction::Activate(control);}
                } else {self.refreshed=false;return NavigationAction::Activate(control);}
            },
            Decision::Wait|Decision::Unsupported=>Some((now.saturating_sub(*self.empty_since.get_or_insert(now)),
                if self.pending.is_some_and(|(c,_)|c==crate::semantic_login::Control::Stadium) {"UIA_STADIUM_NAVIGATION_TIMEOUT"}
                else if self.home_seen {"UIA_STADIUM_ENTRY_NOT_FOUND"} else {"UIA_LOGIN_UNAVAILABLE"})),
        };
        if let Some((duration,code))=stalled {
            if duration>=Duration::from_secs(30) {return NavigationAction::Fail(code);}
            if report.home_page && report.decision==Decision::Wait && self.pending.is_none_or(|(c,_)|c!=crate::semantic_login::Control::Stadium)
                && duration>=Duration::from_secs(2) && self.scrolls<6
                && self.last_scroll.is_none_or(|sent|now.saturating_sub(sent)>=Duration::from_millis(800)) {
                self.scrolls+=1;self.last_scroll=Some(now);return NavigationAction::Scroll;
            }
            if duration>=Duration::from_secs(12) && !self.refreshed {self.refreshed=true;return NavigationAction::Refresh;}
        }
        NavigationAction::Wait
    }
}

fn navigate(
    ui:&mut impl LoginUi,expected:Option<&str>,active:&impl Fn()->bool,
    verify:&mut impl FnMut()->Result<Value,String>,progress:&mut impl FnMut(&str),
    evidence:&mut impl FnMut(Value),elapsed:impl Fn()->Duration,mut pause:impl FnMut(u64)->Result<(),String>,
)->Result<Value,String> {
    let mut state=NavigationState::default();
    while elapsed()<Duration::from_secs(180) {
        if !active() {return Err("已取消".into());}
        let report=match ui.observe() {Ok(report)=>report,Err(code)=>return Ok(manual(&code))};
        if !active() {return Err("已取消".into());}
        let action=state.next(&report,elapsed());
        evidence(json!(report));
        use crate::semantic_login::Control;
        match action {
            NavigationAction::Scroll=> {
                progress("normal_ui_scrolling");
                if !active() {return Err("已取消".into());}
                match ui.scroll_home(active) {
                    Ok(method)=>{if method.is_some() {state.scrolls_sent+=1;} evidence(json!({"scroll_step":state.scrolls,"scroll_method":method}));},
                    Err(code) if code=="已取消"=>return Err(code),
                    Err(code)=>return Ok(manual(&code)),
                }
                pause(800)?;
            },
            NavigationAction::Handoff=> {
                evidence(json!({"diagnostic_capture":true,"reason":"stadium_handoff"}));
                return handoff(expected,active,verify,progress,"uia");
            },
            NavigationAction::Refresh=> {
                evidence(json!({"diagnostic_capture":true,"reason":"UIA_PAGE_STALLED","elapsed_ms":elapsed().as_millis()}));
                if !active() {return Err("已取消".into());}
                progress("normal_ui_refreshing");
                if let Err(code)=ui.refresh() {return Ok(manual(&code));}
                pause(500)?;
            },
            NavigationAction::Activate(control)=> {
                progress("semantic_login_control");
                progress(match control {Control::Login=>"native_login_opening",Control::Agreement=>"native_agreement",Control::Authorize=>"native_authorizing",Control::Stadium=>"entering_stadium"});
                if !active() {return Err("已取消".into());}
                match ui.activate(control) {
                    Ok(Some(method))=>{progress(method);state.pending=Some((control,elapsed()));pause(600)?;},
                    Ok(None)=>pause(300)?,Err(code)=>return Ok(manual(&code)),
                }
            },
            NavigationAction::Wait=>pause(500)?,
            NavigationAction::Fail(code)=>return Ok(manual(if code=="UIA_STADIUM_ENTRY_NOT_FOUND" && state.scrolls_sent==0 {"UIA_HOME_SCROLL_UNAVAILABLE"} else {code})),
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
    fn only_known_navigation_stalls_and_rejected_entries_get_reentry() {
        assert!(should_reenter(&manual("H5_ENTRY_STILL_EXPIRED")));
        assert!(should_reenter(&manual("H5_ENTRY_UNAVAILABLE")));
        assert!(should_reenter(&manual("UIA_LOGIN_UNAVAILABLE")));
        assert!(should_reenter(&manual("UIA_NAVIGATION_TIMEOUT")));
        for result in [manual("NORMAL_UI_TIMEOUT"),manual("UIA_INPUT_OUTCOME_UNKNOWN"),
            json!({"ok":false,"code":"ACCOUNT_CHANGED"}),json!({"ok":false,"code":"NETWORK_ERROR"}),
            json!({"ok":true,"reason":"H5_ENTRY_STILL_EXPIRED"})] {
            assert!(!should_reenter(&result));
        }
    }

    struct FixtureUi {
        time:std::rc::Rc<std::cell::Cell<u64>>,phase:u32,ready_after:u64,
        activations:Vec<crate::semantic_login::Control>,refreshes:u32,unknown_input:bool,
    }
    impl LoginUi for FixtureUi {
        fn observe(&mut self)->Result<crate::semantic_login::Observation,String> {
            use crate::semantic_login::{Control,Observation};
            Ok(match self.phase {
                0=>Observation::fixture(false,&[(Control::Login,true,None)]),
                1=>Observation::fixture(false,&[(Control::Agreement,true,Some(false)),(Control::Authorize,false,None)]),
                2=>Observation::fixture(false,&[(Control::Agreement,true,Some(true)),(Control::Authorize,true,None)]),
                3 if self.time.get()<self.ready_after=>Observation::fixture(false,&[]),
                3=>Observation::fixture(false,&[(Control::Stadium,true,None)]),
                _=>Observation::fixture(true,&[]),
            })
        }
        fn activate(&mut self,control:crate::semantic_login::Control)->Result<Option<&'static str>,String> {
            if self.unknown_input {return Err("UIA_INPUT_OUTCOME_UNKNOWN".into());}
            self.activations.push(control);self.phase+=1;Ok(Some("fixture_input"))
        }
        fn refresh(&mut self)->Result<(),String> {self.refreshes+=1;Ok(())}
        fn scroll_home(&mut self,_:&dyn Fn()->bool)->Result<Option<&'static str>,String> {panic!("unknown page must not scroll")}
    }
    fn fixture(time:std::rc::Rc<std::cell::Cell<u64>>,ready_after:u64)->FixtureUi {
        FixtureUi{time,phase:0,ready_after,activations:vec![],refreshes:0,unknown_input:false}
    }

    #[test]
    fn expired_login_completes_after_authorization_page_is_unrecognized_for_twenty_seconds() {
        use crate::semantic_login::Control;
        let time=std::rc::Rc::new(std::cell::Cell::new(0));let mut ui=fixture(time.clone(),22_000);
        let mut verified=0;let mut captures=0;
        let result=navigate(&mut ui,Some("original"),&||true,&mut || {
            verified+=1;Ok(json!({"ok":true,"authenticated":true,"account_checked":true,"account_fingerprint":"original"}))
        },&mut |_|{},&mut |value| {if value["diagnostic_capture"]==true {captures+=1;}},
            ||Duration::from_millis(time.get()),|ms|{time.set(time.get()+ms);Ok(())}).unwrap();
        assert_eq!(result["ok"],true);assert_eq!(verified,1);assert_eq!(ui.refreshes,1);assert_eq!(captures,2);
        assert_eq!(ui.activations,vec![Control::Login,Control::Agreement,Control::Authorize,Control::Stadium]);
    }

    #[test]
    fn persistent_missing_page_reopens_once_and_preserves_failure() {
        let time=std::rc::Rc::new(std::cell::Cell::new(0));let mut reopens=vec![];
        let result=recovery_attempts(false,&||true,|reopen| {
            reopens.push(reopen);let start=time.get();let mut ui=fixture(time.clone(),u64::MAX);
            let result=navigate(&mut ui,None,&||true,&mut ||panic!("no H5 was reached"),&mut |_|{},&mut |_|{},
                ||Duration::from_millis(time.get()-start),|ms|{time.set(time.get()+ms);Ok(())})?;
            assert_eq!(ui.refreshes,1);assert_eq!(ui.activations.len(),3);
            Ok(result)
        }).unwrap();
        assert_eq!(reopens,vec![false,true]);assert_eq!(result["reason"],"UIA_LOGIN_UNAVAILABLE");
        assert!(time.get()>=60_000 && time.get()<70_000);
    }

    #[test]
    fn reopening_recovers_a_stalled_authorization_without_replaying_booking() {
        let mut opens=vec![];
        let result=recovery_attempts(false,&||true,|reopen| {
            opens.push(reopen);
            if !reopen {return Ok(manual("UIA_LOGIN_UNAVAILABLE"));}
            let time=std::rc::Rc::new(std::cell::Cell::new(0));let mut ui=fixture(time.clone(),0);
            navigate(&mut ui,None,&||true,&mut ||Ok(json!({"ok":true,"authenticated":true,"account_checked":true,"account_fingerprint":"a"})),
                &mut |_|{},&mut |_|{},||Duration::from_millis(time.get()),|ms|{time.set(time.get()+ms);Ok(())})
        }).unwrap();
        assert_eq!(result["ok"],true);assert_eq!(opens,vec![false,true]);
    }

    #[test]
    fn cancellation_and_unknown_input_never_reopen_or_verify() {
        let active=std::cell::Cell::new(true);let mut attempts=0;
        assert_eq!(recovery_attempts(false,&||active.get(),|_| {
            attempts+=1;active.set(false);Ok(manual("UIA_LOGIN_UNAVAILABLE"))
        }).unwrap_err(),"已取消");assert_eq!(attempts,1);
        let time=std::rc::Rc::new(std::cell::Cell::new(0));let mut ui=fixture(time.clone(),0);ui.unknown_input=true;
        let result=recovery_attempts(false,&||true,|reopen| {
            assert!(!reopen);
            navigate(&mut ui,None,&||true,&mut ||panic!("must not verify"),&mut |_|{},&mut |_|{},
                ||Duration::ZERO,|_|panic!("must not wait after unknown input"))
        }).unwrap();
        assert_eq!(result["reason"],"UIA_INPUT_OUTCOME_UNKNOWN");assert!(ui.activations.is_empty());
        let mut ui=fixture(time.clone(),u64::MAX);let active=||time.get()<15_000;
        assert_eq!(navigate(&mut ui,None,&active,&mut ||panic!("cancelled"),&mut |_|{},&mut |_|{},
            ||Duration::from_millis(time.get()),|ms|{time.set(time.get()+ms);Ok(())}).unwrap_err(),"已取消");
    }

    #[test]
    fn unchanged_authorize_button_is_not_clicked_again_while_waiting() {
        struct Stuck {clicks:u32,refreshes:u32}
        impl LoginUi for Stuck {
            fn observe(&mut self)->Result<crate::semantic_login::Observation,String> {
                Ok(crate::semantic_login::Observation::fixture(false,&[(crate::semantic_login::Control::Authorize,true,None)]))
            }
            fn activate(&mut self,_:crate::semantic_login::Control)->Result<Option<&'static str>,String> {
                self.clicks+=1;Ok(Some("fixture_invoke"))
            }
            fn refresh(&mut self)->Result<(),String> {self.refreshes+=1;Ok(())}
            fn scroll_home(&mut self,_:&dyn Fn()->bool)->Result<Option<&'static str>,String> {panic!("authorization must not scroll")}
        }
        let time=std::cell::Cell::new(0);let mut ui=Stuck{clicks:0,refreshes:0};
        let result=navigate(&mut ui,None,&||true,&mut ||panic!("no H5 reached"),&mut |_|{},&mut |_|{},
            ||Duration::from_millis(time.get()),|ms|{time.set(time.get()+ms);Ok(())}).unwrap();
        assert_eq!(result["reason"],"UIA_NAVIGATION_TIMEOUT");assert_eq!(ui.clicks,1);assert_eq!(ui.refreshes,1);
    }

    struct HomeUi {scrolls:u32,clicks:u32,reveal_after:u32,stuck:bool,unknown:bool,unavailable:bool}
    impl LoginUi for HomeUi {
        fn observe(&mut self)->Result<crate::semantic_login::Observation,String> {
            use crate::semantic_login::{Observation,Control};
            if self.clicks>0 && !self.stuck {return Ok(Observation::fixture(true,&[]));}
            if self.scrolls>=self.reveal_after {return Ok(Observation::fixture(false,&[(Control::Stadium,true,None)]));}
            let mut report=Observation::fixture(false,&[]);report.home_page=true;Ok(report)
        }
        fn activate(&mut self,c:crate::semantic_login::Control)->Result<Option<&'static str>,String> {
            assert_eq!(c,crate::semantic_login::Control::Stadium);self.clicks+=1;Ok(Some("fixture_click"))
        }
        fn refresh(&mut self)->Result<(),String> {Ok(())}
        fn scroll_home(&mut self,_:&dyn Fn()->bool)->Result<Option<&'static str>,String> {
            self.scrolls+=1;
            if self.unknown {Err("UIA_INPUT_OUTCOME_UNKNOWN".into())} else if self.unavailable {Ok(None)} else {Ok(Some("fixture_scroll"))}
        }
    }
    fn home(reveal_after:u32)->HomeUi {HomeUi{scrolls:0,clicks:0,reveal_after,stuck:false,unknown:false,unavailable:false}}
    #[test]
    fn home_scroll_reveals_stadium_and_http_still_checks_original_account() {
        for reveal_after in [0,1,3,6] {
            let time=std::cell::Cell::new(0);let mut ui=home(reveal_after);let mut verified=0;
            let result=navigate(&mut ui,Some("original"),&||true,&mut || {
                verified+=1;Ok(json!({"ok":true,"authenticated":true,"account_checked":true,"account_fingerprint":"original"}))
            },&mut |_|{},&mut |_|{},||Duration::from_millis(time.get()),|ms|{time.set(time.get()+ms);Ok(())}).unwrap();
            assert_eq!(result["ok"],true);assert_eq!(verified,1);assert_eq!(ui.scrolls,reveal_after);assert_eq!(ui.clicks,1);
        }
    }
    #[test]
    fn missing_stadium_scroll_is_bounded_and_does_not_reopen_home() {
        let time=std::cell::Cell::new(0);let mut ui=home(u32::MAX);
        let result=navigate(&mut ui,None,&||true,&mut ||panic!("no H5"),&mut |_|{},&mut |_|{},
            ||Duration::from_millis(time.get()),|ms|{time.set(time.get()+ms);Ok(())}).unwrap();
        assert_eq!(ui.scrolls,6);assert_eq!(ui.clicks,0);assert_eq!(result["reason"],"UIA_STADIUM_ENTRY_NOT_FOUND");
        assert!(!should_reenter(&result));assert!(result["message"].as_str().unwrap().contains("已识别厦大体育首页"));
        let time=std::cell::Cell::new(0);let mut ui=home(u32::MAX);ui.unavailable=true;
        let result=navigate(&mut ui,None,&||true,&mut ||panic!("no H5"),&mut |_|{},&mut |_|{},
            ||Duration::from_millis(time.get()),|ms|{time.set(time.get()+ms);Ok(())}).unwrap();
        assert_eq!(result["reason"],"UIA_HOME_SCROLL_UNAVAILABLE");
        assert!(result["message"].as_str().unwrap().contains("未能执行自动滚动"));
    }
    #[test]
    fn stadium_navigation_timeout_does_not_click_or_scroll_again() {
        let time=std::cell::Cell::new(0);let mut ui=home(0);ui.stuck=true;
        let result=navigate(&mut ui,None,&||true,&mut ||panic!("no H5"),&mut |_|{},&mut |_|{},
            ||Duration::from_millis(time.get()),|ms|{time.set(time.get()+ms);Ok(())}).unwrap();
        assert_eq!(ui.clicks,1);assert_eq!(ui.scrolls,0);assert_eq!(result["reason"],"UIA_STADIUM_NAVIGATION_TIMEOUT");
    }
    #[test]
    fn scroll_cancellation_and_unknown_outcome_stop_without_http_or_reentry() {
        let time=std::cell::Cell::new(0);let mut ui=home(3);
        let result=navigate(&mut ui,None,&||time.get()<2800,&mut ||panic!("cancelled"),&mut |_|{},&mut |_|{},
            ||Duration::from_millis(time.get()),|ms|{time.set(time.get()+ms);Ok(())});
        assert_eq!(result.unwrap_err(),"已取消");assert_eq!(ui.scrolls,1);assert_eq!(ui.clicks,0);
        let time=std::cell::Cell::new(0);let mut ui=home(3);ui.unknown=true;
        let result=navigate(&mut ui,None,&||true,&mut ||panic!("unknown input"),&mut |_|{},&mut |_|{},
            ||Duration::from_millis(time.get()),|ms|{time.set(time.get()+ms);Ok(())}).unwrap();
        assert_eq!(result["reason"],"UIA_INPUT_OUTCOME_UNKNOWN");assert_eq!(ui.scrolls,1);assert!(!should_reenter(&result));
    }
    #[test]
    fn manual_failure_messages_distinguish_navigation_visibility_and_expiry() {
        for (reason,expected) in [("MINIPROGRAM_NOT_VISIBLE","未检测到可见"),("UIA_LOGIN_UNAVAILABLE","尚不能确定"),
            ("H5_ENTRY_STILL_EXPIRED","首页登录不代表预约会话有效"),("H5_ENTRY_UNAVAILABLE","未能从本机读取"),("UIA_STADIUM_NAVIGATION_TIMEOUT","已点击")] {
            assert!(manual(reason)["message"].as_str().unwrap().contains(expected));
        }
    }
}
