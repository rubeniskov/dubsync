use anyhow::{Context, Result};
use blake3::Hasher;
use directories::BaseDirs;
use memmap2::Mmap;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::{get_codecs, get_probe};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioData {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

pub fn read_audio<P: AsRef<Path>, F>(path: P, mut progress_callback: Option<F>) -> Result<AudioData>
where
    F: FnMut(f32) -> bool,
{
    let path: &Path = path.as_ref();
    let file =
        File::open(path).with_context(|| format!("Failed to open audio file: {:?}", path))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed =
        get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())?;

    let mut format = probed.format;
    let track = format.default_track().context("No default track found")?;
    let track_id = track.id;
    let total_frames = track.codec_params.n_frames;

    let mut decoder = get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    let mut samples: Vec<f32> = Vec::new();
    let mut sample_rate: u32 = 0;
    let mut channels: u16 = 0;
    let mut decoded_frames = 0u64;

    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = decoder.decode(&packet)?;
        sample_rate = decoded.spec().rate;
        channels = decoded.spec().channels.count() as u16;
        decoded_frames += decoded.frames() as u64;

        if let (Some(total), Some(ref mut cb)) = (total_frames, progress_callback.as_mut()) {
            if !cb(decoded_frames as f32 / total as f32) {
                return Err(anyhow::anyhow!("Cancelled"));
            }
        }

        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buffer.copy_interleaved_ref(decoded);

        samples.extend_from_slice(buffer.samples());
    }

    Ok(AudioData { samples, sample_rate, channels })
}

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
        let metadata = file.metadata()?;
        let total_size = metadata.len();

        if total_size == 0 {
            return Ok(Hasher::new().finalize().to_string());
        }

        // Optimization: For large files on potentially slow I/O (like network drives),
        // mmap is generally good, but we'll use a large chunk size to maximize throughput.
        let mmap = unsafe { Mmap::map(&file)? };
        let mut hasher = Hasher::new();

        // 256MB chunks for high-throughput parallel hashing with progress
        let chunk_size = 1024 * 1024 * 256;

        #[cfg(debug_assertions)]
        {
            if total_size > 1024 * 1024 * 500 {
                eprintln!(
                    "Warning: Hashing a large file in DEBUG mode will be very slow. Use --release for production speed."
                );
            }
        }

        for (i, chunk) in mmap.chunks(chunk_size).enumerate() {
            hasher.update_rayon(chunk);
            if let Some(ref mut cb) = progress_callback {
                let progress =
                    ((i + 1) as f64 * chunk_size as f64 / total_size as f64).min(1.0) as f32;
                if !cb(progress) {
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
        let filename = format!("{}_{}", hash, suffix);
        Ok(cache_dir.join(filename))
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
            return Err(anyhow::anyhow!(
                "Processing video tracks without ffmpeg is not possible. Please install ffmpeg."
            ));
        }

        let ffmpeg_temp_output_path = output_path.with_extension("process"); // e.g., hash_extracted.process

        let mut child = std::process::Command::new("ffmpeg")
            .arg("-i")
            .arg(input_path)
            .arg("-map")
            .arg("0:a:0?")
            .arg("-c:a")
            .arg("flac")
            .arg("-f") // Explicitly tell ffmpeg we want FLAC output format
            .arg("flac")
            .arg("-compression_level")
            .arg("0")
            .arg("-y") // Overwrite if exists
            .arg(&ffmpeg_temp_output_path) // FFmpeg writes to .process file
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let stderr = child.stderr.take().context("Failed to capture ffmpeg stderr")?;
        let reader = std::io::BufReader::new(stderr);
        let mut last_error_line = String::new();

        use std::io::BufRead;
        for line in reader.lines().map_while(Result::ok) {
            if line.to_lowercase().contains("error") || line.starts_with("Unable to") {
                last_error_line = line.clone();
            }

            if let Some(ref mut cb) = progress_callback {
                if let Some(time_idx) = line.find("time=") {
                    let time_str = &line[time_idx + 5..];
                    if let Some(space_idx) = time_str.find(' ') {
                        let time_val = &time_str[..space_idx];
                        let parts: Vec<&str> = time_val.split(':').collect();
                        if parts.len() == 3 {
                            let h: f64 = parts[0].parse().unwrap_or(0.0);
                            let m: f64 = parts[1].parse().unwrap_or(0.0);
                            let s: f64 = parts[2].parse().unwrap_or(0.0);
                            let current_secs = h * 3600.0 + m * 60.0 + s;
                            if total_duration_secs > 0.0 {
                                let progress = (current_secs / total_duration_secs).min(1.0) as f32;
                                if !cb(progress) {
                                    let _ = child.kill();
                                    let _ = std::fs::remove_file(&ffmpeg_temp_output_path);
                                    return Err(anyhow::anyhow!("Cancelled"));
                                }
                            }
                        }
                    }
                }
            }
        }

        let status = child.wait()?;

        if !status.success() {
            let _ = std::fs::remove_file(&ffmpeg_temp_output_path); // Clean up temp file on failure
            let error_msg = if last_error_line.is_empty() {
                "Unknown ffmpeg error".to_string()
            } else {
                last_error_line
            };
            return Err(anyhow::anyhow!("ffmpeg extraction failed: {}", error_msg));
        }

        // Atomic rename of the temporary .process file to its final .flac destination
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
            let audio = read_audio(&mono_path, None::<fn(f32) -> bool>)?;
            return Ok((mono_path, audio));
        }

        // Decode from the appropriate source and convert to mono
        let mut audio = read_audio(source_path, progress_callback)?;
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

        // Save to cache atomically using a temporary file
        Self::save_audio_atomic(&mono_path, &audio)?;

        Ok((mono_path, audio))
    }

    pub fn save_audio_atomic(path: &Path, audio: &AudioData) -> Result<()> {
        let part_path = path.with_extension("process");

        let spec = hound::WavSpec {
            channels: audio.channels,
            sample_rate: audio.sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };

        let mut writer = hound::WavWriter::create(&part_path, spec)?;
        for &sample in &audio.samples {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;

        // Atomic rename
        std::fs::rename(part_path, path)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub version: String,
    pub reference_path: Option<PathBuf>,
    pub target_path: Option<PathBuf>,
    pub alignment_report: Option<dubsync_dsp::util::alignment::AlignmentReport>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            reference_path: None,
            target_path: None,
            alignment_report: None,
        }
    }
}

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    MP3,
    AAC,
    FLAC,
    Vorbis,
    Opus,
    ALAC,
    DTS,
    PcmS16LE,
    PcmS16BE,
    PcmS24LE,
    PcmS24BE,
    PcmS32LE,
    PcmS32BE,
    PcmF32LE,
    PcmF32BE,
    Unknown,
}

