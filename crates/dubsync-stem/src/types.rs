pub use dubsync_core::AudioData;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SplitOptions {
    pub output_dir: String,
    pub model_name: String,
    pub manifest_url_override: Option<String>,
}

impl Default for SplitOptions {
    fn default() -> Self {
        Self {
            output_dir: ".".into(),
            model_name: "htdemucs_ort_v1".into(),
            manifest_url_override: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SplitResult {
    pub vocals_path: String,
    pub drums_path: String,
    pub bass_path: String,
    pub other_path: String,
}
