use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TimelineMode {
    #[default]
    Split,
    Mirrored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlaybackSpeed {
    Slow,
    #[default]
    Normal,
    Fast,
}

impl From<String> for PlaybackSpeed {
    fn from(s: String) -> Self {
        match s.as_str() {
            "Slow" => PlaybackSpeed::Slow,
            "Fast" => PlaybackSpeed::Fast,
            _ => PlaybackSpeed::Normal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlaybackMode {
    #[default]
    Follow,
    Loop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppState {
    pub zoom: f32,
    pub offset: f32,
    pub speed: PlaybackSpeed,
    pub project_path: Option<PathBuf>,
    pub timeline_mode: TimelineMode,
    pub playback_mode: PlaybackMode,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            offset: 0.0,
            speed: PlaybackSpeed::default(),
            project_path: None,
            timeline_mode: TimelineMode::default(),
            playback_mode: PlaybackMode::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DubSyncProjectState {
    pub app_state: AppState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveMode {
    Small,    // Regular arrow (1% viewport)
    Minor,    // Shift + arrow (Minor ticks)
    Major,    // Alt + arrow (Major ticks)
    Boundary, // Ctrl + arrow (Start/End)
}
