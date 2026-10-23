//! OCR backends for Shadow Mode.
//!
//! Downloadable ONNX models belong in the recognition pack (not the main installer).
//! Windows.Media.Ocr is an optional system backend for live observation / comparison.

use std::time::Instant;

use vision_core::{OcrHit, OcrRegionCache, PageRoiBand};

/// Live glance OCR. PP-OCRv5 mobile first; system OCR only if the model cannot run.
pub fn ocr_frame(
    width: u32,
    height: u32,
    bgra: &[u8],
) -> Result<(OcrRegionCache, Vec<String>), String> {
    match crate::rapid_ocr::recognize_bgra(width, height, bgra) {
        Ok(pair) => Ok(pair),
        Err(err) => {
            log::warn!(target: "glance", "[ocr] rapidocr failed, windows media fallback: {err}");
            let (cache, mut notes) = ocr_bgra_windows_media(width, height, bgra)?;
            notes.insert(0, format!("ocr:rapidocr_error={err}"));
            Ok((cache, notes))
        }
    }
}

/// Crop a page ROI (optionally upscale), OCR, map hits back to full-frame coords.
/// Returns mapped hits and elapsed ms.
pub fn ocr_roi_bgra(
    width: u32,
    height: u32,
    bgra: &[u8],
    band: PageRoiBand,
) -> Result<(Vec<OcrHit>, u64), String> {
    let t0 = Instant::now();
    let Some((x0, y0, x1, y1)) = band.pixel_rect(width, height) else {
        return Ok((Vec::new(), t0.elapsed().as_millis() as u64));
    };
    let cw = x1 - x0;
    let ch = y1 - y0;
    let mut crop = Vec::with_capacity((cw * ch * 4) as usize);
    for y in y0..y1 {
        let row = (y * width * 4) as usize;
        let start = row + (x0 * 4) as usize;
        crop.extend_from_slice(&bgra[start..start + (cw * 4) as usize]);
    }
    let scale = band.scale.max(1.0);
    let (ocr_w, ocr_h, ocr_px) = if scale > 1.01 {
        let sw = ((cw as f32) * scale).round().max(8.0) as u32;
        let sh = ((ch as f32) * scale).round().max(8.0) as u32;
        let scaled = nearest_upscale_bgra(cw, ch, &crop, sw, sh);
        (sw, sh, scaled)
    } else {
        (cw, ch, crop)
    };
    let (regions, _) = crate::rapid_ocr::recognize_bgra(ocr_w, ocr_h, &ocr_px)?;
    let sx = if ocr_w > 0 {
        cw as f32 / ocr_w as f32
    } else {
        1.0
    };
    let sy = if ocr_h > 0 {
        ch as f32 / ocr_h as f32
    } else {
        1.0
    };
    let mapped: Vec<OcrHit> = regions
        .hits()
        .iter()
        .map(|h| OcrHit {
            text: h.text.clone(),
            x: x0 as i32 + (h.x as f32 * sx).round() as i32,
            y: y0 as i32 + (h.y as f32 * sy).round() as i32,
            w: ((h.w as f32) * sx).round().max(1.0) as i32,
            h: ((h.h as f32) * sy).round().max(1.0) as i32,
            score: h.score,
        })
        .collect();
    Ok((mapped, t0.elapsed().as_millis() as u64))
}

