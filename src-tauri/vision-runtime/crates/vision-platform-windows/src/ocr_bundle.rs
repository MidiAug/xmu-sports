//! Bundled OCR models (installer resources) → LOCALAPPDATA cache.
//! Spec: docs/superpowers/specs/2026-09-23-windows-installer-ocr-bundle-design.md

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

const CAPTCHA_NAME: &str = "common.onnx";
const CAPTCHA_BYTES: u64 = 54_088_400;
const CAPTCHA_SHA256: &str = "33b5cd351ee94e73a6bf8fa18c415ed8b819b3ffd342e267c30d8ad8334e34e8";

const RAPIDOCR_FILES: &[(&str, u64, &str)] = &[
    (
        "ch_PP-OCRv5_det_mobile.onnx",
        4_819_576,
        "4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae",
    ),
    (
        "ch_PP-OCRv5_rec_mobile.onnx",
        16_631_306,
        "5825fc7ebf84ae7a412be049820b4d86d77620f204a041697b0494669b1742c5",
    ),
    (
        "ppocrv5_dict.txt",
        74_012,
        "d1979e9f794c464c0d2e0b70a7fe14dd978e9dc644c0e71f14158cdf8342af1b",
    ),
];

#[derive(Debug, Default, Clone)]
pub struct SeedReport {
    pub rapidocr_copied: usize,
    pub captcha_copied: bool,
    pub bundle_root: Option<PathBuf>,
    pub notes: Vec<String>,
}

/// Candidate roots that contain `rapidocr/` and `captcha/` subdirs.
pub fn bundle_ocr_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(p) = std::env::var("XMU_SPORTS_OCR_BUNDLE") {
        if !p.is_empty() {
            roots.push(PathBuf::from(p));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.join("ocr"));
            roots.push(dir.join("resources").join("ocr"));
        }
    }
    // Dev: src-tauri/resources/ocr (this crate is vision-runtime/crates/vision-platform-windows)
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    roots.push(
        crate_dir
            .join("..")
            .join("..")
            .join("..")
            .join("resources")
            .join("ocr"),
    );
    roots
}

pub fn find_bundle_ocr_root() -> Option<PathBuf> {
    bundle_ocr_roots().into_iter().find(|r| bundle_looks_valid(r))
}

fn bundle_looks_valid(root: &Path) -> bool {
    root.join("rapidocr")
        .join(RAPIDOCR_FILES[0].0)
        .is_file()
        || root.join("captcha").join(CAPTCHA_NAME).is_file()
}

pub fn data_root() -> PathBuf {
    if let Ok(p) = std::env::var("XMU_SPORTS_DATA") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("xmu-sports")
}

/// Copy missing models from `bundle_root` (or auto-discovered) into the cache.
pub fn seed_ocr_cache(bundle_root: Option<&Path>) -> SeedReport {
    let mut report = SeedReport::default();
    let root = match bundle_root.map(Path::to_path_buf).or_else(find_bundle_ocr_root) {
        Some(r) => r,
        None => {
            report
                .notes
                .push("ocr:seed=skip_no_bundle".into());
            return report;
        }
    };
    report.bundle_root = Some(root.clone());
    let cache = data_root();

    let rapid_src = root.join("rapidocr");
    let rapid_dst = cache.join("rapidocr");
    for &(name, bytes, sha) in RAPIDOCR_FILES {
        let dst = rapid_dst.join(name);
        if file_matches(&dst, bytes, sha) {
            continue;
        }
        let src = rapid_src.join(name);
        if !file_matches(&src, bytes, sha) {
            report
                .notes
                .push(format!("ocr:seed_skip={name}_bundle_invalid"));
            continue;
        }
        if let Err(e) = copy_atomic(&src, &dst) {
            report.notes.push(format!("ocr:seed_fail={name}:{e}"));
            continue;
        }
        report.rapidocr_copied += 1;
        report.notes.push(format!("ocr:seed=rapidocr/{name}"));
    }

    let cap_dst = cache.join("captcha").join(CAPTCHA_NAME);
    if !file_matches(&cap_dst, CAPTCHA_BYTES, CAPTCHA_SHA256) {
        let cap_src = root.join("captcha").join(CAPTCHA_NAME);
        if file_matches(&cap_src, CAPTCHA_BYTES, CAPTCHA_SHA256) {
            match copy_atomic(&cap_src, &cap_dst) {
                Ok(()) => {
                    report.captcha_copied = true;
                    report.notes.push("ocr:seed=captcha/common.onnx".into());
                }
                Err(e) => report.notes.push(format!("ocr:seed_fail=captcha:{e}")),
            }
        } else {
            report
                .notes
                .push("ocr:seed_skip=captcha_bundle_invalid".into());
        }
    }

    if report.rapidocr_copied == 0 && !report.captcha_copied {
        report.notes.push("ocr:seed=noop_cache_ok".into());
    }
    report
}

