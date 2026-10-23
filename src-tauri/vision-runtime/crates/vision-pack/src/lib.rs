//! Recognition pack: staging → hash verify → atomic active pointer → rollback.
//! Pack absence blocks booking; worker binary may still be on disk.

mod install;
mod manifest;
mod perception;

pub use install::{
    active_pack_dir, compute_placeholder_signature, install_from_staging, pack_root, PackError,
    PackState, PackStatus, rollback_to, uninstall_active,
};
pub use manifest::{ManifestFile, PackManifest};
pub use perception::{perception_health, PerceptionHealth, COLORS_REL, DETECTOR_REL, SHELL_TEMPLATES_REL};
