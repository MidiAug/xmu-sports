//! Shell, panel, and search template policy.
//! Pixel scores live in `ncc`.

use crate::ocr::{labels_match_exact, OcrRegionCache};

#[cfg(test)]
pub(super) use super::ncc::resize_gray;
use super::ncc::{
    best_coarse_at_scale, down4, refine_signed, region_gray, sad_score, sidebar_gray, COARSE_FACTOR,
};
use super::needles::{
    bundled_miniprogram_templates, bundled_mp_search_templates, bundled_xmu_templates,
    SHELL_NCC_MIN,
};
use super::parse::{blank, stamp_shell_identity};
use super::color::{pixel_is_icon_ink, pixel_is_notice_blue, pixel_is_venue_purple};
use super::types::{
    BBox, Candidate, CandidateKind, CandidateSource, ShellVisualState, TemplateNeedle,
    ELEMENT_AUTH_LOGIN, ELEMENT_NOTICE_ACK, ELEMENT_SPORT_ICON, ELEMENT_VENUE_BOOKING,
    ELEMENT_XMU_SPORTS,
};

pub(super) fn match_templates(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    templates: &[TemplateNeedle],
    games_y: Option<i32>,
    panel_chrome: bool,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    // One sidebar pass for every Discover theme.  Four separate full scans were
    // the multi-second cost; light and dark still compete on signed correlation.
    let compass: Vec<&TemplateNeedle> = templates
        .iter()
        .filter(|t| t.id.starts_with("discover_compass_"))
        .collect();
    if let Some((index, bbox, score)) = match_compass_family(bgra, frame_w, frame_h, &compass) {
        let t = compass[index];
        let mut c = blank(
            CandidateKind::Icon,
            bbox,
            t.label.clone(),
            score,
            CandidateSource::Template,
            score >= SHELL_NCC_MIN,
        );
        c.template_id = Some(t.id.clone());
        c.evidence.push(format!("template:{}", t.id));
        stamp_shell_identity(&mut c);
        out.push(c);
    }
    // Spec §5.1 / §6.0: only look for the list row after Discover is Active.
    // Chat-list avatars otherwise can outscore a glyph needle and rename the page.
    let discover_active = out.iter().any(|c| {
        c.shell_alias() == Some("发现") && c.shell_visual_state() == Some(ShellVisualState::Active)
    });
    if discover_active {
        let mp = bundled_miniprogram_templates();
        let mp_refs: Vec<&TemplateNeedle> = mp.iter().collect();
        if let Some((index, visual, score, _, _)) =
            match_miniprogram_family(bgra, frame_w, frame_h, &mp_refs)
        {
            let t = &mp[index];
            let mut hit = miniprogram_element_box(&visual);
            hit.x0 = hit.x0.max(0);
            hit.y0 = hit.y0.max(0);
            hit.x1 = hit.x1.min(frame_w);
            hit.y1 = hit.y1.min(frame_h);
            let mut c = blank(
                CandidateKind::ListRow,
                hit.clone(),
                t.label.clone(),
                score,
                CandidateSource::Template,
                true,
            );
            c.template_id = Some(t.id.clone());
            c.visual_bbox = Some(visual);
            c.hit_bbox = Some(hit);
            c.evidence.push(format!("template:{}", t.id));
            stamp_shell_identity(&mut c);
            out.push(c);
        }
    }
    // Panel icons after the mini-program tab is selected, or when OCR already
    // shows panel chrome (Recent / Favorites). The games strip is below 「小游戏」.
    let mp_active = out.iter().any(|c| {
        c.shell_alias() == Some("小程序")
            && c.shell_visual_state() == Some(ShellVisualState::Active)
    });
    if mp_active || panel_chrome {
        push_panel_anchors(&mut out, bgra, frame_w, frame_h, games_y);
    }
    for t in templates {
        if t.id.starts_with("discover_compass_")
            || t.id.starts_with("miniprogram_entry_")
            || t.id.starts_with("xmu_sports_icon")
            || t.id.starts_with("mp_search")
        {
            continue;
        }
        if let Some((bbox, score)) = match_one(bgra, frame_w, frame_h, t) {
            // Compass hits are accepted at the shell correlation floor.  SAD
            // templates keep the stricter 0.82 bar.
            let actionable = if t.id.starts_with("discover_compass") {
                score >= SHELL_NCC_MIN
            } else {
                score >= 0.82
            };
            let mut c = blank(
                CandidateKind::Icon,
                bbox,
                t.label.clone(),
                score,
                CandidateSource::Template,
                actionable,
            );
            c.template_id = Some(t.id.clone());
            c.evidence.push(format!("template:{}", t.id));
            stamp_shell_identity(&mut c);
            out.push(c);
        }
    }
    // Score every needle first.  Then keep one winner per control: the four
    // Discover themes are one family (their ids do not end in `_active`),
    // and other `_active` / `_inactive` pairs still collapse the same way.
    let mut best_state = std::collections::BTreeMap::<&str, usize>::new();
    let mut keep = vec![true; out.len()];
    for (idx, c) in out.iter().enumerate() {
        let Some(id) = c.template_id.as_deref() else {
            continue;
        };
        let Some(base) = shell_template_family(id) else {
            continue;
        };
        if let Some(prior) = best_state.get(base).copied() {
            if out[prior].score >= c.score {
                keep[idx] = false;
            } else {
                keep[prior] = false;
                best_state.insert(base, idx);
            }
        } else {
            best_state.insert(base, idx);
        }
    }
    out.into_iter()
        .enumerate()
        .filter_map(|(idx, c)| keep[idx].then_some(c))
        .collect()
}

pub(super) fn match_one(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    t: &TemplateNeedle,
) -> Option<(BBox, f32)> {
    if t.id.starts_with("discover_compass_") {
        return match_shell_template_ncc(bgra, frame_w, frame_h, t);
    }
    let w = frame_w as u32;
    let h = frame_h as u32;
    if t.width == 0 || t.height == 0 || t.width >= w || t.height >= h {
        return None;
    }
    if bgra.len() < (w * h * 4) as usize || t.gray.len() != (t.width * t.height) as usize {
        return None;
    }
    let step = if w * h > 200_000 { 4 } else { 1 };
    // Miniprogram lives in the discover list strip, not the chat pane.
    let x_lo = if t.id.starts_with("miniprogram_entry") {
        w / 12
    } else {
        0
    };
    let x_hi = if t.id.starts_with("miniprogram_entry") {
        (w * 45 / 100).max(x_lo + t.width + 1)
    } else {
        w
    };
    let mut best = 0.0f32;
    let mut best_xy = (0u32, 0u32);
    let mut y = 0u32;
    while y + t.height < h {
        let mut x = x_lo;
        while x + t.width < x_hi.min(w) {
            let score = sad_score(bgra, w, x, y, t);
            if score > best {
                best = score;
                best_xy = (x, y);
            }
            x += step;
        }
        y += step;
    }
    if best < 0.82 {
        return None;
    }
    Some((
        BBox {
            x0: best_xy.0 as i32,
            y0: best_xy.1 as i32,
            x1: (best_xy.0 + t.width) as i32,
            y1: (best_xy.1 + t.height) as i32,
        },
        best,
    ))
}

