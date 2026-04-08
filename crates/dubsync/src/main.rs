#![allow(dead_code, clippy::needless_range_loop, unused_imports)]
use anyhow::{Result, anyhow};
use clap::{Parser, ValueEnum};
use dubsync_stem::core::audio::{read_audio, write_audio};
use dubsync_stem::{AudioData, SplitOptions, StreamSplitter};
use rustfft::{Fft, FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tempfile::tempdir;

#[derive(Parser, Debug)]
#[command(author, version, about = "Audio voice synchronization CLI")]
struct Args {
    #[arg(short, long)]
    main: PathBuf,
    #[arg(short, long)]
    target: PathBuf,
    #[arg(short, long)]
    output: PathBuf,
    #[arg(long)]
    keep_stems: Option<PathBuf>,
    #[arg(long, default_value = "progress_checkpoint.json")]
    checkpoint: PathBuf,
    #[arg(short, long)]
    duration: Option<String>,
    #[arg(short, long)]
    step: Option<Step>,
    #[arg(long)]
    manual_offset: Option<f32>,
    #[arg(long)]
    ignore_checkpoint: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug, Serialize, Deserialize)]
enum Step {
    Extract,
    Separate,
    Sync,
    Mix,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct Checkpoint {
    main_processed: bool,
    main_mono_path: Option<String>,
    main_vocals_processed: bool,
    main_cleaned_path: Option<String>,
    target_processed: bool,
    target_mono_path: Option<String>,
    sync_completed: bool,
    main_audio_path: Option<String>,
    main_vocals_path: Option<String>,
    target_vocals_path: Option<String>,
    output_path: Option<String>,
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
        let json = serde_json::to_string_pretty(self)?;
        let mut file = File::create(path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }
}

// --- Mel + VAD Feature Extractor ---

#[derive(Clone, Debug)]
struct MelFeat {
    vec: Vec<f32>,
    vad: f32,
}

struct MelFeatureExtractor {
    fft_size: usize,
    num_mels: usize,
    window: Vec<f32>,
    mel_filters: Vec<Vec<f32>>,
    fft_engine: Arc<dyn Fft<f32>>,
}

impl MelFeatureExtractor {
    fn new(sample_rate: f32, fft_size: usize, num_mels: usize) -> Self {
        let mut planner = FftPlanner::new();
        let fft_engine = planner.plan_fft_forward(fft_size);
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (fft_size - 1) as f32).cos())
            })
            .collect();
        let mel_filters = Self::create_mel_filterbank(sample_rate, fft_size, num_mels);
        Self { fft_size, num_mels, window, mel_filters, fft_engine }
    }

    fn create_mel_filterbank(sample_rate: f32, fft_size: usize, num_mels: usize) -> Vec<Vec<f32>> {
        let max_mel = 2595.0 * (1.0 + (sample_rate / 2.0) / 700.0).log10();
        let mels: Vec<f32> =
            (0..num_mels + 2).map(|i| i as f32 * max_mel / (num_mels + 1) as f32).collect();
        let freqs: Vec<f32> =
            mels.iter().map(|&m| 700.0 * (10.0f32.powf(m / 2595.0) - 1.0)).collect();
        let bins: Vec<usize> =
            freqs.iter().map(|&f| (f * (fft_size + 1) as f32 / sample_rate) as usize).collect();
        let mut filters = vec![vec![0.0f32; fft_size / 2 + 1]; num_mels];
        for i in 0..num_mels {
            for j in bins[i]..bins[i + 1] {
                filters[i][j] = (j - bins[i]) as f32 / (bins[i + 1] - bins[i]) as f32;
            }
            for j in bins[i + 1]..bins[i + 2] {
                if j < fft_size / 2 + 1 {
                    filters[i][j] = (bins[i + 2] - j) as f32 / (bins[i + 2] - bins[i + 1]) as f32;
                }
            }
        }
        filters
    }

    fn extract(&self, audio: &[f32], hop_size: usize) -> Vec<MelFeat> {
        let mut emphasized = vec![0.0f32; audio.len()];
        emphasized[0] = audio[0];
        for i in 1..audio.len() {
            emphasized[i] = audio[i] - 0.97 * audio[i - 1];
        }

        let num_frames = (emphasized.len().saturating_sub(self.fft_size)) / hop_size + 1;
        let mut all_features = Vec::with_capacity(num_frames);
        let mut fft_buffer = vec![Complex::new(0.0, 0.0); self.fft_size];

        for i in 0..num_frames {
            let start = i * hop_size;
            let mut zcr = 0.0;
            for k in 0..self.fft_size {
                let s = emphasized[start + k];
                fft_buffer[k] = Complex::new(s * self.window[k], 0.0);
                if k > 0 && (emphasized[start + k] >= 0.0) != (emphasized[start + k - 1] >= 0.0) {
                    zcr += 1.0;
                }
            }
            zcr /= self.fft_size as f32;

            self.fft_engine.process(&mut fft_buffer);
            let mut mel_frame = vec![0.0f32; self.num_mels];
            let mut total_energy = 0.0f32;
            for m in 0..self.num_mels {
                let mut energy = 0.0f32;
                for k in 0..=self.fft_size / 2 {
                    energy += fft_buffer[k].norm_sqr() * self.mel_filters[m][k];
                }
                total_energy += energy;
                mel_frame[m] = (energy + 1e-6).ln();
            }

            let vad_score = (total_energy.ln() + 10.0).max(0.0) * (1.0 - zcr);
            all_features.push(MelFeat { vec: mel_frame, vad: vad_score });
        }

        let mut max_vad = 0.0001f32;
        for f in &all_features {
            max_vad = max_vad.max(f.vad);
        }
        for m in 0..self.num_mels {
            let mut mean = 0.0;
            for f in &all_features {
                mean += f.vec[m];
            }
            mean /= num_frames as f32;
            let mut std = 0.0;
            for f in &all_features {
                std += (f.vec[m] - mean).powi(2);
            }
            std = (std / num_frames as f32).sqrt().max(1e-6);
            for f in &mut all_features {
                f.vec[m] = (f.vec[m] - mean) / std;
                if m == 0 {
                    f.vad /= max_vad;
                }
            }
        }

        let smooth_win = 15;
        let mut smoothed_vad = vec![0.0f32; all_features.len()];
        for i in 0..all_features.len() {
            let start = i.saturating_sub(smooth_win / 2);
            let end = (i + smooth_win / 2).min(all_features.len() - 1);
            let mut sum = 0.0;
            for k in start..=end {
                sum += all_features[k].vad;
            }
            smoothed_vad[i] = sum / (end - start + 1) as f32;
        }
        for i in 0..all_features.len() {
            all_features[i].vad = smoothed_vad[i];
        }

        all_features
    }
}

