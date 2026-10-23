//! Window discovery with bindable identity (HWND + PID + launch + image path).

use vision_contracts::{OwnerIdentity, RectF, SizeI, SurfaceCandidate, TargetSurfaceLease};
use vision_platform_api::ActivationStatus;

#[cfg(windows)]
mod win {
    use super::*;
    use std::collections::HashMap;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, TRUE};
    use windows::Win32::System::ProcessStatus::GetModuleFileNameExW;
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_MENU};
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowRect,
        GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
        SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    #[derive(Clone)]
    pub struct NativeSurface {
        /// HWND stored as usize so the adapter stays `Send`.
        pub hwnd_usize: usize,
        pub hwnd_u64: u64,
        pub title: String,
        #[allow(dead_code)] // retained for diagnostics / future class-based filters
        pub class_name: String,
        pub owner: OwnerIdentity,
        pub bounds: RectF,
        pub scale_x: f64,
        pub scale_y: f64,
        pub surface_fingerprint: String,
        pub visible: bool,
        pub area: i64,
    }

    impl NativeSurface {
        pub fn hwnd(&self) -> HWND {
            HWND(self.hwnd_usize as *mut _)
        }

        pub fn to_candidate(&self) -> SurfaceCandidate {
            SurfaceCandidate {
                candidate_id: format!("win-{}", self.hwnd_u64),
                title_hint: self.title.clone(),
                owner: self.owner.clone(),
                bounds_in_screen_points: self.bounds.clone(),
                surface_fingerprint: self.surface_fingerprint.clone(),
            }
        }

        pub fn to_lease(&self, generation: u64) -> TargetSurfaceLease {
            let w = self.bounds.width.max(1.0) as u32;
            let h = self.bounds.height.max(1.0) as u32;
            TargetSurfaceLease {
                lease_id: format!("lease-win-{}-g{}", self.hwnd_u64, generation),
                generation,
                platform: vision_contracts::PlatformKind::Windows,
                owner: self.owner.clone(),
                surface_fingerprint: self.surface_fingerprint.clone(),
                title_hint: self.title.clone(),
                bounds_in_screen_points: self.bounds.clone(),
                capture_pixel_size: SizeI {
                    width: ((w as f64) * self.scale_x).round().max(1.0) as u32,
                    height: ((h as f64) * self.scale_y).round().max(1.0) as u32,
                },
                scale_x: self.scale_x,
                scale_y: self.scale_y,
                issued_at: String::new(),
                expires_at: String::new(),
            }
            .with_default_ttl()
        }
    }

    fn window_dpi_scale(hwnd: HWND) -> (f64, f64) {
        use windows::Win32::UI::HiDpi::GetDpiForWindow;
        unsafe {
            let dpi = GetDpiForWindow(hwnd);
            if dpi == 0 {
                return (1.0, 1.0);
            }
            let s = (dpi as f64) / 96.0;
            (s, s)
        }
    }

    pub fn title_is_wechat(title: &str) -> bool {
        let t = title.trim();
        if t == "微信"
            || t.starts_with("微信 ")
            || t.starts_with("微信(")
            || t.starts_with("微信（")
            || t.starts_with("微信[")
        {
            return true;
        }
        let lower = t.to_lowercase();
        lower == "wechat"
            || lower == "weixin"
            || lower.starts_with("wechat ")
            || lower.starts_with("weixin ")
            || lower.starts_with("wechat(")
            || lower.starts_with("weixin(")
    }

    /// 本程序窗口标题也含「厦大体育」，不能当成小程序壳。
    pub fn title_is_host_app(title: &str) -> bool {
        title.contains("预约助手")
    }

    pub fn title_is_miniprogram(title: &str) -> bool {
        if title_is_host_app(title) {
            return false;
        }
        // Matches simple-booking title_hints: 厦大体育 / 厦门大学 / 步道乐跑.
        title.contains("厦大体育")
            || title.contains("步道乐跑")
            || title.contains("厦门大学")
    }

    struct EnumCtx {
        hits: Vec<NativeSurface>,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut EnumCtx);
        if !IsWindowVisible(hwnd).as_bool() {
            return TRUE;
        }
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n <= 0 {
            return TRUE;
        }
        let title = String::from_utf16_lossy(&buf[..n as usize]);
        if title.is_empty() || title_is_host_app(&title) {
            return TRUE;
        }
        let mut owner_pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut owner_pid));
        if owner_pid == std::process::id() {
            return TRUE;
        }
        if !title_is_wechat(&title) && !title_is_miniprogram(&title) {
            return TRUE;
        }
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return TRUE;
        }
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        if w < 80 || h < 80 {
            return TRUE;
        }
        if let Ok(native) = build_native(hwnd, &title, &rect) {
            ctx.hits.push(native);
        }
        TRUE
    }

    unsafe fn build_native(hwnd: HWND, title: &str, rect: &RECT) -> Result<NativeSurface, String> {
        let mut class_buf = [0u16; 256];
        let cn = GetClassNameW(hwnd, &mut class_buf);
        let class_name = if cn > 0 {
            String::from_utf16_lossy(&class_buf[..cn as usize])
        } else {
            String::new()
        };

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return Err("no pid".into());
        }

        let (app_identity, launch_identity) = process_identity(pid)?;

        let bounds = RectF {
            x: rect.left as f64,
            y: rect.top as f64,
            width: (rect.right - rect.left) as f64,
            height: (rect.bottom - rect.top) as f64,
        };
        let (scale_x, scale_y) = window_dpi_scale(hwnd);
        let hwnd_u64 = hwnd.0 as usize as u64;
        let surface_fingerprint = format!(
            "hwnd:{hwnd_u64}|class:{class_name}|img:{app_identity}|launch:{launch_identity}"
        );
        let visible = IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool();
        let area = (bounds.width as i64) * (bounds.height as i64);
        Ok(NativeSurface {
            hwnd_usize: hwnd.0 as usize,
            hwnd_u64,
            title: title.to_string(),
            class_name,
            owner: OwnerIdentity {
                pid,
                app_identity,
                launch_identity,
            },
            bounds,
            scale_x,
            scale_y,
            surface_fingerprint,
            visible,
            area,
        })
    }

    unsafe fn process_identity(pid: u32) -> Result<(String, String), String> {
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            false,
            pid,
        )
        .map_err(|e| e.to_string())?;
        let mut path_buf = [0u16; 1024];
        let n = GetModuleFileNameExW(Some(handle), None, &mut path_buf);
        let app = if n > 0 {
            String::from_utf16_lossy(&path_buf[..n as usize])
        } else {
            format!("pid:{pid}")
        };
        let mut creation = Default::default();
        let mut exit = Default::default();
        let mut kernel = Default::default();
        let mut user = Default::default();
        let launch =
            if GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user).is_ok() {
                format!("{}:{}", creation.dwHighDateTime, creation.dwLowDateTime)
            } else {
                format!("pid:{pid}:unknown_boot")
            };
        let _ = CloseHandle(handle);
        Ok((app, launch))
    }

    pub fn discover_wechat_candidates() -> Result<Vec<NativeSurface>, String> {
        let mut ctx = EnumCtx { hits: Vec::new() };
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut EnumCtx as isize));
        }
        // Dedupe by hwnd, prefer larger area
        let mut best: HashMap<u64, NativeSurface> = HashMap::new();
        for h in ctx.hits {
            best.entry(h.hwnd_u64)
                .and_modify(|e| {
                    if h.area > e.area {
                        *e = h.clone();
                    }
                })
                .or_insert(h);
        }
        let mut v: Vec<_> = best.into_values().collect();
        v.sort_by_key(|n| -n.area);
        Ok(v)
    }

    pub fn refresh_native(hwnd: HWND) -> Result<NativeSurface, String> {
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return Err("window_gone".into());
            }
            let mut buf = [0u16; 512];
            let n = GetWindowTextW(hwnd, &mut buf);
            let title = if n > 0 {
                String::from_utf16_lossy(&buf[..n as usize])
            } else {
                String::new()
            };
            let mut rect = RECT::default();
            GetWindowRect(hwnd, &mut rect).map_err(|e| e.to_string())?;
            build_native(hwnd, &title, &rect)
        }
    }

    pub fn activation_status(hwnd: HWND) -> ActivationStatus {
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return ActivationStatus::Denied;
            }
            if GetForegroundWindow() == hwnd {
                ActivationStatus::Foreground
            } else {
                ActivationStatus::CanActivate
            }
        }
    }

    pub fn activate(hwnd: HWND) -> Result<(), String> {
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return Err("window_gone".into());
            }
            // Protocol launch often leaves WeChat main above the mini-program HWND.
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = BringWindowToTop(hwnd);
            if SetForegroundWindow(hwnd).as_bool() {
                return Ok(());
            }
            // OS focus-steal guard: brief Alt pulse (same as Python sidecar window.activate).
            keybd_event(VK_MENU.0 as u8, 0, Default::default(), 0);
            let ok = SetForegroundWindow(hwnd).as_bool();
            keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
            if !ok {
                return Err("SetForegroundWindow failed".into());
            }
            Ok(())
        }
    }
}

