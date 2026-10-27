//! Non-activating, click-through status attached only to the verified login window.
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::atomic::{AtomicU32,AtomicUsize,Ordering};
use std::sync::Arc;
use std::cell::Cell;
use std::thread::JoinHandle;
use std::time::{Duration,Instant};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook,UnhookWinEvent,HWINEVENTHOOK};
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
static DISCOVERY_SCANS:AtomicUsize=AtomicUsize::new(0);
static LOCATION_EVENTS:AtomicUsize=AtomicUsize::new(0);
static BATCH_FAILURES:AtomicUsize=AtomicUsize::new(0);
pub(crate) fn diagnostics()->serde_json::Value {
    serde_json::json!({"target_samples":TARGET_SAMPLES.load(Ordering::Relaxed),"focused_samples":FOCUSED_SAMPLES.load(Ordering::Relaxed),
        "windows_created":CREATED.load(Ordering::Relaxed),"visible_samples":SHOWN.load(Ordering::Relaxed),"mask_paints":MASK_PAINTS.load(Ordering::Relaxed),
        "panel_paints":PANEL_PAINTS.load(Ordering::Relaxed),"layout_updates":LAYOUT_UPDATES.load(Ordering::Relaxed),"discovery_scans":DISCOVERY_SCANS.load(Ordering::Relaxed),
        "location_events":LOCATION_EVENTS.load(Ordering::Relaxed),"batch_failures":BATCH_FAILURES.load(Ordering::Relaxed)})
}

