use anyhow::Result;
pub use dubsync_core::{AudioData, read_audio};
use std::path::Path;

pub fn write_audio(path: &str, audio: &AudioData) -> Result<()> {
    // We can use ResourceManager's save_audio or just implement it here if needed.
    // ResourceManager::save_audio is private currently.
    // I'll implement a public one in core or just keep it here for now using hound.

    let path_obj = Path::new(path);
    if let Some(parent) = path_obj.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let spec = hound::WavSpec {
        channels: audio.channels,
        sample_rate: audio.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(path, spec)?;
    for sample in &audio.samples {
        let s = (sample * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        writer.write_sample(s)?;
    }

    writer.finalize()?;
    Ok(())
}
