use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    Bgra8,
    Rgba8,
    Bgr8,
    Gray8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameStatus {
    Ok,
    Black,
    Incomplete,
    DeviceLost,
    PermissionDenied,
}

/// Which single-window capture path produced the frame. Never "fullscreen".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureBackend {
    GdiWindow,
    WgcHwnd,
    Fake,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RectI {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenTransform {
    /// Map content_rect pixel (x,y) → screen point in the **same physical
    /// space** as `ClientToScreen` under Per-Monitor V2.
    /// For WGC, `scale_*` is `client_px / frame_px` (≈1), never raw dpi/96.
    pub origin_x: f64,
    pub origin_y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
}

/// Pixel buffer for a captured frame. Core may read pixels; adapters own capture.
#[derive(Debug, Clone, PartialEq)]
pub struct FramePixels {
    pub bytes: Vec<u8>,
    pub stride: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub frame_id: String,
    pub lease_id: String,
    pub generation: u64,
    /// Process-local monotonic ms (`vision_contracts::monotonic_now_ms`), not Unix time.
    pub monotonic_capture_time_ms: u64,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub pixel_format: PixelFormat,
    pub content_rect: RectI,
    pub screen_transform: ScreenTransform,
    pub status: FrameStatus,
    pub pixels: Option<FramePixels>,
    /// Which backend produced this frame (diagnostic / Guard metadata).
    pub capture_backend: CaptureBackend,
}

impl Frame {
    /// Vision-usable: status must be Ok (not Black / Incomplete / …) and pixels present.
    /// Near-monochrome diagnostic frames are Incomplete and therefore not usable.
    pub fn is_usable(&self) -> bool {
        matches!(self.status, FrameStatus::Ok) && self.pixels.is_some()
    }
}
