#![allow(clippy::needless_range_loop)]
pub mod alignment;
pub mod resample;

use num_complex::Complex;
use rustfft::FftPlanner;

pub fn get_mono_average(samples: &[f32], channels: u16) -> Vec<f32> {
    let stride = channels as usize;
    let mut mono = vec![0.0f32; samples.len() / stride];
    for (i, chunk) in samples.chunks(stride).enumerate() {
        if i < mono.len() {
            mono[i] = chunk.iter().sum::<f32>() / stride as f32;
        }
    }
    mono
}

pub fn find_global_offset_robust(
    ref_samples: &[f32],
    ref_sr: u32,
    ref_channels: u16,
    target_samples: &[f32],
    target_sr: u32,
    target_channels: u16,
) -> anyhow::Result<isize> {
    let target_analysis_sr = 1000;
    let analysis_secs = 1200;
    let ratio_ref = ref_sr as f32 / target_analysis_sr as f32;
    let ratio_target = target_sr as f32 / target_analysis_sr as f32;

    let a_env = get_binary_profile_single_channel(
        ref_samples,
        ref_channels,
        analysis_secs * target_analysis_sr,
        ratio_ref as usize,
    );
    let b_env = get_binary_profile_single_channel(
        target_samples,
        target_channels,
        analysis_secs * target_analysis_sr,
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

    Ok(lag * (ref_sr as isize / target_analysis_sr as isize))
}

pub fn get_binary_profile_single_channel(
    samples: &[f32],
    channels: u16,
    len: usize,
    step: usize,
) -> Vec<f32> {
    let mut profile = Vec::with_capacity(len);
    let stride = channels as usize;
    for i in 0..len {
        let center = i * step * stride;
        if center >= samples.len() {
            break;
        }
        let mut peak = 0.0f32;
        for c in 0..stride {
            peak = peak.max(samples[center + c].abs());
        }
        profile.push(if peak > 0.01 { 1.0 } else { 0.0 });
    }
    profile
}

pub fn to_planar_stereo(interleaved: &[f32], channels: u16) -> Vec<[f32; 2]> {
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
