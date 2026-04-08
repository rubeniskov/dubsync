mod types;

pub mod core {
    pub mod audio;
    pub mod dsp;
    pub mod engine;
    pub mod splitter;
    pub mod stream_splitter;
}

// Re-exports from dubsync-model
pub use dubsync_model::error::{self, Result};
pub use dubsync_model::io;
pub use dubsync_model::model_manager;
pub use dubsync_model::registry;

// Public API
pub use crate::core::splitter::split_file;
pub use crate::core::stream_splitter::StreamSplitter;
pub use crate::types::{AudioData, SplitOptions, SplitResult};
pub use dubsync_model::io::progress::{
    SplitProgress, set_download_progress_callback, set_split_progress_callback,
};
pub use dubsync_model::model_manager::{ModelHandle, ensure_model};
pub use dubsync_model::types::{Artifact, IODesc, ModelManifest, ResolvedArtifact};

pub fn prepare_model(
    model_name: &str,
    manifest_url_override: Option<&str>,
) -> dubsync_model::error::Result<()> {
    let handle = ensure_model(model_name, manifest_url_override)?;
    crate::core::engine::preload(&handle)?;
    Ok(())
}