// --- Speech Region Matching Engine (Macro Alignment) ---

#[derive(Debug, Clone)]
struct Segment {
    start: usize,
    end: usize,
    center: usize,
}

fn extract_vad_segments(features: &[MelFeat], threshold: f32, min_len: usize) -> Vec<Segment> {
    let mut segs = Vec::new();
    let mut in_speech = false;
    let mut start = 0;
    for i in 0..features.len() {
        if features[i].vad >= threshold && !in_speech {
            start = i;
            in_speech = true;
        } else if features[i].vad < threshold && in_speech {
            if i - start >= min_len {
                segs.push(Segment { start, end: i, center: (start + i) / 2 });
            }
            in_speech = false;
        }
    }
    if in_speech && features.len() - start >= min_len {
        segs.push(Segment { start, end: features.len(), center: (start + features.len()) / 2 });
    }

    let mut merged: Vec<Segment> = Vec::new();
    for seg in segs {
        if let Some(last) = merged.last_mut() {
            if seg.start - last.end < 50 {
                last.end = seg.end;
                last.center = (last.start + last.end) / 2;
                continue;
            }
        }
        merged.push(seg);
    }
    merged
}

fn match_segments(
    ref_segs: &[Segment],
    tgt_segs: &[Segment],
    offset_frames: isize,
) -> Vec<(Segment, Segment)> {
    let mut matches = Vec::new();
    let mut last_t = 0;

    for r in ref_segs {
        let expected_t_center = (r.center as isize + offset_frames).max(0) as usize;
        let mut best_t = None;
        let mut min_dist = 1000; // Allow 10s of drift

        for (j, t) in tgt_segs.iter().enumerate() {
            if j < last_t {
                continue;
            }
            let dist = (t.center as isize - expected_t_center as isize).unsigned_abs();
            if dist < min_dist {
                min_dist = dist;
                best_t = Some((j, t.clone()));
            }
        }

        if let Some((j, t)) = best_t {
            matches.push((r.clone(), t));
            last_t = j + 1;
        }
    }
    matches
}

