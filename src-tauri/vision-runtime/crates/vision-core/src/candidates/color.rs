//! Mint slot cells and captcha patches.

use crate::ocr::{labels_match_exact, OcrRegionCache};
use crate::slots::{parse_time_label, SlotState};

use super::parse::{blank, is_legend};
use super::types::{
    box_iou, BBox, Candidate, CandidateKind, CandidateSource, ColorRules, SlotMeta,
};

pub fn opencv_hsv(b: u8, g: u8, r: u8) -> (u8, u8, u8) {
    let rf = r as f32;
    let gf = g as f32;
    let bf = b as f32;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let v = max;
    let d = max - min;
    let s = if max <= 0.0 { 0.0 } else { d / max * 255.0 };
    let mut h = if d <= 0.0 {
        0.0
    } else if (max - rf).abs() < 0.5 {
        60.0 * (((gf - bf) / d) % 6.0)
    } else if (max - gf).abs() < 0.5 {
        60.0 * (((bf - rf) / d) + 2.0)
    } else {
        60.0 * (((rf - gf) / d) + 4.0)
    };
    if h < 0.0 {
        h += 360.0;
    }
    (
        (h / 2.0).round().clamp(0.0, 179.0) as u8,
        s.round().clamp(0.0, 255.0) as u8,
        v.round().clamp(0.0, 255.0) as u8,
    )
}

pub(super) fn in_range(v: u8, r: [u8; 2]) -> bool {
    v >= r[0] && v <= r[1]
}

/// Not the white card. Dark sport discs and colored logos both count.
pub fn pixel_is_icon_ink(b: u8, g: u8, r: u8) -> bool {
    let (_h, s, v) = opencv_hsv(b, g, r);
    !(s < 28 && v > 220)
}

/// Dark blue fill of the 「我知道了」 button. Distinct from the home-grid purple discs.
pub fn pixel_is_notice_blue(b: u8, g: u8, r: u8) -> bool {
    let (h, s, v) = opencv_hsv(b, g, r);
    (95..=130).contains(&h) && s >= 45 && (40..=220).contains(&v)
}

/// Home-grid disc under 「场馆预约」. Hue sits between blue and magenta so the
/// other colored circles on the same page do not count.
pub fn pixel_is_venue_purple(b: u8, g: u8, r: u8) -> bool {
    let (h, s, v) = opencv_hsv(b, g, r);
    (128..=158).contains(&h) && s >= 60 && v >= 110
}

pub fn pixel_is_mint(b: u8, g: u8, r: u8, rules: &ColorRules) -> bool {
    let (h, s, v) = opencv_hsv(b, g, r);
    let pale = in_range(h, rules.pale_h) && in_range(s, rules.pale_s) && in_range(v, rules.pale_v);
    let strong =
        in_range(h, rules.strong_h) && in_range(s, rules.strong_s) && in_range(v, rules.strong_v);
    pale || strong
}

pub(super) fn slot_cells_from_pixels(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    ocr: &OcrRegionCache,
    rules: &ColorRules,
) -> Vec<Candidate> {
    if frame_w <= 0 || frame_h <= 0 {
        return Vec::new();
    }
    let w = frame_w as u32;
    let h = frame_h as u32;
    if bgra.len() < (w * h * 4) as usize {
        return Vec::new();
    }
    let boxes = mint_components(bgra, w, h, rules);
    let times: Vec<(String, i32)> = ocr
        .hits()
        .iter()
        .filter_map(|hit| {
            let t = parse_time_label(&hit.text)?;
            if hit.y < (frame_h as f32 * 0.12) as i32 {
                return None;
            }
            Some((t, hit.cy()))
        })
        .collect();
    let legends: Vec<BBox> = ocr
        .hits()
        .iter()
        .filter(|hit| is_legend(&hit.text))
        .map(|hit| BBox {
            x0: hit.x,
            y0: hit.y,
            x1: hit.x + hit.w,
            y1: hit.y + hit.h,
        })
        .collect();

    let mut aligned: Vec<(BBox, String)> = Vec::new();
    for b in boxes {
        if legends
            .iter()
            .any(|g| box_iou(&b, g) >= 0.05 || point_in(b.cx(), b.cy(), g))
        {
            continue;
        }
        let Some((time, _)) = times.iter().min_by_key(|(_, ty)| (b.cy() - ty).abs()) else {
            continue;
        };
        let nearest = times
            .iter()
            .map(|(_, ty)| (b.cy() - ty).abs())
            .min()
            .unwrap_or(9999);
        if nearest > b.h().max(28) {
            continue;
        }
        aligned.push((b, time.clone()));
    }
    let mut xs: Vec<i32> = aligned.iter().map(|(b, _)| b.cx()).collect();
    xs.sort_unstable();
    xs.dedup();
    let mut cols: Vec<i32> = Vec::new();
    for x in xs {
        if cols.last().is_some_and(|p| (x - p).abs() < 24) {
            continue;
        }
        cols.push(x);
    }
    aligned
        .into_iter()
        .map(|(b, time)| {
            let court_index = cols
                .iter()
                .position(|c| (b.cx() - c).abs() < 24)
                .map(|i| i as i32);
            let mut c = blank(
                CandidateKind::SlotCell,
                b.clone(),
                format!("slot:{time}"),
                0.9,
                CandidateSource::Color,
                true,
            );
            c.slot_meta = Some(SlotMeta {
                start_time: time.clone(),
                court_index,
                court_label: court_index.map(|i| format!("{i:02}")),
                state: SlotState::Available,
            });
            c.evidence.push(format!("mint_aligned:{time}"));
            c
        })
        .collect()
}

