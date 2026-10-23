//! Persistent WGC capture session: one D3D11 device reused across frames.
//! Callers must not recreate the device per frame during continuous shadow observe.

use std::time::Duration;

use vision_contracts::{
    monotonic_now_ms, CaptureBackend, Frame, FramePixels, PixelFormat, RectI, ScreenTransform,
    TargetSurfaceLease,
};

use crate::discover::NativeSurface;

use super::content::assess_bgra;

const FIRST_FRAME_TIMEOUT: Duration = Duration::from_millis(2500);

#[cfg(windows)]
struct CapturedCpuFrame {
    width: u32,
    height: u32,
    bgra: Vec<u8>,
}

/// Reusable WGC pipeline bound to one HWND + shared D3D11 device.
#[cfg(windows)]
pub struct WgcCaptureSession {
    hwnd_u64: u64,
    d3d: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
    winrt_device: windows::Graphics::DirectX::Direct3D11::IDirect3DDevice,
    item: windows::Graphics::Capture::GraphicsCaptureItem,
    pool: windows::Graphics::Capture::Direct3D11CaptureFramePool,
    session: windows::Graphics::Capture::GraphicsCaptureSession,
    frame_token: i64,
    size_w: i32,
    size_h: i32,
    frames_grabbed: u64,
    device_creates: u64,
    /// Newest frame pulled while draining. Not closed, so the next grab can use it
    /// after the screen has stopped changing.
    held: Option<windows::Graphics::Capture::Direct3D11CaptureFrame>,
}

