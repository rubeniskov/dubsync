# 🧠 DubSync Model

**Robust model manifest resolution and secure artifact management for the DubSync ecosystem.**

[![Crates.io](https://img.shields.io/crates/v/dubsync-model.svg)](https://crates.io/crates/dubsync-model)
[![License](https://img.shields.io/crates/l/dubsync-model.svg)](../../LICENSE-MIT)

---

## 🎧 Overview

`dubsync-model` is a foundational crate for the DubSync workspace, dedicated to the lifecycle of AI models. It abstracts the complexities of manifest resolution, secure downloading, and persistent caching, ensuring that processing engines like `dubsync-stem` have reliable access to validated model artifacts.

### Key Features

- **Registry Resolution** - Maps human-readable model names to remote manifest URLs.
- **Secure Downloads** - Parallel downloading with progress reporting and HTTP/HTTPS support.
- **Integrity Verification** - Mandatory SHA-256 checksum verification for every downloaded artifact.
- **Smart Caching** - Cross-platform persistent caching following OS standards (e.g., XDG on Linux).
- **Manifest Versioning** - Support for versioned model manifests with flexible input/output descriptions.

---

## 🚀 Usage

### Resolve and Ensure a Model

```rust
use dubsync_model::{ensure_model, ModelHandle};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Downloads and verifies the model if not already cached
    let handle = ensure_model("htdemucs_ort_v1", None)?;

    println!("Model local path: {}", handle.local_path.display());
    println!("Manifest version: {}", handle.manifest.version);

    Ok(())
}
```

### Custom Manifest URL

```rust
let handle = ensure_model(
    "custom-model",
    Some("https://example.com/models/my-manifest.json")
)?;
```

---

## 🏗️ Part of the DubSync Ecosystem

- **`dubsync-model`** (This crate): Secure artifact and manifest management.
- **`dubsync-stem`**: Audio processing engine (STFT/Inference).
- **`dubsync`**: High-fidelity voice synchronization CLI.

---

## 📄 License

Licensed under either of:

- Apache License, Version 2.0 ([../../LICENSE-APACHE](../../LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([../../LICENSE-MIT](../../LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

---

**Built with ❤️ by [rubeniskov](https://github.com/rubeniskov)**
