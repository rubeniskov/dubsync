use dubsync_core::{AudioStats, ResourceManager};
use std::path::PathBuf;

pub fn run(path: PathBuf) -> anyhow::Result<()> {
    let expanded_path = ResourceManager::expand_path(path);
    println!("Probing: {:?}", expanded_path);
    let stats = AudioStats::extract(&expanded_path)?;
    println!("--- Metadata ---");
    println!("Duration:    {}", stats.format_duration());
    println!("Sample Rate: {} Hz", stats.sample_rate);
    println!("Bit Depth:   {} bit", stats.bit_depth);
    println!("Channels:    {}", stats.channels);
    println!("Codec:       {}", stats.codec);
    Ok(())
}
