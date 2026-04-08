#![cfg(feature = "engine-mock")]

use httpmock::prelude::*;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

use dubsync_stem::{SplitOptions, StreamSplitter};

// Compute hex sha256 for arbitrary bytes
fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

fn manifest_json(model_url: &str, sha_hex: &str) -> String {
    format!(
        r#"{{
  "name": "mdx_mock",
  "version": "1.0.0",
  "backend": "onnx",
  "sample_rate": 44100,
  "window": 4096,
  "hop": 2048,
  "stems": ["vocals","drums","bass","other"],
  "input_layout": "BCT",
  "output_layout": "BSCT",
  "artifacts": [
    {{
      "file": "mock.onnx",
      "url": "{url}",
      "sha256": "{sha}",
      "size_bytes": 0
    }}
  ]
}}"#,
        url = model_url,
        sha = sha_hex
    )
}

#[test]
fn stream_splitter_basic() {
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("XDG_CACHE_HOME", tmp.path());
    }

    let server = MockServer::start();
    let model_body = b"this is the mock onnx payload";
    let model_sha = sha256_hex(model_body);

    let _model = server.mock(|when, then| {
        when.method(GET).path("/mock.onnx");
        then.status(200)
            .header("Content-Length", model_body.len().to_string().as_str())
            .body(model_body.as_slice());
    });

    let _manifest = server.mock(|when, then| {
        when.method(GET).path("/m.json");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(manifest_json(&format!("{}/mock.onnx", server.base_url()), &model_sha));
    });

    let manifest_url = format!("{}/m.json", server.base_url());

    let opts = SplitOptions {
        model_name: "ignored".into(),
        manifest_url_override: Some(manifest_url),
        output_dir: tmp.path().to_string_lossy().into(),
    };

    let mut splitter = StreamSplitter::new(opts).expect("failed to create splitter");

    // Window is 4096, hop is 2048
    let chunk_size = 1024;
    let mut total_output = 0;

    for _ in 0..10 {
        let left = vec![0.5f32; chunk_size];
        let right = vec![0.5f32; chunk_size];
        let stems = splitter.push(&left, &right).expect("push failed");

        if !stems[0].is_empty() {
            assert_eq!(stems[0].len(), 2048); // hop size
            total_output += stems[0].len();
        }
    }

    let final_stems = splitter.flush().expect("flush failed");
    total_output += final_stems[0].len();

    // We pushed 10 * 1024 = 10240 samples.
    assert_eq!(total_output, 10240);
}
