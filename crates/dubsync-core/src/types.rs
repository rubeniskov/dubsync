use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub version: String,
    pub reference_path: Option<PathBuf>,
    pub target_path: Option<PathBuf>,
    pub alignment_report: Option<dubsync_dsp::util::alignment::AlignmentReport>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            reference_path: None,
            target_path: None,
            alignment_report: None,
        }
    }
}
