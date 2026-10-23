use serde::{Deserialize, Serialize};

use crate::time::{lease_issue_and_expiry, DEFAULT_LEASE_TTL_SECS};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKind {
    Windows,
    Macos,
    Fake,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnerIdentity {
    pub pid: u32,
    pub app_identity: String,
    pub launch_identity: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeI {
    pub width: u32,
    pub height: u32,
}

/// Bound target surface lease — summary only; native_ref stays in the adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetSurfaceLease {
    pub lease_id: String,
    pub generation: u64,
    pub platform: PlatformKind,
    pub owner: OwnerIdentity,
    pub surface_fingerprint: String,
    pub title_hint: String,
    pub bounds_in_screen_points: RectF,
    pub capture_pixel_size: SizeI,
    pub scale_x: f64,
    pub scale_y: f64,
    /// RFC3339 wall clock when the lease was issued.
    pub issued_at: String,
    /// RFC3339 wall clock; after this, validate must return Invalid (rebind required).
    pub expires_at: String,
}

impl TargetSurfaceLease {
    pub fn stamp_ttl(&mut self, ttl_secs: i64) {
        let (issued, expires) = lease_issue_and_expiry(ttl_secs);
        self.issued_at = issued;
        self.expires_at = expires;
    }

    pub fn with_default_ttl(mut self) -> Self {
        self.stamp_ttl(DEFAULT_LEASE_TTL_SECS);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceCandidate {
    pub candidate_id: String,
    pub title_hint: String,
    pub owner: OwnerIdentity,
    pub bounds_in_screen_points: RectF,
    pub surface_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValidateOutcome {
    Valid,
    RebindRequired { reason: String },
    Invalid { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_example_roundtrip() {
        let raw = include_str!("../../../../../docs/contracts/examples/target_surface_lease.json");
        let lease: TargetSurfaceLease = serde_json::from_str(raw).expect("parse lease");
        assert_eq!(lease.platform, PlatformKind::Fake);
        assert_eq!(lease.generation, 0);
        let issued = crate::time::parse_rfc3339(&lease.issued_at).unwrap();
        let expires = crate::time::parse_rfc3339(&lease.expires_at).unwrap();
        assert!(expires > issued);
        let v = serde_json::to_value(&lease).unwrap();
        let again: TargetSurfaceLease = serde_json::from_value(v).unwrap();
        assert_eq!(again.lease_id, lease.lease_id);
    }

    #[test]
    fn stamp_ttl_separates_issue_and_expiry() {
        let mut lease = TargetSurfaceLease {
            lease_id: "L".into(),
            generation: 0,
            platform: PlatformKind::Fake,
            owner: OwnerIdentity {
                pid: 1,
                app_identity: "a".into(),
                launch_identity: "b".into(),
            },
            surface_fingerprint: "fp".into(),
            title_hint: "t".into(),
            bounds_in_screen_points: RectF {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            capture_pixel_size: SizeI {
                width: 10,
                height: 10,
            },
            scale_x: 1.0,
            scale_y: 1.0,
            issued_at: String::new(),
            expires_at: String::new(),
        };
        lease.stamp_ttl(60);
        assert_ne!(lease.issued_at, lease.expires_at);
        assert!(!crate::time::lease_expired(&lease.expires_at, chrono::Local::now()).unwrap());
    }
}
