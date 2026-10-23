//! Pack files required before a booking run may arm.
//!
//! The interactable detector weight is download-only (`models/interactable.onnx`,
//! OmniParser icon-detect or an equivalent YOLO head). It is not committed.
//! This check does not load ONNX Runtime; a missing or empty file blocks arming.

use std::fs;
use std::path::Path;

use serde::Serialize;

pub const COLORS_REL: &str = "rules/colors.json";
pub const SHELL_TEMPLATES_REL: &str = "templates/wechat_shell";
pub const DETECTOR_REL: &str = "models/interactable.onnx";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PerceptionHealth {
    pub colors_ok: bool,
    pub shell_templates_ok: bool,
    pub detector_present: bool,
    pub ready_for_arm: bool,
    pub missing: Vec<String>,
}

pub fn perception_health(pack_dir: &Path) -> PerceptionHealth {
    let mut missing = Vec::new();
    let colors_ok = colors_file_ok(&pack_dir.join(COLORS_REL), &mut missing);
    let shell_templates_ok = shell_templates_ok(&pack_dir.join(SHELL_TEMPLATES_REL), &mut missing);
    let detector_present = detector_file_ok(&pack_dir.join(DETECTOR_REL), &mut missing);
    PerceptionHealth {
        colors_ok,
        shell_templates_ok,
        detector_present,
        ready_for_arm: colors_ok && shell_templates_ok && detector_present,
        missing,
    }
}

fn colors_file_ok(path: &Path, missing: &mut Vec<String>) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        missing.push(COLORS_REL.into());
        return false;
    };
    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(v) if v.get("pale_h").is_some() && v.get("strong_h").is_some() => true,
        _ => {
            missing.push(format!("{COLORS_REL}:invalid"));
            false
        }
    }
}

fn shell_templates_ok(dir: &Path, missing: &mut Vec<String>) -> bool {
    const FAMILIES: [&str; 2] = ["discover_compass", "miniprogram_entry"];
    const VARIANTS: [&str; 4] = [
        "inactive_light",
        "active_light",
        "inactive_dark",
        "active_dark",
    ];
    let mut ok = true;
    for family in FAMILIES {
        let root = dir.join(family);
        let manifest_path = root.join("manifest.json");
        let raw = match fs::read_to_string(&manifest_path) {
            Ok(s) => s,
            Err(_) => {
                missing.push(format!("{family}/manifest.json"));
                ok = false;
                continue;
            }
        };
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&raw);
        let Ok(value) = parsed else {
            missing.push(format!("{family}/manifest.json:invalid"));
            ok = false;
            continue;
        };
        let Some(names) = value.get("variants").and_then(|v| v.as_array()) else {
            missing.push(format!("{family}/manifest.json:invalid"));
            ok = false;
            continue;
        };
        let listed: Vec<&str> = names.iter().filter_map(|v| v.as_str()).collect();
        for variant in VARIANTS {
            if !listed.contains(&variant) {
                missing.push(format!("{family}:{variant}"));
                ok = false;
                continue;
            }
            let png = root.join(format!("{variant}.png"));
            let gray = root.join(format!("{variant}.gray"));
            let present = fs::metadata(&png).map(|m| m.len() > 0).unwrap_or(false)
                || fs::metadata(&gray).map(|m| m.len() > 0).unwrap_or(false);
            if !present {
                missing.push(format!("{family}/{variant}"));
                ok = false;
            }
        }
    }
    ok
}

fn detector_file_ok(path: &Path, missing: &mut Vec<String>) -> bool {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() > 0 => true,
        _ => {
            missing.push(DETECTOR_REL.into());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perception_health_blocks_arm_until_detector_and_rules_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = perception_health(tmp.path());
        assert!(!bare.ready_for_arm);
        assert!(!bare.colors_ok);
        assert!(!bare.detector_present);

        let rules = tmp.path().join("rules");
        fs::create_dir_all(&rules).unwrap();
        fs::write(
            rules.join("colors.json"),
            r#"{"pale_h":[68,98],"strong_h":[35,100]}"#,
        )
        .unwrap();
        let shell = tmp.path().join("templates").join("wechat_shell");
        fs::create_dir_all(&shell).unwrap();
        fs::write(shell.join("discover_compass_2x2.gray"), [1u8, 2, 3, 4]).unwrap();

        let lone = perception_health(tmp.path());
        assert!(lone.colors_ok);
        assert!(!lone.shell_templates_ok, "one gray file is not a shell family");
        assert!(!lone.detector_present);
        assert!(!lone.ready_for_arm);

        write_shell_family(&shell, "discover_compass");
        write_shell_family(&shell, "miniprogram_entry");

        let partial = perception_health(tmp.path());
        assert!(partial.colors_ok);
        assert!(partial.shell_templates_ok);
        assert!(!partial.detector_present);
        assert!(!partial.ready_for_arm);

        let models = tmp.path().join("models");
        fs::create_dir_all(&models).unwrap();
        fs::write(models.join("interactable.onnx"), b"not-a-real-weight").unwrap();
        let ready = perception_health(tmp.path());
        assert!(ready.ready_for_arm, "{:?}", ready.missing);
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
            fs::write(dir.join(format!("{name}.gray")), [1u8, 2, 3, 4]).unwrap();
        }
        fs::write(
            dir.join("manifest.json"),
            r#"{"element_id":"test","variants":["inactive_light","active_light","inactive_dark","active_dark"]}"#,
        )
        .unwrap();
    }
}