fn local_dtw(
    r_seg: &Segment,
    t_seg: &Segment,
    ref_feat: &[MelFeat],
    tgt_feat: &[MelFeat],
) -> Vec<(usize, usize)> {
    let n = r_seg.end - r_seg.start;
    let m = t_seg.end - t_seg.start;
    if n == 0 || m == 0 {
        return vec![];
    }

    let mut cost = vec![vec![f32::INFINITY; m]; n];

    let dist = |i: usize, j: usize| {
        let f1 = &ref_feat[r_seg.start + i];
        let f2 = &tgt_feat[t_seg.start + j];
        // Hard Silence Constraints!
        if f1.vad > 0.5 && f2.vad < 0.2 {
            return 10000.0;
        }
        if f1.vad < 0.2 && f2.vad > 0.5 {
            return 10000.0;
        }
        let base =
            f1.vec.iter().zip(f2.vec.iter()).map(|(a, b)| (a - b).powi(2)).sum::<f32>().sqrt();
        base * (0.1 + (f1.vad * f2.vad * 5.0))
    };

    cost[0][0] = dist(0, 0);
    for i in 1..n {
        cost[i][0] = cost[i - 1][0] + dist(i, 0);
    }
    for j in 1..m {
        cost[0][j] = cost[0][j - 1] + dist(0, j);
    }

    let w_diag = 1.0;
    let w_flat = 2.0;

    for i in 1..n {
        for j in 1..m {
            let c_diag = cost[i - 1][j - 1] * w_diag;
            let c_horiz = cost[i][j - 1] * w_flat;
            let c_vert = cost[i - 1][j] * w_flat;
            cost[i][j] = dist(i, j) + c_diag.min(c_horiz).min(c_vert);
        }
    }

    let mut path = Vec::new();
    let (mut i, mut j) = (n - 1, m - 1);
    while i > 0 || j > 0 {
        path.push((r_seg.start + i, t_seg.start + j));
        if i == 0 {
            j -= 1;
        } else if j == 0 {
            i -= 1;
        } else {
            let c_diag = cost[i - 1][j - 1] * w_diag;
            let c_horiz = cost[i][j - 1] * w_flat;
            let c_vert = cost[i - 1][j] * w_flat;
            if c_diag <= c_horiz && c_diag <= c_vert {
                i -= 1;
                j -= 1;
            } else if c_horiz <= c_diag && c_horiz <= c_vert {
                j -= 1;
            } else {
                i -= 1;
            }
        }
    }
    path.push((r_seg.start, t_seg.start));
    path.reverse();
    path
}

// --- Validation Engine ---

#[derive(Debug, Clone)]
struct SegmentReport {
    start_sec: f32,
    end_sec: f32,
    vad_agreement: f32,
    drift_variance: f32,
    wsola_ncc: f32,
    confidence: f32,
}

struct AlignmentReport {
    global_confidence: f32,
    average_vad_agreement: f32,
    segments: Vec<SegmentReport>,
}

impl std::fmt::Display for AlignmentReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\n=== 🎙️ Alignment Quality Report ===")?;
        writeln!(f, "Overall Confidence:  {:.1}%", self.global_confidence * 100.0)?;
        writeln!(f, "VAD Agreement:       {:.1}%", self.average_vad_agreement * 100.0)?;
        writeln!(f, "====================================")
    }
}

