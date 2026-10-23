//! Offline replay corpus (Plan P6): promote failed-run observations → cases;
//! run `observe_shadow` + `decide_next` against expected; coarse recall / false-click metrics.
//!
//! Layout: `testdata/replay/<case_id>/{manifest.json,frame.json,expected.json}`
//! No raw screenshots in git by default (OCR annotations only).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use vision_contracts::ActionType;

use crate::intent::{decide_next, NextIntent, TaskGoal};
use crate::ocr::{OcrHit, OcrRegionCache};
use crate::page_map::{scan_ocr_sensitivity, FrameSize, LabelSource};
use crate::shadow::{
    observe_shadow, ShadowObserveInput, ShadowTiming, DEFAULT_MIN_CONFIDENCE,
};

/// Target coverage matrix tags (Spec spirit). Filling every cell is out of P6 scope.
pub const REPLAY_MATRIX_TAGS: &[&str] = &[
    "dpi_100",
    "dpi_125",
    "dpi_150",
    "animating",
    "split_boxes",
    "cross_month",
    "date_strip_partial",
    "overlay_notice",
    "slot_grid_unlocalized",
];

pub const REPLAY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplaySource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayManifest {
    pub case_id: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_platform")]
    pub platform: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ReplaySource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_label: Option<String>,
    #[serde(default)]
    pub goal: TaskGoal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default)]
    pub label_source: LabelSource,
    #[serde(default)]
    pub human_confirmed: bool,
}

