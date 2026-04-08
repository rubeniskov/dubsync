# 🎵 DubSync Stem

**High-performance, pure-Rust audio stem separation library powered by ONNX Runtime**

[![Crates.io](https://img.shields.io/crates/v/dubsync-stem.svg)](https://crates.io/crates/dubsync-stem)
[![License](https://img.shields.io/crates/l/dubsync-stem.svg)](../../LICENSE-MIT)

---

## 🎧 Overview

`dubsync-stem` is the core processing engine of the DubSync workspace, designed for splitting audio tracks into isolated stems (vocals, drums, bass, and other instruments) using state-of-the-art AI models.

Part of the [DubSync](https://github.com/rubeniskov/dubsync) ecosystem, it leverages `dubsync-model` for robust model management and provides a clean, high-level API for audio source separation.

### Key Features

- **Pure Rust** - No Python, no heavy dependencies, just high-performance systems code.
- **State-of-the-Art Quality** - Powered by the Hybrid Transformer Demucs (htdemucs) model.
- **Hardware Accelerated** - Built-in support for CUDA, CoreML, DirectML, and oneDNN via ONNX Runtime.
- **Streaming Ready** - Includes a `StreamSplitter` for real-time or chunked processing.
- **Smart Model Management** - Automatic resolution, downloading, and caching via `dubsync-model`.

---

## 🏗️ Architecture

The DubSync workspace is divided into specialized crates:

1.  **`dubsync-model`**: Handles model manifests, the registry, and secure artifact downloading/caching with SHA-256 verification.
2.  **`dubsync-stem`** (This crate): The core engine that performs STFT/iSTFT and orchestrates ONNX inference.
3.  **`dubsync`**: The top-level CLI application for voice synchronization and track replacement.

---

## 🚀 Quick Start

### Basic File Separation

```rust
use dubsync_stem::{split_file, SplitOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure with defaults (htdemucs_ort_v1)
    let options = SplitOptions {
        output_dir: "./stems".to_string(),
        ..Default::default()
    };

    // Split a file
    let result = split_file("input.mp3", options)?;

    println!("Vocals saved to: {}", result.vocals_path);
    Ok(())
}
```

### Streaming / Chunked Processing

For real-time applications or large files, use the `StreamSplitter`:

```rust
use dubsync_stem::{StreamSplitter, SplitOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut splitter = StreamSplitter::new(SplitOptions::default())?;

    // In a loop (e.g., reading from stdin or a network stream)
    let left_channel: Vec<f32> = /* ... */;
    let right_channel: Vec<f32> = /* ... */;

    let stems = splitter.push(&left_channel, &right_channel)?;

    // Process output stems [Vocals, Drums, Bass, Other]
    if !stems[0].is_empty() {
        let vocals = &stems[0]; // Vec<[f32; 2]>
    }

    // Don't forget to flush the remaining samples at the end
    let final_stems = splitter.flush()?;

    Ok(())
}
```

---

## ⚙️ Configuration & Hardware Acceleration

`dubsync-stem` automatically detects and utilizes the best available hardware. You can control this via Cargo features:

- `cuda`: NVIDIA GPU support (Linux/Windows).
- `coreml`: Apple Silicon acceleration (macOS).
- `directml`: Windows GPU acceleration.
- `onednn`: Intel CPU optimizations.

```toml
[dependencies]
dubsync-stem = { version = "0.1.0", features = ["cuda"] }
```

---

## 🧪 Development

### Running the Internal CLI

The crate includes a diagnostic CLI tool:

```bash
# Split a file directly using the core engine
cargo run --release --bin dubsync-stem -- split -i track.mp3 -o ./output

# Run in streaming mode (reading raw f32le from stdin)
ffmpeg -i input.mp3 -f f32le -ac 2 -ar 44100 - | \
  cargo run --release --bin dubsync-stem -- stream --stems vocals | \
  ffplay -f f32le -ac 2 -ar 44100 -
```

### Testing

```bash
# Run all tests (including DSP and Audio roundtrips)
cargo test

# Run with mock engine (no ONNX required)
cargo test --features engine-mock
```

---

## 📄 License

Licensed under either of:

- Apache License, Version 2.0 ([../../LICENSE-APACHE](../../LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([../../LICENSE-MIT](../../LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

---

## 🙏 Acknowledgments

- **Meta Research** - The brilliant minds behind the [Demucs](https://github.com/facebookresearch/demucs) architecture.
- **ONNX Runtime** - Providing the cross-platform inference muscle.
- **Symphonia** - Exceptional pure-Rust audio decoding.

---

**Built with ❤️ by [rubeniskov](https://github.com/rubeniskov)**
