use tauri::{AppHandle, Manager, Runtime};

pub(crate) fn set_background<R: Runtime>(app: &AppHandle<R>, background: bool) {
    #[cfg(windows)]
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.with_webview(move |platform| {
            use windows::core::Interface;
            use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2_19,
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL};
            let result = (|| -> windows::core::Result<()> {
                let webview = unsafe { platform.controller().CoreWebView2()? }.cast::<ICoreWebView2_19>()?;
                unsafe { webview.SetMemoryUsageTargetLevel(if background {
                    COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
                } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL }) }
            })();
            match result {
                Ok(()) => log::debug!(target:"memory", "[webview_memory_target] background={background}"),
                Err(error) => log::warn!(target:"memory", "[webview_memory_target_failed] {error}"),
            }
        });
    }
}