pub(super) const COMPASS_SCALES: [f32; 6] = [0.6, 0.8, 1.0, 1.2, 1.4, 1.6];

#[derive(Clone, Copy)]
pub(super) struct CompassHint {
    x: u32,
    y: u32,
}

pub(super) fn compass_hint() -> &'static std::sync::Mutex<Option<CompassHint>> {
    static HINT: std::sync::Mutex<Option<CompassHint>> = std::sync::Mutex::new(None);
    &HINT
}

/// List strip to the right of the Discover compass, left of the feed.
pub(super) fn discover_list_roi(frame_w: u32, frame_h: u32) -> Option<(u32, u32, u32, u32)> {
    if frame_w < 96 || frame_h < 96 {
        return None;
    }
    let x0 = (frame_w / 8).clamp(48, 140);
    let x1 = (frame_w * 38 / 100).max(x0.saturating_add(64)).min(frame_w);
    let y0 = (frame_h as f32 * 0.08) as u32;
    let y1 = (frame_h as f32 * 0.94) as u32;
    if x1 <= x0.saturating_add(16) || y1 <= y0.saturating_add(16) {
        return None;
    }
    Some((x0, x1, y0, y1))
}

/// Icon plus the 「小程序」 label, in the needle's coordinate system.
///
/// All four reference crops measure the same ink: the glyph is 34×34 and
/// sits at (7,7) inside the 48×48 needle; the label ends at x=144, y=41.
/// The search ROI is not part of this box.
pub(super) fn miniprogram_element_box(visual: &BBox) -> BBox {
    let at = |origin: i32, span: i32, px: i32| origin + (span * px + 24) / 48;
    BBox {
        x0: at(visual.x0, visual.w(), 7),
        y0: at(visual.y0, visual.h(), 7),
        x1: at(visual.x0, visual.w(), 144),
        y1: at(visual.y0, visual.h(), 41),
    }
}

/// One coarse pass over the Discover list, then signed correlation of the
/// four glyph needles. Same pipeline as the compass: polarity is not folded,
/// so the inverted theme loses.
pub(super) fn match_miniprogram_family(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    needles: &[&TemplateNeedle],
) -> Option<(usize, BBox, f32, i32, i32)> {
    if needles.is_empty() || frame_w <= 0 || frame_h <= 0 {
        return None;
    }
    let (w, h) = (frame_w as u32, frame_h as u32);
    if bgra.len() < (w * h * 4) as usize {
        return None;
    }
    let (x0, x1, y0, y1) = discover_list_roi(w, h)?;
    let width = x1 - x0;
    let height = y1 - y0;
    let gray = region_gray(bgra, w, x0, width, y0, height);
    let (small, sw, sh) = down4(&gray, width, height);
    if sw < 4 || sh < 4 {
        return None;
    }
    let mut best: Option<(usize, BBox, f32)> = None;
    for (index, needle) in needles.iter().enumerate() {
        for scale in COMPASS_SCALES {
            let Some((cx, cy)) = best_coarse_at_scale(&small, sw, sh, needle, scale, 0, sw, 0, sh)
            else {
                continue;
            };
            let tw = (needle.width as f32 * scale).round().max(12.0) as u32;
            let th = (needle.height as f32 * scale).round().max(12.0) as u32;
            if tw + 1 >= x1 || y0.saturating_add(th) >= y1 {
                continue;
            }
            let full_x = x0
                .saturating_add(cx * COARSE_FACTOR)
                .min(x1.saturating_sub(tw + 1));
            let full_y = y0
                .saturating_add(cy * COARSE_FACTOR)
                .min(y1.saturating_sub(th + 1));
            let Some((bbox, score)) =
                refine_signed(bgra, w, h, x1, y0, y1, needle, tw, th, full_x, full_y)
            else {
                continue;
            };
            if best.as_ref().map_or(true, |(_, _, prior)| score > *prior) {
                best = Some((index, bbox, score));
            }
            if score >= 0.95 {
                break;
            }
        }
    }
    let (index, bbox, score) = best.filter(|(_, _, score)| *score >= SHELL_NCC_MIN)?;
    Some((index, bbox, score, x0 as i32, x1 as i32))
}

pub(super) fn games_header_y(ocr: &OcrRegionCache) -> Option<i32> {
    ocr.hits()
        .iter()
        .filter(|h| labels_match_exact(&h.text, "小游戏"))
        .map(|h| h.y)
        .min()
}

pub(super) fn panel_chrome(ocr: &OcrRegionCache) -> bool {
    ocr.hits()
        .iter()
        .any(|h| labels_match_exact(&h.text, "最近使用") || labels_match_exact(&h.text, "我的常用"))
}

pub(super) fn search_result_chrome(ocr: &OcrRegionCache) -> bool {
    ocr.hits().iter().any(|h| {
        labels_match_exact(&h.text, "使用过的小程序") || labels_match_exact(&h.text, "搜索小程序")
    })
}

/// OCR only saw the caption. Grow the click box to the crest (above) or the
/// search-row crest (left) so Navigate is not a four-character strip.
pub(super) fn promote_xmu_caption_tiles(
    items: &mut Vec<Candidate>,
    frame_w: i32,
    frame_h: i32,
    games_y: Option<i32>,
    panel: bool,
    search_row: bool,
) {
    if frame_w <= 0 || frame_h <= 0 || (!panel && !search_row) {
        return;
    }
    let roi = if search_row {
        // Dropdown sits in the upper panel; do not cut at 小游戏.
        mp_panel_content_roi(frame_w as u32, frame_h as u32, None)
    } else {
        mp_panel_content_roi(frame_w as u32, frame_h as u32, games_y)
    };
    let Some((rx0, rx1, ry0, ry1)) = roi else {
        return;
    };

    for c in items.iter_mut() {
        if !c.actionable || !labels_match_exact(&c.label, "厦大体育") {
            continue;
        }
        if c.label.contains("小程序") || c.label.contains("场馆") {
            continue;
        }
        // Template tiles already carry crest geometry.
        if c.template_id
            .as_deref()
            .is_some_and(|id| id.starts_with("xmu_sports_icon"))
            && c.visual_bbox.is_some()
            && c.hit_bbox.is_some()
        {
            continue;
        }
        let b = c.bbox.clone();
        if b.cx() < rx0 as i32 || b.cx() > rx1 as i32 || b.cy() < ry0 as i32 || b.cy() > ry1 as i32
        {
            continue;
        }
        if !search_row {
            if let Some(gy) = games_y {
                if b.y0 >= gy {
                    continue;
                }
            }
        }

        let (visual, hit) = if search_row {
            xmu_search_row_from_caption(&b)
        } else {
            xmu_tile_from_caption(&b)
        };
        let mut hit = hit;
        hit.x0 = hit.x0.max(0);
        hit.y0 = hit.y0.max(0);
        hit.x1 = hit.x1.min(frame_w);
        hit.y1 = hit.y1.min(frame_h);
        c.kind = CandidateKind::Icon;
        c.element_id = Some(ELEMENT_XMU_SPORTS.into());
        c.visual_bbox = Some(visual);
        c.hit_bbox = Some(hit.clone());
        c.bbox = hit;
        c.evidence.push(if search_row {
            "xmu_row_from_caption".into()
        } else {
            "xmu_tile_from_caption".into()
        });
    }
}

