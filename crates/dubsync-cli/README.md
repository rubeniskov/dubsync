# DubSync CLI

This crate provides command-line utilities for the DubSync project, including probing and diagnostic tools.

## Usage

```bash
cargo run -p dubsync-cli -- [SUBCOMMAND]
```

## Available Commands

*   `probe <FILE>`: Extracts and displays audio metadata from a media file.
*   `check-cuda`: Diagnostic tool to verify CUDA environment and GPU availability.
*   `check-session`: Stress test for GPU session handling.
