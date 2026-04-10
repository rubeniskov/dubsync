use crate::audio::{AudioData, AudioStats};
use anyhow::{Context, Result};
use blake3::Hasher;
use directories::BaseDirs;
use memmap2::Mmap;
use std::fs::File;
use std::path::{Path, PathBuf};

pub struct ResourceManager;

impl ResourceManager {
    pub fn expand_path<P: AsRef<Path>>(path: P) -> PathBuf {
        let p = path.as_ref();
        if let Ok(suffix) = p.strip_prefix("~") {
            if let Some(base_dirs) = BaseDirs::new() {
                return base_dirs.home_dir().join(suffix);
            }
        }
        p.to_path_buf()
    }

    pub fn compute_hash<P: AsRef<Path>, F>(
        path: P,
        mut progress_callback: Option<F>,
    ) -> Result<String>
    where
        F: FnMut(f32) -> bool,
    {
        let path = path.as_ref();
        let file = File::open(path)?;
        let total_size = file.metadata()?.len();
        if total_size == 0 {
            return Ok(Hasher::new().finalize().to_string());
        }
        let mmap = unsafe { Mmap::map(&file)? };
        let mut hasher = Hasher::new();
        let chunk_size = 1024 * 1024 * 256;
        for (i, chunk) in mmap.chunks(chunk_size).enumerate() {
            hasher.update_rayon(chunk);
            if let Some(ref mut cb) = progress_callback {
                if !cb(((i + 1) as f64 * chunk_size as f64 / total_size as f64).min(1.0) as f32) {
                    return Err(anyhow::anyhow!("Cancelled"));
                }
            }
        }
        Ok(hasher.finalize().to_string())
    }

    pub fn get_cache_dir() -> Result<PathBuf> {
        let base_dirs = BaseDirs::new().context("Could not determine user directories")?;
        let cache_dir = base_dirs.home_dir().join(".cache").join("dubsync");
        std::fs::create_dir_all(&cache_dir)?;
        Ok(cache_dir)
    }

    pub fn get_intermediate_path<F>(
        original_path: &Path,
        suffix: &str,
        progress_callback: Option<F>,
    ) -> Result<PathBuf>
    where
        F: FnMut(f32) -> bool,
    {
        let hash = Self::compute_hash(original_path, progress_callback)?;
        let cache_dir = Self::get_cache_dir()?;
        Ok(cache_dir.join(format!("{}_{}", hash, suffix)))
    }

    pub fn is_ffmpeg_available() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub fn extract_audio_from_video<F>(
        input_path: &Path,
        output_path: &Path,
        total_duration_secs: f64,
        mut progress_callback: Option<F>,
    ) -> Result<()>
    where
        F: FnMut(f32) -> bool,
    {
        if !Self::is_ffmpeg_available() {
            return Err(anyhow::anyhow!("ffmpeg not installed"));
        }
        let ffmpeg_temp_output_path = output_path.with_extension("process");
        let mut child = std::process::Command::new("ffmpeg")
            .args([
                "-i",
                &input_path.to_string_lossy(),
                "-progress",
                "pipe:1",
                "-map",
                "0:a:0?",
                "-c:a",
                "flac",
                "-f",
                "flac",
                "-compression_level",
                "5",
                "-y",
                &ffmpeg_temp_output_path.to_string_lossy(),
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().context("Failed to capture ffmpeg stdout")?;
        let stderr = child.stderr.take().context("Failed to capture ffmpeg stderr")?;
        std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stderr);
            use std::io::BufRead;
            for line in reader.lines().map_while(Result::ok) {
                if line.to_lowercase().contains("error") || line.starts_with("Unable to") {
                    eprintln!("FFmpeg Error: {}", line);
                }
            }
        });

        let reader = std::io::BufReader::new(stdout);
        use std::io::BufRead;
        for line in reader.lines().map_while(Result::ok) {
            if let Some(ref mut cb) = progress_callback {
                if let Some(time_str) = line.strip_prefix("out_time=") {
                    let parts: Vec<&str> = time_str.split(':').collect();
                    if parts.len() == 3 {
                        let current_secs = parts[0].parse::<f64>().unwrap_or(0.0) * 3600.0
                            + parts[1].parse::<f64>().unwrap_or(0.0) * 60.0
                            + parts[2].parse::<f64>().unwrap_or(0.0);
                        if total_duration_secs > 0.0
                            && !cb((current_secs / total_duration_secs).min(1.0) as f32)
                        {
                            let _ = child.kill();
                            let _ = std::fs::remove_file(&ffmpeg_temp_output_path);
                            return Err(anyhow::anyhow!("Cancelled"));
                        }
                    }
                }
            }
        }
        if !child.wait()?.success() {
            let _ = std::fs::remove_file(&ffmpeg_temp_output_path);
            return Err(anyhow::anyhow!("ffmpeg extraction failed"));
        }
        std::fs::rename(&ffmpeg_temp_output_path, output_path)?;
        Ok(())
    }

    pub fn compute_fast_path_hash<P: AsRef<Path>>(path: P) -> Result<String> {
        let path = path.as_ref();
        let mut hasher = Hasher::new();
        hasher.update(path.to_string_lossy().as_bytes());
        if let Ok(metadata) = path.metadata() {
            hasher.update(&metadata.len().to_le_bytes());
            if let Ok(modified) = metadata.modified() {
                if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
                    hasher.update(&duration.as_secs().to_le_bytes());
                }
            }
        }
        Ok(hasher.finalize().to_string())
    }

    pub fn ensure_extracted_audio<F>(
        original_path: &Path,
        progress_callback: Option<F>,
    ) -> Result<PathBuf>
    where
        F: FnMut(f32) -> bool,
    {
        let stats = AudioStats::extract(original_path)?;
        if stats.codec.is_natively_supported() {
            return Ok(original_path.to_path_buf());
        }
        let path_hash = Self::compute_fast_path_hash(original_path)?;
        let cache_dir = Self::get_cache_dir()?.join("extract");
        std::fs::create_dir_all(&cache_dir)?;
        let extracted_path = cache_dir.join(&path_hash);
        if !extracted_path.exists() {
            Self::extract_audio_from_video(
                original_path,
                &extracted_path,
                stats.duration_secs,
                progress_callback,
            )?;
        }
        Ok(extracted_path)
    }

    pub fn prepare_mono_audio<F>(
        source_path: &Path,
        hash: &str,
        progress_callback: Option<F>,
    ) -> Result<(PathBuf, AudioData)>
    where
        F: FnMut(f32) -> bool,
    {
        let cache_dir = Self::get_cache_dir()?.join("master");
        std::fs::create_dir_all(&cache_dir)?;
        let mono_path = cache_dir.join(hash);
        if mono_path.exists() {
            let audio = crate::audio::read_audio(&mono_path, None::<fn(f32) -> bool>)?;
            return Ok((mono_path, audio));
        }
        let mut audio = crate::audio::read_audio(source_path, progress_callback)?;
        if audio.channels > 1 {
            let samples_per_channel = audio.samples.len() / audio.channels as usize;
            let mut mono_samples = vec![0.0f32; samples_per_channel];
            for (i, sample) in mono_samples.iter_mut().enumerate().take(samples_per_channel) {
                let mut sum = 0.0;
                for c in 0..audio.channels {
                    sum += audio.samples[i * audio.channels as usize + c as usize];
                }
                *sample = sum / audio.channels as f32;
            }
            audio.samples = mono_samples;
            audio.channels = 1;
        }
        crate::audio::save_audio_atomic(&mono_path, &audio)?;
        Ok((mono_path, audio))
    }
}