/// Home grid: 「场馆预约」 is the purple disc above those words.
/// The words only locate the column. The box is the disc's own purple pixels.
pub(super) fn promote_venue_booking_tiles(
    items: &mut [Candidate],
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
) {
    if frame_w <= 0 || frame_h <= 0 || bgra.len() < (frame_w * frame_h * 4) as usize {
        return;
    }
    let y_lo = frame_h * 12 / 100;
    let y_hi = frame_h * 78 / 100;
    for c in items.iter_mut() {
        if !c.actionable || !is_venue_caption(&c.label) {
            continue;
        }
        if c.element_id.as_deref() == Some(ELEMENT_VENUE_BOOKING) {
            continue;
        }
        if c.bbox.cy() < y_lo || c.bbox.cy() > y_hi {
            continue;
        }
        let Some(disc) = purple_disc_above(bgra, frame_w, frame_h, &c.bbox) else {
            continue;
        };
        let tile = icon_plus_caption(&disc, &c.bbox, frame_w, frame_h);
        c.kind = CandidateKind::Icon;
        c.element_id = Some(ELEMENT_VENUE_BOOKING.into());
        c.visual_bbox = Some(tile.clone());
        c.hit_bbox = Some(tile.clone());
        c.bbox = tile;
        c.evidence.push("venue_disc_measured".into());
    }
}

/// Axis-aligned bounds of the purple circle sitting on this caption.
/// Neighboring discs lose: the blob must be square and close to these words.
fn purple_disc_above(bgra: &[u8], frame_w: i32, frame_h: i32, caption: &BBox) -> Option<BBox> {
    let reach = caption.w().max(80).clamp(80, 160);
    blob_above(
        bgra,
        frame_w,
        frame_h,
        caption,
        reach,
        reach,
        24,
        40,
        24,
        pixel_is_venue_purple,
    )
}

fn blob_above(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    caption: &BBox,
    reach_x: i32,
    reach_y: i32,
    min_side: i32,
    min_pixels: i32,
    max_gap: i32,
    is_fg: fn(u8, u8, u8) -> bool,
) -> Option<BBox> {
    let x_lo = (caption.cx() - reach_x).max(0);
    let x_hi = (caption.cx() + reach_x).min(frame_w);
    let y_hi = caption.y0.min(frame_h);
    let y_lo = (caption.y0 - reach_y).max(0);
    if x_hi <= x_lo + 8 || y_hi <= y_lo + 8 {
        return None;
    }
    let cols = (x_hi - x_lo) as usize;
    let rows = (y_hi - y_lo) as usize;
    let mut mask = vec![0u8; cols * rows];
    let stride = frame_w as usize;
    for ry in 0..rows {
        for rx in 0..cols {
            let x = x_lo + rx as i32;
            let y = y_lo + ry as i32;
            let i = (y as usize * stride + x as usize) * 4;
            if i + 2 < bgra.len() && is_fg(bgra[i], bgra[i + 1], bgra[i + 2]) {
                mask[ry * cols + rx] = 1;
            }
        }
    }
    let mut seen = vec![0u8; mask.len()];
    let mut best: Option<(i32, BBox)> = None;
    for sy in 0..rows {
        for sx in 0..cols {
            let start = sy * cols + sx;
            if mask[start] == 0 || seen[start] == 1 {
                continue;
            }
            let mut stack = vec![(sx, sy)];
            seen[start] = 1;
            let mut minx = sx;
            let mut miny = sy;
            let mut maxx = sx;
            let mut maxy = sy;
            let mut n = 0i32;
            while let Some((cx, cy)) = stack.pop() {
                n += 1;
                minx = minx.min(cx);
                miny = miny.min(cy);
                maxx = maxx.max(cx);
                maxy = maxy.max(cy);
                for (dx, dy) in [(1i32, 0), (-1, 0), (0, 1), (0, -1)] {
                    let nx = cx as i32 + dx;
                    let ny = cy as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= cols as i32 || ny >= rows as i32 {
                        continue;
                    }
                    let ni = ny as usize * cols + nx as usize;
                    if mask[ni] == 1 && seen[ni] == 0 {
                        seen[ni] = 1;
                        stack.push((nx as usize, ny as usize));
                    }
                }
            }
            if n < min_pixels {
                continue;
            }
            let bx0 = x_lo + minx as i32;
            let by0 = y_lo + miny as i32;
            let bx1 = x_lo + maxx as i32 + 1;
            let by1 = y_lo + maxy as i32 + 1;
            let w = bx1 - bx0;
            let h = by1 - by0;
            if w < min_side || h < min_side || w > reach_x || h > reach_y {
                continue;
            }
            if w.max(h) * 10 > w.min(h) * 14 {
                continue;
            }
            let ccx = bx0 + w / 2;
            let dx = (ccx - caption.cx()).abs();
            let gap = (caption.y0 - by1).abs();
            if dx > reach_x * 2 / 3 || gap > max_gap {
                continue;
            }
            let score = n - dx * 2 - gap * 3;
            let disc = BBox {
                x0: (bx0 - 1).max(0),
                y0: (by0 - 1).max(0),
                x1: (bx1 + 1).min(frame_w),
                y1: (by1 + 1).min(caption.y0.max(by1)),
            };
            if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
                best = Some((score, disc));
            }
        }
    }
    best.map(|(_, disc)| disc)
}

fn is_venue_caption(label: &str) -> bool {
    let n = crate::ocr::normalize_ocr_text(label);
    n.chars().count() <= 6 && labels_match_exact(&n, "场馆预约")
}

