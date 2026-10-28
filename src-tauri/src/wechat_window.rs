//! Small Win32 window/protocol adapter. Login screenshots live in login_evidence.
use std::path::Path;
use windows::core::{w, PCWSTR, BOOL};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, TRUE, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW,MonitorFromWindow,MONITORINFO,MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::HiDpi::{GetDpiForWindow,GetSystemMetricsForDpi,SetThreadDpiAwarenessContext,DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,DPI_AWARENESS_CONTEXT};
use windows::Win32::System::ProcessStatus::GetModuleFileNameExW;
use windows::Win32::System::Threading::{AttachThreadInput,GetCurrentThreadId,GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ};
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone,PartialEq)]
pub struct OwnerIdentity { pub pid:u32, pub app_identity:String, pub launch_identity:String }
#[derive(Clone)]
pub struct NativeSurface { pub hwnd_u64:u64, pub title:String, pub owner:OwnerIdentity, pub visible:bool, area:i64 }
impl NativeSurface { pub fn hwnd(&self)->HWND {HWND(self.hwnd_u64 as usize as *mut _)} }
pub fn is_miniprogram_title(title:&str)->bool {title.contains("厦大体育") && !title.contains("预约助手")}

unsafe extern "system" fn collect(hwnd:HWND, data:LPARAM)->BOOL {
    let out=&mut *(data.0 as *mut Vec<NativeSurface>);
    if !IsWindowVisible(hwnd).as_bool() {return TRUE;}
    let mut buf=[0u16;512];
    let n=GetWindowTextW(hwnd,&mut buf);
    if n<=0 {return TRUE;}
    let title=String::from_utf16_lossy(&buf[..n as usize]);
    if !is_miniprogram_title(&title) {return TRUE;}
    let mut pid=0;
    GetWindowThreadProcessId(hwnd,Some(&mut pid));
    if pid==0 || pid==std::process::id() {return TRUE;}
    let Ok(handle)=OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_VM_READ,false,pid) else{return TRUE;};
    let mut image=[0u16;2048];
    let count=GetModuleFileNameExW(Some(handle),None,&mut image);
    let app_identity=String::from_utf16_lossy(&image[..count as usize]);
    let mut creation=Default::default(); let mut exit=Default::default();
    let mut kernel=Default::default(); let mut user=Default::default();
    let timing=GetProcessTimes(handle,&mut creation,&mut exit,&mut kernel,&mut user);
    let _=CloseHandle(handle);
    if !Path::new(&app_identity).file_name().is_some_and(|v|v.to_string_lossy().eq_ignore_ascii_case("WeChatAppEx.exe")) || timing.is_err() {return TRUE;}
    let mut rect=RECT::default();
    let minimized=IsIconic(hwnd).as_bool();
    if minimized {
        // A minimized window's current rectangle is only a tiny taskbar-sized
        // box. Use its restored bounds so discovery can restore this HWND.
        let mut placement=WINDOWPLACEMENT{length:std::mem::size_of::<WINDOWPLACEMENT>() as u32,..Default::default()};
        if GetWindowPlacement(hwnd,&mut placement).is_err() {return TRUE;}
        rect=placement.rcNormalPosition;
    } else if GetWindowRect(hwnd,&mut rect).is_err() {return TRUE;}
    let area=(rect.right-rect.left) as i64*(rect.bottom-rect.top) as i64;
    if area<6400 {return TRUE;}
    out.push(NativeSurface{hwnd_u64:hwnd.0 as usize as u64,title,
        owner:OwnerIdentity{pid,app_identity,launch_identity:format!("{}:{}",creation.dwHighDateTime,creation.dwLowDateTime)},
        visible:!minimized,area});
    TRUE
}

pub fn discover_wechat_candidates()->Result<Vec<NativeSurface>,String> {
    let mut surfaces:Vec<NativeSurface>=Vec::new();
    unsafe{EnumWindows(Some(collect),LPARAM(&mut surfaces as *mut Vec<NativeSurface> as isize))}.map_err(|_|"WECHAT_WINDOW_DISCOVERY_FAILED")?;
    surfaces.sort_by_key(|v|-v.area);
    Ok(surfaces)
}

