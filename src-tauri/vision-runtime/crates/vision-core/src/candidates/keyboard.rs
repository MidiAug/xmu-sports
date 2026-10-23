//! Soft-keyboard candidates from white key-cap edges + QWERTY labels.

//!

//! First principles: the captcha keyboard is a gray sheet with white rounded

//! keys. Per-letter OCR is flaky on tiny glyphs; edge/brightness blobs of the

//! caps are stable. Pipeline (same idea as OpenCV connectedComponents +

//! PyImageSearch CC filtering; implemented without linking OpenCV):

//!

//! 1. Anchor panel with OCR「取消」

//! 2. Threshold bright low-sat pixels in the key body → connected components

//! 3. NMS + filter by area / aspect (key-sized white rounded rects)

//! 4. Cluster into rows (top→bottom), snap to uniform QWERTY grid per row

//! 5. Fall back to uniform geometry only if too few caps are found

//!

//! Threshold note (2026-09-22 probe): WeChat sheet is already ~235–245 V; keys

//! sit at ~250–255. `v >= 232` merges sheet+caps into one blob and detection

//! collapses to OCR glyph boxes that drift. Use `v >= 248`.



use crate::ocr::{labels_match_exact, OcrRegionCache};



use super::color::opencv_hsv;

use super::parse::{blank, is_soft_key_glyph};

use super::types::{box_iou, BBox, Candidate, CandidateKind, CandidateSource};



const DIGIT_ROW: &str = "1234567890";

const ROW_Q: &str = "qwertyuiop";

const ROW_A: &str = "asdfghjkl";

const ROW_Z: &str = "zxcvbnm";

const QWERTY_ROWS: [&str; 4] = [DIGIT_ROW, ROW_Q, ROW_A, ROW_Z];



/// Sheet is ~235–245; keys are ~250–255. Must sit above the sheet.

const KEY_CAP_V_MIN: u8 = 248;

const KEY_CAP_S_MAX: u8 = 45;

const CAP_NMS_IOU: f32 = 0.30;



/// After OCR attach + promote: prefer white-cap boxes; geometry fills gaps.

pub(super) fn inject_geometric_soft_keys(

    items: &mut Vec<Candidate>,

    ocr: &OcrRegionCache,

    frame_w: i32,

    frame_h: i32,

    pixels_bgra: Option<&[u8]>,

) {

    if frame_w < 80 || frame_h < 120 {

        return;

    }

    let Some(band) = keyboard_band(ocr, frame_h) else {

        return;

    };



    let mut have: std::collections::HashSet<char> = items

        .iter()

        .filter(|c| c.kind == CandidateKind::KeyboardKey && is_soft_key_glyph(&c.label))

        .filter_map(|c| c.label.trim().chars().next())

        .map(|c| c.to_ascii_lowercase())

        .collect();



    let caps = pixels_bgra

        .filter(|px| px.len() >= (frame_w * frame_h * 4) as usize)

        .map(|px| white_key_caps(px, frame_w, frame_h, &band))

        .unwrap_or_default();



    if caps.len() >= 20 {

        inject_from_caps(items, &caps, &mut have, frame_w);

    } else {

        inject_uniform_geometry(items, frame_w, &band, &mut have);

    }

    ensure_done_key(items, &band, frame_w, frame_h, ocr, pixels_bgra);

}



fn inject_from_caps(

    items: &mut Vec<Candidate>,

    caps: &[BBox],

    have: &mut std::collections::HashSet<char>,

    frame_w: i32,

) {

    let rows = cluster_key_rows(caps);

    for (ri, row_boxes) in rows.iter().enumerate() {

        let Some(labels) = QWERTY_ROWS.get(ri) else {

            break;

        };

        let expected = labels.chars().count();

        let snapped = snap_row_to_grid(row_boxes, expected, frame_w);

        for (i, ch) in labels.chars().enumerate() {

            let Some(b) = snapped.get(i) else {

                break;

            };

            upsert_soft_key(items, have, ch, b, "keyboard_cap", 0.88);

        }

    }

}



