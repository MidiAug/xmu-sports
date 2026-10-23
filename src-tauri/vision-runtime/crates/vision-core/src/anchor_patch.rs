//! Cheap pre-click anchor re-location on a verification frame (no OCR).
//!
//! Source-frame semantics stay in the ActionPlan; this only finds where the
//! same local patch sits on a freshly grabbed WGC frame so Guard can measure drift.

use vision_contracts::AnchorBox;

/// Max mean absolute difference (0..255 gray) accepted as the same control.
pub const ANCHOR_PATCH_MAX_MAD: f32 = 28.0;

/// Search radius around the source anchor when re-locating (pixels).
pub const ANCHOR_PATCH_SEARCH_RADIUS: i32 = 24;

/// Re-locate `anchor` from `src_bgra` onto `ver_bgra` by gray SAD in a local window.
///
/// Returns the best-matching box on the verification frame, or `None` when the
/// patch is lost / frames are unusable.
pub fn relocate_anchor_patch(
    src_bgra: &[u8],
    src_w: u32,
    src_h: u32,
    ver_bgra: &[u8],
    ver_w: u32,
    ver_h: u32,
    anchor: &AnchorBox,
    search_radius_px: i32,
) -> Option<AnchorBox> {
    if src_w == 0 || src_h == 0 || ver_w == 0 || ver_h == 0 {
        return None;
    }
    if src_w != ver_w || src_h != ver_h {
        return None;
    }
    if anchor.x1 <= anchor.x0 || anchor.y1 <= anchor.y0 {
        return None;
    }

    let aw = (anchor.x1 - anchor.x0) as u32;
    let ah = (anchor.y1 - anchor.y0) as u32;
    if aw < 4 || ah < 4 {
        return None;
    }

    let needle = extract_gray(src_bgra, src_w, src_h, anchor.x0, anchor.y0, aw, ah)?;
    let radius = search_radius_px.max(0);
    let x0 = (anchor.x0 - radius).max(0);
    let y0 = (anchor.y0 - radius).max(0);
    let x1 = (anchor.x0 + radius).min((ver_w as i32).saturating_sub(aw as i32));
    let y1 = (anchor.y0 + radius).min((ver_h as i32).saturating_sub(ah as i32));
    if x0 > x1 || y0 > y1 {
        return None;
    }

    let mut best_mad = f32::MAX;
    let mut best_xy = (anchor.x0, anchor.y0);
    let mut best_distance = i64::MAX;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let Some(mad) = patch_mad(ver_bgra, ver_w, ver_h, x, y, aw, ah, &needle) else {
                continue;
            };
            // Blank captcha input bands have many equally good matches. Never
            // invent motion from scan order; prefer the closest equal match.
            let dx = i64::from(x - anchor.x0);
            let dy = i64::from(y - anchor.y0);
            let distance = dx * dx + dy * dy;
            if mad < best_mad || (mad == best_mad && distance < best_distance) {
                best_mad = mad;
                best_xy = (x, y);
                best_distance = distance;
            }
        }
    }
    if best_mad > ANCHOR_PATCH_MAX_MAD {
        return None;
    }
    Some(AnchorBox {
        x0: best_xy.0,
        y0: best_xy.1,
        x1: best_xy.0 + aw as i32,
        y1: best_xy.1 + ah as i32,
    })
}

fn extract_gray(
    bgra: &[u8],
    frame_w: u32,
    frame_h: u32,
    x0: i32,
    y0: i32,
    w: u32,
    h: u32,
) -> Option<Vec<u8>> {
    if x0 < 0 || y0 < 0 {
        return None;
    }
    let x0 = x0 as u32;
    let y0 = y0 as u32;
    if x0 + w > frame_w || y0 + h > frame_h {
        return None;
    }
    let mut out = vec![0u8; (w * h) as usize];
    for row in 0..h {
        for col in 0..w {
            let i = (((y0 + row) * frame_w + x0 + col) * 4) as usize;
            if i + 2 >= bgra.len() {
                return None;
            }
            let g =
                0.114 * bgra[i] as f32 + 0.587 * bgra[i + 1] as f32 + 0.299 * bgra[i + 2] as f32;
            out[(row * w + col) as usize] = g as u8;
        }
    }
    Some(out)
}

