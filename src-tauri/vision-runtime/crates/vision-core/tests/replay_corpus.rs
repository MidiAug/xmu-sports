//! P6 replay corpus: load seeded case + promote round-trip.

use std::path::PathBuf;

use serde_json::json;
use vision_core::{
    eval_replay_corpus, load_replay_corpus, promote_observation, write_replay_case,
    PromoteAnnotations, REPLAY_MATRIX_TAGS,
};

fn replay_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/replay")
}

#[test]
fn corpus_seed_mp_panel_scores() {
    let cases = load_replay_corpus(&replay_root()).expect("load corpus");
    assert!(
        cases.iter().any(|c| c.manifest.case_id == "mp_panel_open_xmu"),
        "seed case missing"
    );
    let report = eval_replay_corpus(&cases);
    assert_eq!(report.case_count, cases.len());
    assert!(report.page_recall > 0.0, "page_recall={}", report.page_recall);
    let seed = report
        .cases
        .iter()
        .find(|c| c.case_id == "mp_panel_open_xmu")
        .expect("seed result");
    assert!(seed.page_ok);
    assert!(seed.intent_ok);
    assert!(!seed.false_click);
}

#[test]
fn matrix_tags_document_target_cells() {
    assert!(REPLAY_MATRIX_TAGS.contains(&"dpi_125"));
    assert!(REPLAY_MATRIX_TAGS.contains(&"split_boxes"));
    assert!(REPLAY_MATRIX_TAGS.contains(&"cross_month"));
}

#[test]
fn promote_then_write_roundtrip() {
    let env = json!({
        "run_id": "run-rt",
        "task_id": "task-rt",
        "step_id": "1",
        "payload": {
            "step": 1,
            "goal": {
                "sport": null,
                "venue": null,
                "date": null,
                "slot_time": null
            },
            "observation": {
                "frame_id": "f-rt",
                "frame_width": 930,
                "frame_height": 658,
                "bound_title": "",
                "page": "wechat_mp_panel",
                "intent_kind": "navigate",
                "click_label": "厦大体育",
                "refuse_reason": "OBSERVE_ONLY_PENDING_PAGE_MAP",
                "ocr_hits": [
                    { "text": "小程序", "x": 80, "y": 220, "w": 60, "h": 18, "score": 0.95 },
                    { "text": "厦大体育", "x": 300, "y": 160, "w": 80, "h": 18, "score": 0.95 }
                ]
            }
        }
    });
    let case = promote_observation(
        &env,
        "promote_roundtrip_tmp",
        PromoteAnnotations {
            page_label: Some("wechat_mp_panel".into()),
            intent_kind: Some("navigate".into()),
            intent_label: Some("厦大体育".into()),
            human_confirmed: true,
            ..Default::default()
        },
    )
    .expect("promote");
    let dir = std::env::temp_dir().join(format!(
        "xmu-sports-replay-rt-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    write_replay_case(&dir, &case).expect("write");
    let loaded = vision_core::load_replay_case(&dir).expect("reload");
    assert_eq!(loaded.manifest.case_id, "promote_roundtrip_tmp");
    assert_eq!(loaded.frame.ocr_hits.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}
