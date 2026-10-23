//! Interactive confirm: human types page label while looking at frame.bmp.
//! Sets label_source=human so bulk human_confirmed flips without confirm cannot score.

use std::fs;
use std::path::{Path, PathBuf};

use vision_core::{
    effective_label_source, ExpectedOutcome, KeyBox, LabelSource, PageMapSample,
    DEFAULT_MIN_CONFIDENCE,
};

use crate::common::prompt_line;

fn refuse_for_label(label: &str) -> String {
    match label {
        "select_slot" => "SLOT_GRID_NOT_LOCALIZED".into(),
        "select_slot_empty" => "NO_AVAILABLE_SLOTS".into(),
        "overlay_notice" | "loading" => "OVERLAY_BLOCKS_ACTION".into(),
        "unknown" | "wechat_main" => "UNKNOWN_PAGE".into(),
        _ => "OBSERVE_ONLY_PENDING_PAGE_MAP".into(),
    }
}

pub fn cmd_confirm(args: &[String]) -> Result<(), String> {
    let dir = crate::common::parse_flag(args, "--dir")
        .map(PathBuf::from)
        .ok_or_else(|| "--dir <session_dir> required".to_string())?;
    let samples_dir = dir.join("samples");
    if !samples_dir.is_dir() {
        return Err(format!("missing samples/ under {}", dir.display()));
    }

    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(&samples_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path().join("sample.json");
        if p.is_file() {
            paths.push(p);
        }
    }
    paths.sort();
    if paths.is_empty() {
        return Err("no sample.json found".into());
    }

    println!("Confirm labels while looking at each frame.bmp.");
    println!("Type the page id yourself (Enter alone = skip).");
    println!("Suggested may be wrong — do not blindly accept.");
    println!();

    let mut confirmed = 0usize;
    let mut skipped = 0usize;

    for path in paths {
        let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let mut sample: PageMapSample =
            serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))?;

        if sample.human_confirmed
            && matches!(
                effective_label_source(&sample),
                LabelSource::Human | LabelSource::AnnotatedFixture
            )
        {
            println!("skip already confirmed: {}", sample.id);
            continue;
        }

        let bmp = path
            .parent()
            .map(|d| d.join("frame.bmp"))
            .unwrap_or_default();
        println!("--- {}", sample.id);
        println!("  frame: {}", bmp.display());
        println!(
            "  checklist_hint={:?} suggested={:?}",
            sample.checklist_hint, sample.page_label_suggested
        );
        if !sample.key_boxes_suggested.is_empty() {
            println!(
                "  suggested boxes: {:?}",
                sample
                    .key_boxes_suggested
                    .iter()
                    .map(|b| b.label.as_str())
                    .collect::<Vec<_>>()
            );
        }

        let label = prompt_line("  page_label (required, type explicitly) > ")?;
        if label.is_empty() {
            println!("  skipped");
            skipped += 1;
            continue;
        }
        if sample.page_label_suggested.as_deref() == Some(label.as_str()) {
            let ack = prompt_line(
                "  label equals suggested — confirm you looked at the BMP (yes/no) > ",
            )?;
            if ack.to_lowercase() != "yes" && ack.to_lowercase() != "y" {
                println!("  not confirmed");
                skipped += 1;
                continue;
            }
        }

        let mut venue_name: Option<String> = None;
        if matches!(
            label.as_str(),
            "venue_detail" | "venue_list" | "select_slot" | "select_slot_empty"
        ) {
            let v = prompt_line("  venue_name (or UNKNOWN) > ")?;
            venue_name = Some(if v.is_empty() { "UNKNOWN".into() } else { v });
        }

        let accept_boxes =
            prompt_line("  accept suggested key_boxes as verified? (yes / no / edit-later) > ")?;
        let key_boxes = match accept_boxes.to_lowercase().as_str() {
            "yes" | "y" => sample
                .key_boxes_suggested
                .iter()
                .map(|b| KeyBox {
                    label: b.label.clone(),
                    x0: b.x0,
                    y0: b.y0,
                    x1: b.x1,
                    y1: b.y1,
                    role: b.role.clone(),
                    verified: Some(true),
                })
                .collect(),
            _ => Vec::new(),
        };

        let min_conf = sample
            .capture_meta
            .as_ref()
            .and_then(|m| m.get("min_confidence_at_capture"))
            .and_then(|v| v.as_f64())
            .map(|f| f as f32)
            .unwrap_or(DEFAULT_MIN_CONFIDENCE);

        sample.page_label = Some(label.clone());
        sample.label_source = LabelSource::Human;
        sample.human_confirmed = true;
        sample.venue_name = venue_name.clone();
        sample.key_boxes = key_boxes;
        sample.expected = Some(ExpectedOutcome {
            page: label.clone(),
            min_confidence: Some(min_conf),
            refuse_reason: Some(refuse_for_label(&label)),
            action_type: "stop".into(),
            venue_name,
        });

        fs::write(&path, serde_json::to_string_pretty(&sample).unwrap())
            .map_err(|e| e.to_string())?;
        println!("  confirmed as {label} (label_source=human)");
        confirmed += 1;
    }

    println!("confirmed={confirmed} skipped={skipped}");
    println!(
        "next: eval --dir {} --real-ocr-only  and  --split holdout --real-ocr-only",
        dir.display()
    );
    Ok(())
}

pub fn cmd_confirm_hint(dir: &Path) -> Result<(), String> {
    println!(
        "Use interactive confirm (do not bulk-edit human_confirmed):\n  vision-shadow-diag confirm --dir {}\nThen eval --real-ocr-only for dev and holdout.",
        dir.display()
    );
    Ok(())
}