fn nearest_upscale_bgra(sw: u32, sh: u32, src: &[u8], dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dw * dh * 4) as usize];
    for y in 0..dh {
        let sy = (y as u64 * sh as u64 / dh as u64) as u32;
        for x in 0..dw {
            let sx = (x as u64 * sw as u64 / dw as u64) as u32;
            let si = ((sy * sw + sx) * 4) as usize;
            let di = ((y * dw + x) * 4) as usize;
            if si + 3 < src.len() && di + 3 < out.len() {
                out[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
    out
}

#[cfg(windows)]
pub fn ocr_bgra_windows_media(
    width: u32,
    height: u32,
    bgra: &[u8],
) -> Result<(OcrRegionCache, Vec<String>), String> {
    let mut notes = Vec::new();
    let t_engine = std::time::Instant::now();
    let engine = create_ocr_engine(&mut notes)?;
    let engine_ms = t_engine.elapsed().as_millis();

    let pixel_bytes = (width * height * 4) as usize;
    if bgra.len() < pixel_bytes {
        return Err("bgra buffer too small".into());
    }

    // Dark WeChat UI: try normal then inverted; keep the richer Chinese hit set.
    // Small windows (1080p mini-program ~524px) drop short labels. The sharp
    // display at ~1054px reads them, so feed the engine a 2x frame and map back.
    let scale: i32 = if width < 800 { 2 } else { 1 };
    let (ocr_w, ocr_h, ocr_px) = if scale == 2 {
        notes.push("ocr:upscale=2".into());
        let scaled = scale2_bgra(width, height, bgra);
        (width * 2, height * 2, scaled)
    } else {
        (width, height, bgra.to_vec())
    };

    let (hits_a, notes_a) = recognize_bgra(&engine, "normal", ocr_w, ocr_h, &ocr_px)?;
    notes.extend(notes_a);
    let t_invert = std::time::Instant::now();
    let inverted = invert_bgra(&ocr_px);
    let invert_ms = t_invert.elapsed().as_millis();
    let (hits_b, notes_b) = recognize_bgra(&engine, "inverted", ocr_w, ocr_h, &inverted)?;
    notes.extend(notes_b);

    let hits = pick_better_hits(hits_a, hits_b, &mut notes)
        .into_iter()
        .map(|h| map_hit_from_scale(h, scale))
        .collect::<Vec<_>>();
    notes.push(format!("ocr:hits={}", hits.len()));
    log::info!(
        target: "glance",
        "[timing] phase=ocr px={width}x{height} bytes={pixel_bytes} engine_ms={engine_ms} invert_ms={invert_ms} {}",
        notes.join(" ")
    );
    Ok((OcrRegionCache::from_hits(hits), notes))
}

#[cfg(windows)]
fn create_ocr_engine(notes: &mut Vec<String>) -> Result<windows::Media::Ocr::OcrEngine, String> {
    use windows::core::HSTRING;
    use windows::Globalization::Language;
    use windows::Media::Ocr::OcrEngine;

    for tag in ["zh-Hans", "zh-CN", "zh"] {
        if let Ok(lang) = Language::CreateLanguage(&HSTRING::from(tag)) {
            if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&lang) {
                notes.push("ocr:windows_media".into());
                notes.push(format!("ocr:lang={tag}"));
                return Ok(engine);
            }
            notes.push(format!("ocr:lang_unavailable={tag}"));
        }
    }
    let engine =
        OcrEngine::TryCreateFromUserProfileLanguages().map_err(|e| format!("OcrEngine:{e}"))?;
    notes.push("ocr:windows_media".into());
    notes.push("ocr:lang=user_profile_languages".into());
    Ok(engine)
}

fn map_hit_from_scale(mut hit: OcrHit, scale: i32) -> OcrHit {
    if scale <= 1 {
        return hit;
    }
    hit.x /= scale;
    hit.y /= scale;
    hit.w = (hit.w / scale).max(1);
    hit.h = (hit.h / scale).max(1);
    hit
}

fn scale2_bgra(width: u32, height: u32, bgra: &[u8]) -> Vec<u8> {
    let nw = width * 2;
    let nh = height * 2;
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let s = ((y * width + x) * 4) as usize;
            if s + 3 >= bgra.len() {
                break;
            }
            for dy in 0..2u32 {
                for dx in 0..2u32 {
                    let d = (((y * 2 + dy) * nw + (x * 2 + dx)) * 4) as usize;
                    out[d..d + 4].copy_from_slice(&bgra[s..s + 4]);
                }
            }
        }
    }
    out
}

#[cfg(windows)]
fn invert_bgra(bgra: &[u8]) -> Vec<u8> {
    let mut out = bgra.to_vec();
    for chunk in out.chunks_exact_mut(4) {
        chunk[0] = 255 - chunk[0];
        chunk[1] = 255 - chunk[1];
        chunk[2] = 255 - chunk[2];
        // alpha unchanged
    }
    out
}

#[cfg(windows)]
fn chinese_score(hits: &[OcrHit]) -> usize {
    hits.iter()
        .map(|h| {
            h.text
                .chars()
                .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
                .count()
        })
        .sum()
}

#[cfg(windows)]
fn pick_better_hits(a: Vec<OcrHit>, b: Vec<OcrHit>, notes: &mut Vec<String>) -> Vec<OcrHit> {
    let sa = chinese_score(&a);
    let sb = chinese_score(&b);
    notes.push(format!("ocr:zh_chars_normal={sa}"));
    notes.push(format!("ocr:zh_chars_inverted={sb}"));
    if sb > sa {
        notes.push("ocr:used=inverted".into());
        b
    } else {
        notes.push("ocr:used=normal".into());
        a
    }
}