#[cfg(windows)]
impl WgcCaptureSession {
    pub fn open(native: &NativeSurface) -> Result<(Self, Vec<String>), String> {
        use windows::core::Result as WinResult;
        use windows::Foundation::TypedEventHandler;
        use windows::Graphics::Capture::Direct3D11CaptureFramePool;
        use windows::Graphics::DirectX::DirectXPixelFormat;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::IsWindow;

        let mut notes = Vec::new();
        let hwnd = HWND(native.hwnd_u64 as usize as *mut _);
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return Err("window_gone".into());
            }
        }

        let (d3d, context) = create_d3d11_device()?;
        notes.push("wgc_session:d3d11_device_created".into());
        let winrt_device = d3d_to_winrt_device(&d3d)?;
        notes.push("wgc_session:winrt_device_ok".into());

        let item = create_capture_item(hwnd)?;
        notes.push("wgc_session:CreateForWindow_ok".into());
        let size = item.Size().map_err(|e| format!("item_size:{e}"))?;
        if size.Width <= 0 || size.Height <= 0 {
            return Err(format!("item_size_invalid:{}x{}", size.Width, size.Height));
        }
        notes.push(format!(
            "wgc_session:item_size={}x{}",
            size.Width, size.Height
        ));

        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )
        .map_err(|e| format!("frame_pool_create:{e}"))?;

        let session = pool
            .CreateCaptureSession(&item)
            .map_err(|e| format!("capture_session:{e}"))?;
        let _ = session.SetIsCursorCaptureEnabled(false);
        // Yellow outline so the operator can see which HWND is bound.
        // Missing on older Windows; ignore.
        let _ = session.SetIsBorderRequired(true);

        // Placeholder handler; grab_frame uses TryGetNextFrame after StartCapture.
        let handler =
            TypedEventHandler::<Direct3D11CaptureFramePool, windows::core::IInspectable>::new(
                move |_pool_opt, _| -> WinResult<()> { Ok(()) },
            );
        let token = pool
            .FrameArrived(&handler)
            .map_err(|e| format!("frame_arrived_subscribe:{e}"))?;

        session
            .StartCapture()
            .map_err(|e| format!("start_capture:{e}"))?;
        notes.push("wgc_session:start_capture_ok".into());
        notes.push("wgc_session:device_reused_across_frames".into());

        Ok((
            Self {
                hwnd_u64: native.hwnd_u64,
                d3d,
                context,
                winrt_device,
                item,
                pool,
                session,
                frame_token: token,
                size_w: size.Width,
                size_h: size.Height,
                frames_grabbed: 0,
                device_creates: 1,
                held: None,
            },
            notes,
        ))
    }

    pub fn device_creates(&self) -> u64 {
        self.device_creates
    }

    pub fn frames_grabbed(&self) -> u64 {
        self.frames_grabbed
    }

    pub fn hwnd_u64(&self) -> u64 {
        self.hwnd_u64
    }

    /// Drop older queued frames. The newest stays in `held` for the next grab.
    /// An empty pool is `TryGetNextFrame` returning S_OK (0x0), not a failure.
    pub fn drain_ready(&mut self) -> u32 {
        let mut n = 0u32;
        for _ in 0..4 {
            match self.pool.TryGetNextFrame() {
                Ok(frame) => {
                    if let Some(old) = self.held.take() {
                        let _ = old.Close();
                        n += 1;
                    }
                    self.held = Some(frame);
                }
                Err(e) if e.code().is_ok() => break,
                Err(_) => break,
            }
        }
        n
    }

    pub fn grab_frame(
        &mut self,
        lease: &TargetSurfaceLease,
    ) -> Result<(Frame, Vec<String>), String> {
        self.grab_frame_timeout(lease, FIRST_FRAME_TIMEOUT)
    }

    /// Like `grab_frame`, but callers in a settle loop can use a short budget so a
    /// static UI (WGC stops delivering) does not burn 2.5s per poll.
    pub fn grab_frame_timeout(
        &mut self,
        lease: &TargetSurfaceLease,
        wait: Duration,
    ) -> Result<(Frame, Vec<String>), String> {
        use windows::Graphics::DirectX::DirectXPixelFormat;
        use windows::Win32::Foundation::{HWND, POINT, RECT};
        use windows::Win32::Graphics::Gdi::ClientToScreen;
        use windows::Win32::UI::HiDpi::{
            SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, IsWindow};

        let mut notes = Vec::new();
        let hwnd = HWND(self.hwnd_u64 as usize as *mut _);
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return Err("window_gone".into());
            }
        }

        // Physical coords on a 200% secondary require PMv2; otherwise ClientToScreen
        // is DPI-virtualized and clicks land in the wrong column.
        let (origin_x, origin_y, client_w, client_h) = unsafe {
            let prev = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let mut origin = POINT { x: 0, y: 0 };
            let ok_origin = ClientToScreen(hwnd, &mut origin).as_bool();
            let mut client = RECT::default();
            let ok_client = GetClientRect(hwnd, &mut client).is_ok();
            let _ = SetThreadDpiAwarenessContext(prev);
            if !ok_origin {
                return Err("ClientToScreen failed".into());
            }
            if !ok_client {
                return Err("GetClientRect failed".into());
            }
            (
                origin.x as f64,
                origin.y as f64,
                (client.right - client.left).max(1) as f64,
                (client.bottom - client.top).max(1) as f64,
            )
        };
        notes.push(format!(
            "wgc_session:client_origin_screen=({origin_x},{origin_y});client_px={client_w}x{client_h}"
        ));
        notes.push(format!(
            "wgc_session:device_creates={} frames_so_far={}",
            self.device_creates, self.frames_grabbed
        ));

        // Resize pool if item size changed.
        let size = self.item.Size().map_err(|e| format!("item_size:{e}"))?;
        if size.Width != self.size_w || size.Height != self.size_h {
            self.pool
                .Recreate(
                    &self.winrt_device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    2,
                    size,
                )
                .map_err(|e| format!("pool_recreate:{e}"))?;
            self.size_w = size.Width;
            self.size_h = size.Height;
            notes.push(format!(
                "wgc_session:pool_recreated={}x{}",
                size.Width, size.Height
            ));
        }

        let deadline = std::time::Instant::now() + wait;
        let cpu = loop {
            if let Some(held) = self.held.take() {
                match copy_capture_frame(held, &self.d3d, &self.context) {
                    Ok(frame) => break frame,
                    Err(e) => return Err(e),
                }
            }
            match take_one_frame(&self.pool, &self.d3d, &self.context) {
                Ok(frame) => break frame,
                Err(e) if no_frame_yet(&e) && std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(16));
                    continue;
                }
                Err(e) if no_frame_yet(&e) => {
                    return Err("还没有新的一帧".into());
                }
                Err(e) => {
                    return Err(format!(
                        "frame_timeout_or_err:{e};waited_ms={}",
                        wait.as_millis()
                    ));
                }
            }
        };
        notes.push("wgc_session:frame_ok".into());
        self.frames_grabbed += 1;

        let assessment = assess_bgra(cpu.width, cpu.height, &cpu.bgra);
        notes.extend(assessment.notes.iter().cloned());

        // WGC buffer is already physical. Map frame → client with a measured ratio;
        // never multiply by dpi/96 again (double-scales on 200% monitors).
        let scale_x = client_w / (cpu.width.max(1) as f64);
        let scale_y = client_h / (cpu.height.max(1) as f64);
        notes.push(format!(
            "wgc_session:screen_scale=({scale_x:.4},{scale_y:.4});lease_dpi_scale=({:.3},{:.3})",
            lease.scale_x, lease.scale_y
        ));

        let frame = Frame {
            frame_id: format!("wgc-sess-{}-{}", lease.generation, monotonic_now_ms()),
            lease_id: lease.lease_id.clone(),
            generation: lease.generation,
            monotonic_capture_time_ms: monotonic_now_ms(),
            pixel_width: cpu.width,
            pixel_height: cpu.height,
            pixel_format: PixelFormat::Bgra8,
            content_rect: RectI {
                x: 0,
                y: 0,
                width: cpu.width,
                height: cpu.height,
            },
            screen_transform: ScreenTransform {
                origin_x,
                origin_y,
                scale_x,
                scale_y,
            },
            status: assessment.status,
            pixels: Some(FramePixels {
                stride: (cpu.width * 4) as usize,
                bytes: cpu.bgra,
            }),
            capture_backend: CaptureBackend::WgcHwnd,
        };
        Ok((frame, notes))
    }
}

