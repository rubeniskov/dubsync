use crate::provider::{DownloadHandle, ModelFile, ModelMetadata, ModelProvider};
use async_trait::async_trait;
use hf_hub::api::tokio::Api;
use std::path::Path;

pub struct HuggingFaceProvider {
    api: Api,
}

impl HuggingFaceProvider {
    pub fn new() -> anyhow::Result<Self> {
        let api = Api::new()?;
        Ok(Self { api })
    }
}

#[async_trait]
impl ModelProvider for HuggingFaceProvider {
    async fn resolve(&self, model: &str) -> anyhow::Result<ModelMetadata> {
        let repo = self.api.model(model.to_string());
        let info = repo.info().await?;

        Ok(ModelMetadata {
            files: info
                .siblings
                .into_iter()
                .map(|f| ModelFile {
                    path: f.rfilename.clone(),
                    size: 0, // Siblings only contains rfilename in this version
                    checksum: None,
                    url: format!("https://huggingface.co/{}/resolve/main/{}", model, f.rfilename),
                })
                .collect(),
            total_size: 0,
        })
    }

    async fn download(
        &self,
        model: &str,
        _revision: Option<&str>,
        _dest: &Path,
    ) -> anyhow::Result<DownloadHandle> {
        let _repo = self.api.model(model.to_string());

        let handle = tokio::spawn(async move {
            // Placeholder implementation
            Ok(())
        });

        Ok(DownloadHandle { handle })
    }
}
