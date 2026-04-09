use dubsync_core::Project;
use std::path::PathBuf;

pub async fn load_project_file(path: PathBuf) -> anyhow::Result<Project> {
    tokio::task::spawn_blocking(move || {
        let content = std::fs::read_to_string(path)?;
        let project: Project = serde_json::from_str(&content)?;
        Ok(project)
    })
    .await?
}

pub async fn save_project_file(path: PathBuf, project: Project) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        let content = serde_json::to_string_pretty(&project)?;
        std::fs::write(path, content)?;
        Ok(())
    })
    .await?
}

pub async fn perform_analysis(
    ref_data: dubsync_core::AudioData,
    target_data: dubsync_core::AudioData,
) -> anyhow::Result<dubsync_dsp::util::alignment::AlignmentReport> {
    use dubsync_dsp::mel::MelEngine;
    use dubsync_dsp::util::alignment::evaluate_alignment;
    use dubsync_dsp::util::find_global_offset_robust;

    // 1. Global Offset
    let offset_samples = find_global_offset_robust(
        &ref_data.samples,
        ref_data.sample_rate,
        ref_data.channels,
        &target_data.samples,
        target_data.sample_rate,
        target_data.channels,
    )?;

    // 2. Feature Extraction
    let mel_engine = MelEngine::new(ref_data.sample_rate as f32, 1024, 80);
    let hop_size = (ref_data.sample_rate / 100) as usize; // 10ms frames

    let ref_mono = dubsync_dsp::util::get_mono_average(&ref_data.samples, ref_data.channels);
    let tgt_mono = dubsync_dsp::util::get_mono_average(&target_data.samples, target_data.channels);

    let ref_feat = mel_engine.extract(&ref_mono, hop_size);
    let tgt_feat = mel_engine.extract(&tgt_mono, hop_size);

    // 3. Evaluation based on global offset
    let offset_frames = offset_samples / hop_size as isize;
    let path: Vec<(usize, usize)> = (0..ref_feat.len())
        .map(|r| {
            let t = (r as isize - offset_frames).max(0) as usize;
            (r, t.min(tgt_feat.len() - 1))
        })
        .collect();

    let report = evaluate_alignment(&ref_feat, &tgt_feat, &path, 100, &[]);
    Ok(report)
}
