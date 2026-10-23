//! Shadow annotated replay fixtures — explain page / candidates / refuse, never input.

use serde::Deserialize;
use vision_contracts::ActionType;
use vision_core::{
    observe_shadow, OcrHit, OcrRegionCache, Overlay, ShadowObserveInput, ShadowTiming,
    DEFAULT_MIN_CONFIDENCE,
};

#[derive(Deserialize)]
struct Fixture {
    ocr_hits: Vec<Hit>,
    frame: FrameSize,
    expected: Expected,
}

#[derive(Deserialize)]
struct Hit {
    text: String,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    score: f32,
}

#[derive(Deserialize)]
struct FrameSize {
    width: i32,
    height: i32,
}

#[derive(Deserialize)]
struct Expected {
    page: String,
    #[serde(default)]
    refuse_reason: Option<String>,
    action_type: String,
    #[serde(default)]
    overlay: Option<String>,
    #[serde(default)]
    has_mint_slots: Option<bool>,
}

fn load(name: &str) -> Fixture {
    let path = format!("testdata/shadow/{name}");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&raw).expect("parse")
}

fn run(name: &str) {
    let f = load(name);
    let hits: Vec<OcrHit> = f
        .ocr_hits
        .into_iter()
        .map(|h| OcrHit {
            text: h.text,
            x: h.x,
            y: h.y,
            w: h.w,
            h: h.h,
            score: h.score,
        })
        .collect();
    let ocr = OcrRegionCache::from_hits(hits);
    let r = observe_shadow(ShadowObserveInput {
        run_id: "shadow-test",
        lease_id: "lease-test",
        generation: 1,
        frame_id: "frame-test",
        frame_w: f.frame.width,
        frame_h: f.frame.height,
        ocr: &ocr,
        title_hint: String::new(),
        min_confidence: DEFAULT_MIN_CONFIDENCE,
        target_slot_time: None,
        timing: ShadowTiming::default(),
        perception: vision_core::PerceptionFeed::default(),
    });
    assert!(!r.input_invoked, "{name}: must not invoke input");
    assert_eq!(r.snapshot.page.as_str(), f.expected.page, "{name}: page");
    assert_eq!(r.refuse_reason, f.expected.refuse_reason, "{name}: refuse");
    let at = match r.action_plan.action_type {
        ActionType::Click => "click",
        ActionType::Stop => "stop",
        ActionType::Wait => "wait",
        other => panic!("unexpected action {other:?}"),
    };
    assert_eq!(at, f.expected.action_type, "{name}: action_type");
    if let Some(ov) = f.expected.overlay {
        let got = match r.snapshot.overlay {
            Overlay::Notice => "notice",
            Overlay::Modal => "modal",
            Overlay::None => "none",
            other => panic!("overlay {other:?}"),
        };
        assert_eq!(got, ov, "{name}: overlay");
    }
    if let Some(mint) = f.expected.has_mint_slots {
        assert_eq!(r.snapshot.has_mint_slots, mint, "{name}: mint");
    }
    assert!(
        !r.snapshot.evidence.is_empty() || r.refuse_reason.is_some(),
        "{name}: need evidence"
    );
}

#[test]
fn replay_select_slot_normal() {
    run("select_slot_normal.json");
}

#[test]
fn replay_popup_notice() {
    run("popup_notice.json");
}

#[test]
fn replay_select_slot_empty() {
    run("select_slot_empty.json");
}

#[test]
fn replay_unknown_wechat_chat() {
    run("unknown_wechat_chat.json");
}

#[test]
fn replay_wechat_mp_panel() {
    run("wechat_mp_panel.json");
}

#[test]
fn decide_next_on_mp_panel_opens_xmu() {
    use vision_core::{decide_next, NextIntent, TaskGoal};
    let f = load("wechat_mp_panel.json");
    let hits: Vec<OcrHit> = f
        .ocr_hits
        .into_iter()
        .map(|h| OcrHit {
            text: h.text,
            x: h.x,
            y: h.y,
            w: h.w,
            h: h.h,
            score: h.score,
        })
        .collect();
    let ocr = OcrRegionCache::from_hits(hits);
    let r = observe_shadow(ShadowObserveInput {
        run_id: "shadow-test",
        lease_id: "lease-test",
        generation: 1,
        frame_id: "frame-test",
        frame_w: f.frame.width,
        frame_h: f.frame.height,
        ocr: &ocr,
        title_hint: String::new(),
        min_confidence: DEFAULT_MIN_CONFIDENCE,
        target_slot_time: None,
        timing: ShadowTiming::default(),
        perception: vision_core::PerceptionFeed::default(),
    });
    assert_eq!(r.action_plan.action_type, ActionType::Stop);
    assert!(!r.input_invoked);
    let d = decide_next(&r.snapshot, &r.candidate_set, &TaskGoal::default());
    assert!(
        matches!(d.intent, NextIntent::Navigate { ref label, .. } if label == "厦大体育"),
        "got {:?}",
        d.intent
    );
    assert!(d.next_line.contains("厦大体育"));
    assert!(!d.evidence_boxes.is_empty());
}
