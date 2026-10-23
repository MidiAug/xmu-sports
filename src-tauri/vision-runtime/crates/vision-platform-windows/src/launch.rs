//! Open 厦大体育 via PC WeChat URL protocol (same as desktop .lnk / simple-booking).
//!
//! `ShellExecute` / `Start-Process` success only means Windows handed `weixin://` to WeChat —
//! not that a mini-program window appeared. See `docs/research/pc-miniprogram-launch.md`.

/// Official 厦大体育 mini-program AppID on PC WeChat.
pub const DEFAULT_APP_ID: &str = "wx81a2b2fa90759cb7";

/// How many times to re-fire the protocol when no mini-program window appears.
pub const DEFAULT_LAUNCH_ATTEMPTS: u32 = 3;

/// Per-attempt wait for a titled mini-program window (ms).
pub const DEFAULT_LAUNCH_WAIT_MS: u64 = 15_000;

/// Pause between protocol retries (ms).
pub const LAUNCH_RETRY_GAP_MS: u64 = 800;

/// Stable fail code for host events / UI outcome map.
pub const LAUNCH_MP_NEED_DESKTOP: &str = "LAUNCH_MP_NEED_DESKTOP";

/// Human guidance when protocol never surfaces a mini-program window.
pub const LAUNCH_MP_NEED_DESKTOP_MSG: &str = "协议已唤起微信，但多次仍未出现小程序窗口。请在 PC 微信中手动打开「厦大体育」，点右上角 ··· →「添加到电脑桌面」后再试。";

/// Build `weixin://launchapplet/?app_id=…`.
pub fn launch_url(app_id: &str) -> String {
    let id = app_id.trim();
    let id = if id.is_empty() { DEFAULT_APP_ID } else { id };
    format!("weixin://launchapplet/?app_id={id}")
}

/// Fire the WeChat protocol. Returns the URL used.
#[cfg(windows)]
pub fn launch_xmu_miniprogram(app_id: Option<&str>) -> Result<String, String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let url = launch_url(app_id.unwrap_or(DEFAULT_APP_ID));
    let wide: Vec<u16> = OsStr::new(&url)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // Same as Python os.startfile / Start-Process on weixin://
    let result = unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            windows::core::w!("open"),
            windows::core::PCWSTR(wide.as_ptr()),
            None,
            None,
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    let code = result.0 as isize;
    if code <= 32 {
        return Err(format!(
            "无法打开小程序协议（ShellExecute={code}）。确认已安装并登录 PC 微信"
        ));
    }
    log::info!(target: "glance", "[launch] url={url}");
    Ok(url)
}

#[cfg(not(windows))]
pub fn launch_xmu_miniprogram(_app_id: Option<&str>) -> Result<String, String> {
    Err("打开小程序当前只在 Windows 上可用".into())
}

/// Bring a visible mini-program window above WeChat main (and anything else covering it).
///
/// Protocol launch often only raises the WeChat shell; the mini-program HWND can stay behind.
#[cfg(windows)]
pub fn foreground_miniprogram_window() -> Result<String, String> {
    let found = crate::discover_wechat_candidates()?;
    let n = found
        .iter()
        .find(|n| n.visible && crate::is_miniprogram_title(&n.title))
        .ok_or_else(|| "未找到可见小程序窗，无法置顶".to_string())?;
    crate::discover::activate(n.hwnd()).map_err(|e| format!("小程序窗置顶失败: {e}"))?;
    log::info!(target: "glance", "[launch] foreground title={}", n.title);
    Ok(n.title.clone())
}

#[cfg(not(windows))]
pub fn foreground_miniprogram_window() -> Result<String, String> {
    Err("打开小程序当前只在 Windows 上可用".into())
}