pub(super) fn point_in(x: i32, y: i32, b: &BBox) -> bool {
    x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1
}

pub(super) fn mint_components(bgra: &[u8], w: u32, h: u32, rules: &ColorRules) -> Vec<BBox> {
    let x0 = (w as f32 * 0.12) as u32;
    let y0 = (h as f32 * 0.12) as u32;
    let y1 = (h as f32 * 0.78) as u32;
    let mut mask = vec![0u8; (w * h) as usize];
    for y in y0..y1.min(h) {
        for x in x0..w {
            let i = ((y * w + x) * 4) as usize;
            if pixel_is_mint(bgra[i], bgra[i + 1], bgra[i + 2], rules) {
                mask[(y * w + x) as usize] = 1;
            }
        }
    }
    let mut seen = vec![0u8; mask.len()];
    let mut out = Vec::new();
    for y in y0..y1.min(h) {
        for x in x0..w {
            let start = (y * w + x) as usize;
            if mask[start] == 0 || seen[start] == 1 {
                continue;
            }
            let mut st = vec![(x, y)];
            seen[start] = 1;
            let mut minx = x;
            let mut miny = y;
            let mut maxx = x;
            let mut maxy = y;
            let mut area = 0u32;
            while let Some((cx, cy)) = st.pop() {
                area += 1;
                minx = minx.min(cx);
                miny = miny.min(cy);
                maxx = maxx.max(cx);
                maxy = maxy.max(cy);
                for (dx, dy) in [(1i32, 0), (-1, 0), (0, 1), (0, -1)] {
                    let nx = cx as i32 + dx;
                    let ny = cy as i32 + dy;
                    if nx < x0 as i32 || ny < y0 as i32 || nx >= w as i32 || ny >= y1 as i32 {
                        continue;
                    }
                    let ni = (ny as u32 * w + nx as u32) as usize;
                    if mask[ni] == 1 && seen[ni] == 0 {
                        seen[ni] = 1;
                        st.push((nx as u32, ny as u32));
                    }
                }
            }
            let bw = maxx - minx + 1;
            let bh = maxy - miny + 1;
            if area < rules.min_area || bw < 8 || bh < 8 || bw > 220 || bh > 80 {
                continue;
            }
            out.push(BBox {
                x0: minx as i32,
                y0: miny as i32,
                x1: (maxx + 1) as i32,
                y1: (maxy + 1) as i32,
            });
            if out.len() >= 64 {
                return out;
            }
        }
    }
    out
}

/// Captcha image + one input band to its right (not four underline boxes).
///
/// Flow (docs/research/wechat-booking-flow.md · Python `find_captcha_row`):
/// ```text
/// [验证码]  [noisy tile]  [======== input band ========]
///           ^-- variance    ^-- same height as tile; click here
/// ```
/// Automation types all 4 keys then「完成」then「确定预约」— no per-slot
/// geometry. Progress = OCR / ink count inside the band.
#[derive(Debug, Clone)]
pub struct CaptchaInputState {
    pub image: BBox,
    /// Whole input zone right of the tile (height = tile height).
    pub input_region: BBox,
    pub filled_count: usize,
    /// Length == filled_count; next soft-key index.
    pub typed_proxy: String,
    /// Click target inside `input_region` (upper half — not the underline).
    pub input_click: BBox,
}

