pub mod error;
pub mod model_manager;
pub mod provider;
pub mod registry;
pub mod types;

pub mod io {
    pub mod crypto;
    pub mod net;
    pub mod paths;
    pub mod progress;
}

pub use crate::error::{Result, StemError};
pub use crate::io::progress::{
    SplitProgress, set_download_progress_callback, set_split_progress_callback,
};
pub use crate::model_manager::{ModelHandle, ensure_model};
pub use crate::types::{Artifact, IODesc, ModelManifest, ResolvedArtifact};