/// 「预约须知」card: the words sit on the blue button, they are not a caption under an icon.
pub(super) fn promote_notice_ack(
    items: &mut Vec<Candidate>,
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    ocr: &OcrRegionCache,
) {
    if frame_w <= 0 || frame_h <= 0 || bgra.len() < (frame_w * frame_h * 4) as usize {
        return;
    }
    let titled = notice_titled(ocr);
    if !titled {
        return;
    }
    for c in items.iter_mut() {
        if !c.actionable || !is_ack_caption(&c.label) {
            continue;
        }
        if c.element_id.as_deref() == Some(ELEMENT_NOTICE_ACK) {
            continue;
        }
        let caption = c.bbox.clone();
        if !ack_sits_on_blue(bgra, frame_w, frame_h, &caption) {
            continue;
        }
        let mut hit = ack_blue_around(bgra, frame_w, frame_h, &caption)
            .unwrap_or_else(|| expand_ack_button(&caption));
        hit.x0 = hit.x0.max(0);
        hit.y0 = hit.y0.max(0);
        hit.x1 = hit.x1.min(frame_w);
        hit.y1 = hit.y1.min(frame_h);
        c.kind = CandidateKind::Button;
        c.element_id = Some(ELEMENT_NOTICE_ACK.into());
        c.visual_bbox = Some(hit.clone());
        c.hit_bbox = Some(hit.clone());
        c.bbox = hit;
        c.evidence.push("notice_ack_on_blue".into());
    }
    if items
        .iter()
        .any(|c| c.element_id.as_deref() == Some(ELEMENT_NOTICE_ACK))
    {
        push_notice_card_marker(items, frame_w, frame_h, ocr);
        return;
    }
    // White glyphs on the blue fill often vanish at 524px. The button is still the blue block.
    if let Some(hit) = notice_blue_button(bgra, frame_w, frame_h) {
        let mut c = blank(
            CandidateKind::Button,
            hit.clone(),
            "我知道了".into(),
            0.86,
            CandidateSource::Color,
            true,
        );
        c.element_id = Some(ELEMENT_NOTICE_ACK.into());
        c.visual_bbox = Some(hit.clone());
        c.hit_bbox = Some(hit);
        c.evidence.push("notice_blue_button".into());
        items.push(c);
    }
    push_notice_card_marker(items, frame_w, frame_h, ocr);
}

/// Orange reference: the white「预约须知」card body (not the blue button).
fn push_notice_card_marker(
    items: &mut Vec<Candidate>,
    frame_w: i32,
    frame_h: i32,
    ocr: &OcrRegionCache,
) {
    if items.iter().any(|c| c.label == "notice_card") {
        return;
    }
    let title = ocr
        .find_text("预约须知", 0.45)
        .or_else(|| ocr.find_text("须知", 0.45));
    let Some(title) = title else {
        return;
    };
    let btn_top = items
        .iter()
        .find(|c| c.element_id.as_deref() == Some(ELEMENT_NOTICE_ACK))
        .map(|c| c.bbox.y0)
        .unwrap_or((frame_h as f32 * 0.72) as i32);
    let pad_x = (frame_w as f32 * 0.08) as i32;
    let y0 = (title.y - title.h * 2).max((frame_h as f32 * 0.12) as i32);
    let y1 = (btn_top - 8).max(y0 + 40);
    let card = BBox {
        x0: pad_x.max(0),
        y0,
        x1: (frame_w - pad_x).max(pad_x + 40),
        y1: y1.min(frame_h),
    };
    let mut m = blank(
        CandidateKind::Other,
        card.clone(),
        "notice_card".into(),
        0.45,
        CandidateSource::Ocr,
        false,
    );
    m.visual_bbox = Some(card);
    m.evidence.push("notice_card_body".into());
    items.push(m);
}

fn notice_titled(ocr: &OcrRegionCache) -> bool {
    ocr.find_text("预约须知", 0.45).is_some() || ocr.find_text("须知", 0.45).is_some()
}

/// Centered short blue bar under the notice card. Not the full-width page header.
fn notice_blue_button(bgra: &[u8], frame_w: i32, frame_h: i32) -> Option<BBox> {
    let step = 2i32;
    let x_lo = frame_w * 15 / 100;
    let x_hi = frame_w * 85 / 100;
    let y_lo = frame_h * 30 / 100;
    let y_hi = frame_h * 92 / 100;
    if x_hi <= x_lo + step || y_hi <= y_lo + step {
        return None;
    }
    let stride = frame_w as usize;
    let mut row_n = vec![0i32; frame_h as usize];
    let mut row_x0 = vec![frame_w; frame_h as usize];
    let mut row_x1 = vec![0i32; frame_h as usize];
    let mut y = y_lo;
    while y < y_hi {
        let mut x = x_lo;
        while x < x_hi {
            let i = ((y as usize * stride) + x as usize) * 4;
            if i + 2 < bgra.len()
                && pixel_is_notice_blue(bgra[i], bgra[i + 1], bgra[i + 2])
            {
                let yi = y as usize;
                row_n[yi] += 1;
                row_x0[yi] = row_x0[yi].min(x);
                row_x1[yi] = row_x1[yi].max(x + step);
            }
            x += step;
        }
        y += step;
    }
    let min_count = 4;
    let mut best_score = 0i32;
    let mut best: Option<(i32, i32)> = None;
    let mut run_y0 = -1i32;
    let mut run_score = 0i32;
    y = y_lo;
    while y < y_hi {
        if row_n[y as usize] >= min_count {
            if run_y0 < 0 {
                run_y0 = y;
                run_score = 0;
            }
            run_score += row_n[y as usize];
        } else if run_y0 >= 0 {
            if run_score > best_score {
                best_score = run_score;
                best = Some((run_y0, y));
            }
            run_y0 = -1;
        }
        y += step;
    }
    if run_y0 >= 0 && run_score > best_score {
        best = Some((run_y0, y_hi));
    }
    let (y0, y1) = best?;
    let mut x0 = frame_w;
    let mut x1 = 0i32;
    let mut yy = y0;
    while yy < y1 {
        if row_n[yy as usize] >= min_count {
            x0 = x0.min(row_x0[yy as usize]);
            x1 = x1.max(row_x1[yy as usize]);
        }
        yy += step;
    }
    let w = x1 - x0;
    let h = y1 - y0;
    if w < 36 || h < 10 || w > frame_w * 70 / 100 || h > frame_h * 22 / 100 {
        return None;
    }
    if h > 0 && w / h < 1 {
        return None;
    }
    Some(BBox {
        x0,
        y0,
        x1,
        y1,
    })
}

fn is_ack_caption(label: &str) -> bool {
    let n = crate::ocr::normalize_ocr_text(label);
    n.chars().count() <= 8 && labels_match_exact(&n, "我知道了")
}