fn default_schema_version() -> u32 {
    REPLAY_SCHEMA_VERSION
}
fn default_platform() -> String {
    "windows".into()
}
fn default_kind() -> String {
    "replay_case".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayFrame {
    pub frame: FrameSize,
    pub ocr_hits: Vec<OcrHit>,
    #[serde(default)]
    pub title_hint: String,
    /// Optional local-only BMP relative path; never required for offline scoring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_bmp: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayExpected {
    pub page: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refuse_reason: Option<String>,
    #[serde(default = "default_shadow_action")]
    pub shadow_action_type: String,
    /// `navigate` | `atomic_composite` | `wait` | `stop` | `need_human` | `rebind`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_reason: Option<String>,
    /// Annotated booking date (YYYY-MM-DD or MD label); not always the OCR text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
}

fn default_shadow_action() -> String {
    "stop".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayCase {
    pub manifest: ReplayManifest,
    pub frame: ReplayFrame,
    pub expected: ReplayExpected,
}

#[derive(Debug, Clone, Default)]
pub struct PromoteAnnotations {
    pub page_label: Option<String>,
    pub date: Option<String>,
    pub intent_kind: Option<String>,
    pub intent_label: Option<String>,
    pub intent_reason: Option<String>,
    pub tags: Vec<String>,
    pub notes: Option<String>,
    /// When true, mark `human_confirmed` (operator asserts labels while looking at evidence).
    pub human_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplayCaseResult {
    pub case_id: String,
    pub page_ok: bool,
    pub shadow_refuse_ok: bool,
    pub intent_ok: bool,
    /// Shadow emitted Click, or decide_next chose a clicky intent when expected was stop/wait without click.
    pub false_click: bool,
    pub predicted_page: String,
    pub predicted_intent_kind: String,
    pub predicted_intent_label: Option<String>,
    pub predicted_intent_reason: Option<String>,
    pub shadow_refuse: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplayCorpusReport {
    pub case_count: usize,
    pub scored_count: usize,
    pub page_correct: usize,
    pub intent_correct: usize,
    pub false_click_count: usize,
    /// page_correct / scored_count (0 when unscored)
    pub page_recall: f32,
    /// intent_correct / scored_count
    pub intent_match: f32,
    pub cases: Vec<ReplayCaseResult>,
}

/// Rewrite digit-heavy / person-hint / keyword OCR texts for safe corpus commit.
pub fn desensitize_ocr_hits(hits: &[OcrHit]) -> (Vec<OcrHit>, Vec<String>) {
    let findings = scan_ocr_sensitivity(hits);
    let mut out = Vec::with_capacity(hits.len());
    for h in hits {
        let t = h.text.trim();
        let mut text = h.text.clone();
        let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 8 && digits.len() <= 18 && digits.len() * 2 >= t.chars().count() {
            text = "[REDACTED_DIGITS]".into();
        } else if (t.contains("同学") || t.contains("老师") || t.contains("先生") || t.contains("女士"))
            && t.chars().count() <= 12
        {
            text = "[REDACTED_NAME]".into();
        } else {
            for needle in ["学号", "身份证", "手机号", "电话", "微信号"] {
                if t.contains(needle) {
                    text = format!("[REDACTED:{needle}]");
                    break;
                }
            }
        }
        out.push(OcrHit {
            text,
            x: h.x,
            y: h.y,
            w: h.w,
            h: h.h,
            score: h.score,
        });
    }
    (out, findings)
}

pub fn load_replay_case(dir: &Path) -> Result<ReplayCase, String> {
    let manifest: ReplayManifest = read_json(&dir.join("manifest.json"))?;
    let frame: ReplayFrame = read_json(&dir.join("frame.json"))?;
    let expected: ReplayExpected = read_json(&dir.join("expected.json"))?;
    Ok(ReplayCase {
        manifest,
        frame,
        expected,
    })
}

pub fn write_replay_case(dir: &Path, case: &ReplayCase) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    write_json(&dir.join("manifest.json"), &case.manifest)?;
    write_json(&dir.join("frame.json"), &case.frame)?;
    write_json(&dir.join("expected.json"), &case.expected)?;
    Ok(())
}

pub fn load_replay_corpus(root: &Path) -> Result<Vec<ReplayCase>, String> {
    if !root.is_dir() {
        return Err(format!("corpus root missing: {}", root.display()));
    }
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.join("manifest.json").is_file())
        .collect();
    dirs.sort();
    let mut cases = Vec::new();
    for d in dirs {
        cases.push(load_replay_case(&d)?);
    }
    Ok(cases)
}

/// Build a desensitized case from one TraceEnvelope observation (JSON object).
pub fn promote_observation(
    envelope: &serde_json::Value,
    case_id: &str,
    annotations: PromoteAnnotations,
) -> Result<ReplayCase, String> {
    let payload = envelope
        .get("payload")
        .ok_or_else(|| "envelope missing payload".to_string())?;
    let obs = payload
        .get("observation")
        .ok_or_else(|| "payload missing observation".to_string())?;
    let hits_val = obs
        .get("ocr_hits")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let hits: Vec<OcrHit> = serde_json::from_value(hits_val)
        .map_err(|e| format!("ocr_hits parse: {e}"))?;
    if hits.is_empty() {
        return Err(
            "observation has empty ocr_hits; re-run with detailed diagnostics enabled".into(),
        );
    }
    let (ocr_hits, sensitivity) = desensitize_ocr_hits(&hits);
    let fw = obs
        .get("frame_width")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;
    let fh = obs
        .get("frame_height")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;
    if fw <= 0 || fh <= 0 {
        return Err("observation missing frame_width/frame_height".into());
    }
    let title_hint = obs
        .get("bound_title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let goal: TaskGoal = payload
        .get("goal")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| format!("goal parse: {e}"))?
        .unwrap_or_default();
    let mut goal = goal;
    if let Some(d) = annotations.date.clone() {
        goal.date = Some(d);
    }
    let page_from_obs = obs
        .get("page")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let page_label = annotations
        .page_label
        .clone()
        .or(page_from_obs.clone());
    let intent_kind = annotations.intent_kind.clone().or_else(|| {
        obs.get("intent_kind")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    });
    let intent_label = annotations.intent_label.clone().or_else(|| {
        let label = obs
            .get("click_label")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if label.is_empty() {
            None
        } else {
            Some(label.to_string())
        }
    });
    let intent_reason = annotations.intent_reason.clone().or_else(|| {
        obs.get("refuse_reason")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    });
    let refuse = obs
        .get("refuse_reason")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let step = envelope
        .get("step_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .or_else(|| {
            payload
                .get("step")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32)
        });
    let mut notes = annotations.notes;
    if !sensitivity.is_empty() {
        let hint = format!("desensitized: {}", sensitivity.join("; "));
        notes = Some(match notes {
            Some(n) => format!("{n}; {hint}"),
            None => hint,
        });
    }
    let label_source = if annotations.human_confirmed {
        LabelSource::Human
    } else {
        LabelSource::Pending
    };
    Ok(ReplayCase {
        manifest: ReplayManifest {
            case_id: case_id.to_string(),
            schema_version: REPLAY_SCHEMA_VERSION,
            platform: "windows".into(),
            kind: "replay_case".into(),
            tags: annotations.tags,
            source: Some(ReplaySource {
                task_id: envelope
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                run_id: envelope
                    .get("run_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                step,
                frame_id: obs
                    .get("frame_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            }),
            page_label: page_label.clone(),
            goal,
            notes,
            label_source,
            human_confirmed: annotations.human_confirmed,
        },
        frame: ReplayFrame {
            frame: FrameSize {
                width: fw,
                height: fh,
            },
            ocr_hits,
            title_hint,
            frame_bmp: None,
        },
        expected: ReplayExpected {
            page: page_label.unwrap_or_else(|| "unknown".into()),
            refuse_reason: refuse,
            shadow_action_type: "stop".into(),
            intent_kind,
            intent_label,
            intent_reason,
            date: annotations.date,
        },
    })
}

/// Pick the last observation with non-empty ocr_hits (or a specific step) from `observations.jsonl`.
pub fn promote_from_run_dir(
    run_dir: &Path,
    case_id: &str,
    annotations: PromoteAnnotations,
    step_filter: Option<u32>,
) -> Result<ReplayCase, String> {
    let path = run_dir.join("observations.jsonl");
    let raw = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut chosen: Option<serde_json::Value> = None;
    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("observations.jsonl L{}: {e}", i + 1))?;
        let step = v
            .pointer("/payload/step")
            .and_then(|x| x.as_u64())
            .map(|n| n as u32);
        if let Some(want) = step_filter {
            if step != Some(want) {
                continue;
            }
        }
        let hits = v
            .pointer("/payload/observation/ocr_hits")
            .and_then(|x| x.as_array());
        let nonempty = hits.map(|a| !a.is_empty()).unwrap_or(false);
        if nonempty {
            chosen = Some(v);
            if step_filter.is_some() {
                break;
            }
        }
    }
    let env = chosen.ok_or_else(|| {
        if step_filter.is_some() {
            "no observation with ocr_hits for requested step".to_string()
        } else {
            "no observation with non-empty ocr_hits in run".to_string()
        }
    })?;
    promote_observation(&env, case_id, annotations)
}

pub fn eval_replay_case(case: &ReplayCase) -> ReplayCaseResult {
    let ocr = OcrRegionCache::from_hits(case.frame.ocr_hits.clone());
    let shadow = observe_shadow(ShadowObserveInput {
        run_id: "replay-corpus",
        lease_id: "lease-replay",
        generation: 1,
        frame_id: &case.manifest.case_id,
        frame_w: case.frame.frame.width,
        frame_h: case.frame.frame.height,
        ocr: &ocr,
        title_hint: case.frame.title_hint.clone(),
        min_confidence: DEFAULT_MIN_CONFIDENCE,
        target_slot_time: case.manifest.goal.slot_time.as_deref(),
        timing: ShadowTiming::default(),
        perception: crate::PerceptionFeed::default(),
    });
    let decision = decide_next(&shadow.snapshot, &shadow.candidate_set, &case.manifest.goal);
    let predicted_page = shadow.snapshot.page.as_str().to_string();
    let page_ok = predicted_page == case.expected.page;
    let shadow_refuse_ok = match &case.expected.refuse_reason {
        None => true,
        Some(want) => shadow.refuse_reason.as_deref() == Some(want.as_str()),
    };
    let (pred_kind, pred_label, pred_reason) = intent_parts(&decision.intent);
    let intent_ok = match_intent(
        &case.expected,
        pred_kind,
        pred_label.as_deref(),
        pred_reason.as_deref(),
    );
    let shadow_click = matches!(shadow.action_plan.action_type, ActionType::Click);
    let decide_clicky = matches!(
        decision.intent,
        NextIntent::Navigate { .. } | NextIntent::AtomicComposite { .. }
    );
    let expect_clicky = matches!(
        case.expected.intent_kind.as_deref(),
        Some("navigate") | Some("atomic_composite")
    );
    let false_click = shadow_click || (decide_clicky && !expect_clicky);
    let mut notes = Vec::new();
    if shadow.input_invoked {
        notes.push("input_invoked=true (invariant broken)".into());
    }
    if !case.manifest.human_confirmed
        && matches!(case.manifest.label_source, LabelSource::Pending)
    {
        notes.push("pending_label:scored_anyway_for_skeleton".into());
    }
    ReplayCaseResult {
        case_id: case.manifest.case_id.clone(),
        page_ok,
        shadow_refuse_ok,
        intent_ok,
        false_click,
        predicted_page,
        predicted_intent_kind: pred_kind.to_string(),
        predicted_intent_label: pred_label,
        predicted_intent_reason: pred_reason,
        shadow_refuse: shadow.refuse_reason,
        notes,
    }
}

pub fn eval_replay_corpus(cases: &[ReplayCase]) -> ReplayCorpusReport {
    let mut results = Vec::with_capacity(cases.len());
    let mut page_correct = 0usize;
    let mut intent_correct = 0usize;
    let mut false_click_count = 0usize;
    for c in cases {
        let r = eval_replay_case(c);
        if r.page_ok {
            page_correct += 1;
        }
        if r.intent_ok {
            intent_correct += 1;
        }
        if r.false_click {
            false_click_count += 1;
        }
        results.push(r);
    }
    let scored = cases.len();
    let denom = scored.max(1) as f32;
    ReplayCorpusReport {
        case_count: cases.len(),
        scored_count: scored,
        page_correct,
        intent_correct,
        false_click_count,
        page_recall: page_correct as f32 / denom,
        intent_match: intent_correct as f32 / denom,
        cases: results,
    }
}

fn intent_parts(intent: &NextIntent) -> (&'static str, Option<String>, Option<String>) {
    match intent {
        NextIntent::Navigate { label, .. } => ("navigate", Some(label.clone()), None),
        NextIntent::AtomicComposite { label, .. } => {
            ("atomic_composite", Some(label.clone()), None)
        }
        NextIntent::Wait { reason, .. } => ("wait", None, Some(reason.clone())),
        NextIntent::Stop { reason } => ("stop", None, Some(reason.clone())),
        NextIntent::NeedHuman { reason } => ("need_human", None, Some(reason.clone())),
        NextIntent::Rebind { reason } => ("rebind", None, Some(reason.clone())),
    }
}

fn match_intent(
    expected: &ReplayExpected,
    kind: &str,
    label: Option<&str>,
    reason: Option<&str>,
) -> bool {
    let Some(want_kind) = expected.intent_kind.as_deref() else {
        return true;
    };
    if want_kind != kind {
        return false;
    }
    if let Some(want_label) = expected.intent_label.as_deref() {
        if label != Some(want_label) {
            return false;
        }
    }
    if let Some(want_reason) = expected.intent_reason.as_deref() {
        if reason != Some(want_reason) {
            return false;
        }
    }
    true
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, raw + "\n").map_err(|e| format!("write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn desensitize_rewrites_digit_heavy() {
        let hits = vec![OcrHit {
            text: "学号 229202201234567".into(),
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            score: 0.9,
        }];
        let (out, findings) = desensitize_ocr_hits(&hits);
        assert!(!findings.is_empty());
        assert!(
            out[0].text.contains("REDACTED"),
            "got {}",
            out[0].text
        );
    }

    #[test]
    fn promote_observation_builds_case() {
        let env = json!({
            "schema_version": 1,
            "run_id": "run-1",
            "task_id": "task-1",
            "step_id": "2",
            "payload": {
                "step": 2,
                "goal": { "sport": "羽毛球", "venue": null, "date": "2026-09-28", "slot_time": null },
                "observation": {
                    "frame_id": "f1",
                    "frame_width": 420,
                    "frame_height": 760,
                    "bound_title": "厦大体育",
                    "page": "select_slot",
                    "intent_kind": "stop",
                    "click_label": "",
                    "refuse_reason": "SLOT_GRID_NOT_LOCALIZED",
                    "ocr_hits": [
                        { "text": "选择场次", "x": 24, "y": 48, "w": 88, "h": 22, "score": 0.94 },
                        { "text": "19:00", "x": 28, "y": 220, "w": 44, "h": 18, "score": 0.92 }
                    ]
                }
            }
        });
        let case = promote_observation(
            &env,
            "slot_unlocalized_001",
            PromoteAnnotations {
                page_label: Some("select_slot".into()),
                date: Some("2026-09-28".into()),
                intent_kind: Some("stop".into()),
                intent_reason: Some("SLOT_GRID_NOT_LOCALIZED".into()),
                tags: vec!["slot_grid_unlocalized".into()],
                human_confirmed: true,
                ..Default::default()
            },
        )
        .expect("promote");
        assert_eq!(case.manifest.case_id, "slot_unlocalized_001");
        assert_eq!(case.frame.ocr_hits.len(), 2);
        assert_eq!(case.expected.page, "select_slot");
        assert_eq!(
            case.expected.intent_reason.as_deref(),
            Some("SLOT_GRID_NOT_LOCALIZED")
        );
        assert!(case.manifest.human_confirmed);
    }

    #[test]
    fn eval_seed_style_mp_panel_matches_intent() {
        let case = ReplayCase {
            manifest: ReplayManifest {
                case_id: "mp_panel_open_xmu".into(),
                schema_version: REPLAY_SCHEMA_VERSION,
                platform: "windows".into(),
                kind: "replay_case".into(),
                tags: vec![],
                source: None,
                page_label: Some("wechat_mp_panel".into()),
                goal: TaskGoal::default(),
                notes: None,
                label_source: LabelSource::AnnotatedFixture,
                human_confirmed: true,
            },
            frame: ReplayFrame {
                frame: FrameSize {
                    width: 930,
                    height: 658,
                },
                ocr_hits: vec![
                    hit("发现", 80, 20, 40, 18),
                    hit("朋友圈", 80, 80, 60, 18),
                    hit("小程序", 80, 220, 60, 18),
                    hit("最近使用", 280, 100, 80, 18),
                    hit("厦大体育", 300, 160, 80, 18),
                    hit("我的常用", 280, 280, 80, 18),
                ],
                title_hint: String::new(),
                frame_bmp: None,
            },
            expected: ReplayExpected {
                page: "wechat_mp_panel".into(),
                refuse_reason: Some("OBSERVE_ONLY_PENDING_PAGE_MAP".into()),
                shadow_action_type: "stop".into(),
                intent_kind: Some("navigate".into()),
                intent_label: Some("厦大体育".into()),
                intent_reason: None,
                date: None,
            },
        };
        let r = eval_replay_case(&case);
        assert!(r.page_ok, "page {}", r.predicted_page);
        assert!(r.shadow_refuse_ok, "refuse {:?}", r.shadow_refuse);
        assert!(r.intent_ok, "intent {} {:?}", r.predicted_intent_kind, r.predicted_intent_label);
        assert!(!r.false_click);
    }

    fn hit(text: &str, x: i32, y: i32, w: i32, h: i32) -> OcrHit {
        OcrHit {
            text: text.into(),
            x,
            y,
            w,
            h,
            score: 0.9,
        }
    }
}