/// Locate captcha tile + input band; estimate how many chars are already typed.
pub fn analyze_captcha_input(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    ocr: &OcrRegionCache,
) -> Option<CaptchaInputState> {
    let label = ocr
        .hits()
        .iter()
        .find(|h| labels_match_exact(&h.text, "验证码"))?;
    let lb = BBox {
        x0: label.x,
        y0: label.y,
        x1: label.x + label.w,
        y1: label.y + label.h,
    };
    // The captcha is the only pale-blue rectangle on the label's horizontal
    // strip. Search that whole strip first. A variance seed can land on a
    // middle glyph (as it did with XGEC), which makes its right-hand letters
    // look like input and falsely reports a character as already typed.
    let strip = BBox {
        x0: (lb.x1 + 4).clamp(0, frame_w - 1),
        y0: (lb.y0 - lb.h()).max(0),
        x1: frame_w,
        y1: (lb.y1 + lb.h()).min(frame_h),
    };
    let image = match refine_captcha_tinted_bounds(bgra, frame_w, frame_h, &strip) {
        Some(image) => image,
        None => {
            let mut fallback = find_captcha_patch(bgra, frame_w, frame_h, &lb)?;
            // The variance fallback is deliberately wider than the captcha.
            fallback.x1 = refine_captcha_right_edge(bgra, frame_w, frame_h, &fallback);
            fallback
        }
    };
    // The input is the complete strip beside the image, at the image's height.
    // Its right edge is the mini-program viewport edge; underline slots are only
    // decoration inside this strip, not separate targets.
    let gap = ((image.h() as f32) * 0.04).clamp(1.0, 3.0) as i32;
    let input_region = BBox {
        x0: (image.x1 + gap).min(frame_w - 8),
        y0: image.y0,
        x1: frame_w,
        y1: image.y1.max(image.y0 + 8),
    };
    // Full-frame OCR often misses tiny slot glyphs (keyboard covers most of the
    // page). Prefer max(OCR-in-band, ink columns) — same idea as Python
    // read_captcha_input_text + input_looks_filled. Host may further replace
    // typed_proxy with a dedicated upscaled crop OCR of `input_region`.
    let ocr_text = typed_chars_in_region(ocr, &input_region).unwrap_or_default();
    let ink = count_input_glyphs(bgra, frame_w, frame_h, &input_region);
    let typed_proxy = merge_typed_proxy(&ocr_text, ink);
    let filled_count = typed_proxy.chars().count().min(4);
    // Click upper-mid of the band (docs: 下划线上方，不要点线).
    let pad_x = (input_region.w() as f32 * 0.15).clamp(4.0, 16.0) as i32;
    let pad_y = (input_region.h() as f32 * 0.12).clamp(2.0, 8.0) as i32;
    let input_click = BBox {
        x0: input_region.x0 + pad_x,
        y0: input_region.y0 + pad_y,
        x1: (input_region.x1 - pad_x).max(input_region.x0 + pad_x + 8),
        y1: input_region.y0 + (input_region.h() as f32 * 0.55).max(10.0) as i32,
    };
    Some(CaptchaInputState {
        image,
        input_region,
        filled_count,
        typed_proxy,
        input_click,
    })
}

