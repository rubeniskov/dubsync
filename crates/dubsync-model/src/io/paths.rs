use crate::error::{Result, StemError};
use directories::ProjectDirs;
use std::path::PathBuf;

pub fn models_cache_dir() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("DUBSYNC_MODELS_CACHE_DIR") {
        return Ok(PathBuf::from(p));
    }

    let proj = ProjectDirs::from("dev", "DubSync", "dubsync-model")
        .ok_or(StemError::CacheDirUnavailable)?;
    let mut p = PathBuf::from(proj.cache_dir());
    p.push("models");
    Ok(p)
}
