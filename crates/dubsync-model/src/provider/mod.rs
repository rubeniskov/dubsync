use async_trait::async_trait;
use std::path::Path;
use tokio::task::JoinHandle;

pub mod huggingface;
pub mod modelscope;

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn resolve(&self, model: &str) -> anyhow::Result<ModelMetadata>;

    async fn download(
        &self,
        model: &str,
        revision: Option<&str>,
        dest: &Path,
    ) -> anyhow::Result<DownloadHandle>;
}

pub struct ModelMetadata {
    pub files: Vec<ModelFile>,
    pub total_size: u64,
}

pub struct ModelFile {
    pub path: String,
    pub size: u64,
    pub checksum: Option<String>,
    pub url: String,
}

pub struct DownloadHandle {
    pub handle: JoinHandle<anyhow::Result<()>>,
}

impl DownloadHandle {
    pub async fn wait(self) -> anyhow::Result<()> {
        self.handle.await?
    }
}