fn inject_uniform_geometry(

    items: &mut Vec<Candidate>,

    frame_w: i32,

    band: &KeyboardBand,

    have: &mut std::collections::HashSet<char>,

) {

    let body_h = (band.body_bottom - band.body_top).max(40);

    let row_h = body_h as f32 / QWERTY_ROWS.len() as f32;

    for (ri, row) in QWERTY_ROWS.iter().enumerate() {

        let y = band.body_top as f32 + row_h * (ri as f32 + 0.5);

        let n = row.chars().count() as f32;

        let margin = frame_w as f32 * 0.028;

        let usable = (frame_w as f32 - 2.0 * margin).max(40.0);

        let cell = usable / 10.0;

        let row_w = cell * n;

        let x0 = (frame_w as f32 - row_w) * 0.5;

        let half = (cell * 0.42).clamp(8.0, 22.0);

        for (i, ch) in row.chars().enumerate() {

            let cx = x0 + cell * (i as f32 + 0.5);

            let cy = y;

            let bbox = BBox {

                x0: (cx - half).round() as i32,

                y0: (cy - half).round() as i32,

                x1: (cx + half).round() as i32,

                y1: (cy + half).round() as i32,

            };

            // Always overwrite OCR glyph boxes — they drift on tiny soft keys.

            upsert_soft_key(items, have, ch, &bbox, "keyboard_geom", 0.72);

        }

    }

}



fn upsert_soft_key(

    items: &mut Vec<Candidate>,

    have: &mut std::collections::HashSet<char>,

    ch: char,

    bbox: &BBox,

    evidence: &str,

    score: f32,

) {

    let lower = ch.to_ascii_lowercase();

    if let Some(existing) = items.iter_mut().find(|c| {

        c.kind == CandidateKind::KeyboardKey && c.label.eq_ignore_ascii_case(&lower.to_string())

    }) {

        existing.bbox = bbox.clone();

        existing.visual_bbox = Some(bbox.clone());

        existing.hit_bbox = Some(bbox.clone());

        if !existing.evidence.iter().any(|e| e == evidence) {

            existing.evidence.push(evidence.into());

        }

        have.insert(lower);

        return;

    }

    let mut c = blank(

        CandidateKind::KeyboardKey,

        bbox.clone(),

        lower.to_string(),

        score,

        CandidateSource::Color,

        true,

    );

    c.visual_bbox = Some(bbox.clone());

    c.hit_bbox = Some(bbox.clone());

    c.evidence.push(evidence.into());

    c.evidence.push(format!("qwerty:{lower}"));

    items.push(c);

    have.insert(lower);

}



/// White rounded key caps: brighter than the gray sheet, low saturation.

fn pixel_is_key_cap(b: u8, g: u8, r: u8) -> bool {

    let (_h, s, v) = opencv_hsv(b, g, r);

    s <= KEY_CAP_S_MAX && v >= KEY_CAP_V_MIN

}