/// Blue fill that contains 「我知道了」. The words are only the anchor.
fn ack_blue_around(bgra: &[u8], frame_w: i32, frame_h: i32, caption: &BBox) -> Option<BBox> {
    let step = 2i32;
    let pad_x = caption.w().max(80);
    let x_lo = (caption.x0 - pad_x).max(0);
    let x_hi = (caption.x1 + pad_x).min(frame_w);
    let y_lo = (caption.y0 - 36).max(0);
    let y_hi = (caption.y1 + 36).min(frame_h);
    if x_hi <= x_lo + 8 || y_hi <= y_lo + 8 {
        return None;
    }
    let min_count = (((x_hi - x_lo) / step) * 22 / 100).max(8);
    let stride = frame_w as usize;
    let mut rows: Vec<(i32, i32, i32)> = Vec::new();
    let mut y = y_lo;
    while y < y_hi {
        let mut n = 0i32;
        let mut rx0 = frame_w;
        let mut rx1 = 0i32;
        let mut x = x_lo;
        while x < x_hi {
            let i = ((y as usize * stride) + x as usize) * 4;
            if i + 2 < bgra.len() && pixel_is_notice_blue(bgra[i], bgra[i + 1], bgra[i + 2]) {
                n += 1;
                rx0 = rx0.min(x);
                rx1 = rx1.max(x + step);
            }
            x += step;
        }
        if n >= min_count {
            rows.push((y, rx0, rx1));
        }
        y += step;
    }
    if rows.len() < 4 {
        return None;
    }
    let y0 = rows.iter().map(|r| r.0).min()?;
    let y1 = rows.iter().map(|r| r.0).max()? + step;
    if y1 < caption.y0 || y0 > caption.y1 {
        return None;
    }
    let x0 = rows.iter().map(|r| r.1).min()?;
    let x1 = rows.iter().map(|r| r.2).max()?;
    let w = x1 - x0;
    let h = y1 - y0;
    if w < caption.w() + 12 || h < 16 || h > 96 || w > frame_w * 70 / 100 {
        return None;
    }
    Some(BBox { x0, y0, x1, y1 })
}

fn expand_ack_button(caption: &BBox) -> BBox {
    let pad_x = (caption.h() * 2).clamp(16, 48);
    let pad_y = (caption.h() / 2).clamp(8, 24);
    BBox {
        x0: caption.x0 - pad_x,
        y0: caption.y0 - pad_y,
        x1: caption.x1 + pad_x,
        y1: caption.y1 + pad_y,
    }
}

/// Blue in the padding around the glyphs, or already along the OCR box edge.
fn ack_sits_on_blue(bgra: &[u8], frame_w: i32, frame_h: i32, caption: &BBox) -> bool {
    let outside = expand_ack_button(caption);
    blue_fraction(bgra, frame_w, frame_h, &outside, Some(caption)) >= 0.28
        || blue_fraction(bgra, frame_w, frame_h, caption, None) >= 0.28
}

fn blue_fraction(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    outer: &BBox,
    skip: Option<&BBox>,
) -> f32 {
    let x0 = outer.x0.clamp(0, frame_w - 1);
    let y0 = outer.y0.clamp(0, frame_h - 1);
    let x1 = outer.x1.clamp(x0 + 1, frame_w);
    let y1 = outer.y1.clamp(y0 + 1, frame_h);
    let stride = frame_w as u32;
    let mut blue = 0u32;
    let mut total = 0u32;
    for y in (y0..y1).step_by(2) {
        for x in (x0..x1).step_by(2) {
            if let Some(inner) = skip {
                if x >= inner.x0 && x < inner.x1 && y >= inner.y0 && y < inner.y1 {
                    continue;
                }
            }
            let i = ((y as u32 * stride + x as u32) * 4) as usize;
            if i + 2 >= bgra.len() {
                continue;
            }
            total += 1;
            if pixel_is_notice_blue(bgra[i], bgra[i + 1], bgra[i + 2]) {
                blue += 1;
            }
        }
    }
    if total < 6 {
        0.0
    } else {
        blue as f32 / total as f32
    }
}

/// 「授权登录」 stays gray until 「阅读并同意」 is checked, then the fill becomes dark blue.
/// A wide saturated-blue block around that caption is the enabled button. The pale fill is not.
pub(super) fn promote_auth_login_button(
    items: &mut [Candidate],
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
) {
    if frame_w <= 0 || frame_h <= 0 || bgra.len() < (frame_w * frame_h * 4) as usize {
        return;
    }
    for c in items.iter_mut() {
        if !c.actionable || !labels_match_exact(&c.label, "授权登录") {
            continue;
        }
        if c.evidence.iter().any(|e| e == "auth_login_enabled") {
            continue;
        }
        let Some(hit) = auth_blue_button(bgra, frame_w, frame_h, &c.bbox) else {
            continue;
        };
        c.kind = CandidateKind::Button;
        c.element_id = Some(ELEMENT_AUTH_LOGIN.into());
        c.visual_bbox = Some(hit.clone());
        c.hit_bbox = Some(hit.clone());
        c.bbox = hit;
        c.evidence.push("auth_login_enabled".into());
    }
}

/// Full-width dark blue control that contains the 「授权登录」 glyphs.
/// The step-1 badge is the same hue but only a small disc, so it does not qualify.
fn auth_blue_button(bgra: &[u8], frame_w: i32, frame_h: i32, caption: &BBox) -> Option<BBox> {
    let step = 2i32;
    let y_lo = (caption.y0 - 36).max(0);
    let y_hi = (caption.y1 + 36).min(frame_h);
    let x_lo = frame_w * 4 / 100;
    let x_hi = frame_w - x_lo;
    if y_hi <= y_lo + 8 || x_hi <= x_lo + 8 {
        return None;
    }
    let min_count = (((x_hi - x_lo) / step) * 40 / 100).max(8);
    let stride = frame_w as usize;
    let mut rows: Vec<(i32, i32, i32)> = Vec::new();
    let mut y = y_lo;
    while y < y_hi {
        let mut n = 0i32;
        let mut rx0 = frame_w;
        let mut rx1 = 0i32;
        let mut x = x_lo;
        while x < x_hi {
            let i = ((y as usize * stride) + x as usize) * 4;
            if i + 2 < bgra.len() && pixel_is_notice_blue(bgra[i], bgra[i + 1], bgra[i + 2]) {
                n += 1;
                rx0 = rx0.min(x);
                rx1 = rx1.max(x + step);
            }
            x += step;
        }
        if n >= min_count {
            rows.push((y, rx0, rx1));
        }
        y += step;
    }
    if rows.len() < 4 {
        return None;
    }
    let y0 = rows.iter().map(|r| r.0).min()?;
    let y1 = rows.iter().map(|r| r.0).max()? + step;
    if y1 < caption.y0 || y0 > caption.y1 {
        return None;
    }
    let x0 = rows.iter().map(|r| r.1).min()?;
    let x1 = rows.iter().map(|r| r.2).max()?;
    let w = x1 - x0;
    let h = y1 - y0;
    if w < frame_w * 45 / 100 || h < 16 || h > frame_h * 22 / 100 {
        return None;
    }
    Some(BBox { x0, y0, x1, y1 })
}

