#![allow(dead_code, clippy::needless_range_loop, unused_imports)]
use anyhow::{Result, anyhow};
use clap::{Parser, ValueEnum};
use dubsync_dsp::mel::{MelEngine, MelFeat};
use dubsync_dsp::util::alignment::{
    AlignmentReport, Segment, evaluate_alignment, extract_vad_segments, local_dtw, match_segments,
    professional_wsola_mel_telemetry,
};
use dubsync_dsp::util::{find_global_offset_robust, get_mono_average, to_planar_stereo};
use dubsync_stem::core::audio::{read_audio, write_audio};
use dubsync_stem::{AudioData, SplitOptions, StreamSplitter};
use num_complex::Complex;
use rustfft::FftPlanner;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tempfile::tempdir;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Main multi-channel audio source (e.g., DTS/AC3 from movie)
    #[arg(short, long)]
    main: PathBuf,

    /// Target audio source (e.g., Opus/MP3 from different language)
    #[arg(short, long)]
    target: PathBuf,

    /// Final synchronized output path
    #[arg(short, long)]
    output: PathBuf,

    /// Checkpoint file path
    #[arg(short, long, default_value = "progress_checkpoint.json")]
    checkpoint: PathBuf,

    /// Reset checkpoint and start over
    #[arg(long)]
    ignore_checkpoint: bool,

    /// Optional: Only process first N seconds (ffmpeg format, e.g. "00:05:00")
    #[arg(short, long)]
    duration: Option<String>,

    /// Specific step to start from
    #[arg(long, value_enum)]
    step: Option<Step>,

    /// Keep intermediate stem files in this directory
    #[arg(long)]
    keep_stems: Option<PathBuf>,
}