#[cfg(windows)]
impl Drop for WgcCaptureSession {
    fn drop(&mut self) {
        if let Some(frame) = self.held.take() {
            let _ = frame.Close();
        }
        let _ = self.session.Close();
        let _ = self.pool.RemoveFrameArrived(self.frame_token);
        let _ = self.pool.Close();
    }
}

/// One-shot capture (creates and releases device). Prefer [`WgcCaptureSession`] for loops.
#[cfg(windows)]
pub fn capture_window_wgc_with_notes(
    native: &NativeSurface,
    lease: &TargetSurfaceLease,
) -> Result<(Frame, Vec<String>), String> {
    let (mut sess, mut notes) = WgcCaptureSession::open(native)?;
    notes.push("wgc:one_shot_session".into());
    let (frame, mut grab_notes) = sess.grab_frame(lease)?;
    notes.append(&mut grab_notes);
    notes.push("wgc:resources_released".into());
    Ok((frame, notes))
}

#[cfg(windows)]
#[allow(dead_code)]
pub fn capture_window_wgc(
    native: &NativeSurface,
    lease: &TargetSurfaceLease,
) -> Result<Frame, String> {
    capture_window_wgc_with_notes(native, lease).map(|(f, _)| f)
}

#[cfg(windows)]
fn create_capture_item(
    hwnd: windows::Win32::Foundation::HWND,
) -> Result<windows::Graphics::Capture::GraphicsCaptureItem, String> {
    use windows::Graphics::Capture::GraphicsCaptureItem;
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| format!("factory:{e}"))?;
    unsafe { interop.CreateForWindow(hwnd) }.map_err(|e| format!("CreateForWindow:{e}"))
}

#[cfg(windows)]
fn create_d3d11_device() -> Result<
    (
        windows::Win32::Graphics::Direct3D11::ID3D11Device,
        windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
    ),
    String,
> {
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_11_1,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::Dxgi::DXGI_ERROR_UNSUPPORTED;

    let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
    let try_types = [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP];

    for driver in try_types {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let hr = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                flags,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        };
        match hr {
            Ok(()) => {
                return Ok((
                    device.ok_or_else(|| "d3d_device_null".to_string())?,
                    context.ok_or_else(|| "d3d_context_null".to_string())?,
                ));
            }
            Err(e) if e.code() == DXGI_ERROR_UNSUPPORTED => continue,
            Err(e) => return Err(format!("D3D11CreateDevice:{e}")),
        }
    }
    Err("D3D11CreateDevice:no_supported_driver".into())
}

#[cfg(windows)]
fn d3d_to_winrt_device(
    d3d: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
) -> Result<windows::Graphics::DirectX::Direct3D11::IDirect3DDevice, String> {
    use windows::core::Interface;
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;

    let dxgi: IDXGIDevice = d3d.cast().map_err(|e| format!("cast_dxgi:{e}"))?;
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
        .map_err(|e| format!("CreateDirect3D11DeviceFromDXGIDevice:{e}"))?;
    inspectable
        .cast::<IDirect3DDevice>()
        .map_err(|e| format!("cast_winrt_device:{e}"))
}

