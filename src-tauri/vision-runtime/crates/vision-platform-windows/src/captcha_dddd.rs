//! Captcha-tile OCR via ddddocr only.
//!
//! Same-image retries (RapidOCR variants / multi-scale) do not help — research
//! showed OCR is deterministic on a given crop. If the filtered alnum length is
//! not 4, return Err so the host clicks the tile to refresh (`CAPTCHA_REFRESHED`).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ddddocr_core::traits::Loader;
use ddddocr_core::{ModelMetadata, Normalization, Ocr, Resize};
use ddddocr_ort::loader::ModelLoader;
use ddddocr_ort::OcrRuntime;
use image::imageops::FilterType;
use image::{DynamicImage, Rgb, RgbImage};
use sha2::{Digest, Sha256};

const MODEL_URL: &str =
    "https://raw.githubusercontent.com/sml2h3/ddddocr/master/ddddocr/common.onnx";
const MODEL_SHA256: &str = "33b5cd351ee94e73a6bf8fa18c415ed8b819b3ffd342e267c30d8ad8334e34e8";
const MODEL_BYTES: u64 = 54_088_400;
static CAPTCHA_ENGINE: Mutex<Option<OcrRuntime>> = Mutex::new(None);

/// Recognize exactly 4 alnum characters from a BGRA captcha tile.
///
/// One ddddocr pass only. Short / empty results are errors — caller must refresh
/// the captcha image, not re-OCR the same crop.
pub fn recognize_captcha_bgra(width: u32, height: u32, bgra: &[u8]) -> Result<String, String> {
    if width < 8 || height < 8 {
        return Err("captcha crop too small".into());
    }
    let need = (width * height * 4) as usize;
    if bgra.len() < need {
        return Err("captcha bgra too small".into());
    }
    let rgb = bgra_to_rgb(width, height, &bgra[..need]);
    // Mild upscale for small UI tiles (preprocessing, not a second recognition strategy).
    let scale = if width.max(height) < 160 { 2.5f32 } else { 1.5f32 };
    let big_w = ((width as f32) * scale).round().max(1.0) as u32;
    let big_h = ((height as f32) * scale).round().max(1.0) as u32;
    let big = image::imageops::resize(&rgb, big_w, big_h, FilterType::CatmullRom);

    let code = recognize_with_ddddocr(&big)?;
    let n = code.chars().count();
    if n >= 4 {
        return Ok(code.chars().take(4).collect());
    }
    log::warn!(
        target: "glance",
        "[captcha] ddddocr short len={n} raw={code:?} crop={width}x{height} up={big_w}x{big_h} → refresh tile (no same-image retry)"
    );
    Err("验证码裁剪图没有读出 4 位".into())
}

fn recognize_with_ddddocr(rgb: &RgbImage) -> Result<String, String> {
    let mut guard = CAPTCHA_ENGINE
        .lock()
        .map_err(|e| format!("ddddocr lock: {e}"))?;
    if guard.is_none() {
        let path = ensure_model()?;
        let session = ModelLoader::default()
            .num_threads(2)
            .build_for_path(&path)
            .map_err(|e| format!("ddddocr model load: {e}"))?;
        let metadata = ModelMetadata::from_builtin_beta(
            false,
            Resize::DynamicWidth(64),
            1,
            Normalization::MinusOneToOne,
        );
        *guard = Some(OcrRuntime::new(session, metadata));
    }
    let engine = guard.as_ref().expect("engine inserted above");
    let image = DynamicImage::ImageRgb8(rgb.clone());
    let raw = Ocr::builder()
        .build_with(engine)
        .predict(&image)
        .map_err(|e| format!("ddddocr inference: {e}"))?
        .into_text();
    log::info!(target: "glance", "[captcha] ddddocr raw={raw:?}");
    Ok(normalize_code(&raw))
}

fn normalize_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn ensure_model() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("XMU_SPORTS_CAPTCHA_MODEL") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!("captcha model not found: {}", path.display()));
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = base.join("xmu-sports").join("captcha").join("common.onnx");
    if valid_model(&path) {
        return Ok(path);
    }
    // Prefer installer-bundled model over GitHub (campus networks often block it).
    let seeded = crate::ocr_bundle::seed_ocr_cache(None);
    for note in &seeded.notes {
        log::info!(target: "glance", "[ocr] {note}");
    }
    if valid_model(&path) {
        return Ok(path);
    }
    let parent = path.parent().ok_or("captcha model cache path invalid")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("captcha cache: {e}"))?;
    let temp = parent.join(format!("common-{}.download", std::process::id()));
    let download = (|| -> Result<(), String> {
        let mut response = ureq::get(MODEL_URL)
            .call()
            .map_err(|e| format!("captcha model download: {e}"))?;
        let mut reader = response.body_mut().with_config().limit(MODEL_BYTES + 1).reader();
        let mut output = std::fs::File::create(&temp)
            .map_err(|e| format!("captcha model temp file: {e}"))?;
        std::io::copy(&mut reader, &mut output)
            .map_err(|e| format!("captcha model write: {e}"))?;
        if !valid_model(&temp) {
            return Err("captcha model download checksum mismatch".into());
        }
        std::fs::rename(&temp, &path)
            .map_err(|e| format!("captcha model install: {e}"))?;
        Ok(())
    })();
    if download.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    download.map(|()| path)
}

fn valid_model(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    if file.metadata().map(|m| m.len()).ok() != Some(MODEL_BYTES) {
        return false;
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let Ok(n) = file.read(&mut buffer) else {
            return false;
        };
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    format!("{:x}", hash.finalize()) == MODEL_SHA256
}

fn bgra_to_rgb(width: u32, height: u32, bgra: &[u8]) -> RgbImage {
    let mut out = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            out.put_pixel(x, y, Rgb([bgra[i + 2], bgra[i + 1], bgra[i]]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::normalize_code;

    #[test]
    fn normalization_keeps_only_ascii_alnum() {
        assert_eq!(normalize_code(" a-B 9_2 "), "AB92");
    }

    #[test]
    fn normalize_drops_cjk_leaving_short_code() {
        // Host must refresh when len < 4 — do not retry same crop.
        assert_eq!(normalize_code("工vzy"), "VZY");
        assert_eq!(normalize_code("工vzy").chars().count(), 3);
    }
}
