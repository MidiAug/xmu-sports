//! PP-OCRv5 mobile via RapidOCR. Prefer installer-bundled models; cache under LOCALAPPDATA.
//! Windows.Media.Ocr drops short labels on a 524px mini-program window.

use std::path::PathBuf;
use std::sync::Mutex;

use image::RgbImage;
use rapidocr_core::config::{InferenceOptions, LimitType, PipelineConfig};
use rapidocr_core::model::{model_set_by_name, ModelCache, ModelDownloadMode};
use rapidocr_core::types::Quad;
use rapidocr_core::RapidOcr;
use vision_core::{OcrHit, OcrRegionCache};

const MODEL_SET: &str = "ppocrv5-ch-mobile";
/// Paddle's screen default. The library preset is Min/736, which does not
/// shrink a 1054×1563 window, so detection was running at full resolution.
const DET_MAX_SIDE: u32 = 960;

static ENGINE: Mutex<Option<RapidOcr>> = Mutex::new(None);

struct CachedFrame {
    width: u32,
    height: u32,
    fingerprint: u64,
    regions: OcrRegionCache,
}

static FRAME_CACHE: Mutex<Option<CachedFrame>> = Mutex::new(None);

pub fn recognize_bgra(
    width: u32,
    height: u32,
    bgra: &[u8],
) -> Result<(OcrRegionCache, Vec<String>), String> {
    let pixel_bytes = (width * height * 4) as usize;
    if width == 0 || height == 0 || bgra.len() < pixel_bytes {
        return Err("bgra buffer too small".into());
    }
    let pixels = &bgra[..pixel_bytes];
    let t = std::time::Instant::now();
    let fingerprint = frame_fingerprint(pixels);
    if let Some(cached) = take_cached(width, height, fingerprint)? {
        return Ok(finish(width, height, t, cached, &["ocr:cache=1".into()]));
    }

    let image = bgra_to_rgb(width, height, pixels);
    let mut extras = Vec::new();
    let timed = {
        let mut guard = ENGINE.lock().map_err(|e| format!("ocr lock: {e}"))?;
        if guard.is_none() {
            extras.push("ocr:load=1".into());
            *guard = Some(load_engine()?);
        }
        let engine = guard.as_mut().expect("engine just inserted");
        engine
            .run_image_timed(&image)
            .map_err(|e| format!("rapidocr: {e}"))?
    };
    extras.push(format!("ocr:det_ms={:.0}", timed.timings.det_inference_ms));
    extras.push(format!("ocr:rec_ms={:.0}", timed.timings.rec_inference_ms));
    let hits: Vec<OcrHit> = timed
        .output
        .lines
        .iter()
        .filter_map(|line| hit_from_quad(&line.text, line.score, &line.bbox))
        .collect();
    let regions = OcrRegionCache::from_hits(hits);
    store_cached(width, height, fingerprint, regions.clone())?;
    Ok(finish(width, height, t, regions, &extras))
}

fn finish(
    width: u32,
    height: u32,
    started: std::time::Instant,
    regions: OcrRegionCache,
    extras: &[String],
) -> (OcrRegionCache, Vec<String>) {
    let zh = zh_chars(regions.hits());
    let mut notes = vec![
        "ocr:rapidocr".into(),
        format!("ocr:model={MODEL_SET}"),
        format!("ocr:det_limit=max{DET_MAX_SIDE}"),
    ];
    notes.extend(extras.iter().cloned());
    notes.push(format!("ocr:lines={}", regions.hits().len()));
    notes.push(format!("ocr:zh_chars={zh}"));
    notes.push(format!(
        "ocr:recognize_ms={}",
        started.elapsed().as_millis()
    ));
    log::info!(
        target: "glance",
        "[timing] phase=ocr px={width}x{height} {}",
        notes.join(" ")
    );
    (regions, notes)
}

fn zh_chars(hits: &[OcrHit]) -> usize {
    hits.iter()
        .map(|h| {
            h.text
                .chars()
                .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
                .count()
        })
        .sum()
}

fn take_cached(
    width: u32,
    height: u32,
    fingerprint: u64,
) -> Result<Option<OcrRegionCache>, String> {
    let guard = FRAME_CACHE
        .lock()
        .map_err(|e| format!("ocr cache lock: {e}"))?;
    Ok(guard.as_ref().and_then(|cached| {
        (cached.width == width && cached.height == height && cached.fingerprint == fingerprint)
            .then(|| cached.regions.clone())
    }))
}

fn store_cached(
    width: u32,
    height: u32,
    fingerprint: u64,
    regions: OcrRegionCache,
) -> Result<(), String> {
    let mut guard = FRAME_CACHE
        .lock()
        .map_err(|e| format!("ocr cache lock: {e}"))?;
    *guard = Some(CachedFrame {
        width,
        height,
        fingerprint,
        regions,
    });
    Ok(())
}

/// Exact fingerprint for identical-buffer reuse (OCR cache / post-click gate).
pub(crate) fn frame_fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    let mut chunks = bytes.chunks_exact(8);
    for chunk in chunks.by_ref() {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(chunk);
        hash ^= u64::from_le_bytes(buf);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    for &byte in chunks.remainder() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^ (bytes.len() as u64)
}

