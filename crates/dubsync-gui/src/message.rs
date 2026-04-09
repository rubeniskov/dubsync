use crate::audio_loader::WaveformCache;
use crate::types::{MoveMode, PlaybackSpeed};
use dubsync_core::{AudioData, AudioStats, Project};
use iced::window;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Message {
    LoadProject,
    ProjectFilePicked(Option<PathBuf>),
    ProjectLoaded(Result<Project, String>),
    UploadReference,
    ReferenceFilePicked(Option<PathBuf>),
    UploadTarget,
    TargetFilePicked(Option<PathBuf>),
    ReferenceMeta(AudioStats),
    TargetMeta(AudioStats),
    ReferenceLoadingProgress { name: String, step: u8, total: u8, percent: f32 },
    TargetLoadingProgress { name: String, step: u8, total: u8, percent: f32 },
    UnloadReference,
    UnloadTarget,
    ReferenceLoaded(Result<(AudioData, WaveformCache, AudioStats), String>),
    TargetLoaded(Result<(AudioData, WaveformCache, AudioStats), String>),
    Analyze,
    AnalysisCompleted(Result<dubsync_dsp::util::alignment::AlignmentReport, String>),
    SaveProject,
    ProjectSaveFilePicked(Option<PathBuf>),
    ProjectSaved(Result<(), String>),
    ScreenshotTaken(window::Screenshot),
    WindowIdFetched(Option<window::Id>),
    ZoomChanged(u32), // Discrete steps 0..100
    SetZoom(f32),     // Direct log zoom 1.0..max_zoom
    OffsetChanged(f32),
    ZoomAndOffsetChanged(f32, f32),
    SpeedChanged(PlaybackSpeed),
    ToggleTimelineMode,
    TogglePlay,
    TogglePlaybackMode,
    Pause,
    Resume,
    Stop,
    Tick,
    SeekPlayback(f32),
    ZoomIn,
    ZoomOut,
    StepZoom(bool), // true = in, false = out
    MoveCursorLeft(MoveMode),
    MoveCursorRight(MoveMode),
    Batch(Vec<Message>),
    None,
}