#[cfg(windows)]
pub use win::*;

#[cfg(not(windows))]
mod stub {
    use super::*;
    #[derive(Clone)]
    pub struct NativeSurface {
        pub hwnd_usize: usize,
        pub hwnd_u64: u64,
        pub title: String,
        pub class_name: String,
        pub owner: OwnerIdentity,
        pub bounds: RectF,
        pub scale_x: f64,
        pub scale_y: f64,
        pub surface_fingerprint: String,
        pub visible: bool,
        pub area: i64,
    }
    impl NativeSurface {
        pub fn hwnd(&self) -> usize {
            self.hwnd_usize
        }
        pub fn to_candidate(&self) -> SurfaceCandidate {
            unimplemented!()
        }
        pub fn to_lease(&self, _: u64) -> TargetSurfaceLease {
            unimplemented!()
        }
    }
    pub fn title_is_wechat(_: &str) -> bool {
        false
    }
    pub fn title_is_miniprogram(_: &str) -> bool {
        false
    }
    pub fn discover_wechat_candidates() -> Result<Vec<NativeSurface>, String> {
        Err("windows only".into())
    }
    pub fn refresh_native(_: usize) -> Result<NativeSurface, String> {
        Err("windows only".into())
    }
    pub fn activation_status(_: usize) -> ActivationStatus {
        ActivationStatus::Denied
    }
    pub fn activate(_: usize) -> Result<(), String> {
        Err("windows only".into())
    }
}
#[cfg(not(windows))]
pub use stub::*;

#[cfg(all(test, windows))]
mod title_tests {
    use super::title_is_miniprogram;

    #[test]
    fn host_window_is_not_a_miniprogram() {
        assert!(!title_is_miniprogram("厦大体育 · 预约助手"));
        assert!(title_is_miniprogram("厦大体育"));
        assert!(title_is_miniprogram("步道乐跑"));
        assert!(title_is_miniprogram("厦门大学"));
    }
}
