//! PC 微信窗口探测（Win32 EnumWindows）。
//! 宿主侧原生完成，不冷启 Python。仅供 UI「是否在跑」。
//! Native 开抢必须用 `vision-platform-windows` 的 `TargetSurfaceLease`，
//! 禁止假设 probe 标题与执行器点击窗为同一身份。
//!
//! 微信 4（Weixin.exe）登录后主窗标题仍是「微信」，缩进托盘时
//! `IsWindowVisible` 为 false。只认可见主窗会把已登录报成未运行。
//! 主窗按标题 + 尺寸认（含托盘隐藏）；小程序仍要求可见。

use serde::{Deserialize, Serialize};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, TRUE};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WechatProbe {
    pub running: bool,
    pub title: Option<String>,
    pub miniprogram: bool,
    pub message: String,
}

#[derive(Clone)]
struct WinHit {
    title: String,
    area: i64,
}

struct EnumCtx {
    wechat: Vec<WinHit>,
    mini: Vec<WinHit>,
}

fn title_is_wechat(title: &str) -> bool {
    // 禁止 contains("wechat")：会误匹配编辑器标题如 wechat_probe.rs
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

fn title_is_miniprogram(title: &str) -> bool {
    if title.contains("预约助手") {
        return false;
    }
    title.contains("厦大体育") || title.contains("步道乐跑")
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumCtx);
    // Own-window text retrieval synchronously calls the UI thread, which may
    // already be waiting for the plan lock held by this background probe.
    let mut owner_pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut owner_pid));
    if owner_pid == 0 || owner_pid == std::process::id() {
        return TRUE;
    }
    let visible = IsWindowVisible(hwnd).as_bool();

    let mut buf = [0u16; 512];
    let n = GetWindowTextW(hwnd, &mut buf);
    if n <= 0 {
        return TRUE;
    }
    let title = String::from_utf16_lossy(&buf[..n as usize]);
    if title.is_empty() || title.contains("预约助手") {
        return TRUE;
    }

    let is_wx = title_is_wechat(&title);
    let is_mini = visible && title_is_miniprogram(&title);
    if !is_wx && !is_mini {
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
    let area = (w as i64) * (h as i64);
    let hit = WinHit {
        title: title.clone(),
        area,
    };

    if is_wx {
        ctx.wechat.push(hit.clone());
    }
    if is_mini {
        ctx.mini.push(hit);
    }
    TRUE
}

fn conclude(wechat: Vec<WinHit>, mini: Vec<WinHit>) -> WechatProbe {
    let best = wechat.into_iter().max_by_key(|w| w.area);
    let mini_hit = mini.into_iter().max_by_key(|w| w.area);
    if let Some(best) = best {
        return WechatProbe {
            running: true,
            title: Some(best.title),
            miniprogram: mini_hit.is_some(),
            message: "已检测到微信窗口".into(),
        };
    }
    if let Some(mini) = mini_hit {
        return WechatProbe {
            running: true,
            title: Some(mini.title),
            miniprogram: true,
            message: "已检测到小程序窗口".into(),
        };
    }
    WechatProbe {
        running: false,
        title: None,
        miniprogram: false,
        message: "未找到微信窗口：请先打开并登录 PC 微信".into(),
    }
}

/// 微信主窗（含托盘隐藏）+ 可见小程序壳（厦大体育 / 步道乐跑）。
pub fn probe() -> WechatProbe {
    let mut ctx = EnumCtx {
        wechat: Vec::new(),
        mini: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut EnumCtx as isize));
    }
    conclude(ctx.wechat, ctx.mini)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(title: &str, area: i64) -> WinHit {
        WinHit {
            title: title.into(),
            area,
        }
    }

    #[test]
    fn hidden_main_frame_counts_as_running() {
        let p = conclude(vec![hit("微信", 907 * 655)], vec![]);
        assert!(p.running);
        assert_eq!(p.title.as_deref(), Some("微信"));
        assert!(!p.miniprogram);
    }

    #[test]
    fn visible_miniprogram_without_main_counts_as_running() {
        let p = conclude(vec![], vec![hit("厦大体育", 534 * 726)]);
        assert!(p.running);
        assert!(p.miniprogram);
        assert_eq!(p.title.as_deref(), Some("厦大体育"));
    }

    #[test]
    fn prefers_largest_wechat_title() {
        let p = conclude(
            vec![hit("Weixin", 176 * 199), hit("微信", 907 * 655)],
            vec![hit("厦大体育", 100_000)],
        );
        assert!(p.running);
        assert_eq!(p.title.as_deref(), Some("微信"));
        assert!(p.miniprogram);
    }

    #[test]
    fn nothing_is_not_running() {
        let p = conclude(vec![], vec![]);
        assert!(!p.running);
        assert!(!p.miniprogram);
        assert!(p.title.is_none());
    }

    #[test]
    fn editor_tab_is_not_wechat() {
        assert!(!title_is_wechat("wechat_probe.rs"));
        assert!(title_is_wechat("微信"));
        assert!(title_is_wechat("Weixin"));
    }
}
