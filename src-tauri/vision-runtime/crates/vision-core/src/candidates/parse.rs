//! Assemble detect, template, color, and OCR into one candidate set.

use crate::ocr::{labels_match_exact, OcrRegionCache};
use crate::slots::parse_time_label;

use super::color::{captcha_row_candidates, opencv_hsv, slot_cells_from_pixels};
use super::keyboard::inject_geometric_soft_keys;
use super::template_match::{
    games_header_y, match_templates, panel_chrome, promote_auth_login_button, promote_notice_ack,
    promote_sport_tiles, promote_venue_booking_tiles, promote_xmu_caption_tiles,
    search_result_chrome,
};
use super::types::{
    box_iou, state_from_template_id, BBox, Candidate, CandidateKind, CandidateSet, CandidateSource,
    ParseInput, ShellVisualState, ELEMENT_DISCOVER, ELEMENT_MINIPROGRAM, ELEMENT_MP_SEARCH,
    ELEMENT_XMU_SPORTS,
};

pub fn parse_candidates(input: &ParseInput<'_>) -> CandidateSet {
    let rules = input.rules;
    let mut items: Vec<Candidate> = Vec::new();

    for d in input.detections {
        if d.score < 0.05 || d.x1 <= d.x0 || d.y1 <= d.y0 {
            continue;
        }
        let kind = match d.class_name.as_str() {
            "button" => CandidateKind::Button,
            "list" | "list_row" => CandidateKind::ListRow,
            _ => CandidateKind::Icon,
        };
        items.push(blank(
            kind,
            BBox {
                x0: d.x0,
                y0: d.y0,
                x1: d.x1,
                y1: d.y1,
            },
            String::new(),
            d.score,
            CandidateSource::Detect,
            true,
        ));
    }

    if let Some(px) = input.pixels_bgra {
        for hit in match_templates(
            px,
            input.frame_w,
            input.frame_h,
            input.templates,
            games_header_y(input.ocr),
            panel_chrome(input.ocr),
        ) {
            items.push(hit);
        }
        items.extend(slot_cells_from_pixels(
            px,
            input.frame_w,
            input.frame_h,
            input.ocr,
            rules,
        ));
    }

    attach_ocr(&mut items, input.ocr, rules.label_iou, input.frame_h);
    promote_xmu_caption_tiles(
        &mut items,
        input.frame_w,
        input.frame_h,
        games_header_y(input.ocr),
        panel_chrome(input.ocr),
        search_result_chrome(input.ocr),
    );
    if let Some(px) = input.pixels_bgra {
        promote_venue_booking_tiles(&mut items, px, input.frame_w, input.frame_h);
        promote_sport_tiles(&mut items, px, input.frame_w, input.frame_h);
        promote_notice_ack(&mut items, px, input.frame_w, input.frame_h, input.ocr);
        promote_auth_login_button(&mut items, px, input.frame_w, input.frame_h);
    }
    if let Some(px) = input.pixels_bgra {
        annotate_shell_tab_states(&mut items, px, input.frame_w, input.frame_h);
    }
    if let Some(px) = input.pixels_bgra {
        for c in captcha_row_candidates(px, input.frame_w, input.frame_h, input.ocr) {
            items.push(c);
        }
    }
    promote_keyboard(&mut items, input.frame_h);
    inject_geometric_soft_keys(
        &mut items,
        input.ocr,
        input.frame_w,
        input.frame_h,
        input.pixels_bgra,
    );

    let items = dedup(items, rules.dedup_iou);
    let items = items
        .into_iter()
        .enumerate()
        .map(|(i, mut c)| {
            c.id = format!("c{i}");
            c
        })
        .collect();
    CandidateSet {
        items,
        dedup_iou: rules.dedup_iou,
    }
}