/// Tighten the broad variance seed to the captcha's pale-blue background.
///
/// On the real page the tile is easy to distinguish from the surrounding paper:
/// its blue channel is visibly above its red channel even where no glyph is
/// present. Looking for that fill, rather than the glyph variance alone, avoids
/// adopting the white gap to the left as part of the captcha image.
fn refine_captcha_tinted_bounds(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    seed: &BBox,
) -> Option<BBox> {
    let w = frame_w as u32;
    let h = frame_h as u32;
    if bgra.len() < (w * h * 4) as usize || seed.w() < 20 || seed.h() < 12 {
        return None;
    }
    // The seed may start before the tile. Extend right by a full seed width:
    // a wide captcha can otherwise have its right-hand characters cut off by
    // the variance window before the pale-blue boundary is measured.
    let x0 = seed.x0.max(0) as u32;
    let x1 = (seed.x1 + seed.w()).clamp(seed.x0 + 1, frame_w) as u32;
    let y0 = seed.y0.max(0) as u32;
    let y1 = seed.y1.clamp(seed.y0 + 1, frame_h) as u32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let height = (y1 - y0) as usize;
    let width = (x1 - x0) as usize;
    let mut tinted_cols = vec![false; width];
    for x in x0..x1 {
        let mut n = 0usize;
        for y in y0..y1 {
            let i = ((y * w + x) * 4) as usize;
            let b = bgra[i] as i16;
            let g = bgra[i + 1] as i16;
            let r = bgra[i + 2] as i16;
            // Pale captcha fill is approximately #f3fbfe; paper-white is #fff.
            if b - r >= 5 && g - r >= 2 && b >= 180 {
                n += 1;
            }
        }
        // The label's OCR box may make this search strip taller than the tile.
        // A glyph can also obscure most pixels in a column. A quarter of the
        // strip still gives a reliable pale-blue signal while paper-white has
        // no matching pixels at all.
        tinted_cols[(x - x0) as usize] = n * 4 >= height;
    }
    // A captcha glyph can cover an entire narrow column of the pale-blue fill.
    // Treat such a short interruption as a hole in one rectangular tile, not as
    // an image boundary. The input band is paper-white, so it cannot be joined
    // by this rule.
    let max_glyph_gap = (seed.h() / 2).clamp(12, 28) as usize;
    let mut best = None;
    let mut start = 0usize;
    while start < width {
        while start < width && !tinted_cols[start] {
            start += 1;
        }
        let run_start = start;
        let mut end = start;
        let tile_end = loop {
            while end < width && tinted_cols[end] {
                end += 1;
            }
            let tile_end = end;
            let gap_start = end;
            while end < width && !tinted_cols[end] {
                end += 1;
            }
            // No later pale-blue fill, or too wide to be a glyph occlusion.
            if end == width || end - gap_start > max_glyph_gap {
                break tile_end;
            }
        };
        if tile_end.saturating_sub(run_start) >= 18
            && best
                .as_ref()
                .is_none_or(|(best_start, best_end)| {
                    tile_end - run_start > best_end - best_start
                })
        {
            best = Some((run_start, tile_end));
        }
        start = end;
    }
    let (left, right) = best?;
    let mut top = height;
    let mut bottom = 0usize;
    for y in y0..y1 {
        let mut n = 0usize;
        for x in (x0 + left as u32)..(x0 + right as u32) {
            let i = ((y * w + x) * 4) as usize;
            let b = bgra[i] as i16;
            let g = bgra[i + 1] as i16;
            let r = bgra[i + 2] as i16;
            if b - r >= 5 && g - r >= 2 && b >= 180 {
                n += 1;
            }
        }
        // A glyph may cover a portion of the row too; retain the rectangle's
        // top/bottom as long as most of its pale-blue backing remains visible.
        if n * 2 >= right - left {
            top = top.min((y - y0) as usize);
            bottom = bottom.max((y - y0 + 1) as usize);
        }
    }
    (bottom.saturating_sub(top) >= 12).then_some(BBox {
        x0: (x0 as usize + left) as i32,
        y0: (y0 as usize + top) as i32,
        x1: (x0 as usize + right) as i32,
        y1: (y0 as usize + bottom) as i32,
    })
}

pub(super) fn captcha_row_candidates(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    ocr: &OcrRegionCache,
) -> Vec<Candidate> {
    let Some(state) = analyze_captcha_input(bgra, frame_w, frame_h, ocr) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(3);
    // Orange markers (actionable=false): captcha tile + input band — what this step sees.
    let mut img = blank(
        CandidateKind::Other,
        state.image.clone(),
        "captcha_image".into(),
        0.7,
        CandidateSource::Color,
        false,
    );
    img.visual_bbox = Some(state.image.clone());
    img.evidence.push("captcha_tile".into());
    out.push(img);
    let mut band = blank(
        CandidateKind::Other,
        state.input_region.clone(),
        "captcha_input_band".into(),
        0.55,
        CandidateSource::Color,
        false,
    );
    band.visual_bbox = Some(state.input_region.clone());
    band.evidence.push("input_band".into());
    out.push(band);
    // Green next (when selected): click inside the input band to open the keyboard.
    let mut c = blank(
        CandidateKind::CaptchaHit,
        state.input_click.clone(),
        "captcha_input".into(),
        0.82,
        CandidateSource::Color,
        true,
    );
    c.visual_bbox = Some(state.input_click.clone());
    c.hit_bbox = Some(state.input_click.clone());
    c.evidence.push("right_of_captcha_image".into());
    c.evidence
        .push(format!("captcha_filled:{}", state.filled_count));
    out.push(c);
    out
}

