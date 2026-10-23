//! Write a local fake recognition pack for install-flow acceptance.
//! Usage: cargo run -p vision-pack --example write_fake_pack -- <out_dir> [version] [triple]

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use vision_pack::{compute_placeholder_signature, ManifestFile, PackManifest};

fn sha256_file(path: &std::path::Path) -> String {
    let bytes = fs::read(path).expect("read");
    hex::encode(Sha256::digest(&bytes))
}

fn write_shell_family(shell: &Path, family: &str) {
    let dir = shell.join(family);
    fs::create_dir_all(&dir).unwrap();
    let variants = [
        "inactive_light",
        "active_light",
        "inactive_dark",
        "active_dark",
    ];
    for name in variants {
        fs::write(dir.join(format!("{name}.gray")), [8u8, 8, 8, 8]).unwrap();
    }
    fs::write(
        dir.join("manifest.json"),
        format!(
            r#"{{"element_id":"wechat.{family}","variants":["inactive_light","active_light","inactive_dark","active_dark"]}}"#
        ),
    )
    .unwrap();
}

fn main() {
    let mut args = env::args().skip(1);
    let out = PathBuf::from(args.next().expect("out_dir"));
    let version = args.next().unwrap_or_else(|| "0.0.1-dev".into());
    let triple = args
        .next()
        .unwrap_or_else(|| "x86_64-pc-windows-msvc".into());

    let shell = out.join("templates").join("wechat_shell");
    fs::create_dir_all(&shell).unwrap();
    write_shell_family(&shell, "discover_compass");
    write_shell_family(&shell, "miniprogram_entry");
    fs::create_dir_all(out.join("rules")).unwrap();
    fs::write(
        out.join("rules/colors.json"),
        r#"{"pale_h":[68,98],"pale_s":[10,95],"pale_v":[200,255],"strong_h":[35,100],"strong_s":[40,255],"strong_v":[140,255],"min_area":40,"label_iou":0.1,"dedup_iou":0.3}"#,
    )
    .unwrap();
    let rules = out.join("templates/page_rules.json");
    {
        let mut f = fs::File::create(&rules).unwrap();
        write!(
            f,
            r#"{{"ok":true,"pages":["select_slot","confirm_order","home","venue_sport_list"],"shadow":{{"min_confidence":0.55,"never_guess_buttons":true,"ocr_backend_preference":["pack_onnx_download","windows_media_system"],"note":"ONNX models ship in recognition pack only — not the Tauri main installer"}}}}"#
        )
        .unwrap();
    }
    let health = out.join("health.ok");
    fs::write(&health, b"ok").unwrap();

    let colors = out.join("rules/colors.json");
    let mut files = vec![
        ManifestFile {
            path: "templates/page_rules.json".into(),
            size: fs::metadata(&rules).unwrap().len(),
            sha256: sha256_file(&rules),
        },
        ManifestFile {
            path: "rules/colors.json".into(),
            size: fs::metadata(&colors).unwrap().len(),
            sha256: sha256_file(&colors),
        },
        ManifestFile {
            path: "health.ok".into(),
            size: fs::metadata(&health).unwrap().len(),
            sha256: sha256_file(&health),
        },
    ];
    for family in ["discover_compass", "miniprogram_entry"] {
        let manifest = shell.join(family).join("manifest.json");
        files.push(ManifestFile {
            path: format!("templates/wechat_shell/{family}/manifest.json"),
            size: fs::metadata(&manifest).unwrap().len(),
            sha256: sha256_file(&manifest),
        });
        for name in [
            "inactive_light",
            "active_light",
            "inactive_dark",
            "active_dark",
        ] {
            let gray = shell.join(family).join(format!("{name}.gray"));
            files.push(ManifestFile {
                path: format!("templates/wechat_shell/{family}/{name}.gray"),
                size: fs::metadata(&gray).unwrap().len(),
                sha256: sha256_file(&gray),
            });
        }
    }

    let mut m = PackManifest {
        pack_version: version,
        protocol_version: 1,
        minimum_app_version: "0.1.0".into(),
        target_triple: triple,
        ocr_backend: "download_onnx_baseline".into(),
        files,
        model_versions: Default::default(),
        required_capabilities: vec!["ocr".into(), "captcha".into()],
        signature: String::new(),
        key_id: "dev-test-key".into(),
    };
    m.signature = compute_placeholder_signature(&m);
    fs::write(
        out.join("manifest.json"),
        serde_json::to_string_pretty(&m).unwrap(),
    )
    .unwrap();
    println!("wrote fake pack to {}", out.display());
    println!("signature={}", m.signature);
}