fn white_key_caps(bgra: &[u8], frame_w: i32, frame_h: i32, band: &KeyboardBand) -> Vec<BBox> {

    let w = frame_w as u32;

    let h = frame_h as u32;

    let x0 = ((frame_w as f32) * 0.02).round() as u32;

    let x1 = ((frame_w as f32) * 0.98).round() as u32;

    let y0 = band.body_top.max(0) as u32;

    let y1 = band.body_bottom.min(frame_h) as u32;

    if x1 <= x0 + 8 || y1 <= y0 + 8 {

        return Vec::new();

    }

    let mut mask = vec![0u8; (w * h) as usize];

    for y in y0..y1.min(h) {

        for x in x0..x1.min(w) {

            let i = ((y * w + x) * 4) as usize;

            if pixel_is_key_cap(bgra[i], bgra[i + 1], bgra[i + 2]) {

                mask[(y * w + x) as usize] = 1;

            }

        }

    }

    let mut seen = vec![0u8; mask.len()];

    let mut comps: Vec<(BBox, i32)> = Vec::new();

    let min_area = ((frame_w * frame_h) as f32 * 0.00035).clamp(80.0, 400.0) as i32;

    let max_area = ((frame_w * frame_h) as f32 * 0.012).clamp(800.0, 6000.0) as i32;

    for y in y0..y1.min(h) {

        for x in x0..x1.min(w) {

            let start = (y * w + x) as usize;

            if mask[start] == 0 || seen[start] == 1 {

                continue;

            }

            let mut stack = vec![(x, y)];

            seen[start] = 1;

            let mut min_x = x;

            let mut max_x = x;

            let mut min_y = y;

            let mut max_y = y;

            let mut area = 0i32;

            while let Some((cx, cy)) = stack.pop() {

                area += 1;

                min_x = min_x.min(cx);

                max_x = max_x.max(cx);

                min_y = min_y.min(cy);

                max_y = max_y.max(cy);

                for (nx, ny) in [

                    (cx.wrapping_sub(1), cy),

                    (cx + 1, cy),

                    (cx, cy.wrapping_sub(1)),

                    (cx, cy + 1),

                ] {

                    if nx < x0 || nx >= x1 || ny < y0 || ny >= y1 {

                        continue;

                    }

                    let ni = (ny * w + nx) as usize;

                    if mask[ni] == 0 || seen[ni] == 1 {

                        continue;

                    }

                    seen[ni] = 1;

                    stack.push((nx, ny));

                }

            }

            if area < min_area || area > max_area {

                continue;

            }

            let bw = (max_x - min_x + 1) as i32;

            let bh = (max_y - min_y + 1) as i32;

            if bw < 12 || bh < 12 {

                continue;

            }

            let aspect = bw as f32 / bh as f32;

            // Digit/letter caps are near-square; reject long merges / thin noise.

            if !(0.55..=2.2).contains(&aspect) {

                continue;

            }

            comps.push((

                BBox {

                    x0: min_x as i32,

                    y0: min_y as i32,

                    x1: (max_x + 1) as i32,

                    y1: (max_y + 1) as i32,

                },

                area,

            ));

        }

    }

    nms_caps(comps, CAP_NMS_IOU)

}



fn nms_caps(mut comps: Vec<(BBox, i32)>, thr: f32) -> Vec<BBox> {

    comps.sort_by_key(|(_, area)| std::cmp::Reverse(*area));

    let mut kept: Vec<BBox> = Vec::new();

    for (b, _) in comps {

        if kept.iter().any(|k| box_iou(&b, k) >= thr) {

            continue;

        }

        kept.push(b);

    }

    kept

}



fn cluster_key_rows(caps: &[BBox]) -> Vec<Vec<BBox>> {

    if caps.is_empty() {

        return Vec::new();

    }

    let mut sorted: Vec<BBox> = caps.to_vec();

    sorted.sort_by_key(|b| (b.cy(), b.cx()));

    let med_h = {

        let mut hs: Vec<i32> = sorted.iter().map(|b| b.h()).collect();

        hs.sort_unstable();

        hs[hs.len() / 2].max(16)

    };

    let row_tol = (med_h as f32 * 0.55).round() as i32;

    let mut rows: Vec<Vec<BBox>> = Vec::new();

    for b in sorted {

        if let Some(last) = rows.last_mut() {

            let row_cy = last.iter().map(|x| x.cy()).sum::<i32>() / last.len() as i32;

            if (b.cy() - row_cy).abs() <= row_tol {

                last.push(b);

                continue;

            }

        }

        rows.push(vec![b]);

    }

    for row in &mut rows {

        row.sort_by_key(|b| b.cx());

    }

    // Keep the four densest rows (digit + 3 letter rows); drop chrome leftovers.

    if rows.len() > 4 {

        rows.sort_by_key(|r| std::cmp::Reverse(r.len()));

        rows.truncate(4);

        rows.sort_by_key(|r| r.iter().map(|b| b.cy()).sum::<i32>() / r.len().max(1) as i32);

    }

    rows

}



/// Build `n` equal key boxes from observed row extent + median size.

/// Fixes broken/partial caps (holes, nested OCR leftovers) that would shift labels.

