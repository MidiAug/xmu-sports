//! Anonymous display topology for diagnosing DPI and virtual-desktop mappings.

pub fn device_diagnostics() -> serde_json::Value {
    let mut value = serde_json::json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "logical_cpu_count": std::thread::available_parallelism().ok().map(|n| n.get()),
        "pointer_bits": usize::BITS,
    });
    #[cfg(windows)]
    {
        value["display"] = windows_display();
    }
    value
}

#[cfg(windows)]
fn windows_display() -> serde_json::Value {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    };
    use windows::Win32::UI::HiDpi::{
        GetDpiForMonitor, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        MDT_EFFECTIVE_DPI,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    fn rect(r: RECT) -> serde_json::Value {
        serde_json::json!({"x": r.left, "y": r.top,
            "width": r.right - r.left, "height": r.bottom - r.top})
    }
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let monitors = &mut *(data.0 as *mut Vec<serde_json::Value>);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let (mut dx, mut dy) = (0, 0);
            let dpi_ok = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok();
            monitors.push(serde_json::json!({
                "primary": info.dwFlags & 1 != 0,
                "bounds_px": rect(info.rcMonitor), "work_area_px": rect(info.rcWork),
                "effective_dpi": if dpi_ok { Some([dx, dy]) } else { None },
                "scale_percent": if dpi_ok { Some([dx as f64 / 96.0 * 100.0, dy as f64 / 96.0 * 100.0]) } else { None },
            }));
        }
        TRUE
    }
    unsafe {
        let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut monitors: Vec<serde_json::Value> = Vec::new();
        let ok = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut monitors as *mut _ as isize),
        )
        .as_bool();
        let virtual_desktop = serde_json::json!({
            "x": GetSystemMetrics(SM_XVIRTUALSCREEN), "y": GetSystemMetrics(SM_YVIRTUALSCREEN),
            "width": GetSystemMetrics(SM_CXVIRTUALSCREEN), "height": GetSystemMetrics(SM_CYVIRTUALSCREEN),
        });
        let _ = SetThreadDpiAwarenessContext(previous);
        serde_json::json!({"enumerated": ok, "coordinate_space": "physical_px_pm_v2",
            "virtual_desktop": virtual_desktop, "monitors": monitors})
    }
}