impl Codec {
    pub fn all_extensions() -> &'static [&'static str] {
        &["wav", "mp3", "flac", "m4a", "ogg", "opus", "dts", "mkv", "mp4", "webm"]
    }

    pub fn is_natively_supported(&self) -> bool {
        matches!(
            self,
            Codec::MP3
                | Codec::FLAC
                | Codec::Vorbis
                | Codec::PcmS16LE
                | Codec::PcmS16BE
                | Codec::PcmS24LE
                | Codec::PcmS24BE
                | Codec::PcmS32LE
                | Codec::PcmS32BE
                | Codec::PcmF32LE
                | Codec::PcmF32BE
        )
    }
}

impl fmt::Display for Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Codec::MP3 => "MP3",
            Codec::AAC => "AAC",
            Codec::FLAC => "FLAC",
            Codec::Vorbis => "Vorbis",
            Codec::Opus => "Opus",
            Codec::ALAC => "ALAC",
            Codec::DTS => "DTS",
            Codec::PcmS16LE => "PCM S16LE",
            Codec::PcmS16BE => "PCM S16BE",
            Codec::PcmS24LE => "PCM S24LE",
            Codec::PcmS24BE => "PCM S24BE",
            Codec::PcmS32LE => "PCM S32LE",
            Codec::PcmS32BE => "PCM S32BE",
            Codec::PcmF32LE => "PCM F32LE",
            Codec::PcmF32BE => "PCM F32BE",
            Codec::Unknown => "Unknown",
        };
        write!(f, "{}", s)
    }
}

impl From<symphonia::core::codecs::CodecType> for Codec {
    fn from(ct: symphonia::core::codecs::CodecType) -> Self {
        use symphonia::core::codecs::*;
        match ct {
            CODEC_TYPE_MP3 => Codec::MP3,
            CODEC_TYPE_AAC => Codec::AAC,
            CODEC_TYPE_FLAC => Codec::FLAC,
            CODEC_TYPE_VORBIS => Codec::Vorbis,
            CODEC_TYPE_OPUS => Codec::Opus,
            CODEC_TYPE_ALAC => Codec::ALAC,
            CODEC_TYPE_DCA => Codec::DTS,
            CODEC_TYPE_PCM_S16LE => Codec::PcmS16LE,
            CODEC_TYPE_PCM_S16BE => Codec::PcmS16BE,
            CODEC_TYPE_PCM_S24LE => Codec::PcmS24LE,
            CODEC_TYPE_PCM_S24BE => Codec::PcmS24BE,
            CODEC_TYPE_PCM_S32LE => Codec::PcmS32LE,
            CODEC_TYPE_PCM_S32BE => Codec::PcmS32BE,
            CODEC_TYPE_PCM_F32LE => Codec::PcmF32LE,
            CODEC_TYPE_PCM_F32BE => Codec::PcmF32BE,
            _ => Codec::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioStats {
    pub sample_rate: u32,
    pub channels: u16,
    pub bit_depth: u32,
    pub duration_secs: f64,
    pub codec: Codec,
}

impl AudioStats {
    pub fn extract<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let file =
            File::open(path).with_context(|| format!("Failed to open audio file: {:?}", path))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let probed = get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;

        let format = probed.format;
        let track = format.default_track().context("No default track found")?;
        let params = &track.codec_params;

        let sample_rate = params.sample_rate.unwrap_or(0);
        let channels = params.channels.map(|c| c.count() as u16).unwrap_or(0);
        let bit_depth = params.bits_per_sample.unwrap_or(0);

        let duration_secs = if let Some(n_frames) = params.n_frames {
            if sample_rate > 0 { n_frames as f64 / sample_rate as f64 } else { 0.0 }
        } else {
            0.0
        };

        let codec = Codec::from(params.codec);

        Ok(Self { sample_rate, channels, bit_depth, duration_secs, codec })
    }

    pub fn format_duration(&self) -> String {
        let secs = self.duration_secs as u64;
        let mins = secs / 60;
        let hours = mins / 60;
        format!("{:02}:{:02}:{:02}", hours, mins % 60, secs % 60)
    }
}