/// Walk right from the tile seed until mid-band columns look paper-white (input side).
fn refine_captcha_right_edge(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    image: &BBox,
) -> i32 {
    let w = frame_w as u32;
    let h = frame_h as u32;
    if bgra.len() < (w * h * 4) as usize || image.w() < 20 {
        return image.x1;
    }
    let y0 = (image.y0 + image.h() / 6).max(0) as u32;
    let y1 = (image.y1 - image.h() / 6)
        .max(image.y0 + 4)
        .clamp(0, frame_h) as u32;
    if y1 <= y0 {
        return image.x1;
    }
    let min_w = (image.h().max(24) as f32 * 0.9) as i32;
    let x_lo = (image.x0 + min_w).clamp(image.x0 + 8, frame_w - 1);
    let x_hi = image.x1.min(frame_w);
    let mut cut = image.x1;
    let mut white_run = 0i32;
    for xx in x_lo..x_hi {
        let mut sum = 0.0f32;
        let mut sum2 = 0.0f32;
        let mut n = 0.0f32;
        for yy in y0..y1 {
            let i = ((yy * w + xx as u32) * 4) as usize;
            let g =
                0.114 * bgra[i] as f32 + 0.587 * bgra[i + 1] as f32 + 0.299 * bgra[i + 2] as f32;
            sum += g;
            sum2 += g * g;
            n += 1.0;
        }
        let mean = sum / n.max(1.0);
        let std = ((sum2 / n.max(1.0)) - mean * mean).max(0.0).sqrt();
        // Paper-white / low variance = left edge of the input band.
        if mean >= 220.0 && std < 18.0 {
            white_run += 1;
            if white_run >= 3 {
                cut = xx - white_run + 1;
                break;
            }
        } else {
            white_run = 0;
        }
    }
    cut.max(image.x0 + min_w).min(image.x1)
}

/// Merge full-frame band OCR with ink-column count. Progress must not stick
/// when OCR misses a newly typed glyph but ink already shows more columns.
pub fn merge_typed_proxy(ocr_text: &str, ink: usize) -> String {
    let ocr: String = ocr_text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .take(4)
        .collect();
    let ocr_n = ocr.chars().count();
    let n = ocr_n.max(ink).min(4);
    if n == 0 {
        return String::new();
    }
    if ocr_n >= ink && ocr_n > 0 {
        ocr.chars().take(n).collect()
    } else {
        (0..n).map(|_| 'X').collect()
    }
}

/// Alphanumeric OCR hits whose center sits inside the input band.
fn typed_chars_in_region(ocr: &OcrRegionCache, region: &BBox) -> Option<String> {
    let mut chars = String::new();
    for hit in ocr.hits() {
        let cx = hit.x + hit.w / 2;
        let cy = hit.y + hit.h / 2;
        if cx < region.x0 || cx > region.x1 || cy < region.y0 || cy > region.y1 {
            continue;
        }
        if labels_match_exact(&hit.text, "验证码") {
            continue;
        }
        for ch in hit.text.chars() {
            if ch.is_ascii_alphanumeric() {
                chars.push(ch.to_ascii_uppercase());
                if chars.chars().count() >= 4 {
                    return Some(chars);
                }
            }
        }
    }
    if chars.is_empty() {
        None
    } else {
        Some(chars)
    }
}

