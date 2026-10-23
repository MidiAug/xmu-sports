//! Glance report types and evidence framing.

use serde::Serialize;

use vision_core::{Candidate, ShadowTiming};

#[derive(Debug, Clone, Copy)]
pub struct GlanceMode {
    pub include_image: bool,
}

impl GlanceMode {
    pub fn ui() -> Self {
        Self {
            include_image: true,
        }
    }
    pub fn run() -> Self {
        Self {
            include_image: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DateCardSummary {
    pub month: u32,
    pub day: u32,
    pub status: String,
    pub color_status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DateEvidenceSummary {
    pub cards: Vec<DateCardSummary>,
    pub visible_cols: usize,
    pub ordered: bool,
    pub continuous: bool,
    pub quality: f32,
    pub goal_in_visible_range: bool,
    pub frame_evidence: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GlanceReport {
    pub bound_title: String,
    pub now: String,
    pub next: String,
    pub basis: String,
    pub page: String,
    pub keyboard_open: bool,
    pub refuse_reason: Option<String>,
    pub candidate_count: usize,
    pub input_invoked: bool,
    /// True when the decision is a Navigate with a clickable anchor.
    pub can_click: bool,
    /// `navigate` | `atomic_composite` | `wait` | `stop` | `need_human` | `rebind`
    pub intent_kind: String,
    /// Set when `intent_kind == "wait"`.
    pub wait_ms: Option<u64>,
    /// Navigate / atomic click label (e.g. 「确认预约」). Empty when not a click.
    #[serde(default)]
    pub click_label: String,
    /// Captcha tile OCR result for this glance (persisted by native run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognized_captcha: Option<String>,
    /// Page classify confidence (P4 layered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_confidence: Option<f32>,
    /// Date-strip evidence quality on VenueDetail (P4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_confidence: Option<f32>,
    /// Privacy-safe date card/status summary, persisted even when raw OCR is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_evidence: Option<DateEvidenceSummary>,
    /// Pre-inject click safety score (P4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click_safety_confidence: Option<f32>,
    /// Structured perception trace. The host app persists this for every run step.
    pub frame_id: String,
    pub frame_width: u32,
    pub frame_height: u32,
    pub capture_backend: String,
    /// Physical pixel transform and actual capture age; no user identifiers.
    pub surface_diagnostics: serde_json::Value,
    pub timing: ShadowTiming,
    pub ocr_hits: Vec<vision_core::OcrHit>,
    pub candidates: Vec<Candidate>,
    /// Annotated BMP, standard base64. UI mode only; run mode leaves empty and
    /// hands raw frames to the async trace writer for terminal evidence.
    pub image_b64: String,
}


#[cfg(windows)]
mod imp {
    use super::{DateCardSummary, DateEvidenceSummary, GlanceMode, GlanceReport};
    use std::path::Path;

    use vision_core::{
        click_safety_ok, layered_confidence, AtomicStep, Candidate, CandidateBox, CandidateKind,
        NextIntent, Page, DEFAULT_MIN_CONFIDENCE, ELEMENT_DISCOVER, ELEMENT_MINIPROGRAM,
        ELEMENT_MP_SEARCH, ELEMENT_NOTICE_ACK, ELEMENT_SPORT_ICON, ELEMENT_VENUE_BOOKING,
        ELEMENT_XMU_SPORTS,
    };
    use vision_pack::active_pack_dir;

    use crate::evidence::{draw_boxes_bgra, encode_bgra_bmp};
    use crate::glance::bind::Observed;
    use crate::glance::observe::elapsed_ms;

/// Overlay only the glyph the matcher found. Chat names from OCR are not anchors.
///
/// Color convention (evidence.rs): `actionable=false` → orange reference;
/// green is reserved for the decision's `next` click in `build_report`.
pub(crate) fn glance_visual(c: &Candidate, page: Page) -> bool {
    // Pixel-color candidate generation runs before page classification, so a
    // blue date card can resemble a mint slot cell. Only expose slot evidence
    // while the decision is actually on the slot-selection page.
    if c.kind == CandidateKind::SlotCell {
        return page == Page::SelectSlot;
    }
    // CaptchaHit is an internal click candidate. It is deliberately not an
    // orange reference: when selected by decide_next build_report adds the
    // single green `next` box, and otherwise it must stay invisible.
    // Step reference markers (orange): captcha tile / input band / notice card.
    if matches!(
        c.label.as_str(),
        "captcha_image" | "captcha_input_band" | "notice_card"
    ) {
        return true;
    }
    let element = c.element_id.as_deref();
    if element == Some(ELEMENT_DISCOVER)
        || element == Some(ELEMENT_MINIPROGRAM)
        || element == Some(ELEMENT_XMU_SPORTS)
        || element == Some(ELEMENT_VENUE_BOOKING)
        || element == Some(ELEMENT_NOTICE_ACK)
        || element == Some(ELEMENT_SPORT_ICON)
        || element == Some(ELEMENT_MP_SEARCH)
    {
        return true;
    }
    c.template_id.as_deref().is_some_and(|id| {
        id.starts_with("discover_compass")
            || id.starts_with("miniprogram_entry")
            || id.starts_with("xmu_sports")
            || id.starts_with("mp_search")
    })
}

/// Orange reference markers for the current step — keep them even when the
/// green `next` click sits inside (e.g. captcha image + input band + green hit).
pub(crate) fn is_step_reference_marker(label: &str) -> bool {
    matches!(
        label,
        "captcha_image" | "captcha_input_band" | "notice_card"
    )
}

/// The click box is the control. A second outline on the same pixels is the inner frame.
/// Reference markers (橙) are kept so the step context stays visible beside 绿框.
pub(crate) fn drop_boxes_on_anchor(boxes: &mut Vec<CandidateBox>, x0: i32, y0: i32, x1: i32, y1: i32) {
    boxes.retain(|b| {
        if is_step_reference_marker(&b.label) {
            return true;
        }
        let ix0 = b.x0.max(x0);
        let iy0 = b.y0.max(y0);
        let ix1 = b.x1.min(x1);
        let iy1 = b.y1.min(y1);
        if ix1 <= ix0 || iy1 <= iy0 {
            return true;
        }
        let inter = (ix1 - ix0) as i64 * (iy1 - iy0) as i64;
        let area = (b.x1 - b.x0).max(1) as i64 * (b.y1 - b.y0).max(1) as i64;
        inter * 2 < area
    });
}

/// Overlay boxes for annotated evidence (same as UI glance BMP).
pub(crate) fn decision_overlay_boxes(observed: &Observed) -> Vec<CandidateBox> {
    let notice = observed.result.snapshot.page == vision_core::Page::BookingNotice;
    let mut boxes: Vec<CandidateBox> = observed
        .result
        .candidate_set
        .items
        .iter()
        .filter(|c| glance_visual(c, observed.result.snapshot.page))
        .filter(|c| !notice || c.element_id.as_deref() != Some(ELEMENT_SPORT_ICON))
        .map(|c| {
            let box_ = if c.element_id.as_deref() == Some(ELEMENT_SPORT_ICON) {
                c.hit_bbox.as_ref().unwrap_or(&c.bbox)
            } else {
                c.visual_bbox.as_ref().unwrap_or(&c.bbox)
            };
            CandidateBox {
                label: if c.label.is_empty() {
                    c.kind.as_str().to_string()
                } else {
                    c.label.clone()
                },
                x0: box_.x0,
                y0: box_.y0,
                x1: box_.x1,
                y1: box_.y1,
                score: c.score,
                actionable: false,
            }
        })
        .collect();
    if !observed.decision.evidence_boxes.is_empty()
        && matches!(
            observed.decision.intent,
            NextIntent::Navigate { .. } | NextIntent::AtomicComposite { .. }
        )
    {
        for (i, anchor) in observed.decision.evidence_boxes.iter().enumerate() {
            drop_boxes_on_anchor(&mut boxes, anchor.x0, anchor.y0, anchor.x1, anchor.y1);
            boxes.push(CandidateBox {
                label: if i == 0 {
                    "next".into()
                } else {
                    format!("next_{i}")
                },
                x0: anchor.x0,
                y0: anchor.y0,
                x1: anchor.x1,
                y1: anchor.y1,
                score: observed.decision.confidence,
                actionable: true,
            });
        }
    } else if let NextIntent::Navigate { anchor, .. } = &observed.decision.intent {
        drop_boxes_on_anchor(&mut boxes, anchor.x0, anchor.y0, anchor.x1, anchor.y1);
        boxes.push(CandidateBox {
            label: "next".into(),
            x0: anchor.x0,
            y0: anchor.y0,
            x1: anchor.x1,
            y1: anchor.y1,
            score: observed.decision.confidence,
            actionable: true,
        });
    } else if let NextIntent::AtomicComposite { steps, .. } = &observed.decision.intent {
        if let Some(AtomicStep::Click { anchor, .. }) = steps.first() {
            drop_boxes_on_anchor(&mut boxes, anchor.x0, anchor.y0, anchor.x1, anchor.y1);
            boxes.push(CandidateBox {
                label: "next".into(),
                x0: anchor.x0,
                y0: anchor.y0,
                x1: anchor.x1,
                y1: anchor.y1,
                score: observed.decision.confidence,
                actionable: true,
            });
        }
    }
    boxes
}

/// Raw frame for async terminal evidence encoding (run mode). None when not terminal.
pub(crate) fn terminal_evidence_payload(
    observed: &Observed,
) -> Option<(u32, u32, Vec<u8>, Vec<CandidateBox>)> {
    let terminal = matches!(
        observed.decision.intent,
        NextIntent::Stop { .. } | NextIntent::NeedHuman { .. } | NextIntent::Rebind { .. }
    );
    if !terminal {
        return None;
    }
    Some((
        observed.frame.pixel_width,
        observed.frame.pixel_height,
        observed.pixels.clone(),
        decision_overlay_boxes(observed),
    ))
}

pub(crate) fn build_report(
    title: &str,
    observed: &Observed,
    input_invoked: bool,
    mode: GlanceMode,
) -> Result<GlanceReport, String> {
    let boxes = decision_overlay_boxes(observed);
    let can_click = matches!(
        observed.decision.intent,
        NextIntent::Navigate { .. } | NextIntent::AtomicComposite { .. }
    ) || (observed.result.snapshot.page == Page::ConfirmOrder
        && observed.result.snapshot.keyboard_open
        && matches!(
            observed.decision.intent,
            NextIntent::Wait { ref reason, .. } if reason == "CAPTCHA_OCR_PENDING"
        ));
    // Run mode: never sync-encode BMP/base64 on the hot path (incl. terminal).
    // Terminal evidence is handed to the host trace writer as raw BGRA + boxes.
    let image_b64 = if mode.include_image {
        let mut pixels = observed.pixels.clone();
        draw_boxes_bgra(
            observed.frame.pixel_width,
            observed.frame.pixel_height,
            &mut pixels,
            &boxes,
        );
        let bmp = encode_bgra_bmp(
            observed.frame.pixel_width,
            observed.frame.pixel_height,
            &pixels,
        )?;
        b64(&bmp)
    } else {
        String::new()
    };
    let basis = match &observed.decision.intent {
        NextIntent::Navigate { label, anchor, .. } => format!(
            "{label}（{},{}）-（{},{}）",
            anchor.x0, anchor.y0, anchor.x1, anchor.y1
        ),
        NextIntent::AtomicComposite { label, steps, .. } => {
            let head = steps.first().and_then(|s| match s {
                AtomicStep::Click { anchor, .. } => Some(format!(
                    "（{},{}）-（{},{}）",
                    anchor.x0, anchor.y0, anchor.x1, anchor.y1
                )),
                _ => None,
            });
            match head {
                Some(box_) => format!("{label}{box_}"),
                None => label.clone(),
            }
        }
        NextIntent::Wait { reason, approx_ms } => format!("{reason} {approx_ms}ms"),
        NextIntent::Stop { reason }
        | NextIntent::NeedHuman { reason }
        | NextIntent::Rebind { reason } => reason.clone(),
    };
    let (intent_kind, wait_ms) = match &observed.decision.intent {
        NextIntent::Navigate { .. } => ("navigate".into(), None),
        NextIntent::AtomicComposite { .. } => ("atomic_composite".into(), None),
        NextIntent::Wait { approx_ms, .. } => ("wait".into(), Some(*approx_ms)),
        NextIntent::Stop { .. } => ("stop".into(), None),
        NextIntent::NeedHuman { .. } => ("need_human".into(), None),
        NextIntent::Rebind { .. } => ("rebind".into(), None),
    };
    let click_label = match &observed.decision.intent {
        NextIntent::Navigate { label, .. } | NextIntent::AtomicComposite { label, .. } => {
            label.clone()
        }
        _ => String::new(),
    };
    let (nav_anchor, plan_conf) = match &observed.decision.intent {
        NextIntent::Navigate { anchor, .. } => (Some(anchor), observed.decision.confidence),
        NextIntent::AtomicComposite { steps, .. } => {
            let a = steps.iter().find_map(|s| match s {
                AtomicStep::Click { anchor, .. } => Some(anchor),
                _ => None,
            });
            (a, observed.decision.confidence)
        }
        _ => (None, observed.decision.confidence),
    };
    let (date_cov, date_evidence) = if observed.result.snapshot.page == Page::VenueDetail {
        let fw = observed.frame.pixel_width as i32;
        let fh = observed.frame.pixel_height as i32;
        let mut cards = vision_core::locate_date_cards_ex(&observed.result.candidate_set, fw);
        vision_core::enrich_date_cards_color(
            &mut cards, Some(&observed.pixels), fw as u32, fh as u32,
        );
        let goal_date = observed.decision.goal.date.as_deref().unwrap_or("");
        let coverage = vision_core::build_date_strip_coverage(&cards, goal_date, fw, fh);
        let frame_ev = vision_core::assess_frame_date(&cards, goal_date, &coverage);
        let evidence = DateEvidenceSummary {
            cards: cards.iter().map(|c| DateCardSummary {
                month: c.month,
                day: c.day,
                status: format!("{:?}", c.status),
                color_status: c.status_color.map(|s| format!("{s:?}")),
            }).collect(),
            visible_cols: coverage.visible_cols,
            ordered: coverage.ordered,
            continuous: coverage.continuous,
            quality: coverage.quality,
            goal_in_visible_range: coverage.goal_in_visible_range,
            frame_evidence: match frame_ev {
                vision_core::FrameDateEvidence::Bookable(_) => "bookable".into(),
                vision_core::FrameDateEvidence::UnavailableCandidate(_) => "unavailable_candidate".into(),
                vision_core::FrameDateEvidence::AbsentWithCoverage(_) => "absent_with_coverage".into(),
                vision_core::FrameDateEvidence::Incomplete { why } => format!("incomplete:{why}"),
            },
        };
        log::info!(target: "glance",
            "[date_evidence] frame={} goal={} cards={:?} ordered={} continuous={} quality={:.3} goal_in_range={} frame_evidence={}",
            observed.frame.frame_id, goal_date, evidence.cards,
            evidence.ordered, evidence.continuous, evidence.quality,
            evidence.goal_in_visible_range, evidence.frame_evidence);
        (Some(coverage), Some(evidence))
    } else {
        (None, None)
    };
    let layered = layered_confidence(
        &observed.result.snapshot,
        date_cov.as_ref(),
        date_cov.as_ref().map(|c| c.quality),
        plan_conf,
        nav_anchor,
        None,
        24,
    );
    Ok(GlanceReport {
        bound_title: title.to_string(),
        now: observed.decision.now_line.clone(),
        next: observed.decision.next_line.clone(),
        basis,
        page: observed.result.snapshot.page.as_str().to_string(),
        keyboard_open: observed.result.snapshot.keyboard_open,
        refuse_reason: observed
            .decision
            .refuse_reason
            .clone()
            .or_else(|| observed.result.refuse_reason.clone()),
        candidate_count: observed.result.candidate_set.items.len(),
        input_invoked,
        can_click: can_click && layered.click_safety.map(click_safety_ok).unwrap_or(can_click),
        intent_kind,
        wait_ms,
        click_label,
        recognized_captcha: observed.decision.goal.recognized_captcha.clone(),
        page_confidence: layered.page,
        date_confidence: layered.date,
        date_evidence,
        click_safety_confidence: layered.click_safety,
        frame_id: observed.frame.frame_id.clone(),
        frame_width: observed.frame.pixel_width,
        frame_height: observed.frame.pixel_height,
        capture_backend: format!("{:?}", observed.frame.capture_backend),
        surface_diagnostics: serde_json::json!({
            "screen_transform": observed.frame.screen_transform,
            "generation": observed.frame.generation,
            "capture_monotonic_ms": observed.frame.monotonic_capture_time_ms,
            "capture_age_ms": vision_contracts::monotonic_now_ms().saturating_sub(observed.frame.monotonic_capture_time_ms),
            "page_confidence": layered.page,
            "click_safety_confidence": layered.click_safety,
        }),
        timing: observed.result.timing.clone(),
        ocr_hits: observed.ocr.hits().to_vec(),
        candidates: observed.result.candidate_set.items.clone(),
        image_b64,
    })
}

pub(crate) fn log_timing(t_all: std::time::Instant, title: &str, observed: &Observed, report: &GlanceReport) {
    log::info!(
        target: "glance",
        "[decision] page={} now={} next={} refuse={:?} can_click={} boxes={}",
        report.page,
        report.now,
        report.next,
        report.refuse_reason,
        report.can_click,
        observed.result.candidate_set.items.len()
    );
    log::info!(
        target: "glance",
        "[timing] title={} px={}x{} templates={} candidates_ms={} total_ms={}",
        title,
        observed.frame.pixel_width,
        observed.frame.pixel_height,
        observed.template_count,
        observed.result.timing.candidates_ms,
        elapsed_ms(t_all)
    );
}

pub(crate) fn load_min_confidence(data_root: &Path) -> f32 {
    let Some(active) = active_pack_dir(data_root) else {
        return DEFAULT_MIN_CONFIDENCE;
    };
    let Ok(raw) = std::fs::read_to_string(active.join("templates").join("page_rules.json")) else {
        return DEFAULT_MIN_CONFIDENCE;
    };
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::json!({}));
    v.pointer("/shadow/min_confidence")
        .and_then(|x| x.as_f64())
        .map(|f| f as f32)
        .unwrap_or(DEFAULT_MIN_CONFIDENCE)
}

pub(crate) fn b64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | bytes[i + 2] as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    if i < bytes.len() {
        let b0 = bytes[i] as u32;
        let b1 = if i + 1 < bytes.len() {
            bytes[i + 1] as u32
        } else {
            0
        };
        let n = (b0 << 16) | (b1 << 8);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(T[((n >> 6) & 63) as usize] as char);
            out.push('=');
        } else {
            out.push('=');
            out.push('=');
        }
    }
    out
}
}

#[cfg(windows)]
pub(crate) use imp::*;
