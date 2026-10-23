//! Offline page-map eval: real OCR samples preferred; annotated flagged non-E2E.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use vision_core::{
    build_eval_report, observe_shadow, LabelSource, OcrHit, OcrRegionCache, OcrSource,
    PageMapSample, SampleSplit, ShadowObserveInput, ShadowTiming, CAPTURE_CHECKLIST,
    DEFAULT_MIN_CONFIDENCE,
};

use crate::common::{fixture_path, page_map_root, shadow_out_dir};

pub fn cmd_replay(fixture: &str) -> Result<(), String> {
    let path = fixture_path(fixture);
    let raw = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let sample: PageMapSample = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let ocr = OcrRegionCache::from_hits(sample.ocr_hits.clone());
    let result = observe_shadow(ShadowObserveInput {
        run_id: "shadow-replay",
        lease_id: "lease-replay",
        generation: 1,
        frame_id: &sample.id,
        frame_w: sample.frame.width,
        frame_h: sample.frame.height,
        ocr: &ocr,
        title_hint: sample.page_label.clone().unwrap_or_default(),
        min_confidence: DEFAULT_MIN_CONFIDENCE,
        target_slot_time: None,
        timing: ShadowTiming::default(),
        perception: vision_core::PerceptionFeed::default(),
    });
    assert!(!result.input_invoked);

    let out = shadow_out_dir();
    let stamp = crate::common::chrono_stamp();
    let meta = out.join(format!("replay_{}_{stamp}.json", fixture.replace('.', "_")));
    let report = json!({
        "mode": "replay",
        "fixture": fixture,
        "fixture_id": sample.id,
        "ocr_source": sample.ocr_source,
        "page": result.snapshot.page.as_str(),
        "confidence": result.snapshot.confidence,
        "evidence": result.snapshot.evidence,
        "overlay": format!("{:?}", result.snapshot.overlay),
        "refuse_reason": result.refuse_reason,
        "action_plan": result.action_plan,
        "slots": result.slots,
        "candidate_boxes": result.candidate_boxes,
        "timing": result.timing,
        "input_invoked": false,
        "input_adapter_called": false,
        "note": "Single-fixture replay. Prefer `eval --dir` for confusion matrix. Annotated OCR != E2E accuracy.",
    });
    fs::write(&meta, serde_json::to_string_pretty(&report).unwrap()).map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    println!("wrote {}", meta.display());
    Ok(())
}

pub fn cmd_eval(args: &[String]) -> Result<(), String> {
    let dir = crate::common::parse_flag(args, "--dir")
        .map(PathBuf::from)
        .ok_or_else(|| "--dir <capture_or_import_dir> required".to_string())?;
    let split_filter = crate::common::parse_flag(args, "--split"); // dev|holdout|all
    let require_confirmed = !crate::common::has_flag(args, "--allow-unconfirmed");
    let real_ocr_only = crate::common::has_flag(args, "--real-ocr-only");
    let min_iou: f32 = crate::common::parse_flag(args, "--min-iou")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.3);

    let mut samples = load_samples_from_dir(&dir)?;
    if samples.is_empty() {
        return Err(format!("no page-map samples under {}", dir.display()));
    }

    if let Some(sf) = split_filter.as_deref() {
        samples.retain(|s| match sf {
            "dev" => matches!(s.split, SampleSplit::Dev),
            "holdout" => matches!(s.split, SampleSplit::Holdout),
            "all" => true,
            _ => true,
        });
    }
    if real_ocr_only {
        samples.retain(|s| matches!(s.ocr_source, OcrSource::WindowsMedia | OcrSource::PackOnnx));
    }

    let report = build_eval_report(&samples, require_confirmed, min_iou, CAPTURE_CHECKLIST);

    let out = if dir.join("samples").is_dir() {
        dir.join("eval_report.json")
    } else {
        page_map_root().join(format!("eval_{}.json", crate::common::chrono_stamp()))
    };
    fs::write(&out, serde_json::to_string_pretty(&report).unwrap()).map_err(|e| e.to_string())?;

    println!(
        "samples={} scored={} correct={} wrong={} unknown={} skipped={}",
        report.sample_count,
        report.scored_count,
        report.correct,
        report.wrong,
        report.unknown,
        report.skipped_unconfirmed
    );
    println!(
        "real_ocr={} annotated={} e2e_ready={} input_invoked_any={}",
        report.real_ocr_count,
        report.annotated_only_count,
        report.e2e_ready,
        report.input_invoked_any
    );
    if !report.confusion.is_empty() {
        println!("confusion:");
        for c in &report.confusion {
            println!("  {} -> {}  (n={})", c.expected, c.predicted, c.count);
        }
    }
    if !report.wrong_examples.is_empty() {
        println!("wrong/unknown examples:");
        for e in &report.wrong_examples {
            println!(
                "  {} expect={} got={} refuse={:?}",
                e.id, e.expected_page, e.predicted_page, e.refuse_reason
            );
        }
    }
    for n in &report.gate_notes {
        println!("gate: {n}");
    }
    println!("wrote {}", out.display());

    if report.input_invoked_any {
        return Err("input_invoked must stay false".into());
    }
    Ok(())
}