fn load_engine() -> Result<RapidOcr, String> {
    // Installer ships models under $RESOURCE/ocr; seed before RapidOCR download.
    let seeded = crate::ocr_bundle::seed_ocr_cache(None);
    for note in &seeded.notes {
        log::info!(target: "glance", "[ocr] {note}");
    }
    let model = model_set_by_name(MODEL_SET).ok_or_else(|| format!("unknown model {MODEL_SET}"))?;
    let cache = ModelCache::new(cache_dir());
    let pipeline = PipelineConfig::without_cls();
    log::info!(
        target: "glance",
        "[ocr] ensuring {MODEL_SET} in {}",
        cache.root().display()
    );
    cache
        .ensure_model_set_for_pipeline(model, pipeline, ModelDownloadMode::Missing)
        .map_err(|e| format!("model cache: {e}"))?;
    let mut cfg = cache
        .config_for(model)
        .with_pipeline(pipeline)
        .with_inference_options(InferenceOptions {
            intra_threads: 4,
            inter_threads: 1,
            parallel_execution: false,
            enable_cpu_mem_arena: true,
            execution_provider: rapidocr_core::config::ExecutionProvider::Cpu,
        });
    let det = cfg
        .det
        .as_mut()
        .ok_or_else(|| "rapidocr det config missing".to_string())?;
    det.limit_side_len = DET_MAX_SIDE;
    det.limit_type = LimitType::Max;
    RapidOcr::new(cfg).map_err(|e| format!("rapidocr init: {e}"))
}

/// Crop-only OCR used by captcha recognition. Not for full-page labels.
pub(crate) fn recognize_rgb_text(image: &RgbImage) -> Result<String, String> {
    let mut guard = ENGINE.lock().map_err(|e| format!("ocr lock: {e}"))?;
    if guard.is_none() {
        *guard = Some(load_engine()?);
    }
    let engine = guard.as_mut().expect("engine");
    let timed = engine
        .run_image_timed(image)
        .map_err(|e| format!("rapidocr: {e}"))?;
    Ok(timed
        .output
        .lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join(""))
}

fn cache_dir() -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        if !local.is_empty() {
            return PathBuf::from(local).join("xmu-sports").join("rapidocr");
        }
    }
    PathBuf::from("rapidocr-models")
}

fn bgra_to_rgb(width: u32, height: u32, bgra: &[u8]) -> RgbImage {
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for px in bgra.chunks_exact(4) {
        rgb.push(px[2]);
        rgb.push(px[1]);
        rgb.push(px[0]);
    }
    RgbImage::from_raw(width, height, rgb).unwrap_or_else(|| RgbImage::new(width, height))
}

fn hit_from_quad(text: &str, score: f32, quad: &Quad) -> Option<OcrHit> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (x0, y0, x1, y1) = axis_aligned(quad);
    Some(OcrHit {
        text: text.to_string(),
        x: x0,
        y: y0,
        w: (x1 - x0).max(1),
        h: (y1 - y0).max(1),
        score,
    })
}

fn axis_aligned(quad: &Quad) -> (i32, i32, i32, i32) {
    let mut x0 = f32::INFINITY;
    let mut y0 = f32::INFINITY;
    let mut x1 = f32::NEG_INFINITY;
    let mut y1 = f32::NEG_INFINITY;
    for p in &quad.points {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    (
        x0.floor().max(0.0) as i32,
        y0.floor().max(0.0) as i32,
        x1.ceil().max(0.0) as i32,
        y1.ceil().max(0.0) as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::axis_aligned;
    use rapidocr_core::types::Quad;

    #[test]
    fn fingerprint_changes_when_one_byte_changes() {
        let same = [1u8, 2, 3, 4, 5, 6, 7, 8, 9];
        let mut other = same;
        other[8] = 10;
        assert_eq!(
            super::frame_fingerprint(&same),
            super::frame_fingerprint(&same)
        );
        assert_ne!(
            super::frame_fingerprint(&same),
            super::frame_fingerprint(&other)
        );
    }

    #[test]
    fn quad_becomes_an_axis_aligned_box() {
        let quad = Quad {
            points: [[10.2, 20.8], [40.1, 18.0], [42.0, 36.4], [9.0, 35.0]],
        };
        assert_eq!(axis_aligned(&quad), (9, 18, 42, 37));
    }

    #[test]
    fn rapidocr_reads_gym_when_smoke_image_is_set() {
        let Ok(path) = std::env::var("XMU_OCR_SMOKE") else {
            return;
        };
        let image = image::open(&path)
            .unwrap_or_else(|e| panic!("open {path}: {e}"))
            .to_rgb8();
        let (w, h) = image.dimensions();
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for px in image.pixels() {
            bgra.push(px[2]);
            bgra.push(px[1]);
            bgra.push(px[0]);
            bgra.push(255);
        }
        let (cache, notes) = super::recognize_bgra(w, h, &bgra).expect("rapidocr");
        let texts: Vec<_> = cache.hits().iter().map(|h| h.text.clone()).collect();
        eprintln!("notes={notes:?}");
        eprintln!("hits={}", texts.join(" | "));
        assert!(
            texts.iter().any(|t| t.contains("健身房")),
            "missing 健身房 in {texts:?}"
        );
    }
}
