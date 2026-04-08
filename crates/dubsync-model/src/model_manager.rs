use crate::{
    error::{Result, StemError},
    io::{
        crypto::verify_sha256,
        net::{download_with_progress, http_client},
        paths::models_cache_dir,
    },
    provider::ModelProvider,
    registry::resolve_manifest_url,
    types::ModelManifest,
};

use std::sync::Arc;
use std::{fs, path::PathBuf};

pub struct ModelHandle {
    pub manifest: ModelManifest,
    pub local_path: PathBuf,
}

pub struct ModelManager {
    provider: Arc<dyn ModelProvider>,
    cache_dir: PathBuf,
}

impl ModelManager {
    pub fn new(provider: Arc<dyn ModelProvider>) -> Result<Self> {
        let cache_dir = models_cache_dir()?;
        fs::create_dir_all(&cache_dir)?;
        Ok(Self { provider, cache_dir })
    }

    pub async fn ensure_model(&self, model_name: &str) -> anyhow::Result<ModelHandle> {
        let metadata = self.provider.resolve(model_name).await?;

        // For now, we assume the first file is the model manifest if it's a JSON
        // or we use the resolve logic to find the main artifact.
        // This is a bit complex because ModelProvider is more generic than our current ModelManifest.

        // For compatibility with the rest of the app, let's try to find a manifest.json
        let manifest_file = metadata
            .files
            .iter()
            .find(|f| f.path.ends_with("manifest.json"))
            .ok_or_else(|| anyhow::anyhow!("No manifest.json found in model repo"))?;

        let manifest_path = self.cache_dir.join(format!("{}-manifest.json", model_name));

        // Download manifest if needed
        // (Simplified for now)
        let client = reqwest::Client::new();
        let resp = client.get(&manifest_file.url).send().await?;
        let manifest: ModelManifest = resp.json().await?;

        // Save to cache
        let json = serde_json::to_string_pretty(&manifest)?;
        fs::write(&manifest_path, json)?;

        let a = manifest.resolve_primary_artifact().map_err(|e| anyhow::anyhow!(e))?;
        let ext = a.file.rsplit('.').next().map(|s| format!(".{s}")).unwrap_or_default();
        let file_name = format!("{}-{}{}", manifest.name, &a.sha256[..8], ext);
        let local_path = self.cache_dir.join(file_name);

        let need_download = !matches!(verify_sha256(&local_path, &a.sha256), Ok(true));
        if need_download {
            let handle = self.provider.download(model_name, None, &local_path).await?;
            handle.wait().await?;

            if !verify_sha256(&local_path, &a.sha256)? {
                return Err(anyhow::anyhow!("Checksum mismatch for {}", local_path.display()));
            }
        }

        Ok(ModelHandle { manifest, local_path })
    }
}

pub fn ensure_model(model_name: &str, manifest_url_override: Option<&str>) -> Result<ModelHandle> {
    let manifest_url = manifest_url_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| resolve_manifest_url(model_name).expect("resolve_manifest_url failed"));

    let cache_dir = models_cache_dir()?;
    fs::create_dir_all(&cache_dir)?;

    let manifest_cache_path = cache_dir.join(format!("{}-manifest.json", model_name));

    let client = http_client();

    // Try to fetch remote manifest
    let manifest_res = client
        .get(&manifest_url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .and_then(|resp| resp.error_for_status())
        .and_then(|resp| resp.json::<ModelManifest>());

    let manifest = match manifest_res {
        Ok(m) => {
            // Success! Cache it.
            if let Ok(json) = serde_json::to_string_pretty(&m) {
                let _ = fs::write(&manifest_cache_path, json);
            }
            m
        }
        Err(e) => {
            // Remote failed (e.g. 504). Try local cache.
            if manifest_cache_path.exists() {
                eprintln!("⚠️  Remote manifest fetch failed ({}), using cached manifest.", e);
                let content = fs::read_to_string(&manifest_cache_path)?;
                serde_json::from_str(&content)?
            } else {
                // No local cache either, propagate error.
                return Err(e.into());
            }
        }
    };

    let a = manifest.resolve_primary_artifact().map_err(StemError::Manifest)?;

    let ext = a.file.rsplit('.').next().map(|s| format!(".{s}")).unwrap_or_default();
    let file_name = format!("{}-{}{}", manifest.name, &a.sha256[..8], ext);
    let local_path = cache_dir.join(file_name);

    let need_download = !matches!(verify_sha256(&local_path, &a.sha256), Ok(true));
    if need_download {
        download_with_progress(&client, &a.url, &local_path)?;
        if !verify_sha256(&local_path, &a.sha256)? {
            return Err(StemError::Checksum { path: local_path.display().to_string() });
        }
        if a.size_bytes > 0 {
            let size = fs::metadata(&local_path).map(|m| m.len()).unwrap_or(0);
            if size != a.size_bytes {
                eprintln!(
                    "⚠️  Warning: size mismatch for {}, expected {} bytes, got {} bytes",
                    local_path.display(),
                    a.size_bytes,
                    size
                );
            }
        }
    }

    Ok(ModelHandle { manifest, local_path })
}