fn snap_row_to_grid(row_boxes: &[BBox], n: usize, frame_w: i32) -> Vec<BBox> {

    if n == 0 || row_boxes.is_empty() {

        return Vec::new();

    }

    let mut areas: Vec<i32> = row_boxes.iter().map(|b| b.w() * b.h()).collect();

    areas.sort_unstable();

    let med_a = areas[areas.len() / 2].max(1);

    let mut solid: Vec<&BBox> = row_boxes

        .iter()

        .filter(|b| b.w() * b.h() >= (med_a as f32 * 0.55) as i32)

        .collect();

    if solid.len() < 3 {

        solid = row_boxes.iter().collect();

    }

    // Z-row often picks up the white glyph inside the pink delete key as an
    // 8th cap. Spanning pitch to that blob stretches labels onto the wrong
    // keys (m → delete). Keep only the leftmost `n` solid caps.
    if solid.len() > n {

        solid.sort_by_key(|b| b.cx());

        solid.truncate(n);

    }



    let mut cys: Vec<i32> = solid.iter().map(|b| b.cy()).collect();

    let mut hs: Vec<i32> = solid.iter().map(|b| b.h()).collect();

    let mut ws: Vec<i32> = solid.iter().map(|b| b.w()).collect();

    cys.sort_unstable();

    hs.sort_unstable();

    ws.sort_unstable();

    let cy = cys[cys.len() / 2] as f32;

    let kh = hs[hs.len() / 2].max(12) as f32;

    let kw = ws[ws.len() / 2].max(12) as f32;



    let mut cxs: Vec<f32> = solid.iter().map(|b| b.cx() as f32).collect();

    cxs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));



    let (left_cx, pitch) = if cxs.len() >= n.saturating_sub(1).max(2) {

        let pitch = (cxs[cxs.len() - 1] - cxs[0]) / (n.saturating_sub(1).max(1) as f32);

        (cxs[0], pitch.max(kw * 0.8))

    } else {

        let mut pitches: Vec<f32> = cxs.windows(2).map(|w| w[1] - w[0]).collect();

        pitches.retain(|p| *p > kw * 0.6);

        pitches.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let pitch = if pitches.is_empty() {

            (frame_w as f32) / 10.0

        } else {

            pitches[pitches.len() / 2]

        };

        let left_cx = if cxs[0] < frame_w as f32 * 0.22 {

            cxs[0]

        } else {

            let row_w = pitch * (n.saturating_sub(1) as f32);

            (frame_w as f32 - row_w) * 0.5

        };

        (left_cx, pitch.max(kw * 0.8))

    };



    let half_w = kw * 0.48;

    let half_h = kh * 0.48;

    (0..n)

        .map(|i| {

            let cx = left_cx + i as f32 * pitch;

            BBox {

                x0: (cx - half_w).round() as i32,

                y0: (cy - half_h).round() as i32,

                x1: (cx + half_w).round() as i32,

                y1: (cy + half_h).round() as i32,

            }

        })

        .collect()

}



struct KeyboardBand {

    body_top: i32,

    body_bottom: i32,

    chrome_cy: i32,

}



fn keyboard_band(ocr: &OcrRegionCache, frame_h: i32) -> Option<KeyboardBand> {

    let y_min = (frame_h as f32 * 0.42) as i32;

    let cancel = ocr

        .hits()

        .iter()

        .filter(|h| h.y >= y_min && labels_match_exact(&h.text, "取消"))

        .max_by(|a, b| {

            a.score

                .partial_cmp(&b.score)

                .unwrap_or(std::cmp::Ordering::Equal)

        })?;



    let chrome_top = cancel.y;

    let chrome_h = cancel.h.max(18).saturating_mul(2).min(56);

    let chrome_cy = cancel.y + cancel.h / 2;

    let body_top = (chrome_top + chrome_h).min(frame_h - 80);

    let body_bottom = frame_h.saturating_sub((frame_h as f32 * 0.015) as i32);

    if body_bottom - body_top < 60 {

        return None;

    }

    Some(KeyboardBand {

        body_top,

        body_bottom,

        chrome_cy,

    })

}



