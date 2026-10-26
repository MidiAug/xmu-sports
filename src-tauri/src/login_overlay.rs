//! Non-activating, click-through status attached only to the verified login window.
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::atomic::{AtomicUsize,Ordering};
use std::thread::JoinHandle;
use std::time::Duration;
use windows::core::w;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::Foundation::{COLORREF, HANDLE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::*;

static TARGET_SAMPLES:AtomicUsize=AtomicUsize::new(0);
static FOCUSED_SAMPLES:AtomicUsize=AtomicUsize::new(0);
static CREATED:AtomicUsize=AtomicUsize::new(0);
static SHOWN:AtomicUsize=AtomicUsize::new(0);
static MASK_PAINTS:AtomicUsize=AtomicUsize::new(0);
static PANEL_PAINTS:AtomicUsize=AtomicUsize::new(0);
static LAYOUT_UPDATES:AtomicUsize=AtomicUsize::new(0);
pub(crate) fn diagnostics()->serde_json::Value {
    serde_json::json!({"target_samples":TARGET_SAMPLES.load(Ordering::Relaxed),"focused_samples":FOCUSED_SAMPLES.load(Ordering::Relaxed),
        "windows_created":CREATED.load(Ordering::Relaxed),"visible_samples":SHOWN.load(Ordering::Relaxed),"mask_paints":MASK_PAINTS.load(Ordering::Relaxed),
        "panel_paints":PANEL_PAINTS.load(Ordering::Relaxed),"layout_updates":LAYOUT_UPDATES.load(Ordering::Relaxed)})
}

pub(crate) struct LoginOverlay { sender: Option<Sender<&'static str>>, thread: Option<JoinHandle<()>> }
impl LoginOverlay {
    pub(crate) fn start() -> Self {
        for counter in [&TARGET_SAMPLES,&FOCUSED_SAMPLES,&CREATED,&SHOWN,&MASK_PAINTS,&PANEL_PAINTS,&LAYOUT_UPDATES] {counter.store(0,Ordering::Relaxed);}
        let (sender, receiver) = mpsc::channel();
        let thread = std::thread::Builder::new().name("login-status-overlay".into()).spawn(move || run(receiver)).ok();
        Self { sender: Some(sender), thread }
    }
    pub(crate) fn phase(&self, phase: &str) {
        if let Some(text) = phase_text(phase) {
            if let Some(sender) = &self.sender { let _ = sender.send(text); }
        }
    }
}
impl Drop for LoginOverlay {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

fn phase_text(phase: &str) -> Option<&'static str> {
    Some(match phase {
        "normal_ui_opening" | "normal_ui_foreground" | "native_login_opening" => "正在打开登录页面",
        "native_agreement" => "正在确认登录协议",
        "native_authorizing" => "正在等待微信授权",
        "entering_stadium" => "正在进入场馆预约",
        "normal_ui_http_verifying" => "正在核验登录状态",
        "normal_ui_http_ready" => "登录已验证，正在完成准备",
        "normal_ui_reenter" => "正在重新打开小程序",
        "closing_miniprogram" | "normal_ui_closed" => "登录已完成，正在收起窗口",
        _ => return None,
    })
}

struct PaintState { phase: &'static str, scale: f32, panel: bool }

unsafe fn text(dc: HDC, value: &str, mut rect: RECT, size: i32, bold: bool, color: COLORREF) {
    let font = CreateFontW(-size, 0, 0, 0, if bold {600} else {400}, 0, 0, 0,
                          DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Microsoft YaHei UI"));
    let previous = SelectObject(dc, font.into());
    SetTextColor(dc, color);
    let mut wide = value.encode_utf16().collect::<Vec<_>>();
    DrawTextW(dc, &mut wide, &mut rect, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX);
    SelectObject(dc, previous);
    let _ = DeleteObject(font.into());
}

unsafe fn paint_status(dc: HDC, rect: RECT, state: &PaintState) {
    if state.panel {PANEL_PAINTS.fetch_add(1,Ordering::Relaxed);} else {MASK_PAINTS.fetch_add(1,Ordering::Relaxed);}
    let background=CreateSolidBrush(if state.panel {COLORREF(0x003b3530)} else {COLORREF(0x001b1917)});
    FillRect(dc,&rect,background);
    let _=DeleteObject(background.into());
    if !state.panel {return;}
    SetBkMode(dc,TRANSPARENT);
    let s=|n:i32|(n as f32*state.scale).round() as i32;
    text(dc,"正在登录中，请稍候",RECT{left:s(18),top:s(15),right:rect.right-s(16),bottom:s(41)},s(15),true,COLORREF(0x00ffffff));
    text(dc,state.phase,RECT{left:s(18),top:s(46),right:rect.right-s(16),bottom:s(67)},s(12),false,COLORREF(0x00e8dac7));
    text(dc,"鼠标可自由移动，请暂勿点击小程序",RECT{left:s(18),top:s(76),right:rect.right-s(16),bottom:s(97)},s(11),false,COLORREF(0x00ccd0d5));
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lp.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PRINTCLIENT => {
            let mut rect=RECT::default();
            let _=GetClientRect(hwnd,&mut rect);
            if let Some(state)=(GetWindowLongPtrW(hwnd,GWLP_USERDATA) as *const PaintState).as_ref() {
                paint_status(HDC(wp.0 as *mut _),rect,state);
            }
            LRESULT(0)
        },
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);
            let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const PaintState;
            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            if let Some(state) = state.as_ref() { paint_status(dc, rect, state); }
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        },
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn create(owner: HWND, state: &mut PaintState) -> Option<HWND> {
    let class = if state.panel {w!("XmuSportsLoginPanel")} else {w!("XmuSportsLoginStatus")};
    let instance = GetModuleHandleW(None).ok()?;
    let definition = WNDCLASSW { hInstance: instance.into(), lpfnWndProc: Some(proc), lpszClassName: class, ..Default::default() };
    // Registration is process-wide; a previously registered identical class is fine.
    RegisterClassW(&definition);
    let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
                              class, w!("XMU Sports 登录提示"), WS_POPUP, 0, 0, 1, 1, Some(owner), None, Some(instance.into()),
                              Some(state as *mut PaintState as *const _)).ok()?;
    if SetLayeredWindowAttributes(hwnd, COLORREF(0), if state.panel {255} else {195}, LWA_ALPHA).is_err() { let _=DestroyWindow(hwnd); return None; }
    Some(hwnd)
}

// A window property follows this exact HWND lifetime and disappears if it is destroyed.
// It prevents a recycled handle or another pin operation from inheriting our restoration.
struct Pin { hwnd:HWND, token:Box<u8>, original_topmost:bool }
impl Pin {
    unsafe fn new(hwnd:HWND)->Option<Self> {
        if !GetPropW(hwnd,w!("XmuSportsLoginPin")).0.is_null() {return None;}
        let token=Box::new(0u8);
        let marker=HANDLE((&*token as *const u8).cast_mut().cast());
        let original_topmost=GetWindowLongPtrW(hwnd,GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 !=0;
        SetPropW(hwnd,w!("XmuSportsLoginPin"),Some(marker)).ok()?;
        let pin=Self{hwnd,token,original_topmost};
        if !pin.ensure() {return None;}
        Some(pin)
    }
    unsafe fn owns(&self)->bool {
        GetPropW(self.hwnd,w!("XmuSportsLoginPin")).0==(&*self.token as *const u8).cast_mut().cast()
    }
    unsafe fn ensure(&self)->bool {
        if !self.owns() {return false;}
        if GetWindowLongPtrW(self.hwnd,GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 !=0 {return true;}
        SetWindowPos(self.hwnd,Some(HWND_TOPMOST),0,0,0,0,
            SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE|SWP_NOOWNERZORDER).is_ok()
    }
}
impl Drop for Pin {
    fn drop(&mut self) {unsafe {
        if self.owns() {
            let _=SetWindowPos(self.hwnd,Some(if self.original_topmost {HWND_TOPMOST} else {HWND_NOTOPMOST}),0,0,0,0,
                SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE|SWP_NOOWNERZORDER);
            let _=RemovePropW(self.hwnd,w!("XmuSportsLoginPin"));
        }
    }}
}

struct Layers {
    mask:HWND, panel:HWND, target:crate::wechat_window::NativeSurface, _pin:Pin,
    geometry:Option<[i32;5]>, phase:Option<&'static str>, shown:bool,
}
impl Layers {
    unsafe fn hide(&mut self) {
        if self.shown {let _=ShowWindow(self.panel,SW_HIDE);let _=ShowWindow(self.mask,SW_HIDE);self.shown=false;}
    }
}
impl Drop for Layers {
    fn drop(&mut self) {unsafe {let _=DestroyWindow(self.panel);let _=DestroyWindow(self.mask);}}
}

fn run(receiver: Receiver<&'static str>) {
    unsafe {
        let _=SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut mask_state=Box::new(PaintState{phase:"正在准备登录",scale:1.0,panel:false});
        let mut panel_state=Box::new(PaintState{phase:"正在准备登录",scale:1.0,panel:true});
        let mut layers:Option<Layers>=None;
        loop {
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(phase)=>panel_state.phase=phase,
                Err(mpsc::RecvTimeoutError::Disconnected)=>break,
                Err(mpsc::RecvTimeoutError::Timeout)=>{},
            }
            while let Ok(phase)=receiver.try_recv() {panel_state.phase=phase;}
            let candidates=crate::wechat_window::discover_wechat_candidates().unwrap_or_default();
            let target=if candidates.len()==1 {candidates.first()} else {None};
            if target.is_some() {TARGET_SAMPLES.fetch_add(1,Ordering::Relaxed);}
            if target.is_some_and(|target|GetForegroundWindow()==target.hwnd()) {FOCUSED_SAMPLES.fetch_add(1,Ordering::Relaxed);}
            if layers.as_ref().is_some_and(|old|!IsWindow(Some(old.mask)).as_bool() || !IsWindow(Some(old.panel)).as_bool() ||
                target.is_none_or(|current|current.hwnd_u64!=old.target.hwnd_u64 || current.owner!=old.target.owner)) {layers.take();}
            if layers.is_none() {
                if let Some(target)=target.filter(|target|target.visible && GetForegroundWindow()==target.hwnd()) {
                    if let Some(pin)=Pin::new(target.hwnd()) {
                        if let Some(mask)=create(target.hwnd(),&mut mask_state) {
                            if let Some(panel)=create(mask,&mut panel_state) {
                                CREATED.fetch_add(1,Ordering::Relaxed);
                                layers=Some(Layers{mask,panel,target:target.clone(),_pin:pin,geometry:None,phase:None,shown:false});
                            } else {let _=DestroyWindow(mask);}
                        }
                    }
                }
            }
            if let Some(layers)=layers.as_mut() {
                let _=layers._pin.ensure();
                let mut rect=RECT::default();
                if !IsIconic(layers.target.hwnd()).as_bool() && GetWindowRect(layers.target.hwnd(),&mut rect).is_ok() {
                    let scale=GetDpiForWindow(layers.target.hwnd()).max(96) as f32/96.0;
                    mask_state.scale=scale;panel_state.scale=scale;
                    let s=|n:i32|(n as f32*scale).round() as i32;
                    let width=rect.right-rect.left;let height=rect.bottom-rect.top;
                    if width>=s(220) && height>=s(150) {
                        let panel_width=s(320).min(width-s(32));let panel_height=s(112).min(height-s(32));
                        let geometry=[rect.left,rect.top,width,height,(scale*96.0) as i32];
                        let changed=layers.geometry!=Some(geometry);
                        let resized=layers.geometry.is_none_or(|previous|previous[2..]!=geometry[2..]);
                        if changed {
                            LAYOUT_UPDATES.fetch_add(1,Ordering::Relaxed);
                            for (hwnd,x,y,w,h,radius) in [
                                (layers.mask,rect.left,rect.top,width,height,s(12)),
                                (layers.panel,rect.left+(width-panel_width)/2,rect.top+(height-panel_height)/2,panel_width,panel_height,s(20)),
                            ] {
                                // Retain Z order: repeatedly raising mask then card causes visible flashing.
                                let _=SetWindowPos(hwnd,None,x,y,w,h,SWP_NOZORDER|SWP_NOACTIVATE|SWP_NOOWNERZORDER);
                                if resized {
                                    let region=CreateRoundRectRgn(0,0,w+1,h+1,radius,radius);
                                    if SetWindowRgn(hwnd,Some(region),false)==0 {let _=DeleteObject(region.into());}
                                    let _=InvalidateRect(Some(hwnd),None,false);
                                }
                            }
                            layers.geometry=Some(geometry);
                        }
                        let phase_changed=layers.phase!=Some(panel_state.phase);
                        if phase_changed {let _=InvalidateRect(Some(layers.panel),None,false);layers.phase=Some(panel_state.phase);}
                        if resized {let _=UpdateWindow(layers.mask);}
                        if resized || phase_changed {let _=UpdateWindow(layers.panel);}
                        if !layers.shown {
                            let _=ShowWindow(layers.mask,SW_SHOWNOACTIVATE);let _=ShowWindow(layers.panel,SW_SHOWNOACTIVATE);
                            layers.shown=true;
                        }
                        if IsWindowVisible(layers.mask).as_bool() && IsWindowVisible(layers.panel).as_bool() {SHOWN.fetch_add(1,Ordering::Relaxed);}
                    } else {layers.hide();}
                } else {layers.hide();}
            }

            let mut message=MSG::default();
            while PeekMessageW(&mut message,None,0,0,PM_REMOVE).as_bool() {let _=TranslateMessage(&message);DispatchMessageW(&message);}
        }
        // Destroy our two windows, then restore the original owner's topmost state.
        drop(layers);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_real_login_stages_change_the_message() {
        assert_eq!(phase_text("native_authorizing"),Some("正在等待微信授权"));
        assert_eq!(phase_text("normal_ui_http_verifying"),Some("正在核验登录状态"));
        for phase in ["uia_select","semantic_login_control","submitting", "unknown"] {assert_eq!(phase_text(phase),None);}
    }
    #[test]
    fn native_fixture_does_not_take_focus_and_draws_status() {
        unsafe {
            let before=GetForegroundWindow();
            let owner=CreateWindowExW(WS_EX_TOOLWINDOW,w!("STATIC"),w!("Isolated login fixture"),WS_POPUP,
                                      -10000,-10000,520,815,None,None,None,None).unwrap();
            let mut state=PaintState{phase:"正在核验登录状态",scale:1.0,panel:false};
            let overlay=create(owner,&mut state).expect("native overlay must be creatable");
            let _=SetWindowPos(overlay,None,-10000,-10000,520,815,SWP_NOACTIVATE);
            assert_eq!(GetForegroundWindow(),before,"overlay cannot steal focus");
            let mut bounds=RECT::default();GetWindowRect(overlay,&mut bounds).unwrap();
            assert_eq!((bounds.right-bounds.left,bounds.bottom-bounds.top),(520,815),"mask covers the full mini-program dimensions");
            let flags=GetWindowLongPtrW(overlay,GWL_EXSTYLE) as u32;
            assert_eq!(flags & (WS_EX_NOACTIVATE|WS_EX_TRANSPARENT|WS_EX_TOOLWINDOW|WS_EX_LAYERED).0,
                       (WS_EX_NOACTIVATE|WS_EX_TRANSPARENT|WS_EX_TOOLWINDOW|WS_EX_LAYERED).0);
            assert_eq!(SendMessageW(overlay,WM_NCHITTEST,None,None).0,HTTRANSPARENT as isize);
            assert_eq!(SendMessageW(overlay,WM_MOUSEACTIVATE,None,None).0,MA_NOACTIVATE as isize);
            let dc=CreateCompatibleDC(None);
            let info=BITMAPINFO{bmiHeader:BITMAPINFOHEADER{biSize:std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth:520,biHeight:-815,biPlanes:1,biBitCount:32,biCompression:BI_RGB.0,..Default::default()},..Default::default()};
            let mut pixels=std::ptr::null_mut();
            let bitmap=CreateDIBSection(Some(dc),&info,DIB_RGB_COLORS,&mut pixels,None,0).unwrap();
            let previous=SelectObject(dc,bitmap.into());
            SendMessageW(overlay,WM_PRINTCLIENT,Some(WPARAM(dc.0 as usize)),Some(LPARAM(PRF_CLIENT as isize)));
            let mut panel_state=PaintState{phase:"正在核验登录状态",scale:1.0,panel:true};
            let panel=create(overlay,&mut panel_state).unwrap();
            let _=SetWindowPos(panel,None,-10000,-10000,320,112,SWP_NOACTIVATE);
            let mut panel_alpha=0;GetLayeredWindowAttributes(panel,None,Some(&mut panel_alpha),None).unwrap();
            assert_eq!(panel_alpha,255,"status card must be opaque independently of its mask");
            let _=SetViewportOrgEx(dc,100,351,None);
            SendMessageW(panel,WM_PRINTCLIENT,Some(WPARAM(dc.0 as usize)),Some(LPARAM(PRF_CLIENT as isize)));
            let _=SetViewportOrgEx(dc,0,0,None);
            let _=DestroyWindow(panel);
            let _=GdiFlush();
            let bytes=std::slice::from_raw_parts(pixels as *const u8,520*815*4);
            assert!(bytes.chunks_exact(4).any(|pixel|pixel[0]>200 && pixel[1]>200 && pixel[2]>200),"status text must render");
            let mut ppm=b"P6\n520 815\n255\n".to_vec();
            for pixel in bytes.chunks_exact(4) {ppm.extend_from_slice(&[pixel[2],pixel[1],pixel[0]]);}
            let output=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../artifacts/sidebar-orders");
            std::fs::create_dir_all(&output).unwrap();
            std::fs::write(output.join("login-overlay.ppm"),ppm).unwrap();
            SelectObject(dc,previous);
            let _=DeleteObject(bitmap.into());
            let _=DeleteDC(dc);
            let _=DestroyWindow(overlay);
            let _=DestroyWindow(owner);
            assert!(!IsWindow(Some(overlay)).as_bool());
        }
    }

    #[test]
    fn pin_preserves_original_state_and_outlives_other_normal_windows() {
        unsafe {
            let owner=CreateWindowExW(WS_EX_TOOLWINDOW,w!("STATIC"),w!("Isolated pin fixture"),WS_POPUP,
                -10000,-10000,520,815,None,None,None,None).unwrap();
            let other=CreateWindowExW(WS_EX_TOOLWINDOW,w!("STATIC"),w!("Ordinary moving window"),WS_POPUP,
                -10000,-10000,600,850,None,None,None,None).unwrap();
            let before=GetForegroundWindow();
            let is_top=||GetWindowLongPtrW(owner,GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 !=0;
            assert!(!is_top());
            {
                let pin=Pin::new(owner).unwrap();assert!(is_top());
                for offset in [20,0,35] {
                    SetWindowPos(other,Some(HWND_TOP),-10000+offset,-10000,600,850,SWP_NOACTIVATE).unwrap();
                    assert!(pin.ensure());assert!(is_top());
                    let mut previous=GetWindow(other,GW_HWNDPREV).unwrap_or_default();
                    let mut above=false;
                    while !previous.0.is_null() {if previous==owner {above=true;break;}previous=GetWindow(previous,GW_HWNDPREV).unwrap_or_default();}
                    assert!(above,"ordinary windows stay below pinned owner");
                }
            }
            assert!(!is_top(),"temporary pin must be restored");
            SetWindowPos(owner,Some(HWND_TOPMOST),0,0,0,0,SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE).unwrap();
            {let _pin=Pin::new(owner).unwrap();}
            assert!(is_top(),"an originally topmost owner must remain topmost");
            assert_eq!(GetForegroundWindow(),before,"pinning never steals focus");
            let _=DestroyWindow(other);let _=DestroyWindow(owner);
        }
    }

}
