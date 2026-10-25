//! Small Win32 window/protocol adapter. No capture, OCR, vision packs or hooks.
use std::path::Path;
use windows::core::{w, PCWSTR, BOOL};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, TRUE};
use windows::Win32::System::ProcessStatus::GetModuleFileNameExW;
use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_MENU};

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
    if GetWindowRect(hwnd,&mut rect).is_err() {return TRUE;}
    let area=(rect.right-rect.left) as i64*(rect.bottom-rect.top) as i64;
    if area<6400 {return TRUE;}
    out.push(NativeSurface{hwnd_u64:hwnd.0 as usize as u64,title,
        owner:OwnerIdentity{pid,app_identity,launch_identity:format!("{}:{}",creation.dwHighDateTime,creation.dwLowDateTime)},
        visible:!IsIconic(hwnd).as_bool(),area});
    TRUE
}

pub fn discover_wechat_candidates()->Result<Vec<NativeSurface>,String> {
    let mut surfaces:Vec<NativeSurface>=Vec::new();
    unsafe{EnumWindows(Some(collect),LPARAM(&mut surfaces as *mut Vec<NativeSurface> as isize))}.map_err(|_|"WECHAT_WINDOW_DISCOVERY_FAILED")?;
    surfaces.sort_by_key(|v|-v.area);
    Ok(surfaces)
}

pub fn foreground_miniprogram_window()->Result<String,String> {
    let surfaces=discover_wechat_candidates()?;
    if surfaces.len()!=1 {return Err("UIA_TARGET_AMBIGUOUS_OR_MISSING".into());}
    let target=&surfaces[0];
    foreground_surface(target)?;
    Ok(target.title.clone())
}

/// Rebind identity before focusing; a UIA session must never focus a replacement window.
pub(crate) fn foreground_surface(target:&NativeSurface)->Result<(),String> {
    let surfaces=discover_wechat_candidates()?;
    if surfaces.len()!=1 || surfaces[0].hwnd_u64!=target.hwnd_u64 || surfaces[0].owner!=target.owner {
        return Err("UIA_WINDOW_CHANGED".into());
    }
    unsafe {
        if IsIconic(target.hwnd()).as_bool() {let _=ShowWindow(target.hwnd(),SW_RESTORE);}
        let _=BringWindowToTop(target.hwnd());
        if !SetForegroundWindow(target.hwnd()).as_bool() {
            keybd_event(VK_MENU.0 as u8,0,Default::default(),0);
            let _=SetForegroundWindow(target.hwnd());
            keybd_event(VK_MENU.0 as u8,0,KEYEVENTF_KEYUP,0);
        }
        if GetForegroundWindow()!=target.hwnd() {return Err("LOGIN_FOCUS_REQUIRED".into());}
    }
    Ok(())
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
        let mut rect=RECT::default();let _=GetWindowRect(target,&mut rect);
        let point=windows::Win32::Foundation::POINT{x:rect.left+(rect.right-rect.left)/2,y:rect.top+(rect.bottom-rect.top)/2};
        serde_json::json!({"target_count":1,"minimized":IsIconic(target).as_bool(),
            "foreground":GetForegroundWindow()==target,"covered_at_center":GetAncestor(WindowFromPoint(point),GA_ROOT)!=target})
    }
}

pub fn device_diagnostics()->serde_json::Value {
    serde_json::json!({"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
        "edition":"http-only","login":"windows_uia","visual_components":false,"kernel_hook":false})
}
