//! Page-map annotated fixtures (login gate + wx authorize). Observe-only.

use serde::Deserialize;
use vision_contracts::ActionType;
use vision_core::{
    observe_shadow, OcrHit, OcrRegionCache, ShadowObserveInput, ShadowTiming,
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
}

fn load(name: &str) -> Fixture {
    let path = format!("testdata/page_map/{name}");
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
        run_id: "page-map",
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
    assert!(!r.input_invoked);
    assert_eq!(r.snapshot.page.as_str(), f.expected.page);
    assert_eq!(r.refuse_reason, f.expected.refuse_reason);
    assert!(matches!(r.action_plan.action_type, ActionType::Stop));
    assert_eq!(f.expected.action_type, "stop");
    assert!(
        r.candidate_boxes
            .iter()
            .any(|b| b.label.starts_with("key:")),
        "{name}: expected key:* candidate boxes"
    );
}

#[test]
fn page_map_login_gate() {
    run("xmu_login_gate.json");
}

#[test]
fn page_map_wx_authorize() {
    run("xmu_wx_authorize.json");
}