/// Miniprogram row: prefer a themed template; otherwise promote an exact
/// OCR/list label 「小程序」 and read active/inactive from the green row fill.
/// Do not require a bundled miniprogram needle — Pack may only ship the compass.
pub(super) fn annotate_shell_tab_states(
    items: &mut [Candidate],
    bgra: &[u8],
    frame_w: i32,
    frame_h: i32,
) {
    if frame_w <= 0 || frame_h <= 0 || bgra.len() < (frame_w * frame_h * 4) as usize {
        return;
    }
    for c in items.iter_mut() {
        stamp_shell_identity(c);
        // Identity comes from a template hit. An OCR label is not the icon.
        let from_template = c.element_id.as_deref() == Some(ELEMENT_MINIPROGRAM)
            || c.template_id
                .as_deref()
                .is_some_and(|id| id.starts_with("miniprogram_entry"));
        if !from_template {
            continue;
        }
        // Compass hits must not be rewritten into the list-row identity.
        if c.template_id
            .as_deref()
            .is_some_and(|id| id.starts_with("discover_compass"))
        {
            continue;
        }
        if c.visual_bbox.is_none() {
            c.visual_bbox = Some(c.bbox.clone());
        }
        let visual = c.visual_bbox.clone().unwrap_or_else(|| c.bbox.clone());
        let themed = c.template_id.as_deref().is_some_and(|id| {
            id.starts_with("miniprogram_entry_") && state_from_template_id(id).is_some()
        });
        // The family matcher already set state, visual box, and the row hit box.
        if themed {
            continue;
        }
        if !themed {
            let active = green_coverage(
                bgra,
                frame_w as u32,
                frame_h as u32,
                (visual.x0 - 12).max(0) as u32,
                (visual.y0 - visual.h()).max(0) as u32,
                (visual.x1 + 24).min(frame_w) as u32,
                (visual.y1 + visual.h()).min(frame_h) as u32,
            ) >= 0.42;
            c.template_id = Some(
                if active {
                    "miniprogram_entry_active"
                } else {
                    "miniprogram_entry_inactive"
                }
                .into(),
            );
            c.element_state = Some(if active {
                ShellVisualState::Active
            } else {
                ShellVisualState::Inactive
            });
            c.evidence.push(if active {
                "shell_row:小程序:active_green".into()
            } else {
                "shell_row:小程序:inactive".into()
            });
        }
        c.element_id = Some(ELEMENT_MINIPROGRAM.into());
        if c.label.is_empty() || !labels_match_exact(&c.label, "小程序") {
            c.label = "小程序".into();
        }
        c.kind = CandidateKind::ListRow;
        c.actionable = true;
        c.bbox = BBox {
            x0: (visual.x0 - 100).max(0),
            y0: visual.y0,
            x1: (visual.x1 + 180).min(frame_w),
            y1: visual.y1,
        };
        c.hit_bbox = Some(c.bbox.clone());
    }
}
pub(super) fn theme_from_template_id(id: &str) -> Option<String> {
    if id.contains("_light") {
        Some("light".into())
    } else if id.contains("_dark") {
        Some("dark".into())
    } else {
        None
    }
}

pub(super) fn stamp_shell_identity(c: &mut Candidate) {
    let Some(id) = c.template_id.clone() else {
        return;
    };
    let element = if id.starts_with("discover_compass") {
        ELEMENT_DISCOVER
    } else if id.starts_with("miniprogram_entry") {
        ELEMENT_MINIPROGRAM
    } else if id == "xmu_sports_icon" || id.starts_with("xmu_sports_icon") {
        ELEMENT_XMU_SPORTS
    } else if id.starts_with("mp_search") {
        ELEMENT_MP_SEARCH
    } else {
        return;
    };
    c.element_id = Some(element.to_string());
    if c.visual_bbox.is_none() {
        c.visual_bbox = Some(c.bbox.clone());
    }
    if let Some(state) = state_from_template_id(&id) {
        c.element_state = Some(state);
    }
    if c.theme.is_none() {
        c.theme = theme_from_template_id(&id);
    }
    if element == ELEMENT_MINIPROGRAM {
        c.kind = CandidateKind::ListRow;
    }
    if c.hit_bbox.is_none() {
        c.hit_bbox = Some(c.bbox.clone());
    }
}

