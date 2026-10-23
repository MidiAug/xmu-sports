//! Single-window GDI capture bound to a NativeSurface.
//! Client-area pixels; screen origin from ClientToScreen (never window-rect guess).
//! Never crops from a full-screen bitmap.

use vision_contracts::{
    monotonic_now_ms, CaptureBackend, Frame, FramePixels, PixelFormat, RectI, ScreenTransform,
    TargetSurfaceLease,
};

use crate::discover::NativeSurface;

use super::content::assess_bgra;

#[cfg(windows)]
#[allow(dead_code)]
pub fn capture_window_gdi(
    native: &NativeSurface,
    lease: &TargetSurfaceLease,
) -> Result<Frame, String> {
    capture_window_gdi_with_notes(native, lease).map(|(f, _)| f)
}

/// GDI capture plus path/content notes (BitBlt, optional PrintWindow experiment, assessment).
#[cfg(windows)]
pub fn capture_window_gdi_with_notes(
    native: &NativeSurface,
    lease: &TargetSurfaceLease,
) -> Result<(Frame, Vec<String>), String> {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
        GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS, SRCCOPY,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

    let hwnd = native.hwnd();
    let (buf_w, buf_h, origin_x, origin_y) = unsafe {
        let mut client = RECT::default();
        if GetClientRect(hwnd, &mut client).is_err() {
            return Err("GetClientRect failed".into());
        }
        let cw = (client.right - client.left).max(1) as u32;
        let ch = (client.bottom - client.top).max(1) as u32;
        let mut pt = POINT { x: 0, y: 0 };
        if !ClientToScreen(hwnd, &mut pt).as_bool() {
            return Err("ClientToScreen failed".into());
        }
        let buf_w = ((cw as f64) * lease.scale_x).round().max(1.0) as u32;
        let buf_h = ((ch as f64) * lease.scale_y).round().max(1.0) as u32;
        (buf_w, buf_h, pt.x as f64, pt.y as f64)
    };

    unsafe {
        let hdc_window = GetDC(Some(hwnd));
        if hdc_window.is_invalid() {
            return Err("GetDC failed".into());
        }
        let hdc_mem = CreateCompatibleDC(Some(hdc_window));
        if hdc_mem.is_invalid() {
            ReleaseDC(Some(hwnd), hdc_window);
            return Err("CreateCompatibleDC failed".into());
        }
        let hbmp = CreateCompatibleBitmap(hdc_window, buf_w as i32, buf_h as i32);
        if hbmp.is_invalid() {
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(Some(hwnd), hdc_window);
            return Err("CreateCompatibleBitmap failed".into());
        }
        let old = SelectObject(hdc_mem, hbmp.into());

        let mut notes: Vec<String> = vec![format!(
            "gdi:client_origin_screen=({origin_x},{origin_y});client_size={buf_w}x{buf_h}"
        )];
        let mut any_ok = BitBlt(
            hdc_mem,
            0,
            0,
            buf_w as i32,
            buf_h as i32,
            Some(hdc_window),
            0,
            0,
            SRCCOPY,
        )
        .is_ok();
        notes.push(if any_ok {
            "gdi:bitblt".into()
        } else {
            "gdi:bitblt_failed".into()
        });

        let read_bgra = || -> Result<Vec<u8>, String> {
            let mut pixels = vec![0u8; (buf_w * buf_h * 4) as usize];
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: buf_w as i32,
                    biHeight: -(buf_h as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0 as u32,
                    ..Default::default()
                },
                ..Default::default()
            };
            let got = GetDIBits(
                hdc_mem,
                hbmp,
                0,
                buf_h,
                Some(pixels.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            );
            if got == 0 {
                return Err("GetDIBits failed".into());
            }
            Ok(pixels)
        };

        let mut pixels = if any_ok {
            match read_bgra() {
                Ok(p) => p,
                Err(e) => {
                    SelectObject(hdc_mem, old);
                    let _ = DeleteObject(hbmp.into());
                    let _ = DeleteDC(hdc_mem);
                    ReleaseDC(Some(hwnd), hdc_window);
                    return Err(e);
                }
            }
        } else {
            vec![0u8; (buf_w * buf_h * 4) as usize]
        };

        let mut assessment = assess_bgra(buf_w, buf_h, &pixels);
        notes.extend(assessment.notes.iter().cloned());

        // Diagnostic experiment only — PrintWindow may block; do not assume it fixes WeChat.
        if assessment.near_monochrome || !any_ok {
            const PW_RENDERFULLCONTENT: u32 = 0x0000_0002;
            let pw_ok =
                PrintWindow(hwnd, hdc_mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
            notes.push(if pw_ok {
                "gdi:printwindow_renderfullcontent_attempted".into()
            } else {
                "gdi:printwindow_failed".into()
            });
            if pw_ok {
                match read_bgra() {
                    Ok(p) => {
                        pixels = p;
                        assessment = assess_bgra(buf_w, buf_h, &pixels);
                        notes.extend(assessment.notes.iter().cloned());
                        if assessment.near_monochrome {
                            notes.push("gdi:printwindow_still_low_info".into());
                        } else {
                            notes.push("gdi:printwindow_improved_content".into());
                        }
                    }
                    Err(e) => notes.push(format!("gdi:printwindow_getdibits_failed:{e}")),
                }
                any_ok = true;
            }
        }

        SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbmp.into());
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(Some(hwnd), hdc_window);

        if !any_ok {
            return Err("BitBlt and PrintWindow both failed".into());
        }

        let frame = Frame {
            frame_id: format!("gdi-{}-{}", lease.generation, monotonic_now_ms()),
            lease_id: lease.lease_id.clone(),
            generation: lease.generation,
            monotonic_capture_time_ms: monotonic_now_ms(),
            pixel_width: buf_w,
            pixel_height: buf_h,
            pixel_format: PixelFormat::Bgra8,
            content_rect: RectI {
                x: 0,
                y: 0,
                width: buf_w,
                height: buf_h,
            },
            screen_transform: ScreenTransform {
                origin_x,
                origin_y,
                scale_x: lease.scale_x,
                scale_y: lease.scale_y,
            },
            status: assessment.status,
            pixels: Some(FramePixels {
                stride: (buf_w * 4) as usize,
                bytes: pixels,
            }),
            capture_backend: CaptureBackend::GdiWindow,
        };
        Ok((frame, notes))
    }
}

#[cfg(not(windows))]
pub fn capture_window_gdi(
    _native: &NativeSurface,
    _lease: &TargetSurfaceLease,
) -> Result<Frame, String> {
    Err("windows only".into())
}

#[cfg(not(windows))]
pub fn capture_window_gdi_with_notes(
    _native: &NativeSurface,
    _lease: &TargetSurfaceLease,
) -> Result<(Frame, Vec<String>), String> {
    Err("windows only".into())
}
