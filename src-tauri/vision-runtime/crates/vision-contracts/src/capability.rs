use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    Unsupported,
    NotInstalled,
    PermissionRequired,
    Ready,
    RebindRequired,
    NeedHuman,
    RuntimeFault,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityReport {
    pub status: CapabilityStatus,
    pub reason_code: String,
    pub platform: crate::lease::PlatformKind,
    pub pack_ready: bool,
    pub input_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}