pub fn foreground_miniprogram_window(active:&dyn Fn()->bool)->Result<String,String> {
    let surfaces=discover_wechat_candidates()?;
    if surfaces.len()!=1 {return Err("UIA_TARGET_AMBIGUOUS_OR_MISSING".into());}
    let target=&surfaces[0];
    foreground_surface(target,active)?;
    Ok(target.title.clone())
}

struct WindowDpi(DPI_AWARENESS_CONTEXT);
impl Drop for WindowDpi {fn drop(&mut self) {if !self.0.0.is_null() {unsafe {let _=SetThreadDpiAwarenessContext(self.0);}}}}

fn portrait_bounds(work:RECT,current:RECT,min_width:i32,min_height:i32,max_height:i32)->Result<RECT,String> {
    let width=min_width;let height=(work.bottom-work.top).min(if max_height>0 {max_height} else {i32::MAX});
    if width<=0 || height<min_height || width>=height || width>work.right-work.left {return Err("UIA_PORTRAIT_UNAVAILABLE".into());}
    let left=current.left.clamp(work.left,work.right-width);
    Ok(RECT{left,top:work.top,right:left+width,bottom:work.top+height})
}

pub(crate) fn prepare_portrait(active:&dyn Fn()->bool)->Result<serde_json::Value,String> {
    let targets=discover_wechat_candidates()?;
    if targets.len()!=1 {return Err("UIA_TARGET_AMBIGUOUS_OR_MISSING".into());}
    let started=std::time::Instant::now();let mut stability=LayoutStability::default();let mut samples=Vec::new();
    for _ in 0..40 {
        let mut layout=portrait_surface(&targets[0],active)?;
        let before=(layout["before"]["width"].as_i64().unwrap_or(0),layout["before"]["height"].as_i64().unwrap_or(0));
        let after=(layout["after"]["width"].as_i64().unwrap_or(0),layout["after"]["height"].as_i64().unwrap_or(0));
        if samples.is_empty() || before!=after {samples.push(layout.clone());}
        if stability.ready(started.elapsed(),before,after) {
            layout["startup_samples"]=serde_json::json!(samples);return Ok(layout);
        }
        for _ in 0..5 {
            if !active() {return Err("已取消".into());}
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    Err("UIA_WINDOW_LAYOUT_UNSTABLE".into())
}

#[derive(Default)]
struct LayoutStability {last:Option<(i64,i64)>,since:Option<std::time::Duration>}
impl LayoutStability {
    fn ready(&mut self,now:std::time::Duration,before:(i64,i64),after:(i64,i64))->bool {
        if self.last!=Some(after) || before!=after {self.since=Some(now);}
        self.last=Some(after);
        now>=std::time::Duration::from_millis(1200) &&
            now.saturating_sub(*self.since.get_or_insert(now))>=std::time::Duration::from_millis(400)
    }
}

/// Normalize only this verified mini-program, in physical pixels on its own monitor.
pub(crate) fn portrait_surface(target:&NativeSurface,active:&dyn Fn()->bool)->Result<serde_json::Value,String> {
    if !active() {return Err("已取消".into());}
    let current=discover_wechat_candidates()?;
    if current.len()!=1 || current[0].hwnd_u64!=target.hwnd_u64 || current[0].owner!=target.owner {return Err("UIA_WINDOW_CHANGED".into());}
    unsafe {resize_portrait(target.hwnd(),active)}
}

unsafe fn resize_portrait(hwnd:HWND,active:&dyn Fn()->bool)->Result<serde_json::Value,String> {
    let _dpi=WindowDpi(SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2));
    let mut monitor=MONITORINFO{cbSize:std::mem::size_of::<MONITORINFO>() as u32,..Default::default()};
    if !GetMonitorInfoW(MonitorFromWindow(hwnd,MONITOR_DEFAULTTONEAREST),&mut monitor).as_bool() {return Err("UIA_WINDOW_LAYOUT_UNAVAILABLE".into());}
    if !active() {return Err("已取消".into());}
    if IsIconic(hwnd).as_bool() || IsZoomed(hwnd).as_bool() {let _=ShowWindow(hwnd,SW_RESTORE);}
    let mut before=RECT::default();GetWindowRect(hwnd,&mut before).map_err(|_|"UIA_WINDOW_LAYOUT_UNAVAILABLE")?;
    let dpi=GetDpiForWindow(hwnd).max(96);let scaled=|n:i32|((n as i64*dpi as i64+48)/96) as i32;
    let mut limits=MINMAXINFO::default();let mut reply=0usize;
    if SendMessageTimeoutW(hwnd,WM_GETMINMAXINFO,WPARAM(0),LPARAM((&mut limits as *mut MINMAXINFO) as isize),
        SMTO_ABORTIFHUNG|SMTO_BLOCK,500,Some(&mut reply)).0==0 {return Err("UIA_WINDOW_LAYOUT_UNAVAILABLE".into());}
    // DefWindowProc's generic minimum is not a usable mini-program viewport.
    let generic_min=GetSystemMetricsForDpi(SM_CXMINTRACK,dpi);
    let fallback=limits.ptMinTrackSize.x<=generic_min || limits.ptMinTrackSize.x<scaled(240);
    let min_width=if fallback {scaled(360)} else {limits.ptMinTrackSize.x};
    let desired=portrait_bounds(monitor.rcWork,before,min_width,limits.ptMinTrackSize.y,limits.ptMaxTrackSize.y)?;
    if !active() {return Err("已取消".into());}
    let changed=(before.left,before.top,before.right,before.bottom)!=(desired.left,desired.top,desired.right,desired.bottom);
    if changed {SetWindowPos(hwnd,None,desired.left,desired.top,desired.right-desired.left,desired.bottom-desired.top,
        SWP_NOACTIVATE|SWP_NOZORDER|SWP_NOOWNERZORDER).map_err(|_|"UIA_WINDOW_LAYOUT_UNAVAILABLE")?;}
    let mut actual=RECT::default();GetWindowRect(hwnd,&mut actual).map_err(|_|"UIA_WINDOW_LAYOUT_UNAVAILABLE")?;
    if actual.right-actual.left>=actual.bottom-actual.top || actual.left<monitor.rcWork.left || actual.right>monitor.rcWork.right
        || actual.top<monitor.rcWork.top || actual.bottom>monitor.rcWork.bottom {return Err("UIA_PORTRAIT_UNAVAILABLE".into());}
    Ok(serde_json::json!({"changed":changed,"dpi":dpi,"minimum_width":limits.ptMinTrackSize.x,"fallback_width":fallback,
        "before":{"width":before.right-before.left,"height":before.bottom-before.top},
        "after":{"width":actual.right-actual.left,"height":actual.bottom-actual.top},
        "work_area":{"width":monitor.rcWork.right-monitor.rcWork.left,"height":monitor.rcWork.bottom-monitor.rcWork.top}}))
}

/// Rebind identity before focusing; a UIA session must never focus a replacement window.
struct InputAttachment {from:u32,threads:Vec<u32>}
impl Drop for InputAttachment {fn drop(&mut self) {unsafe {for thread in self.threads.iter().rev() {let _=AttachThreadInput(self.from,*thread,false);}}}}
unsafe fn activate_target(hwnd:HWND) {
    if SetForegroundWindow(hwnd).as_bool() {return;}
    // Share only the foreground/target queues for this activation, then detach.
    // No desktop keystrokes are injected into whatever the user is typing in.
    let mut message=MSG::default();let _=PeekMessageW(&mut message,None,0,0,PM_NOREMOVE);
    let from=GetCurrentThreadId();let mut attached=InputAttachment{from,threads:Vec::new()};
    for thread in [GetWindowThreadProcessId(GetForegroundWindow(),None),GetWindowThreadProcessId(hwnd,None)] {
        if thread!=0 && thread!=from && !attached.threads.contains(&thread) && AttachThreadInput(from,thread,true).as_bool() {attached.threads.push(thread);}
    }
    let _=BringWindowToTop(hwnd);let _=SetForegroundWindow(hwnd);
}
fn focus_settled(since:&mut Option<std::time::Duration>,now:std::time::Duration,ready:bool)->bool {
    if !ready {*since=None;return false;}
    now.saturating_sub(*since.get_or_insert(now))>=std::time::Duration::from_millis(160)
}

pub(crate) fn foreground_surface(target:&NativeSurface,active:&dyn Fn()->bool)->Result<(),String> {
    if !active() {return Err("已取消".into());}
    let surfaces=discover_wechat_candidates()?;
    if surfaces.len()!=1 || surfaces[0].hwnd_u64!=target.hwnd_u64 || surfaces[0].owner!=target.owner {
        return Err("UIA_WINDOW_CHANGED".into());
    }
    unsafe {
        if IsIconic(target.hwnd()).as_bool() {let _=ShowWindow(target.hwnd(),SW_RESTORE);}
        let started=std::time::Instant::now();let mut stable_since=None;
        for attempt in 0..40 {
            if !active() {return Err("已取消".into());}
            let mut pid=0;GetWindowThreadProcessId(target.hwnd(),Some(&mut pid));
            if !IsWindow(Some(target.hwnd())).as_bool() || pid!=target.owner.pid {return Err("UIA_WINDOW_CHANGED".into());}
            let mut rect=RECT::default();GetWindowRect(target.hwnd(),&mut rect).map_err(|_|"UIA_WINDOW_CHANGED")?;
            let point=windows::Win32::Foundation::POINT{x:rect.left+(rect.right-rect.left)/2,y:rect.top+(rect.bottom-rect.top)/2};
            let ready=GetForegroundWindow()==target.hwnd() && crate::login_overlay::target_uncovered_at(point,target.hwnd());
            if focus_settled(&mut stable_since,started.elapsed(),ready) {return Ok(());}
            if !ready || attempt==0 {
                // The overlay owns restoration of the original topmost state.
                let pinned=!GetPropW(target.hwnd(),w!("XmuSportsLoginPin")).0.is_null();
                SetWindowPos(target.hwnd(),Some(if pinned {HWND_TOPMOST} else {HWND_TOP}),0,0,0,0,
                    SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE|SWP_NOOWNERZORDER).map_err(|_|"LOGIN_FOCUS_REQUIRED")?;
                activate_target(target.hwnd());
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
    }
    Err("LOGIN_FOCUS_REQUIRED".into())
}

pub fn launch_xmu_miniprogram(_app_id:Option<&str>)->Result<String,String> {
    launch_with_show(SW_SHOWNORMAL)
}

fn launch_with_show(show:SHOW_WINDOW_CMD)->Result<String,String> {
    let url="weixin://launchapplet/?app_id=wx81a2b2fa90759cb7";
    let wide:Vec<u16>=url.encode_utf16().chain(Some(0)).collect();
    let result=unsafe{windows::Win32::UI::Shell::ShellExecuteW(None,w!("open"),PCWSTR(wide.as_ptr()),None,None,show)};
    if result.0 as isize<=32 {return Err("MINIPROGRAM_LAUNCH_FAILED".into());}
    Ok(url.into())
}

pub fn background_status()->serde_json::Value {
    let targets=discover_wechat_candidates().unwrap_or_default();
    if targets.len()!=1 {return serde_json::json!({"target_count":targets.len()});}
    let target=targets[0].hwnd();
    unsafe {
        let _dpi=WindowDpi(SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2));
        let mut rect=RECT::default();let _=GetWindowRect(target,&mut rect);
        let point=windows::Win32::Foundation::POINT{x:rect.left+(rect.right-rect.left)/2,y:rect.top+(rect.bottom-rect.top)/2};
        serde_json::json!({"target_count":1,"minimized":IsIconic(target).as_bool(),"maximized":IsZoomed(target).as_bool(),
            "width":rect.right-rect.left,"height":rect.bottom-rect.top,"dpi":GetDpiForWindow(target),
            "foreground":GetForegroundWindow()==target,"covered_at_center":!crate::login_overlay::target_uncovered_at(point,target)})
    }
}

pub fn device_diagnostics()->serde_json::Value {
    serde_json::json!({"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
        "edition":"http-only","login":"windows_uia","visual_components":false,"kernel_hook":false,
        "environment":crate::login_evidence::environment()})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_landscape_rebound_resets_portrait_settling() {
        use std::time::Duration;
        let mut state=LayoutStability::default();
        assert!(!state.ready(Duration::ZERO,(1450,414),(360,1032)));
        assert!(!state.ready(Duration::from_millis(300),(1450,414),(528,1032)));
        assert!(!state.ready(Duration::from_millis(700),(528,1032),(528,1032)));
        assert!(state.ready(Duration::from_millis(1200),(528,1032),(528,1032)));
        assert!(!state.ready(Duration::from_millis(1300),(1450,414),(528,1032)));
        assert!(state.ready(Duration::from_millis(1700),(528,1032),(528,1032)));
    }
    #[test]
    fn transient_foreground_and_later_occlusion_do_not_count_as_ready() {
        use std::time::Duration;
        let mut since=None;
        for ms in [0,40,80,120] {assert!(!focus_settled(&mut since,Duration::from_millis(ms),true));}
        assert!(!focus_settled(&mut since,Duration::from_millis(140),false));
        assert!(!focus_settled(&mut since,Duration::from_millis(180),true));
        assert!(!focus_settled(&mut since,Duration::from_millis(339),true));
        assert!(focus_settled(&mut since,Duration::from_millis(340),true));
        assert!(!focus_settled(&mut since,Duration::from_millis(400),false));
    }
    #[test]
    fn portrait_uses_work_area_minimum_width_and_application_height_limits() {
        for (work,min_width) in [(RECT{left:0,top:0,right:1920,bottom:1040},360),
            (RECT{left:-2560,top:-100,right:0,bottom:1340},540),
            (RECT{left:0,top:-2160,right:3840,bottom:-80},720)] {
            let current=RECT{left:work.right-100,top:work.top+200,right:work.right+900,bottom:work.top+600};
            let result=portrait_bounds(work,current,min_width,200,0).unwrap();
            assert_eq!(result.right-result.left,min_width);assert_eq!(result.top,work.top);assert_eq!(result.bottom,work.bottom);
            assert!(result.left>=work.left && result.right<=work.right);
        }
        let work=RECT{left:0,top:0,right:1920,bottom:1040};
        assert_eq!(portrait_bounds(work,work,360,200,800).unwrap().bottom,800);
        assert_eq!(portrait_bounds(work,work,1100,200,0).unwrap_err(),"UIA_PORTRAIT_UNAVAILABLE");
        assert!(portrait_bounds(work,work,360,1200,0).is_err());
    }
    unsafe extern "system" fn constrained(hwnd:HWND,msg:u32,wp:WPARAM,lp:LPARAM)->windows::Win32::Foundation::LRESULT {
        if msg==WM_GETMINMAXINFO {
            let limits=&mut *(lp.0 as *mut MINMAXINFO);let dpi=GetDpiForWindow(hwnd).max(96) as i32;
            limits.ptMinTrackSize.x=300*dpi/96;limits.ptMinTrackSize.y=200*dpi/96;limits.ptMaxTrackSize.y=900*dpi/96;
            return windows::Win32::Foundation::LRESULT(0);
        }
        DefWindowProcW(hwnd,msg,wp,lp)
    }
    #[test]
    fn native_portrait_honors_minimum_and_does_not_activate_or_resize_after_cancel() {
        unsafe {
            let _dpi=WindowDpi(SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2));
            let class=w!("XmuPortraitConstraintsFixture");
            RegisterClassW(&WNDCLASSW{lpfnWndProc:Some(constrained),lpszClassName:class,..Default::default()});
            let hwnd=CreateWindowExW(WS_EX_TOOLWINDOW,class,w!("Isolated portrait fixture"),WS_POPUP,
                -10000,-10000,1200,400,None,None,None,None).unwrap();
            let foreground=GetForegroundWindow();
            let report=resize_portrait(hwnd,&||true).unwrap();
            assert_eq!(report["fallback_width"],false);
            assert_eq!(report["after"]["width"],300*GetDpiForWindow(hwnd).max(96)/96);
            assert!(report["after"]["height"].as_i64().unwrap()>report["after"]["width"].as_i64().unwrap());
            assert_eq!(GetForegroundWindow(),foreground);assert!(!IsWindowVisible(hwnd).as_bool());
            let mut before=RECT::default();GetWindowRect(hwnd,&mut before).unwrap();
            assert_eq!(resize_portrait(hwnd,&||false).unwrap_err(),"已取消");
            let mut after=RECT::default();GetWindowRect(hwnd,&mut after).unwrap();
            assert_eq!((before.left,before.top,before.right,before.bottom),(after.left,after.top,after.right,after.bottom));
            DestroyWindow(hwnd).unwrap();
        }
    }
}
