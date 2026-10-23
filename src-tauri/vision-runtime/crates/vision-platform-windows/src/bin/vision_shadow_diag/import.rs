//! Import OCR+labels only (no BMPs).
//! "No image" != redacted — OCR text is scanned; flagged imports need --ack-ocr-pii-reviewed.

use std::fs;
use std::path::PathBuf;

use serde_json::json;
use vision_core::{
    effective_label_source, scan_ocr_sensitivity, LabelSource, OcrSource, PageMapSample,
    SampleSplit,
};

use crate::common::page_map_root;

pub fn cmd_import(args: &[String]) -> Result<(), String> {
    let from = crate::common::parse_flag(args, "--from")
        .map(PathBuf::from)
        .ok_or_else(|| "--from <session_dir> required".to_string())?;
    let to = crate::common::parse_flag(args, "--to")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            page_map_root().join(format!("import_{}", crate::common::chrono_stamp()))
        });
    let holdout_only = crate::common::has_flag(args, "--holdout-only");
    let require_confirmed = !crate::common::has_flag(args, "--allow-unconfirmed");
    let ack_pii = crate::common::has_flag(args, "--ack-ocr-pii-reviewed");

    fs::create_dir_all(&to).map_err(|e| e.to_string())?;
    let samples_src = from.join("samples");
    if !samples_src.is_dir() {
        return Err(format!("missing samples/ under {}", from.display()));
    }

    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    let mut pii_blocked = Vec::new();
    let mut pii_acked = Vec::new();

    for entry in fs::read_dir(&samples_src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        let sample_json = if p.is_dir() {
            p.join("sample.json")
        } else {
            continue;
        };
        if !sample_json.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&sample_json).map_err(|e| e.to_string())?;
        let mut sample: PageMapSample =
            serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", sample_json.display()))?;

        if holdout_only && !matches!(sample.split, SampleSplit::Holdout) {
            skipped.push(format!("{}:not_holdout", sample.id));
            continue;
        }
        if require_confirmed {
            let src = effective_label_source(&sample);
            if !sample.human_confirmed || matches!(src, LabelSource::Pending) {
                skipped.push(format!("{}:unconfirmed_or_pending_label", sample.id));
                continue;
            }
        }

        let findings = scan_ocr_sensitivity(&sample.ocr_hits);
        if !findings.is_empty() {
            if !ack_pii {
                pii_blocked.push(json!({
                    "id": sample.id,
                    "findings": findings,
                }));
                continue;
            }
            pii_acked.push(json!({
                "id": sample.id,
                "findings": findings,
            }));
        }

        if let Some(meta) = sample.capture_meta.as_mut() {
            if let Some(obj) = meta.as_object_mut() {
                obj.remove("frame_bmp");
                obj.remove("evidence_bmp");
                obj.insert("pixels_imported".into(), json!(false));
                obj.insert(
                    "ocr_pii_scan".into(),
                    json!({
                        "findings": findings,
                        "ack_ocr_pii_reviewed": ack_pii,
                    }),
                );
                obj.insert(
                    "redaction_note".into(),
                    json!("No BMP copied. OCR/metadata may still hold names, chat, student ids — human must scrub before git add."),
                );
            }
        }
        sample.kind = "page_map_imported".into();
        if matches!(sample.ocr_source, OcrSource::Unknown) {
            sample.ocr_source = OcrSource::WindowsMedia;
        }

        let out_name = format!("{}.json", sample.id);
        let out_path = to.join(&out_name);
        fs::write(&out_path, serde_json::to_string_pretty(&sample).unwrap())
            .map_err(|e| e.to_string())?;
        imported.push(out_name);
    }

    let report = json!({
        "from": from.display().to_string(),
        "to": to.display().to_string(),
        "imported": imported,
        "skipped": skipped,
        "pii_blocked": pii_blocked,
        "pii_acked": pii_acked,
        "pixels_copied": false,
        "note": "No image != redacted. Re-run with --ack-ocr-pii-reviewed only after scrubbing sensitive OCR lines. Keep holdout unused for rule tuning.",
    });
    fs::write(
        to.join("_import_report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .map_err(|e| e.to_string())?;

    println!(
        "imported {} -> {} (skipped {} pii_blocked {})",
        imported.len(),
        to.display(),
        skipped.len(),
        pii_blocked.len()
    );
    println!("{}", serde_json::to_string_pretty(&report).unwrap());

    if !pii_blocked.is_empty() {
        return Err(format!(
            "OCR sensitivity findings on {} sample(s). Scrub texts or pass --ack-ocr-pii-reviewed after review.",
            pii_blocked.len()
        ));
    }
    Ok(())
}
