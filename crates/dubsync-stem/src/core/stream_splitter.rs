use crate::{core::engine, types::SplitOptions};
use dubsync_model::{error::Result, model_manager::ensure_model};

pub struct StreamSplitter {
    buffer_left: Vec<f32>,
    buffer_right: Vec<f32>,
    window_size: usize,
    hop_size: usize,
    stems_names: Vec<String>,
    stems_count: usize,
    is_first_chunk: bool,
    total_pushed: usize,
    total_returned: usize,
}

impl StreamSplitter {
    pub fn new(opts: SplitOptions) -> Result<Self> {
        let handle = ensure_model(&opts.model_name, opts.manifest_url_override.as_deref())?;
        engine::preload(&handle)?;

        let mf = engine::manifest();
        if mf.sample_rate != 44100 {
            return Err(anyhow::anyhow!("Currently expecting 44.1k model").into());
        }

        let window_size = mf.window;
        let hop_size = mf.hop;

        if !(window_size > 0 && hop_size > 0 && hop_size <= window_size) {
            return Err(anyhow::anyhow!("Bad window/hop in manifest").into());
        }

        Ok(Self {
            buffer_left: Vec::with_capacity(window_size),
            buffer_right: Vec::with_capacity(window_size),
            window_size,
            hop_size,
            stems_names: mf.stems.clone(),
            stems_count: mf.stems.len().max(4), // Default to 4 if not specified
            is_first_chunk: true,
            total_pushed: 0,
            total_returned: 0,
        })
    }

    /// Push new samples into the splitter.
    /// Returns a vector of stems, where each stem is a vector of [L, R] samples.
    /// Each stem vector will have length `hop_size` if a window was processed, or 0 otherwise.
    pub fn push(&mut self, left: &[f32], right: &[f32]) -> Result<Vec<Vec<[f32; 2]>>> {
        assert_eq!(left.len(), right.len());

        self.total_pushed += left.len();
        self.buffer_left.extend_from_slice(left);
        self.buffer_right.extend_from_slice(right);

        if self.buffer_left.len() >= self.window_size {
            // Extract window
            let win_left = &self.buffer_left[..self.window_size];
            let win_right = &self.buffer_right[..self.window_size];

            // Run inference
            let out = engine::run_window_demucs(win_left, win_right)?;
            let (s_count, _, t_out) = (out.shape()[0], out.shape()[1], out.shape()[2]);

            if self.is_first_chunk {
                self.stems_count = s_count;
                self.is_first_chunk = false;
            }

            let mut result = vec![Vec::with_capacity(self.hop_size); self.stems_count];

            // Copy only the first 'hop_size' samples of each stem
            let copy_len = self.hop_size.min(t_out);
            // Limit by remaining samples to be returned
            let copy_len = copy_len.min(self.total_pushed.saturating_sub(self.total_returned));

            for st in 0..self.stems_count {
                for i in 0..copy_len {
                    result[st].push([out[(st, 0, i)], out[(st, 1, i)]]);
                }
            }

            self.total_returned += copy_len;

            // Shift buffer
            self.buffer_left.drain(..self.hop_size);
            self.buffer_right.drain(..self.hop_size);

            Ok(result)
        } else {
            Ok(vec![Vec::new(); self.stems_count])
        }
    }

    /// Flush remaining samples.
    /// Pads the buffer to `window_size` if necessary and processes it.
    pub fn flush(&mut self) -> Result<Vec<Vec<[f32; 2]>>> {
        let mut final_result = vec![Vec::new(); self.stems_count];

        while self.total_returned < self.total_pushed {
            let original_len = self.buffer_left.len();
            let pad_len = self.window_size.saturating_sub(original_len);

            if pad_len > 0 {
                self.buffer_left.extend(std::iter::repeat_n(0.0, pad_len));
                self.buffer_right.extend(std::iter::repeat_n(0.0, pad_len));
            }

            let win_left = &self.buffer_left[..self.window_size];
            let win_right = &self.buffer_right[..self.window_size];

            let out = engine::run_window_demucs(win_left, win_right)?;
            let (s_count, _, t_out) = (out.shape()[0], out.shape()[1], out.shape()[2]);

            if self.is_first_chunk {
                self.stems_count = s_count;
                self.is_first_chunk = false;
            }

            let copy_len = self.hop_size.min(t_out);
            let copy_len = copy_len.min(self.total_pushed.saturating_sub(self.total_returned));

            for st in 0..self.stems_count {
                for i in 0..copy_len {
                    final_result[st].push([out[(st, 0, i)], out[(st, 1, i)]]);
                }
            }

            self.total_returned += copy_len;

            self.buffer_left.drain(..self.hop_size);
            self.buffer_right.drain(..self.hop_size);
        }

        Ok(final_result)
    }

    pub fn stems_names(&self) -> Vec<String> {
        if self.stems_names.is_empty() {
            vec!["vocals".into(), "drums".into(), "bass".into(), "other".into()]
        } else {
            self.stems_names.clone()
        }
    }
}
