//! Page-map real-capture acceptance (Phase 4): fixture schema + offline eval.
//!
//! Hand-filled OCR fixtures prove "rules work given correct text".
//! Capture + real OCR eval proves "live frames classify correctly".
//! Never treat annotated-only replay as end-to-end accuracy.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use vision_contracts::ActionType;

use crate::ocr::{OcrHit, OcrRegionCache};
use crate::shadow::{
    observe_shadow, CandidateBox, ShadowObserveInput, ShadowTiming, DEFAULT_MIN_CONFIDENCE,
};
use crate::state::Page;

/// Ordered checklist for WeChat -> XMU booking path (plus variants).
pub const CAPTURE_CHECKLIST: &[&str] = &[
    "wechat_main",
    "wechat_discover",
    "wechat_mp_panel",
    "wechat_mp_search",
    "xmu_login_gate",
    "xmu_wx_authorize",
    "xmu_auth_waiting",
    "home",
    "venue_sport_list",
    "venue_list",
    "venue_detail",
    "select_slot",
    // Variants (1-2 each)
    "overlay_notice",
    "loading",
    "select_slot_empty",
    "venue_detail_alt",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleSplit {
    Dev,
    Holdout,
}

impl Default for SampleSplit {
    fn default() -> Self {
        Self::Dev
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrSource {
    /// Human-typed / curated hits -- rules-only proof, not E2E.
    Annotated,
    /// Windows.Media.Ocr (or other system OCR) on a real frame.
    WindowsMedia,
    /// Pack ONNX (when available).
    PackOnnx,
    Unknown,
}

impl Default for OcrSource {
    fn default() -> Self {
        Self::Unknown
    }
}

/// Who authored `page_label` / `expected`. Capture must stay `pending` until confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelSource {
    /// Capture just wrote suggested fields; not scorable.
    Pending,
    /// Human typed/confirmed label while looking at the frame (may equal suggested).
    Human,
    /// Repo hand-annotated fixture (rules regression only, not E2E).
    AnnotatedFixture,
}

impl Default for LabelSource {
    fn default() -> Self {
        Self::Pending
    }
}

/// Core path pages that holdout must cover before e2e_ready.
pub const HOLDOUT_REQUIRED_PAGES: &[&str] = &[
    "wechat_main",
    "wechat_discover",
    "wechat_mp_panel",
    "wechat_mp_search",
    "xmu_login_gate",
    "xmu_wx_authorize",
    "xmu_auth_waiting",
    "home",
    "venue_sport_list",
    "venue_list",
    "venue_detail",
    "select_slot",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameSize {
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyBox {
    pub label: String,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// When false/absent after human review, treat identity as unverified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpectedOutcome {
    pub page: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refuse_reason: Option<String>,
    #[serde(default = "default_stop")]
    pub action_type: String,
    /// Concrete venue name; omit or `"UNKNOWN"` when not human-verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue_name: Option<String>,
}

fn default_stop() -> String {
    "stop".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageMapSample {
    pub id: String,
    #[serde(default = "default_platform")]
    pub platform: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub split: SampleSplit,
    #[serde(default)]
    pub ocr_source: OcrSource,
    /// Suggested by classifier at capture time (may be wrong).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_label_suggested: Option<String>,
    /// Human page label ? must come from confirm / annotated fixture, never auto-copied from suggested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_label: Option<String>,
    /// Checklist slot at capture time (hint only; not a ground-truth label).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checklist_hint: Option<String>,
    #[serde(default)]
    pub label_source: LabelSource,
    #[serde(default)]
    pub human_confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nav_note: Option<String>,
    pub frame: FrameSize,
    pub ocr_hits: Vec<OcrHit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_boxes_suggested: Vec<KeyBox>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_boxes: Vec<KeyBox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<ExpectedOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_meta: Option<serde_json::Value>,
}

fn default_platform() -> String {
    "windows".into()
}

fn default_kind() -> String {
    "page_map_annotated".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageVerdict {
    Correct,
    Wrong,
    Unknown,
    /// Sample not human-confirmed -- skipped from acceptance gate.
    SkippedUnconfirmed,
}

impl PageVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Correct => "correct",
            Self::Wrong => "wrong",
            Self::Unknown => "unknown",
            Self::SkippedUnconfirmed => "skipped_unconfirmed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoxError {
    pub label: String,
    pub iou: f32,
    pub predicted: Option<KeyBox>,
    pub expected: KeyBox,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SampleEval {
    pub id: String,
    pub split: SampleSplit,
    pub ocr_source: OcrSource,
    pub expected_page: String,
    pub predicted_page: String,
    pub confidence: f32,
    pub refuse_reason: Option<String>,
    pub action_type: String,
    pub input_invoked: bool,
    pub verdict: String,
    pub box_errors: Vec<BoxError>,
    pub venue_status: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfusionCell {
    pub expected: String,
    pub predicted: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PageMapEvalReport {
    pub sample_count: usize,
    pub scored_count: usize,
    pub correct: usize,
    pub wrong: usize,
    pub unknown: usize,
    pub skipped_unconfirmed: usize,
    pub annotated_only_count: usize,
    pub real_ocr_count: usize,
    pub input_invoked_any: bool,
    pub confusion: Vec<ConfusionCell>,
    pub samples: Vec<SampleEval>,
    pub wrong_examples: Vec<SampleEval>,
    pub checklist_missing: Vec<String>,
    pub e2e_ready: bool,
    pub gate_notes: Vec<String>,
}

pub fn box_iou(a: &KeyBox, b: &KeyBox) -> f32 {
    let x0 = a.x0.max(b.x0);
    let y0 = a.y0.max(b.y0);
    let x1 = a.x1.min(b.x1);
    let y1 = a.y1.min(b.y1);
    let iw = (x1 - x0).max(0) as f32;
    let ih = (y1 - y0).max(0) as f32;
    let inter = iw * ih;
    if inter <= 0.0 {
        return 0.0;
    }
    let area_a = ((a.x1 - a.x0).max(0) * (a.y1 - a.y0).max(0)) as f32;
    let area_b = ((b.x1 - b.x0).max(0) * (b.y1 - b.y0).max(0)) as f32;
    let uni = area_a + area_b - inter;
    if uni <= 0.0 {
        0.0
    } else {
        inter / uni
    }
}

/// Prefill CTA boxes from OCR needles for human review (never Phase-5 safe).
pub fn suggest_key_boxes(ocr: &OcrRegionCache, page: Page) -> Vec<KeyBox> {
    let needles: &[(&str, &str)] = match page {
        Page::XmuLoginGate => &[
            ("\u{7acb}\u{5373}\u{767b}\u{5f55}", "primary_cta"),
            (
                "\u{7edf}\u{4e00}\u{8eab}\u{4efd}\u{8ba4}\u{8bc1}",
                "secondary_cta",
            ),
        ],
        Page::XmuWxAuthorize => &[
            ("\u{6388}\u{6743}\u{767b}\u{5f55}", "primary_cta"),
            ("\u{53d6}\u{6d88}\u{767b}\u{5f55}", "cancel"),
            ("\u{7528}\u{6237}\u{534f}\u{8bae}", "checkbox_hint"),
        ],
        Page::WechatMpSearch => &[("\u{641c}\u{7d22}", "search")],
        Page::Home => &[
            ("\u{573a}\u{9986}\u{9884}\u{7ea6}", "primary_cta"),
            ("\u{6211}\u{7684}\u{9884}\u{7ea6}", "secondary"),
        ],
        Page::VenueDetail => &[
            ("\u{9884}\u{5b9a}", "primary_cta"),
            ("\u{9884}\u{7ea6}", "primary_cta"),
        ],
        Page::SelectSlot => &[("\u{786e}\u{8ba4}\u{9884}\u{7ea6}", "primary_cta")],
        Page::WechatDiscover => &[("\u{5c0f}\u{7a0b}\u{5e8f}", "nav")],
        Page::WechatMpPanel => &[
            ("\u{641c}\u{7d22}", "search"),
            ("\u{5c0f}\u{7a0b}\u{5e8f}", "chrome"),
        ],
        _ => &[],
    };
    let mut out = Vec::new();
    for (needle, role) in needles {
        if let Some(h) = ocr.find_text(needle, 0.5) {
            out.push(KeyBox {
                label: (*needle).into(),
                x0: h.x,
                y0: h.y,
                x1: h.x + h.w,
                y1: h.y + h.h,
                role: Some((*role).into()),
                verified: Some(false),
            });
        }
    }
    out
}

fn predicted_key_boxes_from_shadow(boxes: &[CandidateBox]) -> Vec<KeyBox> {
    boxes
        .iter()
        .filter(|b| b.label.starts_with("key:"))
        .map(|b| KeyBox {
            label: b.label.trim_start_matches("key:").to_string(),
            x0: b.x0,
            y0: b.y0,
            x1: b.x1,
            y1: b.y1,
            role: Some("predicted".into()),
            verified: None,
        })
        .collect()
}

fn match_box_errors(expected: &[KeyBox], predicted: &[KeyBox], min_iou: f32) -> Vec<BoxError> {
    let mut errs = Vec::new();
    for exp in expected {
        if exp.verified == Some(false) {
            continue;
        }
        let best = predicted
            .iter()
            .filter(|p| {
                p.label == exp.label || exp.label.contains(&p.label) || p.label.contains(&exp.label)
            })
            .map(|p| (box_iou(exp, p), p))
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        match best {
            Some((iou, _)) if iou >= min_iou => {}
            Some((iou, p)) => errs.push(BoxError {
                label: exp.label.clone(),
                iou,
                predicted: Some(p.clone()),
                expected: exp.clone(),
            }),
            None => errs.push(BoxError {
                label: exp.label.clone(),
                iou: 0.0,
                predicted: None,
                expected: exp.clone(),
            }),
        }
    }
    errs
}

fn venue_status(sample: &PageMapSample) -> String {
    let name = sample
        .venue_name
        .as_deref()
        .or_else(|| {
            sample
                .expected
                .as_ref()
                .and_then(|e| e.venue_name.as_deref())
        })
        .unwrap_or("");
    let needs = matches!(
        sample
            .page_label
            .as_deref()
            .or(sample.expected.as_ref().map(|e| e.page.as_str())),
        Some("venue_detail" | "venue_list" | "select_slot")
    );
    if !needs {
        return "n/a".into();
    }
    if name.is_empty() || name.eq_ignore_ascii_case("UNKNOWN") {
        "UNKNOWN".into()
    } else {
        "verified".into()
    }
}

/// Effective authorship of the ground-truth page label.
pub fn effective_label_source(sample: &PageMapSample) -> LabelSource {
    match sample.label_source {
        LabelSource::Human | LabelSource::AnnotatedFixture => sample.label_source,
        LabelSource::Pending => {
            // Legacy repo fixtures: annotated + confirmed + expected, before label_source existed.
            if sample.human_confirmed
                && matches!(sample.ocr_source, OcrSource::Annotated)
                && sample.expected.is_some()
            {
                LabelSource::AnnotatedFixture
            } else {
                LabelSource::Pending
            }
        }
    }
}

fn has_ground_truth_page(sample: &PageMapSample) -> bool {
    sample
        .page_label
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
        || sample
            .expected
            .as_ref()
            .map(|e| !e.page.trim().is_empty())
            .unwrap_or(false)
}

/// Heuristic PII / sensitive OCR scan. No BMP does not mean safe to commit.
pub fn scan_ocr_sensitivity(hits: &[OcrHit]) -> Vec<String> {
    let mut findings = Vec::new();
    for h in hits {
        let t = h.text.trim();
        if t.is_empty() {
            continue;
        }
        let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 8 && digits.len() <= 18 && digits.len() * 2 >= t.chars().count() {
            findings.push(format!("digit_heavy:{t}"));
        }
        let lower = t.to_lowercase();
        for needle in [
            "学号",
            "身份证",
            "手机号",
            "电话",
            "微信号",
            "文件传输助手",
            "@",
        ] {
            if t.contains(needle) || lower.contains(needle) {
                findings.push(format!("keyword:{needle} in {t}"));
            }
        }
        if (t.contains("同学") || t.contains("老师") || t.contains("先生") || t.contains("女士"))
            && t.chars().count() <= 12
        {
            findings.push(format!("person_hint:{t}"));
        }
    }
    findings.sort();
    findings.dedup();
    findings
}

/// Evaluate one annotated / captured sample. `require_confirmed` skips unconfirmed captures.
pub fn eval_sample(
    sample: &PageMapSample,
    require_confirmed: bool,
    min_box_iou: f32,
) -> SampleEval {
    let mut notes = Vec::new();
    if matches!(sample.ocr_source, OcrSource::Annotated) {
        notes.push("ocr_source=annotated_not_e2e".into());
    }
    let expected_page = sample
        .page_label
        .clone()
        .or_else(|| sample.expected.as_ref().map(|e| e.page.clone()))
        .unwrap_or_else(|| "unknown".into());

    let label_src = effective_label_source(sample);
    // Unconfirmed / pending labels never score -- even if older capture prefilled `expected`.
    if require_confirmed
        && (!sample.human_confirmed
            || matches!(label_src, LabelSource::Pending)
            || !has_ground_truth_page(sample))
    {
        return SampleEval {
            id: sample.id.clone(),
            split: sample.split,
            ocr_source: sample.ocr_source,
            expected_page,
            predicted_page: String::new(),
            confidence: 0.0,
            refuse_reason: None,
            action_type: "stop".into(),
            input_invoked: false,
            verdict: PageVerdict::SkippedUnconfirmed.as_str().into(),
            box_errors: Vec::new(),
            venue_status: venue_status(sample),
            notes: {
                if !sample.human_confirmed {
                    notes.push("awaiting_human_confirm".into());
                }
                if matches!(label_src, LabelSource::Pending) {
                    notes.push("label_source_pending_use_confirm_cmd".into());
                }
                if !has_ground_truth_page(sample) {
                    notes.push("page_label_missing".into());
                }
                notes
            },
        };
    }

    let ocr = OcrRegionCache::from_hits(sample.ocr_hits.clone());
    let r = observe_shadow(ShadowObserveInput {
        run_id: "page-map-eval",
        lease_id: "lease-eval",
        generation: 1,
        frame_id: &sample.id,
        frame_w: sample.frame.width,
        frame_h: sample.frame.height,
        ocr: &ocr,
        title_hint: String::new(),
        min_confidence: sample
            .expected
            .as_ref()
            .and_then(|e| e.min_confidence)
            .unwrap_or(DEFAULT_MIN_CONFIDENCE),
        target_slot_time: None,
        timing: ShadowTiming::default(),
        perception: crate::candidates::PerceptionFeed::default(),
    });

    let predicted = r.snapshot.page.as_str().to_string();
    let verdict = if predicted == "unknown" && expected_page != "unknown" {
        PageVerdict::Unknown
    } else if predicted == expected_page {
        PageVerdict::Correct
    } else {
        PageVerdict::Wrong
    };

    if expected_page == "select_slot" {
        let refuse = r.refuse_reason.as_deref().unwrap_or("");
        if refuse != "SLOT_GRID_NOT_LOCALIZED" && refuse != "NO_AVAILABLE_SLOTS" {
            notes.push(format!(
                "select_slot_refuse_expected_SLOT_GRID_or_EMPTY got={refuse}"
            ));
        }
        if !matches!(r.action_plan.action_type, ActionType::Stop) {
            notes.push("select_slot_must_Stop".into());
        }
    }

    // Only human key_boxes count; never treat suggested boxes as ground truth.
    let human_boxes: Vec<KeyBox> = sample
        .key_boxes
        .iter()
        .filter(|b| b.verified != Some(false))
        .cloned()
        .collect();
    let predicted_boxes = predicted_key_boxes_from_shadow(&r.candidate_boxes);
    let box_errors = match_box_errors(&human_boxes, &predicted_boxes, min_box_iou);

    if r.input_invoked {
        notes.push("INPUT_INVOKED_VIOLATION".into());
    }

    SampleEval {
        id: sample.id.clone(),
        split: sample.split,
        ocr_source: sample.ocr_source,
        expected_page,
        predicted_page: predicted,
        confidence: r.snapshot.confidence,
        refuse_reason: r.refuse_reason,
        action_type: format!("{:?}", r.action_plan.action_type).to_lowercase(),
        input_invoked: r.input_invoked,
        verdict: verdict.as_str().into(),
        box_errors,
        venue_status: venue_status(sample),
        notes,
    }
}

pub fn build_eval_report(
    samples: &[PageMapSample],
    require_confirmed: bool,
    min_box_iou: f32,
    checklist: &[&str],
) -> PageMapEvalReport {
    let mut evals: Vec<SampleEval> = samples
        .iter()
        .map(|s| eval_sample(s, require_confirmed, min_box_iou))
        .collect();

    let mut confusion_map: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut correct = 0usize;
    let mut wrong = 0usize;
    let mut unknown = 0usize;
    let mut skipped = 0usize;
    let mut annotated = 0usize;
    let mut real_ocr = 0usize;
    let mut input_any = false;
    let mut covered: BTreeSet<String> = BTreeSet::new();

    for e in &evals {
        input_any |= e.input_invoked;
        match e.ocr_source {
            OcrSource::Annotated => annotated += 1,
            OcrSource::WindowsMedia | OcrSource::PackOnnx => real_ocr += 1,
            OcrSource::Unknown => {}
        }
        match e.verdict.as_str() {
            "correct" => {
                correct += 1;
                covered.insert(e.expected_page.clone());
                *confusion_map
                    .entry((e.expected_page.clone(), e.predicted_page.clone()))
                    .or_default() += 1;
            }
            "wrong" => {
                wrong += 1;
                covered.insert(e.expected_page.clone());
                *confusion_map
                    .entry((e.expected_page.clone(), e.predicted_page.clone()))
                    .or_default() += 1;
            }
            "unknown" => {
                unknown += 1;
                covered.insert(e.expected_page.clone());
                *confusion_map
                    .entry((e.expected_page.clone(), e.predicted_page.clone()))
                    .or_default() += 1;
            }
            _ => skipped += 1,
        }
    }

    let confusion: Vec<ConfusionCell> = confusion_map
        .into_iter()
        .map(|((expected, predicted), count)| ConfusionCell {
            expected,
            predicted,
            count,
        })
        .collect();

    let wrong_examples: Vec<SampleEval> = evals
        .iter()
        .filter(|e| e.verdict == "wrong" || e.verdict == "unknown")
        .cloned()
        .collect();

    let checklist_missing: Vec<String> = checklist
        .iter()
        .filter(|p| {
            let optional = matches!(
                **p,
                "overlay_notice" | "loading" | "select_slot_empty" | "venue_detail_alt"
            );
            !optional && !covered.contains(**p)
        })
        .map(|s| (*s).to_string())
        .collect();

    let mut gate_notes = Vec::new();
    if annotated > 0 && real_ocr == 0 {
        gate_notes.push(
            "only_annotated_ocr: proves rule routing, NOT real-screenshot E2E accuracy".into(),
        );
    }
    if input_any {
        gate_notes.push("FAIL: input_invoked observed".into());
    }
    if !checklist_missing.is_empty() {
        gate_notes.push(format!(
            "checklist_incomplete: {}",
            checklist_missing.join(",")
        ));
    }
    if wrong > 0 || unknown > 0 {
        gate_notes.push(format!("misclassified_or_unknown={}+{}", wrong, unknown));
    }
    let similar_confusions: Vec<_> = confusion
        .iter()
        .filter(|c| c.expected != c.predicted)
        .collect();
    if !similar_confusions.is_empty() {
        gate_notes.push(format!(
            "confusion_pairs={}",
            similar_confusions
                .iter()
                .map(|c| format!("{}->{}", c.expected, c.predicted))
                .collect::<Vec<_>>()
                .join(";")
        ));
    }

    let mut holdout_covered: BTreeSet<String> = BTreeSet::new();
    let mut human_labeled = 0usize;
    let mut pending_or_unconfirmed = 0usize;
    for (s, e) in samples.iter().zip(evals.iter()) {
        let src = effective_label_source(s);
        if matches!(src, LabelSource::Human) {
            human_labeled += 1;
        }
        if e.verdict == "skipped_unconfirmed" {
            pending_or_unconfirmed += 1;
        }
        if matches!(s.split, SampleSplit::Holdout)
            && e.verdict != "skipped_unconfirmed"
            && matches!(src, LabelSource::Human)
        {
            holdout_covered.insert(e.expected_page.clone());
        }
    }
    let holdout_missing: Vec<String> = HOLDOUT_REQUIRED_PAGES
        .iter()
        .filter(|p| !holdout_covered.contains(**p))
        .map(|s| (*s).to_string())
        .collect();
    if !holdout_missing.is_empty() {
        gate_notes.push(format!("holdout_incomplete: {}", holdout_missing.join(",")));
    }
    if human_labeled == 0 && real_ocr > 0 {
        gate_notes.push("no_independent_human_labels_on_real_ocr".into());
    }
    if pending_or_unconfirmed > 0 {
        gate_notes.push(format!(
            "unconfirmed_or_pending_labels={pending_or_unconfirmed}"
        ));
    }

    let slot_ok = evals.iter().all(|e| {
        if e.expected_page != "select_slot" || e.verdict == "skipped_unconfirmed" {
            return true;
        }
        matches!(
            e.refuse_reason.as_deref(),
            Some("SLOT_GRID_NOT_LOCALIZED") | Some("NO_AVAILABLE_SLOTS")
        ) && e.action_type.contains("stop")
    });

    let e2e_ready = real_ocr > 0
        && wrong == 0
        && unknown == 0
        && !input_any
        && checklist_missing.is_empty()
        && holdout_missing.is_empty()
        && human_labeled > 0
        && pending_or_unconfirmed == 0
        && slot_ok;

    if e2e_ready {
        gate_notes.push("e2e_path_gate_pass_pending_human_box_venue_signoff".into());
    } else {
        gate_notes.push("e2e_path_gate_fail_do_not_enter_phase_5".into());
    }

    evals.sort_by(|a, b| a.id.cmp(&b.id));

    PageMapEvalReport {
        sample_count: samples.len(),
        scored_count: correct + wrong + unknown,
        correct,
        wrong,
        unknown,
        skipped_unconfirmed: skipped,
        annotated_only_count: annotated,
        real_ocr_count: real_ocr,
        input_invoked_any: input_any,
        confusion,
        samples: evals,
        wrong_examples,
        checklist_missing,
        e2e_ready,
        gate_notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(text: &str, x: i32, y: i32) -> OcrHit {
        OcrHit {
            text: text.into(),
            x,
            y,
            w: 80,
            h: 20,
            score: 0.9,
        }
    }

    #[test]
    fn iou_overlap() {
        let a = KeyBox {
            label: "a".into(),
            x0: 0,
            y0: 0,
            x1: 10,
            y1: 10,
            role: None,
            verified: None,
        };
        let b = KeyBox {
            label: "a".into(),
            x0: 0,
            y0: 0,
            x1: 10,
            y1: 10,
            role: None,
            verified: None,
        };
        assert!((box_iou(&a, &b) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn annotated_login_scores_correct_but_not_e2e_ready_alone() {
        let sample = PageMapSample {
            id: "t1".into(),
            platform: "windows".into(),
            kind: "page_map_annotated".into(),
            split: SampleSplit::Dev,
            ocr_source: OcrSource::Annotated,
            page_label_suggested: None,
            page_label: Some("xmu_login_gate".into()),
            checklist_hint: None,
            label_source: LabelSource::AnnotatedFixture,
            human_confirmed: true,
            source: None,
            nav_note: None,
            frame: FrameSize {
                width: 390,
                height: 780,
            },
            ocr_hits: vec![
                hit("\u{53a6}\u{95e8}\u{5927}\u{5b66}", 150, 48),
                hit("\u{7acb}\u{5373}\u{767b}\u{5f55}", 70, 520),
                hit("\u{7edf}\u{4e00}\u{8eab}\u{4efd}\u{8ba4}\u{8bc1}", 70, 580),
            ],
            key_boxes_suggested: Vec::new(),
            key_boxes: vec![KeyBox {
                label: "\u{7acb}\u{5373}\u{767b}\u{5f55}".into(),
                x0: 70,
                y0: 520,
                x1: 320,
                y1: 564,
                role: Some("primary_cta".into()),
                verified: Some(true),
            }],
            expected: Some(ExpectedOutcome {
                page: "xmu_login_gate".into(),
                min_confidence: Some(0.55),
                refuse_reason: Some("OBSERVE_ONLY_PENDING_PAGE_MAP".into()),
                action_type: "stop".into(),
                venue_name: None,
            }),
            venue_name: None,
            ocr_backend: None,
            ocr_language: None,
            window: None,
            capture_meta: None,
        };
        let report = build_eval_report(&[sample], true, 0.3, CAPTURE_CHECKLIST);
        assert_eq!(report.correct, 1);
        assert!(!report.e2e_ready);
        assert!(report.gate_notes.iter().any(|n| n.contains("annotated")));
        assert!(!report.input_invoked_any);
    }

    #[test]
    fn wrong_page_fills_confusion() {
        let sample = PageMapSample {
            id: "t2".into(),
            platform: "windows".into(),
            kind: "page_map_capture".into(),
            split: SampleSplit::Holdout,
            ocr_source: OcrSource::WindowsMedia,
            page_label_suggested: Some("home".into()),
            page_label: Some("home".into()),
            checklist_hint: None,
            label_source: LabelSource::Human,
            human_confirmed: true,
            source: None,
            nav_note: None,
            frame: FrameSize {
                width: 400,
                height: 700,
            },
            ocr_hits: vec![
                hit("\u{53a6}\u{95e8}\u{5927}\u{5b66}", 150, 48),
                hit("\u{7acb}\u{5373}\u{767b}\u{5f55}", 70, 520),
                hit("\u{7edf}\u{4e00}\u{8eab}\u{4efd}\u{8ba4}\u{8bc1}", 70, 580),
            ],
            key_boxes_suggested: Vec::new(),
            key_boxes: Vec::new(),
            expected: Some(ExpectedOutcome {
                page: "home".into(),
                min_confidence: None,
                refuse_reason: None,
                action_type: "stop".into(),
                venue_name: None,
            }),
            venue_name: None,
            ocr_backend: Some("windows_media".into()),
            ocr_language: Some("zh-Hans".into()),
            window: None,
            capture_meta: None,
        };
        let report = build_eval_report(&[sample], true, 0.3, &["home"]);
        assert_eq!(report.wrong, 1);
        assert!(!report.e2e_ready);
        assert!(report
            .confusion
            .iter()
            .any(|c| c.expected == "home" && c.predicted == "xmu_login_gate"));
    }

    #[test]
    fn expected_prefill_without_confirm_does_not_score() {
        let sample = PageMapSample {
            id: "trap".into(),
            platform: "windows".into(),
            kind: "page_map_capture".into(),
            split: SampleSplit::Dev,
            ocr_source: OcrSource::WindowsMedia,
            page_label_suggested: Some("xmu_login_gate".into()),
            page_label: Some("xmu_login_gate".into()),
            checklist_hint: Some("xmu_login_gate".into()),
            label_source: LabelSource::Pending,
            human_confirmed: false,
            source: None,
            nav_note: None,
            frame: FrameSize {
                width: 390,
                height: 780,
            },
            ocr_hits: vec![
                hit("厦门大学", 150, 48),
                hit("立即登录", 70, 520),
                hit("统一身份认证", 70, 580),
            ],
            key_boxes_suggested: Vec::new(),
            key_boxes: Vec::new(),
            expected: Some(ExpectedOutcome {
                page: "xmu_login_gate".into(),
                min_confidence: None,
                refuse_reason: None,
                action_type: "stop".into(),
                venue_name: None,
            }),
            venue_name: None,
            ocr_backend: None,
            ocr_language: None,
            window: None,
            capture_meta: None,
        };
        let e = eval_sample(&sample, true, 0.3);
        assert_eq!(e.verdict, "skipped_unconfirmed");
        let report = build_eval_report(&[sample], true, 0.3, &["xmu_login_gate"]);
        assert_eq!(report.correct, 0);
        assert!(!report.e2e_ready);
    }

    #[test]
    fn bulk_confirm_still_pending_label_source_does_not_score() {
        let sample = PageMapSample {
            id: "bulk".into(),
            platform: "windows".into(),
            kind: "page_map_capture".into(),
            split: SampleSplit::Dev,
            ocr_source: OcrSource::WindowsMedia,
            page_label_suggested: Some("home".into()),
            page_label: Some("home".into()),
            checklist_hint: Some("home".into()),
            label_source: LabelSource::Pending,
            human_confirmed: true,
            source: None,
            nav_note: None,
            frame: FrameSize {
                width: 400,
                height: 700,
            },
            ocr_hits: vec![hit("场馆预约", 40, 200), hit("我的预约", 40, 260)],
            key_boxes_suggested: Vec::new(),
            key_boxes: Vec::new(),
            expected: Some(ExpectedOutcome {
                page: "home".into(),
                min_confidence: None,
                refuse_reason: None,
                action_type: "stop".into(),
                venue_name: None,
            }),
            venue_name: None,
            ocr_backend: None,
            ocr_language: None,
            window: None,
            capture_meta: None,
        };
        let e = eval_sample(&sample, true, 0.3);
        assert_eq!(e.verdict, "skipped_unconfirmed");
    }

    #[test]
    fn scan_flags_digit_heavy_ocr() {
        let hits = vec![hit("31820221123456", 10, 10)];
        let f = scan_ocr_sensitivity(&hits);
        assert!(f.iter().any(|x| x.starts_with("digit_heavy:")));
    }
}
