use crate::provider::{DownloadHandle, ModelMetadata, ModelProvider};
use async_trait::async_trait;
use std::path::Path;

#[derive(Default)]
pub struct ModelScopeProvider;

impl ModelScopeProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ModelProvider for ModelScopeProvider {
    async fn resolve(&self, _model: &str) -> anyhow::Result<ModelMetadata> {
        // Placeholder for models-cat implementation
        Ok(ModelMetadata { files: vec![], total_size: 0 })
    }

    async fn download(
        &self,
        _model: &str,
        _revision: Option<&str>,
        _dest: &Path,
    ) -> anyhow::Result<DownloadHandle> {
        let handle = tokio::spawn(async move {
            // Placeholder: models_cat::download_model_with_progress(...)
            Ok(())
        });

        Ok(DownloadHandle { handle })
    }
}