pub(super) fn green_coverage(
    bgra: &[u8],
    stride: u32,
    height: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> f32 {
    let x1 = x1.min(stride);
    let y1 = y1.min(height);
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }
    let mut green = 0u32;
    let mut total = 0u32;
    for y in (y0..y1).step_by(2) {
        for x in (x0..x1).step_by(2) {
            let i = ((y * stride + x) * 4) as usize;
            let (h, s, v) = opencv_hsv(bgra[i], bgra[i + 1], bgra[i + 2]);
            if (35..=100).contains(&h) && s >= 70 && v >= 70 {
                green += 1;
            }
            total += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        green as f32 / total as f32
    }
}
pub(super) fn blank(
    kind: CandidateKind,
    bbox: BBox,
    label: String,
    score: f32,
    source: CandidateSource,
    actionable: bool,
) -> Candidate {
    Candidate {
        id: String::new(),
        bbox,
        kind,
        label,
        score,
        sources: vec![source],
        actionable,
        template_id: None,
        slot_meta: None,
        evidence: Vec::new(),
        element_id: None,
        element_state: None,
        theme: None,
        visual_bbox: None,
        hit_bbox: None,
    }
}

pub(super) fn attach_ocr(
    items: &mut Vec<Candidate>,
    ocr: &OcrRegionCache,
    label_iou: f32,
    frame_h: i32,
) {
    for hit in ocr.hits() {
        if hit.score < 0.45 || hit.w <= 0 || hit.h <= 0 {
            continue;
        }
        let hb = BBox {
            x0: hit.x,
            y0: hit.y,
            x1: hit.x + hit.w,
            y1: hit.y + hit.h,
        };
        let host = items.iter_mut().find(|c| {
            c.kind != CandidateKind::Text
                && c.kind != CandidateKind::SlotCell
                && box_iou(&c.bbox, &hb) >= label_iou
        });
        if let Some(host) = host {
            if !host.sources.contains(&CandidateSource::Ocr) {
                host.sources.push(CandidateSource::Ocr);
            }
            if host.label.is_empty() || hit.score > host.score {
                host.label = hit.text.clone();
            }
            host.evidence.push(format!("ocr_label:{}", hit.text));
            continue;
        }
        let legend = is_legend(&hit.text);
        let time = parse_time_label(&hit.text).is_some();
        let mut c = blank(
            CandidateKind::Text,
            hb,
            hit.text.clone(),
            hit.score,
            CandidateSource::Ocr,
            !legend && !time,
        );
        if legend {
            c.evidence.push("legend_not_cell".into());
        }
        if time {
            c.evidence.push("time_label_not_cell".into());
            c.actionable = false;
        }
        if frame_h > 0 && hit.y > (frame_h as f32 * 0.52) as i32 && is_keyboard_word(&hit.text) {
            c.kind = CandidateKind::KeyboardKey;
            c.actionable = true;
            c.evidence.push("keyboard".into());
        }
        items.push(c);
    }
}

pub(super) fn promote_keyboard(items: &mut [Candidate], frame_h: i32) {
    if frame_h <= 0 {
        return;
    }
    // Soft keyboard occupies the lower half. Use cy so the「完成/取消」header
    // still counts when its top edge sits near the mid cut.
    let y_cut = (frame_h as f32 * 0.48) as i32;
    let in_band = |c: &Candidate| c.bbox.cy() >= y_cut;
    let chrome = items.iter().any(|c| in_band(c) && is_keyboard_word(&c.label));
    let glyph_n = items
        .iter()
        .filter(|c| in_band(c) && is_soft_key_glyph(&c.label))
        .count();
    // QWERTY leaves dozens of single-letter OCR hits; don't require「完成」
    // OCR (often flaky) before promoting glyphs.
    let soft_open = chrome || glyph_n >= 6;
    if !soft_open {
        return;
    }
    for c in items.iter_mut() {
        if !in_band(c) {
            continue;
        }
        let key = is_keyboard_word(&c.label) || is_soft_key_glyph(&c.label);
        if key {
            c.kind = CandidateKind::KeyboardKey;
            c.actionable = true;
            if !c.evidence.iter().any(|e| e == "keyboard") {
                c.evidence.push("keyboard".into());
            }
        }
    }
}

pub(super) fn is_keyboard_word(text: &str) -> bool {
    labels_match_exact(text, "完成") || labels_match_exact(text, "取消")
}

pub(super) fn is_soft_key_glyph(text: &str) -> bool {
    let t = text.trim();
    let mut chars = t.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => c.is_ascii_alphanumeric(),
        _ => false,
    }
}

pub(super) fn is_legend(text: &str) -> bool {
    labels_match_exact(text, "可预约")
        || labels_match_exact(text, "不可选")
        || labels_match_exact(text, "已约满")
}

pub(super) fn dedup(items: Vec<Candidate>, thr: f32) -> Vec<Candidate> {
    let mut sorted = items;
    sorted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut kept: Vec<Candidate> = Vec::new();
    for c in sorted {
        if let Some(k) = kept
            .iter_mut()
            .find(|k| k.kind == c.kind && box_iou(&k.bbox, &c.bbox) >= thr)
        {
            for s in &c.sources {
                if !k.sources.contains(s) {
                    k.sources.push(*s);
                }
            }
            if k.label.is_empty() {
                k.label = c.label;
            }
            if k.template_id.is_none() {
                k.template_id = c.template_id;
            }
            continue;
        }
        kept.push(c);
    }
    kept
}