fn evaluate_alignment(
    ref_feat: &[MelFeat],
    target_feat: &[MelFeat],
    path: &[(usize, usize)],
    frame_rate: usize,
    ncc_log: &[f32],
) -> AlignmentReport {
    let segment_dur = 10.0;
    let frames_per_seg = (segment_dur * frame_rate as f32) as usize;
    let mut segments = Vec::new();
    let path_map: HashMap<usize, usize> = path.iter().cloned().collect();

    for start in (0..ref_feat.len()).step_by(frames_per_seg) {
        let end = (start + frames_per_seg).min(ref_feat.len());
        let mut vad_sum = 0.0;
        let mut matched = 0;
        let mut shifts = Vec::new();

        for r in start..end {
            if let Some(&t) = path_map.get(&r) {
                if (ref_feat[r].vad > 0.5) == (target_feat[t].vad > 0.5) {
                    vad_sum += 1.0;
                }
                shifts.push((r as isize - t as isize) as f32 / frame_rate as f32);
                matched += 1;
            }
        }
        if matched == 0 {
            continue;
        }

        let mean_shift = shifts.iter().sum::<f32>() / matched as f32;
        let drift_var =
            shifts.iter().map(|s| (s - mean_shift).powi(2)).sum::<f32>() / matched as f32;

        let wsola_idx = (start as f32 / ref_feat.len() as f32 * ncc_log.len() as f32) as usize;
        let ncc = ncc_log.get(wsola_idx).copied().unwrap_or(0.8);

        let mut conf: f32 = 1.0;
        let vad_agr = vad_sum / matched as f32;
        if vad_agr < 0.6 {
            conf -= 0.3;
        }
        if drift_var > 1.5 {
            conf -= 0.4;
        }
        if ncc < 0.5 {
            conf -= 0.2;
        }
        conf = conf.max(0.0);

        segments.push(SegmentReport {
            start_sec: start as f32 / frame_rate as f32,
            end_sec: end as f32 / frame_rate as f32,
            vad_agreement: vad_agr,
            drift_variance: drift_var,
            wsola_ncc: ncc,
            confidence: conf,
        });
    }

    let global_conf = segments.iter().map(|s| s.confidence).sum::<f32>() / segments.len() as f32;
    let avg_vad = segments.iter().map(|s| s.vad_agreement).sum::<f32>() / segments.len() as f32;
    AlignmentReport { global_confidence: global_conf, average_vad_agreement: avg_vad, segments }
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
        let extractor = MelFeatureExtractor::new(main_v.sample_rate as f32, 2048, 40);

        let global_offset = find_global_offset_robust(&main_mono_audio, &target_mono_audio)?;
        let global_offset_frames = global_offset / hop_size as isize;

        println!("     1. Extracting speech features...");
        let ref_feat = extractor.extract(&get_mono_average(&main_v), hop_size);
        let target_feat = extractor.extract(&get_mono_average(&target_v), hop_size);

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
        let (synced_samples, ncc_log) =
            professional_wsola_mel_telemetry(&main_v, &target_v, &global_path, frame_rate)?;

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

fn professional_wsola_mel_telemetry(
    main_ref: &AudioData,
    target_v: &AudioData,
    path: &[(usize, usize)],
    frame_rate: usize,
) -> Result<(Vec<f32>, Vec<f32>)> {
    let sample_rate = main_ref.sample_rate as usize;
    let target_stride = target_v.channels as usize;
    let out_len = main_ref.samples.len() / main_ref.channels as usize;
    let mut out = vec![0.0f32; out_len * 2];
    let win_size = sample_rate / 40;
    let hop_size = win_size / 2;
    let search_range = win_size / 4;
    let mut ncc_log = Vec::new();

    let mut time_map = vec![0.0f32; out_len / hop_size + 2];
    let samples_per_frame = sample_rate / frame_rate;
    let mut last_mapped = 0;
    for &(r_frame, t_frame) in path {
        let out_idx = (r_frame * samples_per_frame) / hop_size;
        if out_idx < time_map.len() {
            time_map[out_idx] = (t_frame * samples_per_frame) as f32;
            for i in last_mapped + 1..out_idx {
                let alpha = (i - last_mapped) as f32 / (out_idx - last_mapped) as f32;
                time_map[i] = time_map[last_mapped] * (1.0 - alpha) + time_map[out_idx] * alpha;
            }
            last_mapped = out_idx;
        }
    }
    for i in last_mapped + 1..time_map.len() {
        time_map[i] = time_map[last_mapped] + (i - last_mapped) as f32 * hop_size as f32;
    }

    let mut out_pos = 0;
    let mut last_target_end = 0isize;
    let mut smoothed_offset = 0.0f32;

    while out_pos + win_size < out_len {
        let map_idx = out_pos / hop_size;
        let ideal_target_pos =
            time_map.get(map_idx).copied().unwrap_or(last_target_end as f32) as isize;
        let mut best_offset = 0isize;
        let mut max_ncc = -1.0f32;

        if out_pos > 0 && last_target_end > 0 {
            let ideal_start = last_target_end - hop_size as isize;
            for delta in -(search_range as isize)..search_range as isize {
                let test_pos = ideal_target_pos + delta;
                if test_pos < 0
                    || (test_pos as usize + win_size) >= target_v.samples.len() / target_stride
                {
                    continue;
                }
                let mut corr = 0.0;
                let mut na = 0.0;
                let mut nb = 0.0;
                for k in 0..hop_size {
                    let a = target_v.samples[(ideal_start as usize + k) * target_stride];
                    let b = target_v.samples[(test_pos as usize + k) * target_stride];
                    corr += a * b;
                    na += a * a;
                    nb += b * b;
                }
                let ncc = corr / (na * nb).sqrt().max(1e-6);
                if ncc > max_ncc {
                    max_ncc = ncc;
                    best_offset = delta;
                }
            }
        }

        ncc_log.push(max_ncc.max(0.0));

        // Reject bad NCC and fallback to ideal offset naturally smoothly
        if max_ncc < 0.3 {
            best_offset = 0;
        }

        // Smooth offsets
        smoothed_offset = 0.8 * smoothed_offset + 0.2 * best_offset as f32;
        let final_target_pos =
            (ideal_target_pos + smoothed_offset.round() as isize).max(0) as usize;

        for k in 0..win_size {
            let d_idx = (out_pos + k) * 2;
            let s_idx = (final_target_pos + k) * target_stride;
            if d_idx + 1 < out.len() && s_idx + 1 < target_v.samples.len() {
                let weight = 0.5
                    * (1.0 - (2.0 * std::f32::consts::PI * k as f32 / (win_size - 1) as f32).cos());
                for c in 0..2 {
                    out[d_idx + c] += target_v.samples[s_idx + c] * weight;
                }
            }
        }
        last_target_end = final_target_pos as isize + win_size as isize;
        out_pos += hop_size;
    }
    Ok((out, ncc_log))
}

fn get_mono_average(audio: &AudioData) -> Vec<f32> {
    let stride = audio.channels as usize;
    let mut mono = vec![0.0f32; audio.samples.len() / stride];
    for (i, chunk) in audio.samples.chunks(stride).enumerate() {
        if i < mono.len() {
            mono[i] = chunk.iter().sum::<f32>() / stride as f32;
        }
    }
    mono
}

fn find_global_offset_robust(ref_audio: &AudioData, target_audio: &AudioData) -> Result<isize> {
    let target_sr = 1000;
    let analysis_secs = 1200;
    let ratio_ref = ref_audio.sample_rate as f32 / target_sr as f32;
    let ratio_target = target_audio.sample_rate as f32 / target_sr as f32;
    let a_env =
        get_binary_profile_single_channel(ref_audio, analysis_secs * target_sr, ratio_ref as usize);
    let b_env = get_binary_profile_single_channel(
        target_audio,
        analysis_secs * target_sr,
        ratio_target as usize,
    );
    let fft_len = (a_env.len() + b_env.len()).next_power_of_two();
    let mut a_comp: Vec<Complex<f32>> = a_env.iter().map(|&x| Complex::new(x, 0.0)).collect();
    a_comp.resize(fft_len, Complex::new(0.0, 0.0));
    let mut b_comp: Vec<Complex<f32>> = b_env.iter().map(|&x| Complex::new(x, 0.0)).collect();
    b_comp.resize(fft_len, Complex::new(0.0, 0.0));
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(fft_len);
    fft.process(&mut a_comp);
    fft.process(&mut b_comp);
    for i in 0..fft_len {
        a_comp[i] *= b_comp[i].conj();
    }
    let ifft = planner.plan_fft_inverse(fft_len);
    ifft.process(&mut a_comp);
    let mut max_val = 0.0;
    let mut best_lag = 0;
    for i in 0..fft_len {
        let mag = a_comp[i].norm();
        if mag > max_val {
            max_val = mag;
            best_lag = i;
        }
    }
    let lag = if best_lag > fft_len / 2 {
        best_lag as isize - fft_len as isize
    } else {
        best_lag as isize
    };
    Ok(lag * (ref_audio.sample_rate as isize / target_sr as isize))
}

fn get_binary_profile_single_channel(audio: &AudioData, len: usize, step: usize) -> Vec<f32> {
    let mut profile = Vec::with_capacity(len);
    let stride = audio.channels as usize;
    for i in 0..len {
        let center = i * step * stride;
        if center >= audio.samples.len() {
            break;
        }
        let mut peak = 0.0f32;
        for c in 0..stride {
            peak = peak.max(audio.samples[center + c].abs());
        }
        profile.push(if peak > 0.01 { 1.0 } else { 0.0 });
    }
    profile
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

fn to_planar_stereo(interleaved: &[f32], channels: u16) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(interleaved.len() / (channels as usize));
    for chunk in interleaved.chunks(channels as usize) {
        if chunk.len() >= 2 {
            out.push([chunk[0], chunk[1]]);
        } else if chunk.len() == 1 {
            out.push([chunk[0], chunk[0]]);
        }
    }
    out
}

fn get_stem_indices(splitter: &StreamSplitter) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for (i, name) in splitter.stems_names().iter().enumerate() {
        map.insert(name.to_lowercase(), i);
    }
    map
}