const SPORT_LABELS: &[&str] = &[
    "乒乓球",
    "羽毛球",
    "篮球",
    "足球",
    "排球",
    "网球",
    "健身房",
    "瑜伽",
    "游泳馆",
    "游泳",
    "匹克球",
    "动感单车室",
    "动感单车",
];

/// Sport grid: the caption names the sport. The box is that icon plus the words under it.
pub(super) fn promote_sport_tiles(items: &mut [Candidate], bgra: &[u8], frame_w: i32, frame_h: i32) {
    if frame_w <= 0 || frame_h <= 0 || bgra.len() < (frame_w * frame_h * 4) as usize {
        return;
    }
    let y_lo = frame_h * 8 / 100;
    let y_hi = frame_h * 78 / 100;
    for c in items.iter_mut() {
        if !c.actionable || !is_sport_caption(&c.label) {
            continue;
        }
        if c.element_id.as_deref() == Some(ELEMENT_SPORT_ICON) {
            continue;
        }
        if c.bbox.cy() < y_lo || c.bbox.cy() > y_hi {
            continue;
        }
        let Some(disc) = ink_disc_above(bgra, frame_w, frame_h, &c.bbox) else {
            continue;
        };
        let tile = icon_plus_caption(&disc, &c.bbox, frame_w, frame_h);
        c.kind = CandidateKind::Icon;
        c.element_id = Some(ELEMENT_SPORT_ICON.into());
        c.visual_bbox = Some(tile.clone());
        c.hit_bbox = Some(tile.clone());
        c.bbox = tile;
        c.evidence.push("sport_tile_measured".into());
    }
}

fn is_sport_caption(label: &str) -> bool {
    let n = crate::ocr::normalize_ocr_text(label);
    if n.chars().count() > 8 {
        return false;
    }
    SPORT_LABELS
        .iter()
        .any(|sport| n.chars().count() <= sport.chars().count() + 1 && labels_match_exact(&n, sport))
}

fn ink_disc_above(bgra: &[u8], frame_w: i32, frame_h: i32, caption: &BBox) -> Option<BBox> {
    let col = (frame_w.max(1) / 4).clamp(64, 220);
    blob_above(
        bgra,
        frame_w,
        frame_h,
        caption,
        col,
        col,
        16,
        30,
        (col / 2).clamp(28, 64),
        pixel_is_icon_ink,
    )
}

/// Tight rectangle around the measured icon and the caption under it.
fn icon_plus_caption(icon: &BBox, caption: &BBox, frame_w: i32, frame_h: i32) -> BBox {
    BBox {
        x0: icon.x0.min(caption.x0).max(0),
        y0: icon.y0.min(caption.y0).max(0),
        x1: icon.x1.max(caption.x1).min(frame_w),
        y1: icon.y1.max(caption.y1).min(frame_h),
    }
}

/// Vertical grid tile: crest above the grey caption.
pub(super) fn xmu_tile_from_caption(caption: &BBox) -> (BBox, BBox) {
    let side = caption.w().max(caption.h() * 2).clamp(36, 120);
    let gap = (side / 10).max(4);
    let visual = BBox {
        x0: caption.cx() - side / 2,
        y0: caption.y0 - gap - side,
        x1: caption.cx() + side / 2,
        y1: caption.y0 - gap,
    };
    let hit = BBox {
        x0: visual.x0.min(caption.x0) - 6,
        y0: visual.y0 - 4,
        x1: visual.x1.max(caption.x1) + 6,
        y1: caption.y1 + 4,
    };
    (visual, hit)
}

/// Search dropdown row: crest to the left of the green title.
pub(super) fn xmu_search_row_from_caption(caption: &BBox) -> (BBox, BBox) {
    let side = caption.h().max(28).clamp(28, 72);
    let gap = (side / 5).max(6);
    let visual = BBox {
        x0: caption.x0 - gap - side,
        y0: caption.cy() - side / 2,
        x1: caption.x0 - gap,
        y1: caption.cy() + side / 2,
    };
    let hit = BBox {
        x0: visual.x0 - 4,
        y0: visual.y0.min(caption.y0) - 4,
        x1: caption.x1 + 8,
        y1: visual.y1.max(caption.y1) + 4,
    };
    (visual, hit)
}

pub(super) fn push_panel_anchors(
    out: &mut Vec<Candidate>,
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    games_y: Option<i32>,
) {
    let xmu = bundled_xmu_templates();
    let xmu_refs: Vec<&TemplateNeedle> = xmu.iter().collect();
    if let Some((index, visual, score)) =
        match_xmu_family(bgra, frame_w, frame_h, &xmu_refs, games_y)
    {
        let t = &xmu[index];
        let mut hit = xmu_tile_hit_box(&visual);
        hit.x0 = hit.x0.max(0);
        hit.y0 = hit.y0.max(0);
        hit.x1 = hit.x1.min(frame_w);
        hit.y1 = hit.y1.min(frame_h);
        let mut c = blank(
            CandidateKind::Icon,
            hit.clone(),
            t.label.clone(),
            score,
            CandidateSource::Template,
            true,
        );
        c.template_id = Some(t.id.clone());
        c.visual_bbox = Some(visual);
        c.hit_bbox = Some(hit);
        c.evidence.push(format!("template:{}", t.id));
        stamp_shell_identity(&mut c);
        out.push(c);
    }

    let search = bundled_mp_search_templates();
    let search_refs: Vec<&TemplateNeedle> = search.iter().collect();
    if let Some((index, visual, score)) =
        match_mp_search_family(bgra, frame_w, frame_h, &search_refs)
    {
        let t = &search[index];
        let mut hit = visual.clone();
        hit.x0 = hit.x0.max(0);
        hit.y0 = hit.y0.max(0);
        hit.x1 = hit.x1.min(frame_w);
        hit.y1 = hit.y1.min(frame_h);
        let mut c = blank(
            CandidateKind::Button,
            hit.clone(),
            t.label.clone(),
            score,
            CandidateSource::Template,
            true,
        );
        c.template_id = Some(t.id.clone());
        c.visual_bbox = Some(visual);
        c.hit_bbox = Some(hit);
        c.evidence.push(format!("template:{}", t.id));
        stamp_shell_identity(&mut c);
        out.push(c);
    }
}

/// Tile hit box: crest plus the caption under it.
pub(super) fn xmu_tile_hit_box(visual: &BBox) -> BBox {
    let pad_x = (visual.w() as f32 * 0.35).round() as i32;
    let pad_top = (visual.h() as f32 * 0.12).round() as i32;
    let pad_bottom = (visual.h() as f32 * 0.72).round() as i32;
    BBox {
        x0: visual.x0 - pad_x,
        y0: visual.y0 - pad_top,
        x1: visual.x1 + pad_x,
        y1: visual.y1 + pad_bottom,
    }
}