/// Ink blobs in the input band ≈ typed character count (when OCR misses them).
pub fn count_input_glyphs(bgra: &[u8], frame_w: i32, frame_h: i32, region: &BBox) -> usize {
    let w = frame_w as u32;
    let h = frame_h as u32;
    if bgra.len() < (w * h * 4) as usize || region.w() < 8 || region.h() < 8 {
        return 0;
    }
    let x0 = region.x0.max(0) as u32;
    let x1 = region.x1.clamp(region.x0 + 1, frame_w) as u32;
    let y0 = region.y0.max(0) as u32;
    let y1 = (region.y0 + (region.h() as f32 * 0.7) as i32)
        .clamp(region.y0 + 1, frame_h) as u32;
    if x1 <= x0 || y1 <= y0 {
        return 0;
    }
    let mut ink_col = vec![false; (x1 - x0) as usize];
    for xx in x0..x1 {
        let mut dark = 0i32;
        for yy in y0..y1 {
            let i = ((yy * w + xx) * 4) as usize;
            let g =
                0.114 * bgra[i] as f32 + 0.587 * bgra[i + 1] as f32 + 0.299 * bgra[i + 2] as f32;
            // Match Python input_looks_filled (gray < 120); 100 was too strict.
            if g < 120.0 {
                dark += 1;
            }
        }
        ink_col[(xx - x0) as usize] = dark >= 2;
    }
    let mut n = 0usize;
    let mut i = 0usize;
    while i < ink_col.len() {
        if !ink_col[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < ink_col.len() && ink_col[i] {
            i += 1;
        }
        if i - start >= 3 {
            n += 1;
            if n >= 4 {
                break;
            }
        }
    }
    n
}

/// Captcha tile = first high-variance block right of「验证码」(Python find_captcha_row).
pub fn find_captcha_patch(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    label: &BBox,
) -> Option<BBox> {
    let w = frame_w as u32;
    let h = frame_h as u32;
    if bgra.len() < (w * h * 4) as usize {
        return None;
    }
    let y0 = (label.y0 - label.h()).max(0) as u32;
    let y1 = ((label.y1 + label.h()) as u32).min(h);
    let x_start = (label.x1 + 4).clamp(0, frame_w - 1) as u32;
    // Python: win_w = max(70, min(label.w * 2.8, 150))
    let win_w = ((label.w() as f32) * 2.8)
        .clamp(70.0, 150.0)
        .min((w.saturating_sub(x_start)) as f32) as u32;
    if y1 <= y0 || win_w < 24 || x_start + win_w > w {
        return None;
    }
    let strip_w = w - x_start;
    let search_right = (win_w * 2)
        .max((strip_w as f32 * 0.55) as u32)
        .min(strip_w.saturating_sub(win_w));
    let mut best = -1.0f32;
    let mut best_x = 0u32;
    let step = (win_w / 10).max(2);
    let mut xx = 0u32;
    while xx < search_right {
        let std = patch_std(bgra, w, x_start + xx, y0, win_w, y1 - y0);
        let mean = patch_mean(bgra, w, x_start + xx, y0, win_w, y1 - y0);
        if (120.0..250.0).contains(&mean) {
            let score = std - xx as f32 * 0.02;
            if score > best {
                best = score;
                best_x = xx;
            }
        }
        xx += step;
    }
    if best < 8.0 {
        return Some(BBox {
            x0: label.x1 + (label.w() / 4).max(8),
            y0: label.y0 - label.h() / 3,
            x1: label.x1 + (label.w() as f32 * 3.0) as i32,
            y1: label.y1 + label.h() / 3,
        });
    }
    Some(BBox {
        x0: (x_start + best_x) as i32,
        y0: y0 as i32,
        x1: (x_start + best_x + win_w) as i32,
        y1: y1 as i32,
    })
}

pub(super) fn patch_mean(bgra: &[u8], stride: u32, x: u32, y: u32, w: u32, h: u32) -> f32 {
    let mut sum = 0.0f32;
    let mut n = 0.0f32;
    for yy in y..y + h {
        for xx in x..x + w {
            let i = ((yy * stride + xx) * 4) as usize;
            let g =
                0.114 * bgra[i] as f32 + 0.587 * bgra[i + 1] as f32 + 0.299 * bgra[i + 2] as f32;
            sum += g;
            n += 1.0;
        }
    }
    if n == 0.0 {
        0.0
    } else {
        sum / n
    }
}

pub(super) fn patch_std(bgra: &[u8], stride: u32, x: u32, y: u32, w: u32, h: u32) -> f32 {
    let mean = patch_mean(bgra, stride, x, y, w, h);
    let mut acc = 0.0f32;
    let mut n = 0.0f32;
    for yy in y..y + h {
        for xx in x..x + w {
            let i = ((yy * stride + xx) * 4) as usize;
            let g =
                0.114 * bgra[i] as f32 + 0.587 * bgra[i + 1] as f32 + 0.299 * bgra[i + 2] as f32;
            let d = g - mean;
            acc += d * d;
            n += 1.0;
        }
    }
    if n == 0.0 {
        0.0
    } else {
        (acc / n).sqrt()
    }
}
