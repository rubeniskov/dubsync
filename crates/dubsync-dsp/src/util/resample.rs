/// A simple linear resampler
pub fn resample_linear(input: &[f32], from_sr: u32, target_sr: u32) -> Vec<f32> {
    if from_sr == target_sr {
        return input.to_vec();
    }

    let ratio = target_sr as f64 / from_sr as f64;
    let out_len = (input.len() as f64 * ratio).floor() as usize;
    let mut output = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let pos = i as f64 / ratio;
        let idx = pos.floor() as usize;
        let frac = pos - idx as f64;

        if idx + 1 < input.len() {
            let s1 = input[idx];
            let s2 = input[idx + 1];
            output.push((s1 as f64 * (1.0 - frac) + s2 as f64 * frac) as f32);
        } else if idx < input.len() {
            output.push(input[idx]);
        }
    }

    output
}