const WAKE:u32=WM_APP+49;
enum OverlayCommand { Phase(&'static str), Suspend(Sender<()>), Resume }
pub(crate) struct LoginOverlay {
    sender:Option<Sender<OverlayCommand>>, thread:Option<JoinHandle<()>>, thread_id:Arc<AtomicU32>,
}
impl LoginOverlay {
    pub(crate) fn start() -> Self {
        for counter in [&TARGET_SAMPLES,&FOCUSED_SAMPLES,&CREATED,&SHOWN,&MASK_PAINTS,&PANEL_PAINTS,&LAYOUT_UPDATES,&DISCOVERY_SCANS,&LOCATION_EVENTS,&BATCH_FAILURES] {counter.store(0,Ordering::Relaxed);}
        let (sender, receiver) = mpsc::channel();
        let thread_id=Arc::new(AtomicU32::new(0));let id=thread_id.clone();
        let thread=std::thread::Builder::new().name("login-status-overlay".into()).spawn(move||run(receiver,id)).ok();
        Self{sender:Some(sender),thread,thread_id}
    }
    pub(crate) fn phase(&self, phase: &str) {
        if let Some(text) = phase_text(phase) {
            if let Some(sender) = &self.sender { let _ = sender.send(OverlayCommand::Phase(text)); }
            self.wake();
        }
    }
    /// Wait for both layers to hide before capturing the actual mini-program.
    /// A dropped guard restores them even when capture fails.
    pub(crate) fn suspend(&self)->Result<OverlaySuspension<'_>,String> {
        let guard=OverlaySuspension(self);
        let (tx,rx)=mpsc::channel();
        self.sender.as_ref().ok_or("OVERLAY_UNAVAILABLE")?.send(OverlayCommand::Suspend(tx)).map_err(|_|"OVERLAY_UNAVAILABLE")?;
        self.wake();
        rx.recv_timeout(Duration::from_secs(1)).map_err(|_|"OVERLAY_HIDE_TIMEOUT")?;
        Ok(guard)
    }
    fn wake(&self) {
        let id=self.thread_id.load(Ordering::Acquire);
        if id!=0 {unsafe{let _=PostThreadMessageW(id,WAKE,WPARAM(0),LPARAM(0));}}
    }
}
pub(crate) struct OverlaySuspension<'a>(&'a LoginOverlay);
impl Drop for OverlaySuspension<'_> {
    fn drop(&mut self) {
        if let Some(sender)=&self.0.sender {let _=sender.send(OverlayCommand::Resume);self.0.wake();}
    }
}
impl Drop for LoginOverlay {
    fn drop(&mut self) {
        self.sender.take();self.wake();
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
        "normal_ui_refreshing" => "页面加载中，正在重新识别",
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

#[derive(Clone,Copy)]
struct Target { hwnd_u64:u64, pid:u32 }
impl Target { fn hwnd(&self)->HWND {HWND(self.hwnd_u64 as usize as *mut _)} }

thread_local! {
    static WATCH_TARGET:Cell<usize>=const{Cell::new(0)};
    static WATCH_DIRTY:Cell<bool>=const{Cell::new(false)};
}
unsafe extern "system" fn on_window_event(_:HWINEVENTHOOK,event:u32,hwnd:HWND,object:i32,child:i32,_:u32,_:u32) {
    // Out-of-context notification on our message thread: no enumeration or painting here.
    if object==OBJID_WINDOW.0 && child==0 && WATCH_TARGET.with(|target|target.get()==hwnd.0 as usize) {
        WATCH_DIRTY.with(|dirty|dirty.set(true));
        if event==EVENT_OBJECT_LOCATIONCHANGE {LOCATION_EVENTS.fetch_add(1,Ordering::Relaxed);}
    }
}
struct EventWatch { hooks:Vec<HWINEVENTHOOK>, locations:bool }
impl EventWatch {
    unsafe fn new(target:Target)->Self {
        WATCH_TARGET.with(|value|value.set(target.hwnd_u64 as usize));
        WATCH_DIRTY.with(|dirty|dirty.set(true));
        let mut hooks=Vec::new();let mut locations=false;
        for (first,last) in [(EVENT_OBJECT_LOCATIONCHANGE,EVENT_OBJECT_LOCATIONCHANGE),
            (EVENT_OBJECT_SHOW,EVENT_OBJECT_HIDE),(EVENT_OBJECT_DESTROY,EVENT_OBJECT_DESTROY),
            (EVENT_SYSTEM_MINIMIZESTART,EVENT_SYSTEM_MINIMIZEEND)] {
            let hook=SetWinEventHook(first,last,None,Some(on_window_event),target.pid,0,WINEVENT_OUTOFCONTEXT);
            if !hook.0.is_null() {if first==EVENT_OBJECT_LOCATIONCHANGE {locations=true;}hooks.push(hook);}
        }
        Self{hooks,locations}
    }
}
impl Drop for EventWatch {
    fn drop(&mut self) {unsafe {
        WATCH_TARGET.with(|value|value.set(0));
        for hook in self.hooks.drain(..) {let _=UnhookWinEvent(hook);}
    }}
}

struct Layers {
    mask:HWND, panel:HWND, target:Target, _pin:Pin, events:EventWatch,
    geometry:Option<[i32;5]>, phase:Option<&'static str>, shown:bool,
}
impl Layers {
    unsafe fn alive(&self)->bool {
        let mut pid=0;GetWindowThreadProcessId(self.target.hwnd(),Some(&mut pid));
        self._pin.owns() && pid==self.target.pid && IsWindow(Some(self.mask)).as_bool() && IsWindow(Some(self.panel)).as_bool()
    }
    unsafe fn hide(&mut self) {
        if self.shown {let _=ShowWindow(self.panel,SW_HIDE);let _=ShowWindow(self.mask,SW_HIDE);self.shown=false;}
    }
}
impl Drop for Layers {
    fn drop(&mut self) {unsafe {let _=DestroyWindow(self.panel);let _=DestroyWindow(self.mask);}}
}

unsafe fn move_layers(mask:HWND,panel:HWND,rect:RECT,panel_width:i32,panel_height:i32,resized:bool) {
    let width=rect.right-rect.left;let height=rect.bottom-rect.top;
    let flags=SWP_NOZORDER|SWP_NOACTIVATE|SWP_NOOWNERZORDER|if resized {SET_WINDOW_POS_FLAGS(0)} else {SWP_NOSIZE};
    // Both owned popups are desktop-level windows. Commit both positions together.
    let moved=(||->windows::core::Result<()> {
        let mut batch=BeginDeferWindowPos(2)?;
        batch=DeferWindowPos(batch,mask,None,rect.left,rect.top,width,height,flags)?;
        batch=DeferWindowPos(batch,panel,None,rect.left+(width-panel_width)/2,rect.top+(height-panel_height)/2,panel_width,panel_height,flags)?;
        EndDeferWindowPos(batch)
    })();
    if moved.is_err() {
        BATCH_FAILURES.fetch_add(1,Ordering::Relaxed);
        let _=SetWindowPos(mask,None,rect.left,rect.top,width,height,flags);
        let _=SetWindowPos(panel,None,rect.left+(width-panel_width)/2,rect.top+(height-panel_height)/2,panel_width,panel_height,flags);
    }
}

fn run(receiver:Receiver<OverlayCommand>,thread_id:Arc<AtomicU32>) {
    run_with_discovery(receiver,thread_id,|| {
        let candidates=crate::wechat_window::discover_wechat_candidates().ok()?;
        let target=if candidates.len()==1 {candidates.first()?} else {return None;};
        if !target.visible || unsafe{GetForegroundWindow()}!=target.hwnd() {return None;}
        Some(Target{hwnd_u64:target.hwnd_u64,pid:target.owner.pid})
    });
}

fn run_with_discovery(receiver:Receiver<OverlayCommand>,thread_id:Arc<AtomicU32>,mut discover:impl FnMut()->Option<Target>) {
    unsafe {
        let _=SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut message=MSG::default();
        let _=PeekMessageW(&mut message,None,0,0,PM_NOREMOVE); // Create the wake-up queue.
        thread_id.store(GetCurrentThreadId(),Ordering::Release);
        let mut mask_state=Box::new(PaintState{phase:"正在准备登录",scale:1.0,panel:false});
        let mut panel_state=Box::new(PaintState{phase:"正在准备登录",scale:1.0,panel:true});
        let mut layers:Option<Layers>=None;
        let mut suspended=false;
        let mut next_discovery=Instant::now();let mut next_safety=Instant::now();
        'running:loop {
            // Drain notifications first; location bursts coalesce into one latest rectangle.
            while PeekMessageW(&mut message,None,0,0,PM_REMOVE).as_bool() {let _=TranslateMessage(&message);DispatchMessageW(&message);}
            loop {match receiver.try_recv() {
                Ok(OverlayCommand::Phase(phase))=>panel_state.phase=phase,
                Ok(OverlayCommand::Suspend(ack))=> {
                    suspended=true;
                    if let Some(layers)=layers.as_mut() {layers.hide();}
                    let _=ack.send(());
                },
                Ok(OverlayCommand::Resume)=> {suspended=false;WATCH_DIRTY.with(|dirty|dirty.set(true));},
                Err(mpsc::TryRecvError::Empty)=>break,
                Err(mpsc::TryRecvError::Disconnected)=>break 'running,
            }}
            let now=Instant::now();
            if layers.as_ref().is_some_and(|layers|!layers.alive()) {layers.take();next_discovery=now;}
            if layers.is_none() && now>=next_discovery {
                DISCOVERY_SCANS.fetch_add(1,Ordering::Relaxed);next_discovery=now+Duration::from_millis(250);
                if let Some(target)=discover() {
                    if let Some(pin)=Pin::new(target.hwnd()) {
                        if let Some(mask)=create(target.hwnd(),&mut mask_state) {
                            if let Some(panel)=create(mask,&mut panel_state) {
                                CREATED.fetch_add(1,Ordering::Relaxed);
                                layers=Some(Layers{mask,panel,target,_pin:pin,events:EventWatch::new(target),geometry:None,phase:None,shown:false});
                                next_safety=now;
                            } else {let _=DestroyWindow(mask);}
                        }
                    }
                }
            }
            let dirty=WATCH_DIRTY.with(|dirty|dirty.replace(false));
            if let Some(layers)=layers.as_mut() {
                let safety=now>=next_safety;
                if safety {next_safety=now+Duration::from_millis(if layers.events.locations {250} else {16});let _=layers._pin.ensure();}
                if dirty || safety || layers.phase!=Some(panel_state.phase) {
                    TARGET_SAMPLES.fetch_add(1,Ordering::Relaxed);
                    if GetForegroundWindow()==layers.target.hwnd() {FOCUSED_SAMPLES.fetch_add(1,Ordering::Relaxed);}
                    let mut rect=RECT::default();
                    if !suspended && IsWindowVisible(layers.target.hwnd()).as_bool() && !IsIconic(layers.target.hwnd()).as_bool() && GetWindowRect(layers.target.hwnd(),&mut rect).is_ok() {
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
                                move_layers(layers.mask,layers.panel,rect,panel_width,panel_height,resized);
                                if resized {
                                    for (hwnd,w,h,radius) in [(layers.mask,width,height,s(12)),(layers.panel,panel_width,panel_height,s(20))] {
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
            }
            // Native location notifications and phase/stop messages wake this immediately.
            let deadline=if layers.is_some() {next_safety} else {next_discovery};
            let timeout=deadline.saturating_duration_since(Instant::now()).as_millis().min(250) as u32;
            let waited=MsgWaitForMultipleObjectsEx(None,timeout,QS_ALLINPUT,MWMO_INPUTAVAILABLE);
            if waited.0==u32::MAX {std::thread::sleep(Duration::from_millis(16));}
        }
        drop(layers); // Unsubscribe on this thread, destroy our windows and restore the pin.
        thread_id.store(0,Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Native fixtures share desktop Z-order and diagnostic counters.
    static NATIVE_FIXTURE:std::sync::Mutex<()>=std::sync::Mutex::new(());
    #[test]
    fn only_real_login_stages_change_the_message() {
        assert_eq!(phase_text("native_authorizing"),Some("正在等待微信授权"));
        assert_eq!(phase_text("normal_ui_http_verifying"),Some("正在核验登录状态"));
        for phase in ["uia_select","semantic_login_control","submitting", "unknown"] {assert_eq!(phase_text(phase),None);}
    }
    #[test]
    fn native_fixture_does_not_take_focus_and_draws_status() {
        let _serial=NATIVE_FIXTURE.lock().unwrap_or_else(|error|error.into_inner());
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
        let _serial=NATIVE_FIXTURE.lock().unwrap_or_else(|error|error.into_inner());
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

    #[test]
    fn drag_events_follow_without_desktop_scans_or_move_repainting() {
        let _serial=NATIVE_FIXTURE.lock().unwrap_or_else(|error|error.into_inner());
        unsafe {
            let old_dpi=SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let owner=CreateWindowExW(WS_EX_TOOLWINDOW,w!("STATIC"),w!("Isolated drag trajectory"),WS_POPUP,
                -10000,-10000,520,815,None,None,None,None).unwrap();
            let _=ShowWindow(owner,SW_SHOWNOACTIVATE);
            let pump=|| {
                let mut message=MSG::default();
                while PeekMessageW(&mut message,None,0,0,PM_REMOVE).as_bool() {let _=TranslateMessage(&message);DispatchMessageW(&message);}
            };
            let owned=|target:HWND| {
                let mut current=GetTopWindow(None).unwrap_or_default();
                while !current.0.is_null() {
                    if GetWindow(current,GW_OWNER).unwrap_or_default()==target {return current;}
                    current=GetWindow(current,GW_HWNDNEXT).unwrap_or_default();
                }
                HWND::default()
            };
            for counter in [&DISCOVERY_SCANS,&LOCATION_EVENTS,&BATCH_FAILURES,&LAYOUT_UPDATES] {counter.store(0,Ordering::Relaxed);}
            let (sender,receiver)=mpsc::channel();let thread_id=Arc::new(AtomicU32::new(0));
            let id=thread_id.clone();let handle=owner.0 as usize;
            let thread=std::thread::spawn(move||run_with_discovery(receiver,id,||Some(Target{hwnd_u64:handle as u64,pid:std::process::id()})));
            let controller=LoginOverlay{sender:Some(sender),thread:Some(thread),thread_id};
            let checked=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let started=Instant::now();
            let (mask,panel)=loop {
                pump();let mask=owned(owner);let panel=owned(mask);
                if !mask.0.is_null() && !panel.0.is_null() && IsWindowVisible(panel).as_bool() {break (mask,panel);}
                assert!(started.elapsed()<Duration::from_secs(5),"native fixture did not bind");
                std::thread::sleep(Duration::from_millis(1));
            };
            let before_mask=MASK_PAINTS.load(Ordering::Relaxed);let before_panel=PANEL_PAINTS.load(Ordering::Relaxed);
            let mut latencies=Vec::new();
            for step in 1..=32 {
                let x=-10000+step*7;let y=-10000+step*3;
                let sent=Instant::now();
                SetWindowPos(owner,None,x,y,520,815,SWP_NOZORDER|SWP_NOACTIVATE|SWP_NOSIZE).unwrap();
                loop {
                    pump();let mut rect=RECT::default();GetWindowRect(mask,&mut rect).unwrap();
                    if rect.left==x && rect.top==y {break;}
                    assert!(sent.elapsed()<Duration::from_millis(500),"overlay failed to track a native window move");
                    std::thread::sleep(Duration::from_millis(1));
                }
                latencies.push(sent.elapsed().as_micros() as u64);
                let mut mask_rect=RECT::default();let mut panel_rect=RECT::default();
                GetWindowRect(mask,&mut mask_rect).unwrap();GetWindowRect(panel,&mut panel_rect).unwrap();
                assert_eq!((panel_rect.left, panel_rect.top),(mask_rect.left+100,mask_rect.top+351),"both layers move together");
                std::thread::sleep(Duration::from_millis(16));
            }
            latencies.sort_unstable();
            let mask_paints=MASK_PAINTS.load(Ordering::Relaxed)-before_mask;
            let panel_paints=PANEL_PAINTS.load(Ordering::Relaxed)-before_panel;
            assert!(LOCATION_EVENTS.load(Ordering::Relaxed)>=16,"tracking must use actual location notifications");
            assert_eq!(DISCOVERY_SCANS.load(Ordering::Relaxed),1,"bound dragging must not enumerate windows/processes");
            assert_eq!(BATCH_FAILURES.load(Ordering::Relaxed),0,"both top-level layers must batch successfully");
            assert!(latencies[30]<150_000,"native event tracking should beat the old 200ms polling interval");
            assert!(mask_paints<=4 && panel_paints<=4,"pure movement must reuse painted surfaces");
            let report=serde_json::json!({"moves":32,"latency_p50_us":latencies[16],"latency_p95_us":latencies[30],
                "latency_max_us":latencies[31],"discovery_scans":DISCOVERY_SCANS.load(Ordering::Relaxed),
                "location_events":LOCATION_EVENTS.load(Ordering::Relaxed),"mask_move_paints":mask_paints,
                "panel_move_paints":panel_paints,"batch_failures":BATCH_FAILURES.load(Ordering::Relaxed)});
            let output=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../artifacts/sidebar-orders");
            std::fs::create_dir_all(&output).unwrap();std::fs::write(output.join("drag-benchmark.json"),report.to_string()).unwrap();
            println!("drag benchmark: {report}");
            // Capturing hides both layers across safety ticks and restores them on guard drop.
            let mask_value=mask.0 as usize;let panel_value=panel.0 as usize;
            std::thread::scope(|scope| {
                let checker=scope.spawn(|| {
                    let hidden=controller.suspend().expect("overlay must acknowledge capture suspension");
                    for _ in 0..3 {
                        assert!(!IsWindowVisible(HWND(mask_value as *mut _)).as_bool());
                        assert!(!IsWindowVisible(HWND(panel_value as *mut _)).as_bool());
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    drop(hidden);
                });
                while !checker.is_finished() {pump();std::thread::sleep(Duration::from_millis(1));}
                checker.join().unwrap();
            });
            let resumed=Instant::now();
            while !IsWindowVisible(panel).as_bool() {
                pump();assert!(resumed.elapsed()<Duration::from_secs(2));std::thread::sleep(Duration::from_millis(1));
            }
            (mask.0 as usize,panel.0 as usize)
            }));
            // The owner must pump cross-thread window messages while the guard restores it.
            let stopper=std::thread::spawn(move||drop(controller));let stopped=Instant::now();
            while !stopper.is_finished() {pump();assert!(stopped.elapsed()<Duration::from_secs(2));std::thread::sleep(Duration::from_millis(1));}
            stopper.join().unwrap();
            if let Ok((mask,panel))=&checked {
                assert!(!IsWindow(Some(HWND(*mask as *mut _))).as_bool() && !IsWindow(Some(HWND(*panel as *mut _))).as_bool());
            }

            assert_eq!(GetWindowLongPtrW(owner,GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0,0);
            let _=DestroyWindow(owner);let _=SetThreadDpiAwarenessContext(old_dpi);
            if let Err(panic)=checked {std::panic::resume_unwind(panic);}
        }
    }

}