#[cfg(windows)]
fn no_frame_yet(err: &str) -> bool {
    err == "no_frame" || err.contains("0x00000000") || err.contains("操作成功完成")
}

#[cfg(windows)]
fn take_one_frame(
    pool: &windows::Graphics::Capture::Direct3D11CaptureFramePool,
    d3d: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
    context: &windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
) -> Result<CapturedCpuFrame, String> {
    match pool.TryGetNextFrame() {
        Ok(frame) => copy_capture_frame(frame, d3d, context),
        // WinRT null: "no frame queued", HRESULT S_OK. Not a capture failure.
        Err(e) if e.code().is_ok() => Err("no_frame".into()),
        Err(e) => Err(format!("TryGetNextFrame:{e}")),
    }
}

#[cfg(windows)]
fn copy_capture_frame(
    frame: windows::Graphics::Capture::Direct3D11CaptureFrame,
    d3d: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
    context: &windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
) -> Result<CapturedCpuFrame, String> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Direct3D11::{
        ID3D11Texture2D, D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
        D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    };
    use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;

    let surface = frame.Surface().map_err(|e| format!("frame_surface:{e}"))?;
    let access: IDirect3DDxgiInterfaceAccess = surface
        .cast()
        .map_err(|e| format!("surface_dxgi_access:{e}"))?;
    let src: ID3D11Texture2D = unsafe { access.GetInterface::<ID3D11Texture2D>() }
        .map_err(|e| format!("GetInterface_texture:{e}"))?;

    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { src.GetDesc(&mut desc) };
    let width = desc.Width;
    let height = desc.Height;
    if width == 0 || height == 0 {
        let _ = frame.Close();
        return Err("texture_size_zero".into());
    }

    desc.BindFlags = 0;
    desc.MiscFlags = 0;
    desc.Usage = D3D11_USAGE_STAGING;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;

    let mut staging: Option<ID3D11Texture2D> = None;
    unsafe {
        d3d.CreateTexture2D(&desc, None, Some(&mut staging))
            .map_err(|e| format!("CreateTexture2D_staging:{e}"))?;
    }
    let staging = staging.ok_or_else(|| "staging_null".to_string())?;
    unsafe {
        context.CopyResource(&staging, &src);
    }

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        context
            .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|e| format!("Map:{e}"))?;
    }

    let row_pitch = mapped.RowPitch as usize;
    let mut bgra = vec![0u8; (width * height * 4) as usize];
    unsafe {
        let src_ptr = mapped.pData as *const u8;
        for y in 0..height as usize {
            let src_row = src_ptr.add(y * row_pitch);
            let dst_row = bgra.as_mut_ptr().add(y * (width as usize) * 4);
            std::ptr::copy_nonoverlapping(src_row, dst_row, (width as usize) * 4);
        }
        context.Unmap(&staging, 0);
    }
    let _ = frame.Close();

    Ok(CapturedCpuFrame {
        width,
        height,
        bgra,
    })
}

#[cfg(not(windows))]
pub struct WgcCaptureSession;

#[cfg(not(windows))]
impl WgcCaptureSession {
    pub fn open(_native: &NativeSurface) -> Result<(Self, Vec<String>), String> {
        Err("windows only".into())
    }
    pub fn grab_frame(
        &mut self,
        _lease: &TargetSurfaceLease,
    ) -> Result<(Frame, Vec<String>), String> {
        Err("windows only".into())
    }
    pub fn grab_frame_timeout(
        &mut self,
        lease: &TargetSurfaceLease,
        _wait: Duration,
    ) -> Result<(Frame, Vec<String>), String> {
        self.grab_frame(lease)
    }
    pub fn device_creates(&self) -> u64 {
        0
    }
    pub fn frames_grabbed(&self) -> u64 {
        0
    }
}

#[cfg(not(windows))]
pub fn capture_window_wgc(
    _native: &NativeSurface,
    _lease: &TargetSurfaceLease,
) -> Result<Frame, String> {
    Err("windows only".into())
}

#[cfg(not(windows))]
pub fn capture_window_wgc_with_notes(
    _native: &NativeSurface,
    _lease: &TargetSurfaceLease,
) -> Result<(Frame, Vec<String>), String> {
    Err("windows only".into())
}