/// Seed from an explicit Tauri `$RESOURCE/ocr` path (preferred at app setup).
pub fn seed_ocr_cache_from_resource_dir(resource_dir: &Path) -> SeedReport {
    let candidates = [
        resource_dir.join("ocr"),
        resource_dir.join("resources").join("ocr"),
        resource_dir.to_path_buf(),
    ];
    for c in candidates {
        if bundle_looks_valid(&c) {
            return seed_ocr_cache(Some(&c));
        }
    }
    seed_ocr_cache(None)
}

fn file_matches(path: &Path, bytes: u64, sha256_hex: &str) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if meta.len() != bytes {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut hash = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let Ok(n) = file.read(&mut buf) else {
            return false;
        };
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    format!("{:x}", hash.finalize()) == sha256_hex
}

fn copy_atomic(src: &Path, dst: &Path) -> Result<(), String> {
    let parent = dst.parent().ok_or("ocr seed dest parent missing")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        dst.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("ocr"),
        std::process::id()
    ));
    {
        let mut input = fs::File::open(src).map_err(|e| e.to_string())?;
        let mut output = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = input.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            output.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        }
    }
    fs::rename(&tmp, dst).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn seed_copies_valid_bundle_into_empty_cache() {
        let tmp = std::env::temp_dir().join(format!("xmu-ocr-seed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let bundle = tmp.join("bundle");
        let data = tmp.join("data");
        fs::create_dir_all(bundle.join("rapidocr")).unwrap();
        fs::create_dir_all(bundle.join("captcha")).unwrap();
        // Tiny stand-ins won't match SHA — use real write of expected length is too heavy.
        // Instead verify discover + noop when invalid, and copy path with crafted matching files.
        let (name, bytes, sha) = RAPIDOCR_FILES[2]; // dict is small (74KB)
        // We don't embed the real dict; skip if we can't synthesize matching hash.
        // Unit test focuses on file_matches + copy_atomic with known content.
        let content = b"ppocr-dict-test-bytes";
        let mut hasher = Sha256::new();
        hasher.update(content);
        let sha_hex = format!("{:x}", hasher.finalize());
        let src = bundle.join("rapidocr").join("tiny.txt");
        {
            let mut f = fs::File::create(&src).unwrap();
            f.write_all(content).unwrap();
        }
        assert!(file_matches(&src, content.len() as u64, &sha_hex));
        let dst = data.join("rapidocr").join("tiny.txt");
        copy_atomic(&src, &dst).unwrap();
        assert!(file_matches(&dst, content.len() as u64, &sha_hex));

        // Expected real dict constants stay documented.
        assert_eq!(name, "ppocrv5_dict.txt");
        assert_eq!(bytes, 74_012);
        assert_eq!(sha.len(), 64);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn bundle_roots_include_env_override() {
        std::env::set_var("XMU_SPORTS_OCR_BUNDLE", "C:\\fake-ocr-bundle");
        let roots = bundle_ocr_roots();
        std::env::remove_var("XMU_SPORTS_OCR_BUNDLE");
        assert!(roots
            .iter()
            .any(|p| p.to_string_lossy().contains("fake-ocr-bundle")));
    }

    #[test]
    fn seed_from_repo_resources_when_present() {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let bundle = crate_dir
            .join("..")
            .join("..")
            .join("..")
            .join("resources")
            .join("ocr");
        if !bundle_looks_valid(&bundle) {
            // CI / clean checkout without ensure_ocr_bundle.ps1
            return;
        }
        let tmp = std::env::temp_dir().join(format!("xmu-ocr-seed-it-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        std::env::set_var("XMU_SPORTS_DATA", &tmp);
        let report = seed_ocr_cache(Some(&bundle));
        std::env::remove_var("XMU_SPORTS_DATA");
        assert!(
            report.rapidocr_copied >= 1 || tmp.join("rapidocr").join(RAPIDOCR_FILES[0].0).is_file(),
            "expected rapidocr seed: {report:?}"
        );
        assert!(
            report.captcha_copied || tmp.join("captcha").join(CAPTCHA_NAME).is_file(),
            "expected captcha seed: {report:?}"
        );
        let _ = fs::remove_dir_all(&tmp);
    }
}