#[cfg(windows)]
fn recognize_bgra(
    engine: &windows::Media::Ocr::OcrEngine,
    pass: &str,
    width: u32,
    height: u32,
    bgra: &[u8],
) -> Result<(Vec<OcrHit>, Vec<String>), String> {
    use windows::core::{Interface, HSTRING};
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Win32::System::WinRT::IBufferByteAccess;

    let mut notes = Vec::new();
    let pixel_bytes = (width * height * 4) as usize;
    let t_bitmap = std::time::Instant::now();
    let buffer = {
        use windows::Storage::Streams::Buffer;
        let buf = Buffer::Create(pixel_bytes as u32).map_err(|e| format!("Buffer:{e}"))?;
        buf.SetLength(pixel_bytes as u32)
            .map_err(|e| format!("SetLength:{e}"))?;
        let access: IBufferByteAccess = buf.cast().map_err(|e| format!("IBufferByteAccess:{e}"))?;
        unsafe {
            let ptr = access.Buffer().map_err(|e| format!("Buffer_ptr:{e}"))?;
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), ptr, pixel_bytes);
        }
        buf
    };

    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        width as i32,
        height as i32,
    )
    .map_err(|e| format!("SoftwareBitmap:{e}"))?;
    notes.push(format!(
        "ocr:{pass}_bitmap_ms={}",
        t_bitmap.elapsed().as_millis()
    ));

    let t_rec = std::time::Instant::now();
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| format!("RecognizeAsync:{e}"))?
        .get()
        .map_err(|e| format!("RecognizeAsync_get:{e}"))?;
    notes.push(format!(
        "ocr:{pass}_recognize_ms={}",
        t_rec.elapsed().as_millis()
    ));

    let lines = result.Lines().map_err(|e| format!("Lines:{e}"))?;
    let mut hits = Vec::new();
    let count = lines.Size().map_err(|e| format!("Size:{e}"))?;
    for i in 0..count {
        let line = lines.GetAt(i).map_err(|e| format!("GetAt:{e}"))?;
        let text = line
            .Text()
            .unwrap_or_else(|_| HSTRING::from(""))
            .to_string();
        if text.trim().is_empty() {
            continue;
        }
        let cleaned = clean_ocr_text(&text);
        if cleaned.is_empty() {
            continue;
        }

        // Prefer word boxes so anchors land on「厦大体育」not the whole row.
        let mut word_hits = Vec::new();
        if let Ok(words) = line.Words() {
            let n = words.Size().unwrap_or(0);
            for wi in 0..n {
                if let Ok(word) = words.GetAt(wi) {
                    let wt = word
                        .Text()
                        .unwrap_or_else(|_| HSTRING::from(""))
                        .to_string();
                    let wt = clean_ocr_text(&wt);
                    if wt.is_empty() {
                        continue;
                    }
                    if let Ok(rect) = word.BoundingRect() {
                        word_hits.push(OcrHit {
                            text: wt,
                            x: rect.X as i32,
                            y: rect.Y as i32,
                            w: (rect.Width as i32).max(1),
                            h: (rect.Height as i32).max(1),
                            score: 0.8,
                        });
                    }
                }
            }
        }

        if !word_hits.is_empty() {
            // Also keep the full line so multi-char needles still match.
            let (x, y, w, h) = union_box(&word_hits);
            hits.push(OcrHit {
                text: cleaned,
                x,
                y,
                w,
                h,
                score: 0.75,
            });
            hits.extend(word_hits);
        } else {
            hits.push(OcrHit {
                text: cleaned,
                x: 0,
                y: (i as i32) * 20,
                w: 80,
                h: 16,
                score: 0.7,
            });
        }
    }
    notes.push(format!("ocr:{pass}_lines={count}"));
    Ok((hits, notes))
}

fn clean_ocr_text(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn union_box(hits: &[OcrHit]) -> (i32, i32, i32, i32) {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = 0i32;
    let mut max_y = 0i32;
    for h in hits {
        min_x = min_x.min(h.x);
        min_y = min_y.min(h.y);
        max_x = max_x.max(h.x + h.w);
        max_y = max_y.max(h.y + h.h);
    }
    if min_x == i32::MAX {
        (0, 0, 40, 16)
    } else {
        (min_x, min_y, (max_x - min_x).max(1), (max_y - min_y).max(1))
    }
}

#[cfg(not(windows))]
pub fn ocr_bgra_windows_media(
    _width: u32,
    _height: u32,
    _bgra: &[u8],
) -> Result<(OcrRegionCache, Vec<String>), String> {
    Err("windows only".into())
}

pub fn ocr_from_annotated_hits(hits: Vec<OcrHit>) -> OcrRegionCache {
    OcrRegionCache::from_hits(hits)
}

#[cfg(test)]
mod tests {
    use super::map_hit_from_scale;
    use vision_core::OcrHit;

    #[test]
    fn upscaled_word_box_maps_back_to_the_frame() {
        let hit = map_hit_from_scale(
            OcrHit {
                text: "健身房".into(),
                x: 600,
                y: 720,
                w: 80,
                h: 28,
                score: 0.8,
            },
            2,
        );
        assert_eq!((hit.x, hit.y, hit.w, hit.h), (300, 360, 40, 14));
    }
}