fn load_samples_from_dir(dir: &Path) -> Result<Vec<PageMapSample>, String> {
    let mut out = Vec::new();
    let samples_dir = dir.join("samples");
    if samples_dir.is_dir() {
        for entry in fs::read_dir(&samples_dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            let sample_json = if p.is_dir() {
                p.join("sample.json")
            } else if p.extension().and_then(|e| e.to_str()) == Some("json") {
                p
            } else {
                continue;
            };
            if sample_json.is_file() {
                out.push(load_one(&sample_json)?);
            }
        }
    } else {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("json") {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name == "session.json" || name == "manifest.json" || name.starts_with("eval_") {
                    continue;
                }
                // Prefer PageMapSample; skip if not matching
                if let Ok(s) = load_one(&p) {
                    out.push(s);
                }
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn load_one(path: &Path) -> Result<PageMapSample, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // Support legacy AnnotatedFixture shape used by shadow fixtures.
    if let Ok(sample) = serde_json::from_str::<PageMapSample>(&raw) {
        return Ok(normalize_legacy(sample));
    }
    #[derive(serde::Deserialize)]
    struct Legacy {
        id: String,
        #[serde(default)]
        page_label: Option<String>,
        frame: vision_core::FrameSize,
        ocr_hits: Vec<OcrHit>,
        #[serde(default)]
        expected: Option<vision_core::ExpectedOutcome>,
        #[serde(default)]
        kind: Option<String>,
    }
    let leg: Legacy = serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(PageMapSample {
        id: leg.id,
        platform: "windows".into(),
        kind: leg.kind.unwrap_or_else(|| "page_map_annotated".into()),
        split: SampleSplit::Dev,
        ocr_source: OcrSource::Annotated,
        page_label_suggested: None,
        page_label: leg.page_label.clone(),
        checklist_hint: None,
        label_source: LabelSource::AnnotatedFixture,
        human_confirmed: true,
        source: None,
        nav_note: None,
        frame: leg.frame,
        ocr_hits: leg.ocr_hits,
        key_boxes_suggested: Vec::new(),
        key_boxes: Vec::new(),
        expected: leg.expected.or_else(|| {
            leg.page_label.map(|p| vision_core::ExpectedOutcome {
                page: p,
                min_confidence: None,
                refuse_reason: None,
                action_type: "stop".into(),
                venue_name: None,
            })
        }),
        venue_name: None,
        ocr_backend: None,
        ocr_language: None,
        window: None,
        capture_meta: None,
    })
}

fn normalize_legacy(mut s: PageMapSample) -> PageMapSample {
    if matches!(s.ocr_source, OcrSource::Unknown) {
        if s.kind.contains("annotated") || s.kind.contains("shadow") {
            s.ocr_source = OcrSource::Annotated;
        }
    }
    if s.page_label.is_none() {
        if let Some(exp) = &s.expected {
            s.page_label = Some(exp.page.clone());
        }
    }
    if s.expected.is_some() && matches!(s.ocr_source, OcrSource::Annotated) {
        if !s.human_confirmed {
            s.human_confirmed = true;
        }
        if matches!(s.label_source, LabelSource::Pending) {
            s.label_source = LabelSource::AnnotatedFixture;
        }
    }
    s
}