fn patch_mad(
    bgra: &[u8],
    frame_w: u32,
    frame_h: u32,
    x0: i32,
    y0: i32,
    w: u32,
    h: u32,
    needle: &[u8],
) -> Option<f32> {
    if x0 < 0 || y0 < 0 {
        return None;
    }
    let x0 = x0 as u32;
    let y0 = y0 as u32;
    if x0 + w > frame_w || y0 + h > frame_h {
        return None;
    }
    if needle.len() != (w * h) as usize {
        return None;
    }
    let mut sum = 0f32;
    let n = (w * h) as f32;
    for row in 0..h {
        for col in 0..w {
            let i = (((y0 + row) * frame_w + x0 + col) * 4) as usize;
            if i + 2 >= bgra.len() {
                return None;
            }
            let g =
                0.114 * bgra[i] as f32 + 0.587 * bgra[i + 1] as f32 + 0.299 * bgra[i + 2] as f32;
            let n_g = needle[(row * w + col) as usize] as f32;
            sum += (g - n_g).abs();
        }
    }
    Some(sum / n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_input_band_does_not_drift_when_other_content_changes() {
        let (w, h) = (100, 80);
        let src = solid(w, h, 255, 255, 255);
        let mut ver = src.clone();
        paint_rect(&mut ver, w, 0, 0, 8, 8, (0, 0, 0));
        let anchor = AnchorBox {
            x0: 40,
            y0: 35,
            x1: 70,
            y1: 50,
        };
        let relocated = relocate_anchor_patch(&src, w, h, &ver, w, h, &anchor, 24).unwrap();
        assert_eq!(relocated, anchor);
        assert!(crate::click_safety_ok(crate::click_safety_score(
            0.9,
            Some(&anchor),
            Some(&relocated),
            24,
        )));
    }

    fn solid(w: u32, h: u32, b: u8, g: u8, r: u8) -> Vec<u8> {
        let mut v = vec![0u8; (w * h * 4) as usize];
        for i in 0..(w * h) as usize {
            v[i * 4] = b;
            v[i * 4 + 1] = g;
            v[i * 4 + 2] = r;
            v[i * 4 + 3] = 255;
        }
        v
    }

    fn paint_rect(buf: &mut [u8], fw: u32, x0: i32, y0: i32, w: u32, h: u32, bgr: (u8, u8, u8)) {
        for y in y0..(y0 + h as i32) {
            for x in x0..(x0 + w as i32) {
                if x < 0 || y < 0 {
                    continue;
                }
                let i = ((y as u32 * fw + x as u32) * 4) as usize;
                if i + 2 < buf.len() {
                    buf[i] = bgr.0;
                    buf[i + 1] = bgr.1;
                    buf[i + 2] = bgr.2;
                }
            }
        }
    }

    #[test]
    fn finds_shifted_patch_within_radius() {
        let w = 80u32;
        let h = 60u32;
        let mut src = solid(w, h, 200, 200, 200);
        let mut ver = solid(w, h, 200, 200, 200);
        paint_rect(&mut src, w, 20, 15, 16, 12, (40, 80, 200));
        paint_rect(&mut ver, w, 28, 18, 16, 12, (40, 80, 200));
        let anchor = AnchorBox {
            x0: 20,
            y0: 15,
            x1: 36,
            y1: 27,
        };
        let found = relocate_anchor_patch(&src, w, h, &ver, w, h, &anchor, 16).unwrap();
        assert_eq!(found.x0, 28);
        assert_eq!(found.y0, 18);
    }

    #[test]
    fn rejects_when_patch_gone() {
        let w = 80u32;
        let h = 60u32;
        let mut src = solid(w, h, 200, 200, 200);
        let ver = solid(w, h, 200, 200, 200);
        paint_rect(&mut src, w, 20, 15, 16, 12, (40, 80, 200));
        let anchor = AnchorBox {
            x0: 20,
            y0: 15,
            x1: 36,
            y1: 27,
        };
        assert!(relocate_anchor_patch(&src, w, h, &ver, w, h, &anchor, 16).is_none());
    }
}