/// Recent / Favorites strip. Starts at the right edge of the Discover list.
pub(super) fn mp_panel_content_roi(
    frame_w: u32,
    frame_h: u32,
    games_y: Option<i32>,
) -> Option<(u32, u32, u32, u32)> {
    if frame_w < 160 || frame_h < 160 {
        return None;
    }
    let list_right = discover_list_roi(frame_w, frame_h)
        .map(|(_, x1, _, _)| x1)
        .unwrap_or_else(|| (frame_w * 28 / 100).max(120));
    let x0 = list_right.min(frame_w.saturating_sub(80));
    let x1 = (frame_w * 98 / 100).max(x0.saturating_add(80)).min(frame_w);
    let y0 = (frame_h as f32 * 0.08) as u32;
    let mut y1 = (frame_h as f32 * 0.78) as u32;
    if let Some(gy) = games_y {
        if gy > 0 {
            y1 = y1.min(gy as u32);
        }
    }
    if x1 <= x0.saturating_add(24) || y1 <= y0.saturating_add(24) {
        return None;
    }
    Some((x0, x1, y0, y1))
}

pub(super) fn mp_search_roi(frame_w: u32, frame_h: u32) -> Option<(u32, u32, u32, u32)> {
    if frame_w < 160 || frame_h < 120 {
        return None;
    }
    let x0 = (frame_w * 78 / 100).min(frame_w.saturating_sub(48));
    let x1 = frame_w.saturating_sub(8).max(x0.saturating_add(24));
    let y0 = (frame_h as f32 * 0.02) as u32;
    let y1 = ((frame_h as f32 * 0.16) as u32)
        .max(y0.saturating_add(28))
        .min(frame_h);
    if x1 <= x0.saturating_add(12) || y1 <= y0.saturating_add(12) {
        return None;
    }
    Some((x0, x1, y0, y1))
}

pub(super) fn match_xmu_family(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    needles: &[&TemplateNeedle],
    games_y: Option<i32>,
) -> Option<(usize, BBox, f32)> {
    let (w, h) = (frame_w.max(0) as u32, frame_h.max(0) as u32);
    let (x0, x1, y0, y1) = mp_panel_content_roi(w, h, games_y)?;
    // Panel tiles are often larger than the 48px needle; keep a longer scale ladder.
    match_roi_family(
        bgra,
        w,
        h,
        x0,
        x1,
        y0,
        y1,
        needles,
        &[0.7, 0.9, 1.1, 1.35, 1.6, 1.9, 2.3],
    )
}

pub(super) fn match_mp_search_family(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    needles: &[&TemplateNeedle],
) -> Option<(usize, BBox, f32)> {
    let (w, h) = (frame_w.max(0) as u32, frame_h.max(0) as u32);
    let (x0, x1, y0, y1) = mp_search_roi(w, h)?;
    match_roi_family(bgra, w, h, x0, x1, y0, y1, needles, &COMPASS_SCALES)
}

pub(super) fn match_roi_family(
    bgra: &[u8],
    frame_w: u32,
    frame_h: u32,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
    needles: &[&TemplateNeedle],
    scales: &[f32],
) -> Option<(usize, BBox, f32)> {
    if needles.is_empty() || bgra.len() < (frame_w * frame_h * 4) as usize {
        return None;
    }
    let width = x1 - x0;
    let height = y1 - y0;
    let gray = region_gray(bgra, frame_w, x0, width, y0, height);
    let (small, sw, sh) = down4(&gray, width, height);
    if sw < 4 || sh < 4 {
        return None;
    }
    let mut best: Option<(usize, BBox, f32)> = None;
    for (index, needle) in needles.iter().enumerate() {
        for &scale in scales {
            let Some((cx, cy)) = best_coarse_at_scale(&small, sw, sh, needle, scale, 0, sw, 0, sh)
            else {
                continue;
            };
            let tw = (needle.width as f32 * scale).round().max(12.0) as u32;
            let th = (needle.height as f32 * scale).round().max(12.0) as u32;
            if tw + 1 >= x1 || y0.saturating_add(th) >= y1 {
                continue;
            }
            let full_x = x0
                .saturating_add(cx * COARSE_FACTOR)
                .min(x1.saturating_sub(tw + 1));
            let full_y = y0
                .saturating_add(cy * COARSE_FACTOR)
                .min(y1.saturating_sub(th + 1));
            let Some((bbox, score)) = refine_signed(
                bgra, frame_w, frame_h, x1, y0, y1, needle, tw, th, full_x, full_y,
            ) else {
                continue;
            };
            if best.as_ref().map_or(true, |(_, _, prior)| score > *prior) {
                best = Some((index, bbox, score));
            }
            if score >= 0.95 {
                break;
            }
        }
    }
    best.filter(|(_, _, score)| *score >= SHELL_NCC_MIN)
}

/// Presence score for a gray needle in a vertical band (full width).
/// Uses the same down×4 + sparse ZNCC + local refine path as shell/ROI matchers.
/// Returns best score in `[0, 1]` (0 = no usable peak). Caller applies its own threshold.
pub fn template_band_best_score(
    bgra: &[u8],
    frame_w: u32,
    frame_h: u32,
    needle: &TemplateNeedle,
    y0_frac: f32,
    y1_frac: f32,
    scales: &[f32],
) -> f32 {
    if frame_w < 32 || frame_h < 32 || bgra.len() < (frame_w * frame_h * 4) as usize {
        return 0.0;
    }
    let y0 = ((frame_h as f32) * y0_frac).max(0.0) as u32;
    let y1 = ((frame_h as f32) * y1_frac)
        .min(frame_h as f32)
        .max(y0 as f32 + 16.0) as u32;
    let needles = [needle];
    match_roi_family_unfiltered(bgra, frame_w, frame_h, 0, frame_w, y0, y1, &needles, scales)
        .map(|(_, _, s)| s)
        .unwrap_or(0.0)
}

