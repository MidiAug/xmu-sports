use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::manifest::PackManifest;

#[derive(Debug, Error)]
pub enum PackError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackStatus {
    NotInstalled,
    Installing,
    Ready,
    Fault,
}

#[derive(Debug, Clone)]
pub struct PackState {
    pub status: PackStatus,
    pub active_version: Option<String>,
    pub reason: String,
}

pub fn pack_root(data_root: &Path) -> PathBuf {
    data_root.join("vision-packs")
}

pub fn active_pack_dir(data_root: &Path) -> Option<PathBuf> {
    let pointer = pack_root(data_root).join("active");
    let raw = fs::read_to_string(pointer).ok()?;
    let ver = raw.trim();
    if ver.is_empty() {
        return None;
    }
    let dir = pack_root(data_root).join("versions").join(ver);
    if dir.is_dir() {
        Some(dir)
    } else {
        None
    }
}

fn sha256_file(path: &Path) -> Result<String, PackError> {
    let mut f = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Verify staging directory against manifest; on success atomically set active pointer.
/// `expected_signature` is a shared secret / placeholder until real signing lands —
/// for local fake packs use the literal signature field from the manifest.
pub fn install_from_staging(
    data_root: &Path,
    staging_dir: &Path,
    expected_triple: &str,
    min_protocol: u32,
) -> Result<PackState, PackError> {
    let manifest_path = staging_dir.join("manifest.json");
    let raw = fs::read_to_string(&manifest_path)?;
    let manifest: PackManifest = serde_json::from_str(&raw)?;
    manifest
        .validate_paths()
        .map_err(PackError::Message)?;
    if manifest.protocol_version < min_protocol {
        return Err(PackError::Message(format!(
            "incompatible protocol_version {}",
            manifest.protocol_version
        )));
    }
    if manifest.target_triple != expected_triple {
        return Err(PackError::Message(format!(
            "target_triple mismatch: {} != {}",
            manifest.target_triple, expected_triple
        )));
    }
    if manifest.signature.is_empty() || manifest.key_id.is_empty() {
        return Err(PackError::Message("missing signature/key_id".into()));
    }
    // Placeholder signature check: signature must equal sha256 of canonical file list.
    let expected_sig = compute_placeholder_signature(&manifest);
    if manifest.signature != expected_sig {
        return Err(PackError::Message("bad pack signature".into()));
    }

    for file in &manifest.files {
        let path = staging_dir.join(&file.path);
        if !path.is_file() {
            return Err(PackError::Message(format!("missing file {}", file.path)));
        }
        let meta = fs::metadata(&path)?;
        if meta.len() != file.size {
            return Err(PackError::Message(format!("size mismatch {}", file.path)));
        }
        let digest = sha256_file(&path)?;
        if !digest.eq_ignore_ascii_case(&file.sha256) {
            return Err(PackError::Message(format!("hash mismatch {}", file.path)));
        }
        // Reject symlink escape
        if path
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(PackError::Message(format!("symlink forbidden {}", file.path)));
        }
    }

    // Health: require templates/rules marker file if listed, else any listed file ok.
    let health = staging_dir.join("health.ok");
    if health.is_file() {
        let ok = fs::read_to_string(&health)?;
        if !ok.trim().eq_ignore_ascii_case("ok") {
            return Err(PackError::Message("health check failed".into()));
        }
    }

    let versions = pack_root(data_root).join("versions");
    fs::create_dir_all(&versions)?;
    let dest = versions.join(&manifest.pack_version);
    if dest.exists() {
        fs::remove_dir_all(&dest)?;
    }
    copy_dir_all(staging_dir, &dest)?;

    let active_ptr = pack_root(data_root).join("active");
    let tmp = pack_root(data_root).join("active.tmp");
    fs::write(&tmp, &manifest.pack_version)?;
    fs::rename(&tmp, &active_ptr)?;

    Ok(PackState {
        status: PackStatus::Ready,
        active_version: Some(manifest.pack_version),
        reason: "installed".into(),
    })
}

pub fn compute_placeholder_signature(manifest: &PackManifest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(manifest.pack_version.as_bytes());
    hasher.update(manifest.target_triple.as_bytes());
    hasher.update(manifest.key_id.as_bytes());
    for f in &manifest.files {
        hasher.update(f.path.as_bytes());
        hasher.update(f.sha256.as_bytes());
    }
    hex::encode(hasher.finalize())
}

pub fn rollback_to(data_root: &Path, version: &str) -> Result<PackState, PackError> {
    let dir = pack_root(data_root).join("versions").join(version);
    if !dir.is_dir() {
        return Err(PackError::Message(format!("version not found: {version}")));
    }
    let active_ptr = pack_root(data_root).join("active");
    let tmp = pack_root(data_root).join("active.tmp");
    fs::write(&tmp, version)?;
    fs::rename(&tmp, &active_ptr)?;
    Ok(PackState {
        status: PackStatus::Ready,
        active_version: Some(version.to_string()),
        reason: "rolled_back".into(),
    })
}

