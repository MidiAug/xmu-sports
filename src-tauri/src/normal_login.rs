//! Login via Windows UI Automation, followed by verified HTTP handoff.
use std::path::Path;
use std::time::{Duration,Instant};
use serde_json::{json,Value};
use crate::semantic_login::{SearchPage,ScrollDirection};
pub(crate) fn manual(reason: &str) -> Value {
    let message=match reason {
        "UIA_STADIUM_ENTRY_NOT_FOUND"=>"已识别厦大体育首页，已尝试向下查找和回顶重扫，仍未找到可操作的“场馆预约”入口；可能是页面布局、缩放、控件未暴露或滚动未生效。请手动进入场馆预约后重试；预约会话尚未核验，未提交预约",
        "UIA_LOGIN_CONTROL_NOT_FOUND"=>"已识别小程序登录页，已尝试向下查找和回顶重扫，仍未找到可操作的“立即登录”。请查看页面是否加载完整或滚动是否生效，手动点击立即登录后重试；未提交预约",
        "UIA_AUTH_CONTROL_NOT_FOUND"=>"已识别协议或微信授权页，已尝试向下查找和回顶重扫，仍未找到可操作的协议勾选或“授权登录”；也可能是协议状态无法读取、按钮仍禁用或滚动未生效。请查看当前页面并完成协议及授权后重试；未提交预约",
        "UIA_SCROLL_UNAVAILABLE"=>"已识别登录或授权页面，但当前窗口或页面未能执行自动滚动。请保持小程序可见，手动滚动找到登录、协议或授权控件后重试；未提交预约",
        "UIA_SCROLL_LIMIT_REACHED"=>"查找登录或场馆入口时已达到单方向48步上限，尚未确认页面首尾，已停止自动查找。请手动定位所需控件后重试；未提交预约",
        "UIA_SEARCH_TIMEOUT"=>"登录或场馆入口查找已达到180秒总时限，本次扫描尚未完成，已停止自动查找。请查看小程序当前页面，手动定位所需控件后重试；未提交预约",
        "UIA_PORTRAIT_UNAVAILABLE"=>"小程序窗口受当前屏幕或应用尺寸限制，未能调整为完整可见的竖屏；横屏可能使表单溢出且无法滚动。请手动缩窄窗口并拉高后重试；未提交预约",
        "UIA_WINDOW_LAYOUT_UNAVAILABLE"=>"未能读取或调整小程序窗口尺寸，可能是窗口无响应或已变化。请确认厦大体育窗口可操作后重试；未提交预约",
        "LOGIN_FOCUS_REQUIRED"=>"小程序未能稳定保持在前台，可能被最大化的微信主窗或其他窗口遮挡、抢走焦点。已停止自动操作，请切回厦大体育后重试；未提交预约",
        "UIA_PIN_UNAVAILABLE"=>"未能完成小程序及登录遮罩的置顶准备，尚未开始页面操作。请确认只有一个厦大体育窗口并重新检查；未提交预约",
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
            let result=recover_once(root,run_id,expected,&active,&overlay,&mut verify,&mut report,&mut observe)?;
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
    _root:&Path,_run_id:&str,expected:Option<&str>,active:impl Fn()->bool,overlay:&crate::login_overlay::LoginOverlay,
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
    if !active() {return Err("已取消".into());}
    progress("normal_ui_layout");
    match crate::wechat_window::prepare_portrait(&active) {
        Ok(layout)=>element_evidence(json!({"window_layout":layout})),
        Err(code) if code=="已取消"=>return Err(code),Err(code)=>return Ok(manual(&code)),
    }
    pause(300)?;
    progress("normal_ui_pinning");
    if let Err(code)=overlay.prepare_target(&active) {return if code=="已取消" {Err(code)} else {Ok(manual(&code))};}
    if let Err(code)=crate::wechat_window::foreground_miniprogram_window(&active) {return if code=="已取消" {Err(code)} else {Ok(manual(&code))};}
    progress("normal_ui_foreground");
    let mut semantic=match crate::semantic_login::Session::open() {
        Ok(session)=>session,Err(code)=>return Ok(manual(&code)),
    };
    navigate(&mut semantic,expected,&active,&mut verify,&mut progress,&mut element_evidence,
        ||started.elapsed(),pause)
}

trait LoginUi {
    fn observe(&mut self)->Result<crate::semantic_login::Observation,String>;
    fn activate(&mut self,control:crate::semantic_login::Control,active:&dyn Fn()->bool)->Result<Option<&'static str>,String>;
    fn refresh(&mut self,active:&dyn Fn()->bool)->Result<(),String>;
    fn scroll(&mut self,direction:ScrollDirection,active:&dyn Fn()->bool)->Result<Option<&'static str>,String>;
}
impl LoginUi for crate::semantic_login::Session {
    fn observe(&mut self)->Result<crate::semantic_login::Observation,String> {Self::observe(self)}
    fn activate(&mut self,control:crate::semantic_login::Control,active:&dyn Fn()->bool)->Result<Option<&'static str>,String> {Self::activate(self,control,active)}
    fn refresh(&mut self,active:&dyn Fn()->bool)->Result<(),String> {Self::refresh(self,active)}
    fn scroll(&mut self,direction:ScrollDirection,active:&dyn Fn()->bool)->Result<Option<&'static str>,String> {Self::scroll(self,direction,active)}
}

#[derive(Default)]
struct NavigationState {
    pending:Option<(crate::semantic_login::Control,Duration)>,
    empty_since:Option<Duration>,refreshed:bool,
    search:SearchState,
}
#[derive(Debug,PartialEq)]
enum NavigationAction { Handoff,Activate(crate::semantic_login::Control),Scroll(ScrollDirection),Refresh,Wait,Fail(&'static str) }

struct SearchState {
    page:Option<SearchPage>,pass:u8,direction:ScrollDirection,steps:u32,
    previous_mark:Option<u64>,unchanged:u8,awaiting:bool,
    started:Option<Duration>,last_scroll:Option<Duration>,unavailable:u8,
    boundary:Option<&'static str>,
}
impl Default for SearchState {
    fn default()->Self {Self{page:None,pass:0,direction:ScrollDirection::Down,steps:0,
        previous_mark:None,unchanged:0,awaiting:false,started:None,last_scroll:None,unavailable:0,boundary:None}}
}
impl SearchState {
    fn next(&mut self,report:&crate::semantic_login::Observation,now:Duration)->NavigationAction {
        let page=report.search_page.or(if report.home_page {Some(SearchPage::Home)} else {None});
        if self.page!=page {*self=Self{page,..Self::default()};}
        if page.is_none() {return NavigationAction::Wait;}
        if now.saturating_sub(*self.started.get_or_insert(now))<Duration::from_secs(2) {return NavigationAction::Wait;}
        if self.last_scroll.is_some_and(|sent|now.saturating_sub(sent)<Duration::from_millis(800)) {return NavigationAction::Wait;}
        if self.awaiting {
            self.unchanged=if self.previous_mark.is_some() && self.previous_mark==report.viewport_mark {self.unchanged+1} else {0};
            self.awaiting=false;
        }
        let boundary=match self.direction {ScrollDirection::Down=>report.at_bottom,ScrollDirection::Up=>report.at_top};
        // Geometry is a fallback only when the provider has no scroll position.
        let reached=boundary==Some(true) || (boundary.is_none() && self.unchanged>=2);
        if reached {
            self.boundary=Some(if boundary==Some(true) {if self.direction==ScrollDirection::Down {"uia_bottom"} else {"uia_top"}} else {"geometry_unchanged_twice"});
            match (self.direction,self.pass) {
                (ScrollDirection::Down,0)=>{self.pass=1;self.direction=ScrollDirection::Up;},
                (ScrollDirection::Up,_)=>self.direction=ScrollDirection::Down,
                (ScrollDirection::Down,_)=>return NavigationAction::Fail(match page {
                    Some(SearchPage::Home)=>"UIA_STADIUM_ENTRY_NOT_FOUND",
                    Some(SearchPage::Login)=>"UIA_LOGIN_CONTROL_NOT_FOUND",
                    _=>"UIA_AUTH_CONTROL_NOT_FOUND",
                }),
            }
            self.steps=0;self.unchanged=0;self.previous_mark=None;
            // Observe again after changing direction, before dispatching input.
            return NavigationAction::Wait;
        }
        if self.steps>=48 {return NavigationAction::Fail("UIA_SCROLL_LIMIT_REACHED");}
        self.steps+=1;self.previous_mark=report.viewport_mark;self.awaiting=true;self.last_scroll=Some(now);
        NavigationAction::Scroll(self.direction)
    }
}
impl NavigationState {
    fn next(&mut self,report:&crate::semantic_login::Observation,now:Duration)->NavigationAction {
        use crate::semantic_login::Decision;
        if report.has_controls() || report.decision==Decision::Handoff {self.empty_since=None;}
        // While rewinding, finish returning to the top before resuming clicks.
        if self.search.direction==ScrollDirection::Up && self.search.page==report.search_page
            && report.decision!=Decision::Handoff {return self.search.next(report,now);}
        let stalled=match report.decision {
            Decision::Handoff=>return NavigationAction::Handoff,
            Decision::Activate(control)=> {
                if let Some((previous,sent))=self.pending {
                    if previous==control {Some((now.saturating_sub(sent),if control==crate::semantic_login::Control::Stadium {"UIA_STADIUM_NAVIGATION_TIMEOUT"} else {"UIA_NAVIGATION_TIMEOUT"}))}
                    else {self.refreshed=false;return NavigationAction::Activate(control);}
                } else {self.refreshed=false;return NavigationAction::Activate(control);}
            },
            Decision::Wait|Decision::Unsupported=> {
                if let Some((control,sent))=self.pending.filter(|(c,_)|Some(*c)==report.next_control || (*c==crate::semantic_login::Control::Stadium && report.home_page)) {
                    Some((now.saturating_sub(sent),if control==crate::semantic_login::Control::Stadium {"UIA_STADIUM_NAVIGATION_TIMEOUT"} else {"UIA_NAVIGATION_TIMEOUT"}))
                } else if report.search_page.is_some() || report.home_page {return self.search.next(report,now);}
                else {Some((now.saturating_sub(*self.empty_since.get_or_insert(now)),"UIA_LOGIN_UNAVAILABLE"))}
            },
        };
        if let Some((duration,code))=stalled {
            if duration>=Duration::from_secs(30) {return NavigationAction::Fail(code);}
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
        if let Some(boundary)=state.search.boundary.take() {
            evidence(json!({"search_boundary":boundary,"search_page":state.search.page,
                "scroll_pass":state.search.pass+1,"next_direction":state.search.direction}));
        }
        use crate::semantic_login::Control;
        match action {
            NavigationAction::Scroll(direction)=> {
                progress(if direction==ScrollDirection::Up {"normal_ui_scroll_top"} else {"normal_ui_scrolling"});
                if !active() {return Err("已取消".into());}
                match ui.scroll(direction,active) {
                    Ok(method)=>{
                        state.search.unavailable=if method.is_none() {state.search.unavailable+1} else {0};
                        evidence(json!({"scroll_step":state.search.steps,"scroll_pass":state.search.pass+1,"scroll_direction":direction,
                            "search_page":state.search.page,"scroll_method":method,"boundary_basis":if report.at_bottom.is_some() {"uia_percent"} else {"geometry_stability"}}));
                        if state.search.unavailable>=3 {return Ok(manual(if state.search.page==Some(SearchPage::Home) {"UIA_HOME_SCROLL_UNAVAILABLE"} else {"UIA_SCROLL_UNAVAILABLE"}));}
                    },
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
                if let Err(code)=ui.refresh(active) {return if code=="已取消" {Err(code)} else {Ok(manual(&code))};}
                pause(500)?;
            },
            NavigationAction::Activate(control)=> {
                progress("semantic_login_control");
                progress(match control {Control::Login=>"native_login_opening",Control::Agreement=>"native_agreement",Control::Authorize=>"native_authorizing",Control::Stadium=>"entering_stadium"});
                if !active() {return Err("已取消".into());}
                match ui.activate(control,active) {
                    Ok(Some(method))=>{progress(method);state.pending=Some((control,elapsed()));state.search=SearchState::default();pause(600)?;},
                    Ok(None)=>pause(300)?,Err(code) if code=="已取消"=>return Err(code),Err(code)=>return Ok(manual(&code)),
                }
            },
            NavigationAction::Wait=>pause(500)?,
            NavigationAction::Fail(code)=>return Ok(manual(code)),
        }
    }
    if state.search.page.is_some() {
        let mut failure=manual("UIA_SEARCH_TIMEOUT");failure["search_page"]=json!(state.search.page);
        failure["scroll_direction"]=json!(state.search.direction);failure["scroll_pass"]=json!(state.search.pass+1);
        failure["scroll_step"]=json!(state.search.steps);return Ok(failure);
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
        fn activate(&mut self,control:crate::semantic_login::Control,_:&dyn Fn()->bool)->Result<Option<&'static str>,String> {
            if self.unknown_input {return Err("UIA_INPUT_OUTCOME_UNKNOWN".into());}
            self.activations.push(control);self.phase+=1;Ok(Some("fixture_input"))
        }
        fn refresh(&mut self,_:&dyn Fn()->bool)->Result<(),String> {self.refreshes+=1;Ok(())}
        fn scroll(&mut self,_:ScrollDirection,_:&dyn Fn()->bool)->Result<Option<&'static str>,String> {panic!("unknown page must not scroll")}
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
                Ok(crate::semantic_login::Observation::fixture(false,&[(crate::semantic_login::Control::Authorize,true,None),(crate::semantic_login::Control::Agreement,true,Some(true))]))
            }
            fn activate(&mut self,_:crate::semantic_login::Control,_:&dyn Fn()->bool)->Result<Option<&'static str>,String> {
                self.clicks+=1;Ok(Some("fixture_invoke"))
            }
            fn refresh(&mut self,_:&dyn Fn()->bool)->Result<(),String> {self.refreshes+=1;Ok(())}
            fn scroll(&mut self,_:ScrollDirection,_:&dyn Fn()->bool)->Result<Option<&'static str>,String> {panic!("pending authorization must not scroll")}
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
            let mut report=Observation::fixture(false,&[]);report.home_page=true;report.search_page=Some(SearchPage::Home);
            report.viewport_mark=Some(self.scrolls.min(6) as u64);Ok(report)
        }
        fn activate(&mut self,c:crate::semantic_login::Control,_:&dyn Fn()->bool)->Result<Option<&'static str>,String> {
            assert_eq!(c,crate::semantic_login::Control::Stadium);self.clicks+=1;Ok(Some("fixture_click"))
        }
        fn refresh(&mut self,_:&dyn Fn()->bool)->Result<(),String> {Ok(())}
        fn scroll(&mut self,_:ScrollDirection,_:&dyn Fn()->bool)->Result<Option<&'static str>,String> {
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
        assert!(ui.scrolls>6 && ui.scrolls<48);assert_eq!(ui.clicks,0);assert_eq!(result["reason"],"UIA_STADIUM_ENTRY_NOT_FOUND");
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

    // Layouts mirror images/5,6,8 using synthetic text, with a 320px viewport.
    // The fixture clips controls by geometry instead of revealing them by step count.
    struct ViewportUi {
        page:u8,offset:i32,checked:bool,geometry_only:bool,hide_first:bool,rewound:bool,
        absent:bool,oversized:bool,trace:Vec<Value>,clicks:Vec<crate::semantic_login::Control>,
    }
    impl ViewportUi {
        fn new()->Self {Self{page:0,offset:0,checked:false,geometry_only:false,hide_first:false,rewound:false,
            absent:false,oversized:false,trace:vec![],clicks:vec![]}}
        fn height(&self)->i32 {if self.oversized {20_000} else {match self.page {0=>1000,1=>1250,_=>1900}}}
        fn desired(&self)->(crate::semantic_login::Control,i32) {
            use crate::semantic_login::Control;
            match self.page {0=>(Control::Login,600),1 if !self.checked=>(Control::Agreement,690),1=>(Control::Authorize,920),_=>(Control::Stadium,1450)}
        }
        fn visible(&self,y:i32)->bool {y>=self.offset && y+50<=self.offset+320 && !self.absent && !(self.hide_first && !self.rewound)}
        fn page_kind(&self)->SearchPage {match self.page {0=>SearchPage::Login,1=>SearchPage::Authorization,_=>SearchPage::Home}}
    }
    impl LoginUi for ViewportUi {
        fn observe(&mut self)->Result<crate::semantic_login::Observation,String> {
            use crate::semantic_login::{Control,Decision,Observation};
            if self.page==3 {return Ok(Observation::fixture(true,&[]));}
            let (desired,y)=self.desired();let visible=self.visible(y);
            let controls=if self.page==1 {
                [(Control::Agreement,true,Some(self.checked),690),(Control::Authorize,self.checked,None,920)]
                    .into_iter().filter(|(_,_,_,y)|self.visible(*y)).map(|(c,e,s,_)|(c,e,s)).collect::<Vec<_>>()
            } else if visible {vec![(desired,true,None)]} else {vec![]};
            let mut report=Observation::fixture(false,&controls);
            report.decision=if visible {Decision::Activate(desired)} else {Decision::Wait};
            report.next_control=Some(desired);report.search_page=Some(self.page_kind());report.home_page=self.page==2;
            report.viewport_mark=Some(self.offset as u64);
            report.at_top=if self.geometry_only {None} else {Some(self.offset==0)};
            report.at_bottom=if self.geometry_only {None} else {Some(self.offset==self.height()-320)};
            self.trace.push(json!({"event":"observe","page":self.page_kind(),"offset":self.offset,
                "height":self.height(),"target":desired,"target_y":y,"target_visible":visible,"checked":self.checked}));
            Ok(report)
        }
        fn activate(&mut self,control:crate::semantic_login::Control,_:&dyn Fn()->bool)->Result<Option<&'static str>,String> {
            let (desired,y)=self.desired();assert_eq!(control,desired);assert!(self.visible(y));
            self.trace.push(json!({"event":"activate","page":self.page_kind(),"offset":self.offset,"control":control}));self.clicks.push(control);
            if control==crate::semantic_login::Control::Agreement {assert!(!self.checked);self.checked=true;}
            else {if control==crate::semantic_login::Control::Authorize {assert!(self.checked);}
                self.page+=1;self.offset=0;}
            Ok(Some("fixture_click"))
        }
        fn refresh(&mut self,_:&dyn Fn()->bool)->Result<(),String> {Ok(())}
        fn scroll(&mut self,direction:ScrollDirection,active:&dyn Fn()->bool)->Result<Option<&'static str>,String> {
            assert!(active());let before=self.offset;
            self.offset=(self.offset+if direction==ScrollDirection::Down {160} else {-160}).clamp(0,self.height()-320);
            if direction==ScrollDirection::Up && self.offset==0 {self.rewound=true;}
            self.trace.push(json!({"event":"scroll","page":self.page_kind(),"direction":direction,"before":before,"offset":self.offset}));
            Ok(Some("fixture_wheel"))
        }
    }
    fn run_viewport(ui:&mut ViewportUi)->Value {
        let time=std::cell::Cell::new(0);
        navigate(ui,Some("original"),&||true,&mut ||Ok(json!({"ok":true,"authenticated":true,"account_checked":true,"account_fingerprint":"original"})),
            &mut |_|{},&mut |_|{},||Duration::from_millis(time.get()),|ms|{time.set(time.get()+ms);Ok(())}).unwrap()
    }
    #[test]
    fn small_viewport_scans_login_agreement_authorize_and_home_then_verifies_http() {
        use crate::semantic_login::Control;
        for geometry_only in [false,true] {
            let mut ui=ViewportUi::new();ui.geometry_only=geometry_only;
            assert_eq!(run_viewport(&mut ui)["ok"],true);
            assert_eq!(ui.clicks,vec![Control::Login,Control::Agreement,Control::Authorize,Control::Stadium]);
            for page in [SearchPage::Login,SearchPage::Authorization,SearchPage::Home] {
                assert!(ui.trace.iter().any(|e|e["event"]=="scroll" && e["page"]==json!(page)));
            }
            let output=Path::new(env!("CARGO_MANIFEST_DIR")).join("../artifacts/login-scroll-fixture");
            std::fs::create_dir_all(&output).unwrap();
            std::fs::write(output.join(if geometry_only {"geometry-chain.json"} else {"percent-chain.json"}),serde_json::to_vec_pretty(&ui.trace).unwrap()).unwrap();
        }
    }
    #[test]
    fn first_sweep_misses_then_returns_to_top_and_second_sweep_finds_button() {
        for geometry_only in [false,true] {
            for page in 0..=2 {
                let mut ui=ViewportUi::new();ui.page=page;ui.hide_first=true;ui.geometry_only=geometry_only;
                assert_eq!(run_viewport(&mut ui)["ok"],true);assert!(ui.rewound);
                let up=ui.trace.iter().position(|e|e["direction"]=="up").unwrap();
                let top=ui.trace[up..].iter().position(|e|e["event"]=="scroll" && e["offset"]==0).unwrap()+up;
                assert!(ui.trace[top+1..].iter().any(|e|e["direction"]=="down"));
                assert!(ui.trace[..top].iter().all(|e|e["event"]!="activate"),"no input while returning to top");
                let output=Path::new(env!("CARGO_MANIFEST_DIR")).join("../artifacts/login-scroll-fixture");
                std::fs::create_dir_all(&output).unwrap();
                std::fs::write(output.join(format!("retry-{page}-{geometry_only}.json")),serde_json::to_vec_pretty(&ui.trace).unwrap()).unwrap();
            }
        }
    }
    #[test]
    fn two_sweeps_missing_or_scroll_limit_report_the_actual_failure() {
        for geometry_only in [false,true] {
            for (page,reason) in [(0,"UIA_LOGIN_CONTROL_NOT_FOUND"),(1,"UIA_AUTH_CONTROL_NOT_FOUND"),(2,"UIA_STADIUM_ENTRY_NOT_FOUND")] {
                let mut ui=ViewportUi::new();ui.page=page;ui.absent=true;ui.geometry_only=geometry_only;
                let result=run_viewport(&mut ui);assert_eq!(result["reason"],reason);assert!(ui.rewound);assert!(ui.clicks.is_empty());
                assert!(!should_reenter(&result));
            }
        }
        let mut ui=ViewportUi::new();ui.absent=true;ui.oversized=true;
        let result=run_viewport(&mut ui);assert_eq!(result["reason"],"UIA_SCROLL_LIMIT_REACHED");
        assert!(!ui.rewound);assert!(ui.clicks.is_empty());
        let mut ui=ViewportUi::new();let time=std::cell::Cell::new(179_000);
        let result=navigate(&mut ui,None,&||true,&mut ||panic!("no H5 reached"),&mut |_|{},&mut |_|{},
            ||Duration::from_millis(time.get()),|ms|{time.set(time.get()+ms);Ok(())}).unwrap();
        assert_eq!(result["reason"],"UIA_SEARCH_TIMEOUT");assert_eq!(result["search_page"],"login");
    }
}