fn match_roi_family_unfiltered(
    bgra: &[u8],
    frame_w: u32,
    frame_h: u32,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
    needles: &[&TemplateNeedle],
    scales: &[f32],
) -> Option<(usize, BBox, f32)> {
    if needles.is_empty() || bgra.len() < (frame_w * frame_h * 4) as usize || x1 <= x0 || y1 <= y0 {
        return None;
    }
    let width = x1 - x0;
    let height = y1 - y0;
    let gray = region_gray(bgra, frame_w, x0, width, y0, height);
    let (small, sw, sh) = down4(&gray, width, height);
    if sw < 4 || sh < 4 {
        return None;
    }
    let mut best: Option<(usize, BBox, f32)> = None;
    for (index, needle) in needles.iter().enumerate() {
        for &scale in scales {
            let Some((cx, cy)) = best_coarse_at_scale(&small, sw, sh, needle, scale, 0, sw, 0, sh)
            else {
                continue;
            };
            let tw = (needle.width as f32 * scale).round().max(12.0) as u32;
            let th = (needle.height as f32 * scale).round().max(12.0) as u32;
            if tw + 1 >= x1 || y0.saturating_add(th) >= y1 {
                continue;
            }
            let full_x = x0
                .saturating_add(cx * COARSE_FACTOR)
                .min(x1.saturating_sub(tw + 1));
            let full_y = y0
                .saturating_add(cy * COARSE_FACTOR)
                .min(y1.saturating_sub(th + 1));
            let Some((bbox, score)) = refine_signed(
                bgra, frame_w, frame_h, x1, y0, y1, needle, tw, th, full_x, full_y,
            ) else {
                continue;
            };
            if best.as_ref().map_or(true, |(_, _, prior)| score > *prior) {
                best = Some((index, bbox, score));
            }
            if score >= 0.95 {
                break;
            }
        }
    }
    best
}

/// Sidebar-only signed correlation for the Discover compass.
///
/// Light and dark needles are separate because edge polarity flips with the
/// theme.  The coefficient is not absolute-valued: an inverted theme must
/// lose to the matching needle.  Scales cover about 29–77 px on the 48 px needle.
///
/// The sidebar is grayed once and searched at 1/4 resolution.  Full-resolution
/// correlation runs only around that proposal, then around the previous hit.
pub(super) fn match_shell_template_ncc(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    t: &TemplateNeedle,
) -> Option<(BBox, f32)> {
    let needles = [t];
    match_compass_family(bgra, frame_w, frame_h, &needles).map(|(_, bbox, score)| (bbox, score))
}

pub(super) fn match_compass_family(
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
    needles: &[&TemplateNeedle],
) -> Option<(usize, BBox, f32)> {
    if needles.is_empty() {
        return None;
    }
    let (w, h) = (frame_w.max(0) as u32, frame_h.max(0) as u32);
    if w == 0 || h == 0 || bgra.len() < (w * h * 4) as usize {
        return None;
    }
    let x_limit = (w / 8).clamp(48, 140);
    let y_start = (h as f32 * 0.12) as u32;
    let y_limit = (h as f32 * 0.82) as u32;
    if y_limit <= y_start.saturating_add(16) || x_limit < 16 {
        return None;
    }
    let gray = sidebar_gray(bgra, w, x_limit, y_start, y_limit);
    let strip_h = y_limit - y_start;
    let (small, sw, sh) = down4(&gray, x_limit, strip_h);
    if sw < 4 || sh < 4 {
        return None;
    }

    let hint = {
        let guard = compass_hint().lock().unwrap_or_else(|err| err.into_inner());
        *guard
    };
    let hit = hint
        .and_then(|hint| {
            propose_and_refine(
                bgra,
                w,
                h,
                x_limit,
                y_start,
                y_limit,
                &small,
                sw,
                sh,
                needles,
                Some(hint),
            )
        })
        .filter(|(_, _, score)| *score >= SHELL_NCC_MIN)
        .or_else(|| {
            propose_and_refine(
                bgra, w, h, x_limit, y_start, y_limit, &small, sw, sh, needles, None,
            )
        })
        .filter(|(_, _, score)| *score >= SHELL_NCC_MIN);

    if let Some((_, bbox, _)) = &hit {
        if let Ok(mut guard) = compass_hint().lock() {
            *guard = Some(CompassHint {
                x: bbox.x0.max(0) as u32,
                y: bbox.y0.max(0) as u32,
            });
        }
    }
    hit
}

pub(super) fn propose_and_refine(
    bgra: &[u8],
    frame_w: u32,
    frame_h: u32,
    x_limit: u32,
    y_start: u32,
    y_limit: u32,
    small: &[u8],
    sw: u32,
    sh: u32,
    needles: &[&TemplateNeedle],
    hint: Option<CompassHint>,
) -> Option<(usize, BBox, f32)> {
    let (cx0, cx1, cy0, cy1) = hint_coarse_window(hint, y_start, sw, sh);
    let mut best: Option<(usize, BBox, f32)> = None;
    for (index, needle) in needles.iter().enumerate() {
        // Refine every scale's own coarse peak.  Picking one scale first was
        // losing the exact 1.0 paint to a wrong 0.8 proposal on the 4× grid.
        for scale in COMPASS_SCALES {
            let Some((cx, cy)) =
                best_coarse_at_scale(small, sw, sh, needle, scale, cx0, cx1, cy0, cy1)
            else {
                continue;
            };
            let tw = (needle.width as f32 * scale).round().max(12.0) as u32;
            let th = (needle.height as f32 * scale).round().max(12.0) as u32;
            if tw >= x_limit || y_start + th >= y_limit {
                continue;
            }
            let full_x = (cx * COARSE_FACTOR).min(x_limit.saturating_sub(tw + 1));
            let full_y = y_start
                .saturating_add(cy * COARSE_FACTOR)
                .min(y_limit.saturating_sub(th + 1));
            let Some((bbox, score)) = refine_signed(
                bgra, frame_w, frame_h, x_limit, y_start, y_limit, needle, tw, th, full_x, full_y,
            ) else {
                continue;
            };
            if best.as_ref().map_or(true, |(_, _, prior)| score > *prior) {
                best = Some((index, bbox, score));
            }
            if score >= 0.95 {
                break;
            }
        }
    }
    best
}

pub(super) fn hint_coarse_window(
    hint: Option<CompassHint>,
    y_start: u32,
    sw: u32,
    sh: u32,
) -> (u32, u32, u32, u32) {
    let Some(hint) = hint else {
        return (0, sw, 0, sh);
    };
    // ±40 px around the previous top-left, in the 4× grid.
    let rad = 40 / COARSE_FACTOR;
    let cx = hint.x / COARSE_FACTOR;
    let cy = hint.y.saturating_sub(y_start) / COARSE_FACTOR;
    (
        cx.saturating_sub(rad),
        (cx + rad + 1).min(sw),
        cy.saturating_sub(rad),
        (cy + rad + 1).min(sh),
    )
}

pub(super) fn shell_template_family(id: &str) -> Option<&str> {
    if id.starts_with("discover_compass") {
        return Some("discover_compass");
    }
    if id.starts_with("xmu_sports_icon") {
        return Some("xmu_sports_icon");
    }
    if id.starts_with("mp_search") {
        return Some("mp_search");
    }
    id.strip_suffix("_active")
        .or_else(|| id.strip_suffix("_inactive"))
        .filter(|base| !base.is_empty())
}
