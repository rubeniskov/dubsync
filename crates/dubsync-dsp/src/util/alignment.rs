#![allow(clippy::needless_range_loop)]
use crate::mel::MelFeat;
use std::collections::HashMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    pub center: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SegmentReport {
    pub start_sec: f32,
    pub end_sec: f32,
    pub vad_agreement: f32,
    pub drift_variance: f32,
    pub wsola_ncc: f32,
    pub confidence: f32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AlignmentReport {
    pub global_confidence: f32,
    pub average_vad_agreement: f32,
    pub segments: Vec<SegmentReport>,
}

impl std::fmt::Display for AlignmentReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\n=== 🎙️ Alignment Quality Report ===")?;
        writeln!(f, "Overall Confidence:  {:.1}%", self.global_confidence * 100.0)?;
        writeln!(f, "VAD Agreement:       {:.1}%", self.average_vad_agreement * 100.0)?;
        writeln!(f, "====================================")
    }
}

pub fn extract_vad_segments(features: &[MelFeat], threshold: f32, min_len: usize) -> Vec<Segment> {
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

pub fn match_segments(
    ref_segs: &[Segment],
    tgt_segs: &[Segment],
    offset_frames: isize,
) -> Vec<(Segment, Segment)> {
    let mut matches = Vec::new();
    let mut t_idx = 0;
    for r_seg in ref_segs {
        let mut best_tgt: Option<(usize, f32)> = None;
        let r_center_shifted = r_seg.center as isize - offset_frames;

        for (i, t_seg) in tgt_segs.iter().enumerate().skip(t_idx) {
            let dist = (t_seg.center as isize - r_center_shifted).abs();
            if dist < 300 {
                let overlap = r_seg.end.min(t_seg.end + offset_frames as usize) as isize
                    - r_seg.start.max(t_seg.start + offset_frames as usize) as isize;
                let score =
                    overlap as f32 / (r_seg.end - r_seg.start).max(t_seg.end - t_seg.start) as f32;
                if score > 0.3 {
                    best_tgt = Some((i, score));
                    break;
                }
            }
            if t_seg.center as isize > r_center_shifted + 500 {
                break;
            }
        }
        if let Some((idx, _)) = best_tgt {
            matches.push((r_seg.clone(), tgt_segs[idx].clone()));
            t_idx = idx + 1;
        }
    }
    matches
}

pub fn local_dtw(
    r_seg: &Segment,
    t_seg: &Segment,
    ref_feat: &[MelFeat],
    tgt_feat: &[MelFeat],
) -> Vec<(usize, usize)> {
    let n = r_seg.end - r_seg.start;
    let m = t_seg.end - t_seg.start;
    let mut cost = vec![vec![0.0f32; m]; n];

    let dist = |i: usize, j: usize| {
        let a = &ref_feat[r_seg.start + i].vec;
        let b = &tgt_feat[t_seg.start + j].vec;
        a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum::<f32>().sqrt()
    };

    cost[0][0] = dist(0, 0);
    for i in 1..n {
        cost[i][0] = cost[i - 1][0] + dist(i, 0);
    }
    for j in 1..m {
        cost[0][j] = cost[0][j - 1] + dist(0, j);
    }

    let w_diag = 1.0;
    let w_flat = 1.5;

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

pub fn evaluate_alignment(
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

pub fn professional_wsola_mel_telemetry(
    ref_len: usize,
    ref_sr: u32,
    ref_channels: u16,
    target_samples: &[f32],
    target_channels: u16,
    path: &[(usize, usize)],
    frame_rate: usize,
) -> anyhow::Result<(Vec<f32>, Vec<f32>)> {
    let sample_rate = ref_sr as usize;
    let target_stride = target_channels as usize;
    let out_len = ref_len / ref_channels as usize;
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
                    || (test_pos as usize + win_size) >= target_samples.len() / target_stride
                {
                    continue;
                }
                let mut corr = 0.0;
                let mut na = 0.0;
                let mut nb = 0.0;
                for k in 0..hop_size {
                    let a = target_samples[(ideal_start as usize + k) * target_stride];
                    let b = target_samples[(test_pos as usize + k) * target_stride];
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
        if max_ncc < 0.3 {
            best_offset = 0;
        }

        smoothed_offset = 0.8 * smoothed_offset + 0.2 * best_offset as f32;
        let final_target_pos =
            (ideal_target_pos + smoothed_offset.round() as isize).max(0) as usize;

        for k in 0..win_size {
            let d_idx = (out_pos + k) * 2;
            let s_idx = (final_target_pos + k) * target_stride;
            if d_idx + 1 < out.len() && s_idx + 1 < target_samples.len() {
                let weight = 0.5
                    * (1.0 - (2.0 * std::f32::consts::PI * k as f32 / (win_size - 1) as f32).cos());
                for c in 0..2 {
                    out[d_idx + c] += target_samples[s_idx + c] * weight;
                }
            }
        }
        last_target_end = final_target_pos as isize + win_size as isize;
        out_pos += hop_size;
    }
    Ok((out, ncc_log))
}
