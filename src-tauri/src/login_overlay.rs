//! Non-activating, click-through status attached only to the verified login window.
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;
use windows::core::w;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::*;

pub(crate) struct LoginOverlay { sender: Option<Sender<&'static str>>, thread: Option<JoinHandle<()>> }
impl LoginOverlay {
    pub(crate) fn start() -> Self {
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

struct PaintState { phase: &'static str, scale: f32 }

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
    let brush = CreateSolidBrush(COLORREF(0x00352a20));
    FillRect(dc, &rect, brush);
    let _ = DeleteObject(brush.into());
    SetBkMode(dc, TRANSPARENT);
    let s = |n: i32| (n as f32 * state.scale).round() as i32;
    text(dc, "正在登录中，请稍候", RECT {left:s(14),top:s(8),right:rect.right-s(12),bottom:s(31)}, s(14), true, COLORREF(0x00ffffff));
    text(dc, state.phase, RECT {left:s(14),top:s(32),right:rect.right-s(12),bottom:s(51)}, s(11), false, COLORREF(0x00e8dac7));
    text(dc, "请暂勿操作小程序", RECT {left:s(14),top:s(52),right:rect.right-s(12),bottom:s(72)}, s(11), false, COLORREF(0x00bcc0c5));
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
    let class = w!("XmuSportsLoginStatus");
    let instance = GetModuleHandleW(None).ok()?;
    let definition = WNDCLASSW { hInstance: instance.into(), lpfnWndProc: Some(proc), lpszClassName: class, ..Default::default() };
    // Registration is process-wide; a previously registered identical class is fine.
    RegisterClassW(&definition);
    let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
                              class, w!("XMU Sports 登录提示"), WS_POPUP, 0, 0, 1, 1, Some(owner), None, Some(instance.into()),
                              Some(state as *mut PaintState as *const _)).ok()?;
    if SetLayeredWindowAttributes(hwnd, COLORREF(0), 248, LWA_ALPHA).is_err() { let _=DestroyWindow(hwnd); return None; }
    Some(hwnd)
}

fn run(receiver: Receiver<&'static str>) {
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut state = Box::new(PaintState { phase: "正在准备登录", scale: 1.0 });
        let mut overlay: Option<(HWND, crate::wechat_window::NativeSurface)> = None;
        loop {
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(phase) => state.phase = phase,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {},
            }
            while let Ok(phase) = receiver.try_recv() {state.phase=phase;}
            let candidates = crate::wechat_window::discover_wechat_candidates().unwrap_or_default();
            let target = if candidates.len()==1 && candidates[0].visible {candidates.first()} else {None};
            if overlay.as_ref().is_some_and(|(hwnd, old)| !IsWindow(Some(*hwnd)).as_bool() ||
                target.is_none_or(|current|current.hwnd_u64!=old.hwnd_u64 || current.owner!=old.owner)) {
                if let Some((hwnd,_))=overlay.take() {let _=DestroyWindow(hwnd);}
            }
            if overlay.is_none() {
                if let Some(target) = target {
                    if let Some(hwnd) = create(target.hwnd(), &mut state) {overlay=Some((hwnd,target.clone()));}
                }
            }
            if let Some((hwnd, target)) = &overlay {
                if GetForegroundWindow()!=target.hwnd() || IsIconic(target.hwnd()).as_bool() {
                    let _ = ShowWindow(*hwnd, SW_HIDE);
                } else {
                    let mut rect=RECT::default();
                    if GetWindowRect(target.hwnd(), &mut rect).is_ok() {
                        state.scale=GetDpiForWindow(target.hwnd()).max(96) as f32/96.0;
                        let s=|n:i32|(n as f32*state.scale).round() as i32;
                        let width=s(300).min(rect.right-rect.left-s(24));
                        if width>=s(220) {
                            let _=SetWindowPos(*hwnd, Some(HWND_TOPMOST), rect.left+(rect.right-rect.left-width)/2,
                                               rect.top+s(44), width, s(80), SWP_NOACTIVATE | SWP_SHOWWINDOW);
                            let region=CreateRoundRectRgn(0,0,width+1,s(80)+1,s(18),s(18));
                            if SetWindowRgn(*hwnd,Some(region),true)==0 {let _=DeleteObject(region.into());}
                            let _=InvalidateRect(Some(*hwnd),None,false);
                        } else {let _=ShowWindow(*hwnd, SW_HIDE);}
                    } else {let _=ShowWindow(*hwnd, SW_HIDE);}
                }
            }
            let mut message=MSG::default();
            while PeekMessageW(&mut message,None,0,0,PM_REMOVE).as_bool() {
                let _=TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if let Some((hwnd,_))=overlay {let _=DestroyWindow(hwnd);}
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
            let mut state=PaintState{phase:"正在核验登录状态",scale:1.0};
            let overlay=create(owner,&mut state).expect("native overlay must be creatable");
            let _=SetWindowPos(overlay,None,-10000,-10000,300,80,SWP_NOACTIVATE);
            assert_eq!(GetForegroundWindow(),before,"overlay cannot steal focus");
            let flags=GetWindowLongPtrW(overlay,GWL_EXSTYLE) as u32;
            assert_eq!(flags & (WS_EX_NOACTIVATE|WS_EX_TRANSPARENT|WS_EX_TOOLWINDOW|WS_EX_LAYERED).0,
                       (WS_EX_NOACTIVATE|WS_EX_TRANSPARENT|WS_EX_TOOLWINDOW|WS_EX_LAYERED).0);
            assert_eq!(SendMessageW(overlay,WM_NCHITTEST,None,None).0,HTTRANSPARENT as isize);
            assert_eq!(SendMessageW(overlay,WM_MOUSEACTIVATE,None,None).0,MA_NOACTIVATE as isize);
            let dc=CreateCompatibleDC(None);
            let info=BITMAPINFO{bmiHeader:BITMAPINFOHEADER{biSize:std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth:300,biHeight:-80,biPlanes:1,biBitCount:32,biCompression:BI_RGB.0,..Default::default()},..Default::default()};
            let mut pixels=std::ptr::null_mut();
            let bitmap=CreateDIBSection(Some(dc),&info,DIB_RGB_COLORS,&mut pixels,None,0).unwrap();
            let previous=SelectObject(dc,bitmap.into());
            SendMessageW(overlay,WM_PRINTCLIENT,Some(WPARAM(dc.0 as usize)),Some(LPARAM(PRF_CLIENT as isize)));
            let _=GdiFlush();
            let bytes=std::slice::from_raw_parts(pixels as *const u8,300*80*4);
            assert!(bytes.chunks_exact(4).any(|pixel|pixel[0]>200 && pixel[1]>200 && pixel[2]>200),"status text must render");
            let mut ppm=b"P6\n300 80\n255\n".to_vec();
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

}