fn ensure_done_key(
    items: &mut Vec<Candidate>,
    band: &KeyboardBand,
    frame_w: i32,
    frame_h: i32,
    ocr: &OcrRegionCache,
    pixels_bgra: Option<&[u8]>,
) {
    // 「完成」lives on the chrome right. OCR often returns only「完」on the left
    // of the word; growing that box leftward makes clicks miss. Prefer green
    // ink / right-aligned geometry (same idea as Python keyboard_done).
    let geom = right_chrome_done_box(band, frame_w);

    let green = pixels_bgra
        .filter(|px| px.len() >= (frame_w * frame_h * 4) as usize)
        .and_then(|px| green_done_blob(px, frame_w, frame_h, band));

    let ocr_hit = ocr
        .hits()
        .iter()
        .filter(|h| labels_match_exact(&h.text, "完成") && h.y >= band.body_top - 80)
        .max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    let (bbox, score, source, evidence): (BBox, f32, CandidateSource, &str) =
        if let Some(g) = green {
            (g, 0.9, CandidateSource::Color, "keyboard_done_green")
        } else if let Some(done) = ocr_hit {
            // Keep OCR vertical, but snap horizontally to the right chrome.
            let mut b = geom.clone();
            let mid_y = done.y + done.h.max(18) / 2;
            let half_h = b.h().max(18) / 2;
            b.y0 = (mid_y - half_h).max(0);
            b.y1 = mid_y + half_h;
            (b, done.score.max(0.8), CandidateSource::Ocr, "keyboard_done_ocr_right")
        } else {
            (geom, 0.7, CandidateSource::Color, "keyboard_done_vs_cancel")
        };

    if let Some(existing) = items
        .iter_mut()
        .find(|c| c.kind == CandidateKind::KeyboardKey && labels_match_exact(&c.label, "完成"))
    {
        existing.bbox = bbox.clone();
        existing.visual_bbox = Some(bbox.clone());
        existing.hit_bbox = Some(bbox);
        existing.score = existing.score.max(score);
        if !existing.evidence.iter().any(|e| e == evidence) {
            existing.evidence.push(evidence.into());
        }
        return;
    }

    let mut c = blank(
        CandidateKind::KeyboardKey,
        bbox.clone(),
        "完成".into(),
        score,
        source,
        true,
    );
    c.visual_bbox = Some(bbox.clone());
    c.hit_bbox = Some(bbox);
    c.evidence.push("keyboard".into());
    c.evidence.push(evidence.into());
    items.push(c);
}

fn right_chrome_done_box(band: &KeyboardBand, frame_w: i32) -> BBox {
    let w = ((frame_w as f32) * 0.14).clamp(40.0, 72.0) as i32;
    let h = 28;
    let x1 = frame_w.saturating_sub((frame_w as f32 * 0.03) as i32);
    let x0 = (x1 - w).max(0);
    let cy = band.chrome_cy;
    BBox {
        x0,
        y0: (cy - h / 2).max(0),
        x1,
        y1: cy + h / 2,
    }
}

/// Green「完成」ink in the keyboard chrome (right side).
fn green_done_blob(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    band: &KeyboardBand,
) -> Option<BBox> {
    let w = frame_w as u32;
    let h = frame_h as u32;
    let x0 = ((frame_w as f32) * 0.55).round() as u32;
    let x1 = w;
    let y0 = (band.chrome_cy - 28).max(0) as u32;
    let y1 = ((band.chrome_cy + 28) as u32).min(h);
    if x1 <= x0 + 8 || y1 <= y0 + 4 {
        return None;
    }
    let mut min_x = x1;
    let mut max_x = x0;
    let mut min_y = y1;
    let mut max_y = y0;
    let mut count = 0i32;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * w + x) * 4) as usize;
            if i + 3 >= bgra.len() {
                continue;
            }
            let (hh, s, v) = opencv_hsv(bgra[i], bgra[i + 1], bgra[i + 2]);
            // OpenCV H for green ≈ 35–95; toolbar green is saturated.
            if (35..=95).contains(&hh) && s >= 40 && v >= 70 {
                count += 1;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if count < 40 {
        return None;
    }
    let mut bbox = BBox {
        x0: min_x as i32,
        y0: min_y as i32,
        x1: (max_x + 1) as i32,
        y1: (max_y + 1) as i32,
    };
    // Expand to a usable two-glyph hit target, anchored on the right.
    let min_w = (bbox.h().max(14) as f32 * 2.6).round() as i32 + 10;
    if bbox.w() < min_w {
        bbox.x0 = (bbox.x1 - min_w).max(x0 as i32);
    }
    let pad_y = ((26 - bbox.h()) / 2).max(2);
    bbox.y0 = (bbox.y0 - pad_y).max(0);
    bbox.y1 = bbox.y1 + pad_y;
    Some(bbox)
}
