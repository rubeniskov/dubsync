use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::File;
use std::path::Path;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelLayout {
    Mono,
    Stereo,
    Surround5_1,
    Surround7_1,
    Other(u16),
}

impl fmt::Display for ChannelLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChannelLayout::Mono => write!(f, "Mono"),
            ChannelLayout::Stereo => write!(f, "Stereo"),
            ChannelLayout::Surround5_1 => write!(f, "5.1"),
            ChannelLayout::Surround7_1 => write!(f, "7.1"),
            ChannelLayout::Other(c) => write!(f, "{}ch", c),
        }
    }
}

impl ChannelLayout {
    pub fn from_channels(channels: u16) -> Self {
        match channels {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            6 => ChannelLayout::Surround5_1,
            8 => ChannelLayout::Surround7_1,
            c => ChannelLayout::Other(c),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioStats {
    pub sample_rate: u32,
    pub channels: ChannelLayout,
    pub bit_depth: Option<u32>,
    pub duration_secs: f64,
    pub codec: Codec,
}

impl AudioStats {
    pub fn extract<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();

        // 1. Always try FFmpeg first as it is more robust for containers (MKV, etc.)
        if let Ok(f) = Self::extract_ffmpeg(path) {
            return Ok(f);
        }

        // 2. Fallback to native Symphonia
        Self::extract_native(path)
    }

    fn extract_native<P: AsRef<Path>>(path: P) -> Result<Self> {
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
            &FormatOptions { enable_gapless: false, ..Default::default() },
            &MetadataOptions {
                limit_metadata_bytes: symphonia::core::meta::Limit::Maximum(1024 * 64),
                limit_visual_bytes: symphonia::core::meta::Limit::Maximum(0),
            },
        )?;

        let format = probed.format;
        let track = format.default_track().context("No default track found")?;
        let params = &track.codec_params;
        let sample_rate = params.sample_rate.unwrap_or(0);
        let channels_raw = params
            .channel_layout
            .map(|cl| cl.into_channels().count() as u16)
            .or_else(|| params.channels.map(|c| c.count() as u16))
            .unwrap_or(0);
        let bit_depth = params.bits_per_sample.filter(|&b| b > 0).map(|b| b as u32);
        let duration_secs = params
            .n_frames
            .and_then(|n| if sample_rate > 0 { Some(n as f64 / sample_rate as f64) } else { None })
            .unwrap_or(0.0);

        Ok(Self {
            sample_rate,
            channels: ChannelLayout::from_channels(channels_raw),
            bit_depth,
            duration_secs,
            codec: Codec::from(params.codec),
        })
    }

    fn extract_ffmpeg<P: AsRef<Path>>(path: P) -> Result<Self> {
        let output = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "quiet",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
                "-select_streams",
                "a:0",
                &path.as_ref().to_string_lossy(),
            ])
            .output()?;
        if !output.status.success() {
            return Err(anyhow::anyhow!("ffprobe failed"));
        }
        let info: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let stream = info["streams"]
            .as_array()
            .and_then(|arr| arr.first())
            .context("No audio stream found by ffprobe")?;
        let sample_rate =
            stream["sample_rate"].as_str().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        let channels_raw = stream["channels"].as_u64().unwrap_or(0) as u16;
        let bit_depth = stream["bits_per_sample"]
            .as_str()
            .and_then(|s| s.parse::<u16>().ok())
            .or_else(|| stream["bits_per_raw_sample"].as_str().and_then(|s| s.parse::<u16>().ok()))
            .filter(|&b| b > 0)
            .map(|b| b as u32);
        let duration_secs =
            info["format"]["duration"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        let codec_name = stream["codec_name"].as_str().unwrap_or("unknown");
        Ok(Self {
            sample_rate,
            channels: ChannelLayout::from_channels(channels_raw),
            bit_depth,
            duration_secs,
            codec: match codec_name {
                "mp3" => Codec::MP3,
                "flac" => Codec::FLAC,
                "opus" => Codec::Opus,
                "vorbis" => Codec::Vorbis,
                "dts" => Codec::DTS,
                _ => Codec::Unknown,
            },
        })
    }

    pub fn format_duration(&self) -> String {
        let secs = self.duration_secs as u64;
        format!("{:02}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
    }
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

pub fn save_audio_atomic(path: &Path, audio: &AudioData) -> Result<()> {
    use hound;
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
    std::fs::rename(part_path, path)?;
    Ok(())
}
