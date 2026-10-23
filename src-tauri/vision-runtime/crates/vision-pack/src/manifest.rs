use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackManifest {
    pub pack_version: String,
    pub protocol_version: u32,
    pub minimum_app_version: String,
    pub target_triple: String,
    pub ocr_backend: String,
    pub files: Vec<ManifestFile>,
    #[serde(default)]
    pub model_versions: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    pub signature: String,
    pub key_id: String,
}

impl PackManifest {
    pub fn validate_paths(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for f in &self.files {
            if f.path.is_empty()
                || f.path.contains("..")
                || f.path.starts_with('/')
                || f.path.starts_with('\\')
                || f.path.contains(':')
            {
                return Err(format!("illegal path in manifest: {}", f.path));
            }
            if !seen.insert(f.path.clone()) {
                return Err(format!("duplicate path: {}", f.path));
            }
        }
        Ok(())
    }
}
