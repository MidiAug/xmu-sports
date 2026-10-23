//! WeChat bind + WGC grab surface (session building blocks).

#![cfg(windows)]

use std::path::Path;
use std::time::Duration;

use vision_contracts::{Frame, TargetSurfaceLease};
use vision_core::{ObservationDecision, ShadowObserveResult};
use vision_pack::active_pack_dir;
use vision_platform_api::PlatformPort;

use crate::{
    discover_wechat_candidates, is_miniprogram_title, CapturePreference, NativeSurface,
    WgcCaptureSession, WindowsPlatform,
};

pub(crate) struct BoundWechat {
    pub(crate) plat: WindowsPlatform,
    pub(crate) lease: TargetSurfaceLease,
    pub(crate) native: NativeSurface,
    pub(crate) session: WgcCaptureSession,
    /// Session / UI run id shared by plans produced from this bind.
    pub(crate) run_id: String,
    pub(crate) capture_open_count: u32,
}

impl BoundWechat {
    /// WGC is change-driven. A fresh session produces a new initial capture even
    /// when the window remains static, for independent read-only verification.
    pub(crate) fn restart_capture(&mut self) -> Result<(), String> {
        let (new_session, _) = WgcCaptureSession::open(&self.native)?;
        self.session = new_session;
        self.capture_open_count = self.capture_open_count.saturating_add(1);
        Ok(())
    }
}

pub(crate) struct Observed {
    pub(crate) frame: Frame,
    pub(crate) pixels: Vec<u8>,
    pub(crate) ocr: vision_core::OcrRegionCache,
    pub(crate) result: ShadowObserveResult,
    pub(crate) decision: ObservationDecision,
    pub(crate) template_count: usize,
}

/// WinRT capture must be initialized on the worker thread. The UI thread stays free.
pub(crate) fn ensure_winrt_thread() {
    thread_local! {
        static READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    READY.with(|ready| {
        if ready.get() {
            return;
        }
        unsafe {
            let _ = windows::Win32::System::WinRT::RoInitialize(
                windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
            );
        }
        ready.set(true);
    });
}

pub(crate) fn bind_wechat(data_root: &Path) -> Result<BoundWechat, String> {
    bind_wechat_with_generation(data_root, 1)
}

pub(crate) fn bind_wechat_with_generation(
    data_root: &Path,
    generation: u64,
) -> Result<BoundWechat, String> {
    if active_pack_dir(data_root).is_none() {
        return Err("还没安装识别组件，装好之后才能看这一页".into());
    }

    let mut plat = WindowsPlatform::new();
    plat.pack_ready = true;
    plat.input_enabled = false;
    plat.capture_preference = CapturePreference::WgcOnly;
    plat.discover_surfaces("wechat")
        .map_err(|e| e.to_string())?;

    let found = discover_wechat_candidates().map_err(|e| e.to_string())?;
    for n in &found {
        log::info!(
            target: "glance",
            "[candidate] title={} pid={} visible={} mini={}",
            n.title,
            n.owner.pid,
            n.visible,
            is_miniprogram_title(&n.title)
        );
    }
    let native = found
        .iter()
        .find(|n| n.visible && is_miniprogram_title(&n.title))
        .or_else(|| {
            found
                .iter()
                .find(|n| n.visible && crate::is_wechat_title(&n.title))
        })
        .or_else(|| found.iter().find(|n| n.visible))
        .cloned()
        .ok_or_else(|| "没有找到微信窗口。先打开微信，窗口不要缩到托盘".to_string())?;
    if native.owner.pid == std::process::id() || native.title.contains("预约助手") {
        log::warn!(
            target: "glance",
            "[reject_self] title={} pid={}",
            native.title,
            native.owner.pid
        );
        return Err("抓到的是本程序自己的窗口，不是微信。先打开微信再试".into());
    }
    log::info!(
        target: "glance",
        "[bound] title={} pid={} hwnd={} candidates={} bounds={:?} scale=({:.3},{:.3})",
        native.title,
        native.owner.pid,
        native.hwnd_u64,
        found.len(),
        native.bounds,
        native.scale_x,
        native.scale_y,
    );
    log::info!(target: "glance", "[device] {}", crate::device_diagnostics());

    let cand = native.to_candidate();
    let _initial_lease = plat
        .bind_surface(&cand.candidate_id)
        .map_err(|e| e.to_string())?;
    let lease = plat.set_bound_generation(generation)?;
    let (session, _) = WgcCaptureSession::open(&native)?;
    Ok(BoundWechat {
        plat,
        lease,
        native,
        session,
        run_id: String::new(),
        capture_open_count: 1,
    })
}

pub(crate) fn grab_bound_timeout(
    bound: &mut BoundWechat,
    wait: Duration,
) -> Result<(Frame, Vec<u8>), String> {
    let (frame, _notes) = bound.session.grab_frame_timeout(&bound.lease, wait)?;
    if !frame.is_usable() {
        log::warn!(
            target: "glance",
            "[frame_unusable] title={} {}x{} status={:?}",
            bound.native.title,
            frame.pixel_width,
            frame.pixel_height,
            frame.status
        );
        return Err("抓到的画面不能用。把微信窗口露出来再试一次".into());
    }
    log::info!(
        target: "glance",
        "[frame] title={} {}x{}",
        bound.native.title,
        frame.pixel_width,
        frame.pixel_height
    );
    let pixels = frame
        .pixels
        .as_ref()
        .ok_or_else(|| "没有抓到像素".to_string())?
        .bytes
        .clone();
    Ok((frame, pixels))
}