/// Poll until a mini-program window appears (厦大体育 / 步道乐跑 / 厦门大学), then raise it.
#[cfg(windows)]
pub fn wait_miniprogram_window(timeout_ms: u64) -> Result<String, String> {
    use std::thread;
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(500));
    while Instant::now() < deadline {
        if let Ok(found) = crate::discover_wechat_candidates() {
            if let Some(n) = found
                .iter()
                .find(|n| n.visible && crate::is_miniprogram_title(&n.title))
            {
                let title = n.title.clone();
                // Brief settle: WeChat shell may still be animating to foreground.
                thread::sleep(Duration::from_millis(200));
                if let Err(e) = foreground_miniprogram_window() {
                    log::warn!(target: "glance", "[launch] foreground after wait: {e}");
                }
                return Ok(title);
            }
        }
        thread::sleep(Duration::from_millis(400));
    }
    Err("已发启动协议，但超时未看到小程序窗（标题含 厦大体育 / 步道乐跑 / 厦门大学）。微信需已登录".into())
}

#[cfg(not(windows))]
pub fn wait_miniprogram_window(_timeout_ms: u64) -> Result<String, String> {
    Err("打开小程序当前只在 Windows 上可用".into())
}

/// Launch + wait (default attempt count). Window-timeout after all retries → need-desktop.
pub fn launch_and_wait(app_id: Option<&str>, timeout_ms: u64) -> Result<LaunchResult, String> {
    launch_and_wait_n(app_id, timeout_ms, DEFAULT_LAUNCH_ATTEMPTS)
}

/// Launch + wait with explicit retries. Re-fires protocol between attempts.
pub fn launch_and_wait_n(
    app_id: Option<&str>,
    timeout_ms: u64,
    attempts: u32,
) -> Result<LaunchResult, String> {
    use std::thread;
    use std::time::Duration;

    let attempts = attempts.max(1);
    let mut last_window_err: Option<String> = None;

    for i in 1..=attempts {
        log::info!(
            target: "glance",
            "[launch] attempt={i}/{attempts} timeout_ms={timeout_ms}"
        );
        let url = launch_xmu_miniprogram(app_id)?;
        match wait_miniprogram_window(timeout_ms) {
            Ok(title) => {
                return Ok(LaunchResult { url, title });
            }
            Err(e) => {
                last_window_err = Some(e);
                if i < attempts {
                    thread::sleep(Duration::from_millis(LAUNCH_RETRY_GAP_MS));
                }
            }
        }
    }

    let detail = last_window_err.unwrap_or_else(|| "unknown".into());
    log::warn!(
        target: "glance",
        "[launch] need_desktop after {attempts} attempts detail={detail}"
    );
    Err(format!("{LAUNCH_MP_NEED_DESKTOP}: {LAUNCH_MP_NEED_DESKTOP_MSG}"))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchResult {
    pub url: String,
    pub title: String,
}

/// Parse host fail code from a launch error string (`CODE: message` or bare message).
pub fn launch_fail_code(err: &str) -> &'static str {
    if err.starts_with(LAUNCH_MP_NEED_DESKTOP) {
        LAUNCH_MP_NEED_DESKTOP
    } else if err.contains("ShellExecute") {
        "LAUNCH_MP_PROTOCOL"
    } else {
        "failed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_url() {
        assert_eq!(
            launch_url(""),
            format!("weixin://launchapplet/?app_id={DEFAULT_APP_ID}")
        );
        assert!(launch_url("wxabc").contains("wxabc"));
    }

    #[test]
    fn fail_code_need_desktop() {
        let e = format!("{LAUNCH_MP_NEED_DESKTOP}: {LAUNCH_MP_NEED_DESKTOP_MSG}");
        assert_eq!(launch_fail_code(&e), LAUNCH_MP_NEED_DESKTOP);
    }

    #[test]
    fn fail_code_protocol() {
        assert_eq!(
            launch_fail_code("无法打开小程序协议（ShellExecute=2）。确认已安装并登录 PC 微信"),
            "LAUNCH_MP_PROTOCOL"
        );
    }
}
