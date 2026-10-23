//! P6: promote failed run → replay case; offline corpus eval.

use std::fs;
use std::path::PathBuf;

use vision_core::{
    eval_replay_corpus, load_replay_corpus, promote_from_run_dir, write_replay_case,
    PromoteAnnotations, REPLAY_MATRIX_TAGS,
};

use crate::common::{data_root, parse_flag};

pub fn cmd_promote(args: &[String]) -> Result<(), String> {
    let run_dir = parse_flag(args, "--run-dir")
        .map(PathBuf::from)
        .ok_or_else(|| "--run-dir <runs/task/run> required".to_string())?;
    let case_id =
        parse_flag(args, "--case-id").ok_or_else(|| "--case-id <id> required".to_string())?;
    let step = parse_flag(args, "--step").and_then(|s| s.parse().ok());
    let to_repo = has_flag(args, "--to-repo");
    let out = parse_flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if to_repo {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../vision-core/testdata/replay")
                    .join(&case_id)
            } else {
                data_root().join("diag").join("replay").join(&case_id)
            }
        });

    let mut tags = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--tag" && i + 1 < args.len() {
            tags.push(args[i + 1].clone());
            i += 2;
            continue;
        }
        i += 1;
    }

    let annotations = PromoteAnnotations {
        page_label: parse_flag(args, "--page"),
        date: parse_flag(args, "--date"),
        intent_kind: parse_flag(args, "--intent-kind"),
        intent_label: parse_flag(args, "--intent-label"),
        intent_reason: parse_flag(args, "--intent-reason"),
        tags,
        notes: parse_flag(args, "--notes"),
        human_confirmed: has_flag(args, "--confirm"),
    };

    let case = promote_from_run_dir(&run_dir, &case_id, annotations, step)?;
    write_replay_case(&out, &case)?;
    println!(
        "promoted case_id={} → {}\n  page={} intent={:?}/{:?} tags={:?} confirmed={}",
        case.manifest.case_id,
        out.display(),
        case.expected.page,
        case.expected.intent_kind,
        case.expected.intent_label,
        case.manifest.tags,
        case.manifest.human_confirmed
    );
    if !case.manifest.human_confirmed {
        println!("note: label_source=pending; pass --confirm after reviewing evidence");
    }
    Ok(())
}

pub fn cmd_corpus(args: &[String]) -> Result<(), String> {
    let dir = parse_flag(args, "--dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vision-core/testdata/replay")
        });
    let cases = load_replay_corpus(&dir)?;
    if cases.is_empty() {
        return Err(format!("no replay cases under {}", dir.display()));
    }
    let report = eval_replay_corpus(&cases);

    let report_path = data_root()
        .join("diag")
        .join("replay")
        .join("corpus_report.json");
    if let Some(parent) = report_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(
        &report_path,
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    println!(
        "cases={} page_recall={:.2} intent_match={:.2} false_click={}",
        report.case_count, report.page_recall, report.intent_match, report.false_click_count
    );
    for (case, r) in cases.iter().zip(report.cases.iter()) {
        let mark = if r.page_ok && r.intent_ok && !r.false_click {
            "OK"
        } else {
            "MISS"
        };
        println!(
            "  [{mark}] {} page {}→{} intent {:?}→{} false_click={}",
            r.case_id,
            case.expected.page,
            r.predicted_page,
            case.expected.intent_kind,
            r.predicted_intent_kind,
            r.false_click
        );
    }
    println!("matrix tags (target): {}", REPLAY_MATRIX_TAGS.join(", "));
    println!("wrote {}", report_path.display());
    Ok(())
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}
