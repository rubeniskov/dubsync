use dubsync_core::{AudioStats, ResourceManager};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;

pub fn run(input: PathBuf, output: Option<PathBuf>) -> anyhow::Result<()> {
    let expanded_path = ResourceManager::expand_path(input);
    let style = ProgressStyle::with_template(
        "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {percent}% ({eta})",
    )?
    .progress_chars("#>-");

    if let Some(output_path) = output {
        let stats = AudioStats::extract(&expanded_path)?;
        println!("Extracting to: {:?}", output_path);

        let pb = ProgressBar::new(100);
        pb.set_style(style);

        ResourceManager::extract_audio_from_video(
            &expanded_path,
            &output_path,
            stats.duration_secs,
            Some(|p| {
                pb.set_position((p * 100.0) as u64);
                true
            }),
        )?;
        pb.finish_with_message("Done");
    } else {
        println!("Extracting to cache...");

        let pb = ProgressBar::new(100);
        pb.set_style(style);

        let path = ResourceManager::ensure_extracted_audio(
            &expanded_path,
            Some(|p| {
                pb.set_position((p * 100.0) as u64);
                true
            }),
        )?;
        pb.finish_with_message("Done");
        println!("Extracted to: {:?}", path);
    }
    Ok(())
}
