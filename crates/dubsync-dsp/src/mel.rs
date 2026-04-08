#![allow(clippy::needless_range_loop)]
use rustfft::{Fft, FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MelFeat {
    pub vec: Vec<f32>,
    pub vad: f32,
}

pub struct MelEngine {
    pub fft_size: usize,
    pub num_mels: usize,
    pub window: Vec<f32>,
    pub mel_filters: Vec<Vec<f32>>,
    pub fft_engine: Arc<dyn Fft<f32>>,
}

impl MelEngine {
    pub fn new(sample_rate: f32, fft_size: usize, num_mels: usize) -> Self {
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
            #[allow(clippy::needless_range_loop)]
            for j in bins[i]..bins[i + 1] {
                filters[i][j] = (j - bins[i]) as f32 / (bins[i + 1] - bins[i]) as f32;
            }
            #[allow(clippy::needless_range_loop)]
            for j in bins[i + 1]..bins[i + 2] {
                if j < fft_size / 2 + 1 {
                    filters[i][j] = (bins[i + 2] - j) as f32 / (bins[i + 2] - bins[i + 1]) as f32;
                }
            }
        }
        filters
    }

    pub fn extract(&self, audio: &[f32], hop_size: usize) -> Vec<MelFeat> {
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
                #[allow(clippy::needless_range_loop)]
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
            #[allow(clippy::needless_range_loop)]
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
