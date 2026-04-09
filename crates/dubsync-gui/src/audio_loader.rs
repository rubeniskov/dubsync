use dubsync_core::{AudioData, AudioStats, ResourceManager};
use futures::stream::Stream;
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct WaveformCache {
    pub peaks: Vec<(f32, f32)>, // (min, max) pairs
}

impl WaveformCache {
    pub fn from_audio(audio: &AudioData, target_points: usize) -> Self {
        let stride = (audio.samples.len() / (audio.channels as usize) / target_points).max(1);
        let mut peaks = Vec::with_capacity(target_points);
        let mono_samples: Vec<f32> = audio
            .samples
            .chunks(audio.channels as usize)
            .map(|chunk| chunk.iter().sum::<f32>() / audio.channels as f32)
            .collect();

        for chunk in mono_samples.chunks(stride) {
            let mut min = 0.0f32;
            let mut max = 0.0f32;
            for &s in chunk {
                if s < min {
                    min = s;
                }
                if s > max {
                    max = s;
                }
            }
            peaks.push((min, max));
        }

        Self { peaks }
    }
}

pub enum LoadingStep {
    Meta(AudioStats),
    Progress { name: String, step: u8, total: u8, percent: f32 },
    Result(anyhow::Result<(AudioData, WaveformCache, AudioStats)>),
}

pub fn load_audio_file(
    path: PathBuf,
    cancel_token: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> impl Stream<Item = LoadingStep> {
    async_stream::stream! {
        let (tx, mut rx) = mpsc::channel(100);

        tokio::task::spawn_blocking(move || {
            let tx_inner = tx.clone();

            let result = (|| -> anyhow::Result<(AudioData, WaveformCache, AudioStats)> {
                // 1. Immediate Metadata extraction (from original)
                let stats = AudioStats::extract(&path)?;
                let _ = tx_inner.blocking_send(LoadingStep::Meta(stats.clone()));

                let needs_ffmpeg = !stats.codec.is_natively_supported();
                let total_steps = if needs_ffmpeg { 4 } else { 3 };
                let mut current_step = 1;

                // 2. Optional Extraction (Step 1/4 if video)
                let source_path = if needs_ffmpeg {
                    if cancel_token.load(std::sync::atomic::Ordering::Relaxed) { return Err(anyhow::anyhow!("Cancelled")); }
                    let tx_extract = tx_inner.clone();
                    let step_idx = current_step;
                    let token_extract = cancel_token.clone();

                    let extracted = ResourceManager::ensure_extracted_audio(&path, Some(move |p| {
                        if token_extract.load(std::sync::atomic::Ordering::Relaxed) { return false; }
                        let _ = tx_extract.blocking_send(LoadingStep::Progress {
                            name: "Extracting Audio".to_string(),
                            step: step_idx,
                            total: total_steps,
                            percent: p
                        });
                        true
                    }))?;
                    current_step += 1;
                    extracted
                } else {
                    path.clone()
                };

                if cancel_token.load(std::sync::atomic::Ordering::Relaxed) { return Err(anyhow::anyhow!("Cancelled")); }

                // 3. Hash computation on the smallest valid input (Step 2/4 or 1/3)
                let tx_hash = tx_inner.clone();
                let token_hash = cancel_token.clone();
                let hash = ResourceManager::compute_hash(&source_path, Some(|p| {
                    if token_hash.load(std::sync::atomic::Ordering::Relaxed) { return false; }
                    let _ = tx_hash.blocking_send(LoadingStep::Progress {
                        name: "Hashing".to_string(),
                        step: current_step,
                        total: total_steps,
                        percent: p
                    });
                    true
                }))?;
                current_step += 1;

                // 4. Prepare mono audio (Step 3/4 or 2/3 - includes decoding)
                let tx_progress = tx_inner.clone();
                let step_idx = current_step;
                let step_name = if needs_ffmpeg { "Decoding".to_string() } else { "Decoding/Mono".to_string() };
                let token_decode = cancel_token.clone();
                let (_, audio) = ResourceManager::prepare_mono_audio(&source_path, &hash, Some(|p| {
                    if token_decode.load(std::sync::atomic::Ordering::Relaxed) { return false; }
                    let _ = tx_progress.blocking_send(LoadingStep::Progress {
                        name: step_name.clone(),
                        step: step_idx,
                        total: total_steps,
                        percent: p
                    });
                    true
                }))?;
                current_step += 1;

                if cancel_token.load(std::sync::atomic::Ordering::Relaxed) { return Err(anyhow::anyhow!("Cancelled")); }

                // 5. Build waveform cache (Step 4/4 or 3/3)
                let _ = tx_inner.blocking_send(LoadingStep::Progress {
                    name: "Generating Waveform".to_string(),
                    step: current_step,
                    total: total_steps,
                    percent: 0.0
                });
                let cache = WaveformCache::from_audio(&audio, 48000);

                if cancel_token.load(std::sync::atomic::Ordering::Relaxed) { return Err(anyhow::anyhow!("Cancelled")); }

                let _ = tx_inner.blocking_send(LoadingStep::Progress {
                    name: "Generating Waveform".to_string(),
                    step: current_step,
                    total: total_steps,
                    percent: 1.0
                });

                Ok((audio, cache, stats))
            })();

            let _ = tx.blocking_send(LoadingStep::Result(result));
        });

        while let Some(step) = rx.recv().await {
            yield step;
        }
    }
}