pub fn uninstall_active(data_root: &Path) -> Result<PackState, PackError> {
    let active_ptr = pack_root(data_root).join("active");
    if active_ptr.exists() {
        fs::remove_file(&active_ptr)?;
    }
    Ok(PackState {
        status: PackStatus::NotInstalled,
        active_version: None,
        reason: "uninstalled".into(),
    })
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), PackError> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), &to)?;
        } else {
            return Err(PackError::Message(format!(
                "unsupported entry {:?}",
                entry.path()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ManifestFile;
    use std::io::Write;

    fn write_pack(dir: &Path, version: &str, triple: &str, bad_hash: bool) -> PackManifest {
        fs::create_dir_all(dir.join("templates")).unwrap();
        let rules = dir.join("templates").join("page_rules.json");
        {
            let mut f = fs::File::create(&rules).unwrap();
            write!(f, "{{\"ok\":true}}").unwrap();
        }
        let health = dir.join("health.ok");
        fs::write(&health, "ok").unwrap();
        let digest = sha256_file(&rules).unwrap();
        let size = fs::metadata(&rules).unwrap().len();
        let mut files = vec![
            ManifestFile {
                path: "templates/page_rules.json".into(),
                size,
                sha256: if bad_hash {
                    "0".repeat(64)
                } else {
                    digest
                },
            },
            ManifestFile {
                path: "health.ok".into(),
                size: 2,
                sha256: sha256_file(&health).unwrap(),
            },
        ];
        // fix health size
        files[1].size = fs::metadata(&health).unwrap().len();

        let mut m = PackManifest {
            pack_version: version.into(),
            protocol_version: 1,
            minimum_app_version: "0.1.0".into(),
            target_triple: triple.into(),
            ocr_backend: "download_onnx_baseline".into(),
            files,
            model_versions: Default::default(),
            required_capabilities: vec!["ocr".into(), "captcha".into()],
            signature: String::new(),
            key_id: "dev-test-key".into(),
        };
        m.signature = compute_placeholder_signature(&m);
        fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&m).unwrap()).unwrap();
        m
    }

    #[test]
    fn install_verify_rollback() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let staging = tmp.path().join("staging-v1");
        write_pack(&staging, "1.0.0", "x86_64-pc-windows-msvc", false);
        let st = install_from_staging(&data, &staging, "x86_64-pc-windows-msvc", 1).unwrap();
        assert_eq!(st.status, PackStatus::Ready);
        assert_eq!(st.active_version.as_deref(), Some("1.0.0"));
        assert!(active_pack_dir(&data).unwrap().join("health.ok").is_file());

        let staging2 = tmp.path().join("staging-v2");
        write_pack(&staging2, "1.0.1", "x86_64-pc-windows-msvc", false);
        install_from_staging(&data, &staging2, "x86_64-pc-windows-msvc", 1).unwrap();
        assert_eq!(
            active_pack_dir(&data)
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "1.0.1"
        );

        rollback_to(&data, "1.0.0").unwrap();
        assert_eq!(
            fs::read_to_string(pack_root(&data).join("active"))
                .unwrap()
                .trim(),
            "1.0.0"
        );
    }

    #[test]
    fn bad_hash_keeps_previous() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let staging = tmp.path().join("staging-v1");
        write_pack(&staging, "1.0.0", "x86_64-pc-windows-msvc", false);
        install_from_staging(&data, &staging, "x86_64-pc-windows-msvc", 1).unwrap();

        let staging_bad = tmp.path().join("staging-bad");
        write_pack(&staging_bad, "9.9.9", "x86_64-pc-windows-msvc", true);
        let err = install_from_staging(&data, &staging_bad, "x86_64-pc-windows-msvc", 1).unwrap_err();
        assert!(err.to_string().contains("hash mismatch"));
        assert_eq!(
            fs::read_to_string(pack_root(&data).join("active"))
                .unwrap()
                .trim(),
            "1.0.0"
        );
    }

    #[test]
    fn rejects_path_traversal() {
        let mut m = PackManifest {
            pack_version: "1".into(),
            protocol_version: 1,
            minimum_app_version: "0.1.0".into(),
            target_triple: "x".into(),
            ocr_backend: "x".into(),
            files: vec![ManifestFile {
                path: "../etc/passwd".into(),
                size: 1,
                sha256: "a".into(),
            }],
            model_versions: Default::default(),
            required_capabilities: vec![],
            signature: "s".into(),
            key_id: "k".into(),
        };
        assert!(m.validate_paths().is_err());
        m.files[0].path = "ok/file.bin".into();
        assert!(m.validate_paths().is_ok());
    }
}
