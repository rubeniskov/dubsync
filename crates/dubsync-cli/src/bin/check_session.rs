use ort::{
    execution_providers::CUDAExecutionProvider,
    init,
    session::builder::SessionBuilder,
    value::{DynValue, Value},
};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    init().with_name("diagnostic").commit()?;

    let cache_dir = directories::ProjectDirs::from("dev", "DubSync", "dubsync")
        .map(|pd| pd.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("dubsync");
    let model_path = cache_dir.join("silero_vad.onnx");

    if !model_path.exists() {
        println!("Silero VAD model not found at {:?}", model_path);
        return Ok(());
    }

    println!("Creating CUDA provider...");
    let cuda_provider = CUDAExecutionProvider::default().build();

    println!("Creating SessionBuilder with CUDA...");
    let mut session = SessionBuilder::new()?
        .with_execution_providers(vec![cuda_provider])?
        .commit_from_file(model_path)?;

    println!("✅ SUCCESS: Session created with CUDA!");

    let chunk_size = 512;
    let chunk = vec![0.0f32; chunk_size];
    let state = vec![0.0f32; 2 * 128];
    let sr_data = vec![16000i64; 1];

    let input_names: Vec<String> = session.inputs.iter().map(|i| i.name.clone()).collect();
    let is_v5 = input_names.contains(&String::from("state"));

    let input_tensor = Value::from_array((vec![1, chunk_size], chunk))?;
    let sr_tensor = Value::from_array((vec![1], sr_data))?;

    println!("Executing a single inference pass to check hardware placement...");
    let _outputs = if is_v5 {
        let state_tensor = Value::from_array((vec![2, 1, 128], state))?;
        session.run(vec![
            (String::from("input"), DynValue::from(input_tensor)),
            (String::from("sr"), DynValue::from(sr_tensor)),
            (String::from("state"), DynValue::from(state_tensor)),
        ])?
    } else {
        let h_tensor = Value::from_array((vec![2, 1, 64], vec![0.0f32; 128]))?;
        let c_tensor = Value::from_array((vec![2, 1, 64], vec![0.0f32; 128]))?;
        session.run(vec![
            (String::from("input"), DynValue::from(input_tensor)),
            (String::from("sr"), DynValue::from(sr_tensor)),
            (String::from("h"), DynValue::from(h_tensor)),
            (String::from("c"), DynValue::from(c_tensor)),
        ])?
    };

    println!("Inference completed successfully.");
    Ok(())
}