#[derive(ValueEnum, Clone, Debug, PartialEq, PartialOrd)]
enum Step {
    Extract,
    Separate,
    Sync,
    Mix,
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct Checkpoint {
    main_processed: bool,
    main_audio_path: Option<String>,
    main_mono_path: Option<String>,
    main_vocals_path: Option<String>,
    main_vocals_processed: bool,
    main_cleaned_path: Option<String>,

    target_processed: bool,
    target_mono_path: Option<String>,
    target_vocals_path: Option<String>,

    sync_completed: bool,
}

impl Checkpoint {
    fn load(path: &Path) -> Self {
        if let Ok(file) = File::open(path) {
            let reader = BufReader::new(file);
            serde_json::from_reader(reader).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    fn save(&self, path: &Path) -> Result<()> {
        let file = File::create(path)?;
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }
}

// --- Main Pipeline ---

fn main() -> Result<()> {
    let args = Args::parse();
    let mut checkpoint = if args.ignore_checkpoint {
        Checkpoint::default()
    } else {
        Checkpoint::load(&args.checkpoint)
    };
    if let Some(s) = args.step {
        if s <= Step::Extract {
            checkpoint.main_processed = false;
            checkpoint.main_mono_path = None;
        }
        if s <= Step::Separate {
            checkpoint.main_vocals_processed = false;
            checkpoint.target_processed = false;
            checkpoint.main_cleaned_path = None;
            checkpoint.target_mono_path = None;
        }
        if s <= Step::Sync {
            checkpoint.sync_completed = false;
        }
    }
    let tmp_dir = tempdir()?;
    let work_dir = args.keep_stems.clone().unwrap_or_else(|| tmp_dir.path().to_path_buf());
    std::fs::create_dir_all(&work_dir)?;

    if !checkpoint.main_processed || checkpoint.main_mono_path.is_none() {
        println!("[Step 1/4] Extracting main audio...");
        let main_wav = work_dir.join("main_multi.wav");
        let mut ff_args = vec!["-i", args.main.to_str().unwrap()];
        if let Some(ref d) = args.duration {
            ff_args.push("-t");
            ff_args.push(d);
        }
        ff_args.push(main_wav.to_str().unwrap());
        run_ffmpeg(&ff_args)?;
        checkpoint.main_audio_path = Some(main_wav.to_string_lossy().into_owned());
        let main_mono = work_dir.join("main_full_mono.wav");
        run_ffmpeg(&["-i", main_wav.to_str().unwrap(), "-ac", "1", main_mono.to_str().unwrap()])?;
        checkpoint.main_mono_path = Some(main_mono.to_string_lossy().into_owned());
        checkpoint.main_processed = true;
        checkpoint.save(&args.checkpoint)?;
    }

    if !checkpoint.main_vocals_processed
        || !checkpoint.target_processed
        || checkpoint.main_cleaned_path.is_none()
    {
        println!("[Step 2/4] AI Separation & Stem Preparation...");
        let main_wav_path = checkpoint.main_audio_path.as_ref().expect("Main audio missing");
        if checkpoint.target_mono_path.is_none() {
            let target_mono = work_dir.join("target_full_mono.wav");
            let mut ff_args = vec!["-i", args.target.to_str().unwrap()];
            if let Some(ref d) = args.duration {
                ff_args.push("-t");
                ff_args.push(d);
            }
            ff_args.extend(vec!["-ac", "1", target_mono.to_str().unwrap()]);
            run_ffmpeg(&ff_args)?;
            checkpoint.target_mono_path = Some(target_mono.to_string_lossy().into_owned());
            checkpoint.save(&args.checkpoint)?;
        }
        if !checkpoint.main_vocals_processed {
            let center_wav = work_dir.join("main_center_temp.wav");
            run_ffmpeg(&[
                "-i",
                main_wav_path,
                "-filter_complex",
                "[0:a]pan=stereo|c0=c2|c1=c2",
                center_wav.to_str().unwrap(),
            ])?;
            let vocals_path = work_dir.join("main_vocals.wav");
            separate_track(&center_wav, &vocals_path, "Ref Vocals", true, None)?;
            checkpoint.main_vocals_path = Some(vocals_path.to_string_lossy().into_owned());
            checkpoint.main_vocals_processed = true;
            std::fs::remove_file(center_wav).ok();
            checkpoint.save(&args.checkpoint)?;
        }
        if !checkpoint.target_processed {
            let target_temp_wav = work_dir.join("target_temp.wav");
            let mut ff_args = vec!["-i", args.target.to_str().unwrap()];
            if let Some(ref d) = args.duration {
                ff_args.push("-t");
                ff_args.push(d);
            }
            ff_args.push(target_temp_wav.to_str().unwrap());
            run_ffmpeg(&ff_args)?;
            let vocals_path = work_dir.join("target_vocals.wav");
            separate_track(&target_temp_wav, &vocals_path, "Target Vocals", true, None)?;
            checkpoint.target_vocals_path = Some(vocals_path.to_string_lossy().into_owned());
            checkpoint.target_processed = true;
            std::fs::remove_file(target_temp_wav).ok();
            checkpoint.save(&args.checkpoint)?;
        }
        if checkpoint.main_cleaned_path.is_none() {
            let cleaned_wav = work_dir.join("main_cleaned_background.wav");
            clean_english_track(Path::new(main_wav_path), &cleaned_wav, &work_dir, false)?;
            checkpoint.main_cleaned_path = Some(cleaned_wav.to_string_lossy().into_owned());
            checkpoint.save(&args.checkpoint)?;
        }
    }

    if !checkpoint.sync_completed {
        println!("[Step 3/4] Two-Stage Macro/Micro Alignment Engine...");
        let main_v = read_audio(checkpoint.main_vocals_path.as_ref().unwrap())?;
        let target_v = read_audio(checkpoint.target_vocals_path.as_ref().unwrap())?;

        let main_mono_audio = read_audio(checkpoint.main_mono_path.as_ref().unwrap())?;
        let target_mono_audio = read_audio(checkpoint.target_mono_path.as_ref().unwrap())?;

        let frame_rate = 100;
        let hop_size = main_v.sample_rate as usize / frame_rate;
        let extractor = MelEngine::new(main_v.sample_rate as f32, 2048, 40);

        let global_offset = find_global_offset_robust(
            &main_mono_audio.samples,
            main_mono_audio.sample_rate,
            main_mono_audio.channels,
            &target_mono_audio.samples,
            target_mono_audio.sample_rate,
            target_mono_audio.channels,
        )?;
        let global_offset_frames = global_offset / hop_size as isize;

        println!("     1. Extracting speech features...");
        let ref_feat =
            extractor.extract(&get_mono_average(&main_v.samples, main_v.channels), hop_size);
        let target_feat =
            extractor.extract(&get_mono_average(&target_v.samples, target_v.channels), hop_size);

        println!("     2. Segmenting Speech Regions (Macro-Alignment)...");
        let ref_segs = extract_vad_segments(&ref_feat, 0.3, 10);
        let tgt_segs = extract_vad_segments(&target_feat, 0.3, 10);

        println!("     3. Matching Speech Segments...");
        let matched_segs = match_segments(&ref_segs, &tgt_segs, global_offset_frames);
        println!("        Matched {}/{} reference segments.", matched_segs.len(), ref_segs.len());

        println!("     4. Solving Local DTW for matched segments (Hard Constraints)...");
        let mut global_path = Vec::new();
        let mut current_r = 0;
        let mut current_t_offset = -global_offset_frames;

        for (r_seg, t_seg) in matched_segs {
            for r in current_r..r_seg.start {
                let t = (r as isize + current_t_offset).max(0) as usize;
                global_path.push((r, t.min(target_feat.len().saturating_sub(1))));
            }

            let local = local_dtw(&r_seg, &t_seg, &ref_feat, &target_feat);
            global_path.extend(local);

            current_r = r_seg.end;
            current_t_offset = t_seg.end as isize - r_seg.end as isize;
        }

        for r in current_r..ref_feat.len() {
            let t = (r as isize + current_t_offset).max(0) as usize;
            global_path.push((r, t.min(target_feat.len().saturating_sub(1))));
        }

        println!("     5. Synthesizing Final High-Fidelity Audio with Smooth WSOLA...");
        let (synced_samples, ncc_log) = professional_wsola_mel_telemetry(
            main_v.samples.len(),
            main_v.sample_rate,
            main_v.channels,
            &target_v.samples,
            target_v.channels,
            &global_path,
            frame_rate,
        )?;

        let report =
            evaluate_alignment(&ref_feat, &target_feat, &global_path, frame_rate, &ncc_log);
        println!("{}", report);

        write_audio(
            work_dir.join("target_vocals_synced.wav").to_str().unwrap(),
            &AudioData { samples: synced_samples, sample_rate: main_v.sample_rate, channels: 2 },
        )?;
        checkpoint.sync_completed = true;
        checkpoint.save(&args.checkpoint)?;
    }

    println!("[Step 4/4] Final Recomposition...");
    run_ffmpeg(&[
        "-i",
        checkpoint.main_cleaned_path.as_ref().unwrap(),
        "-i",
        work_dir.join("target_vocals_synced.wav").to_str().unwrap(),
        "-filter_complex",
        "[0:a][1:a]amerge=inputs=2,pan=7.1|c0=c0|c1=c1|c2=c8|c3=c3|c4=c4|c5=c5|c6=c6|c7=c7",
        "-c:a",
        "flac",
        args.output.to_str().unwrap(),
    ])?;
    println!("Success! High-Fidelity Output at: {:?}", args.output);
    Ok(())
}

fn run_ffmpeg(args: &[&str]) -> Result<()> {
    let status = Command::new("ffmpeg").args(args).arg("-y").status()?;
    if !status.success() {
        return Err(anyhow!("ffmpeg failed"));
    }
    Ok(())
}

fn clean_english_track(
    input: &Path,
    output: &Path,
    work_dir: &Path,
    _keep_all_stems: bool,
) -> Result<()> {
    let audio = read_audio(input.to_str().unwrap())?;
    let channels = audio.channels as usize;
    let mut cleaned_channels: Vec<Vec<f32>> =
        vec![vec![0.0; audio.samples.len() / channels]; channels];
    for i in (0..channels).step_by(2) {
        if i + 1 >= channels {
            let mono_path = work_dir.join(format!("temp_pair_{}.wav", i));
            run_ffmpeg(&[
                "-i",
                input.to_str().unwrap(),
                "-filter_complex",
                &format!("[0:a]pan=stereo|c0=c{}|c1=c{}", i, i),
                mono_path.to_str().unwrap(),
            ])?;
            let cleaned_path = work_dir.join(format!("temp_pair_{}_cleaned.wav", i));
            separate_track(&mono_path, &cleaned_path, &format!("Cleaning Ch {}", i), false, None)?;
            let cleaned = read_audio(cleaned_path.to_str().unwrap())?;
            for j in 0..cleaned_channels[i].len() {
                cleaned_channels[i][j] = cleaned.samples[j * 2];
            }
            std::fs::remove_file(mono_path).ok();
            std::fs::remove_file(cleaned_path).ok();
            continue;
        }
        let pair_path = work_dir.join(format!("temp_pair_{}_{}.wav", i, i + 1));
        run_ffmpeg(&[
            "-i",
            input.to_str().unwrap(),
            "-filter_complex",
            &format!("[0:a]pan=stereo|c0=c{}|c1=c{}", i, i + 1),
            pair_path.to_str().unwrap(),
        ])?;
        let cleaned_path = work_dir.join(format!("temp_pair_{}_{}_cleaned.wav", i, i + 1));
        separate_track(
            &pair_path,
            &cleaned_path,
            &format!("Cleaning Ch {}-{}", i, i + 1),
            false,
            None,
        )?;
        let cleaned = read_audio(cleaned_path.to_str().unwrap())?;
        for j in 0..cleaned_channels[i].len() {
            cleaned_channels[i][j] = cleaned.samples[j * 2] * 0.95;
            cleaned_channels[i + 1][j] = cleaned.samples[j * 2 + 1] * 0.95;
        }
        std::fs::remove_file(pair_path).ok();
        std::fs::remove_file(cleaned_path).ok();
    }
    let mut interleaved = Vec::with_capacity(audio.samples.len());
    for i in 0..(audio.samples.len() / channels) {
        for c in 0..channels {
            interleaved.push(cleaned_channels[c][i]);
        }
    }
    write_audio(
        output.to_str().unwrap(),
        &AudioData {
            samples: interleaved,
            sample_rate: audio.sample_rate,
            channels: audio.channels,
        },
    )?;
    Ok(())
}

fn separate_track(
    input_path: &Path,
    output_path: &Path,
    label: &str,
    keep_vocals: bool,
    save_stems_to: Option<&Path>,
) -> Result<()> {
    let mut splitter = StreamSplitter::new(SplitOptions::default())?;
    let audio = read_audio(input_path.to_str().unwrap())?;
    let stereo = to_planar_stereo(&audio.samples, audio.channels);
    let left: Vec<f32> = stereo.iter().map(|s| s[0]).collect();
    let right: Vec<f32> = stereo.iter().map(|s| s[1]).collect();
    let stems = split_streaming(&mut splitter, &left, &right, label)?;
    let indices = get_stem_indices(&splitter);
    if let Some(dir) = save_stems_to {
        std::fs::create_dir_all(dir)?;
        for (name, &idx) in &indices {
            let mut mono = Vec::with_capacity(stems[idx].len());
            for s in &stems[idx] {
                mono.push((s[0] + s[1]) / 2.0);
            }
            write_audio(
                dir.join(format!("{}.wav", name)).to_str().unwrap(),
                &AudioData { samples: mono, sample_rate: audio.sample_rate, channels: 1 },
            )?;
        }
    }
    let mut final_samples = Vec::with_capacity(left.len() * 2);
    if keep_vocals {
        let v_idx = *indices.get("vocals").unwrap_or(&0);
        for s in &stems[v_idx] {
            final_samples.push(s[0]);
            final_samples.push(s[1]);
        }
    } else {
        let d_idx = *indices.get("drums").unwrap_or(&1);
        let b_idx = *indices.get("bass").unwrap_or(&2);
        let o_idx = *indices.get("other").unwrap_or(&3);
        for i in 0..stems[d_idx].len() {
            let l = stems[d_idx][i][0] + stems[b_idx][i][0] + stems[o_idx][i][0];
            let r = stems[d_idx][i][1] + stems[b_idx][i][1] + stems[o_idx][i][1];
            final_samples.push(l);
            final_samples.push(r);
        }
    }
    write_audio(
        output_path.to_str().unwrap(),
        &AudioData { samples: final_samples, sample_rate: audio.sample_rate, channels: 2 },
    )?;
    Ok(())
}

fn split_streaming(
    splitter: &mut StreamSplitter,
    left: &[f32],
    right: &[f32],
    label: &str,
) -> Result<Vec<Vec<[f32; 2]>>> {
    let mut all_stems: Vec<Vec<[f32; 2]>> = vec![];
    let chunk_size = 16384;
    let mut pos = 0;
    while pos < left.len() {
        let end = (pos + chunk_size).min(left.len());
        let stems = splitter.push(&left[pos..end], &right[pos..end])?;
        if all_stems.is_empty() && !stems.is_empty() {
            all_stems = vec![vec![]; stems.len()];
        }
        for (i, stem_chunk) in stems.into_iter().enumerate() {
            if i < all_stems.len() {
                all_stems[i].extend(stem_chunk);
            }
        }
        pos = end;
        if pos % (chunk_size * 100) == 0 {
            println!("{}: {:.1}%", label, (pos as f32 / left.len() as f32) * 100.0);
        }
    }
    let final_stems = splitter.flush()?;
    for (i, stem_chunk) in final_stems.into_iter().enumerate() {
        if i < all_stems.len() {
            all_stems[i].extend(stem_chunk);
        }
    }
    Ok(all_stems)
}

fn get_stem_indices(splitter: &StreamSplitter) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for (i, name) in splitter.stems_names().iter().enumerate() {
        map.insert(name.to_lowercase(), i);
    }
    map
}
