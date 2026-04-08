use ort::execution_providers::{CUDAExecutionProvider, ExecutionProvider};

fn main() {
    let provider = CUDAExecutionProvider::default();
    match provider.is_available() {
        Ok(available) => println!("CUDA_AVAILABLE: {}", available),
        Err(e) => println!("CUDA_ERROR: {:?}", e),
    }
}
