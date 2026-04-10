use crate::audio_loader::{LoadingStep, WaveformCache, load_audio_file};
use crate::message::Message;
use crate::tasks::{load_project_file, perform_analysis, save_project_file};
use crate::theme::{GEIST_MONO, LUCIDE};
use crate::types::{DubSyncProjectState, MoveMode, PlaybackMode, PlaybackSpeed, TimelineMode};
use crate::widgets::waveform::{Navigator, TimelineViewport};
use dubsync_core::{AudioData, AudioStats, ChannelLayout, Codec, Project, ResourceManager};
use iced::widget::{
    Canvas, button, column, container, horizontal_space, progress_bar, row, slider, stack, text,
};
use iced::window;
use iced::{Alignment, Border, Color, Element, Font, Length, Padding, Task, Theme, font};
use lucide_icons::Icon;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct TrackLoadingState {
    pub step_name: String,
    pub step: u8,
    pub total: u8,
    pub percent: f32,
    pub is_active: bool,
    pub error_message: Option<String>,
}

pub struct DubSyncGui {
    pub project: Project,
    pub reference_data: Option<AudioData>,
    pub target_data: Option<AudioData>,
    pub reference_cache: Option<WaveformCache>,
    pub target_cache: Option<WaveformCache>,
    pub reference_stats: Option<AudioStats>,
    pub target_stats: Option<AudioStats>,
    pub is_loading: bool,
    pub is_analyzing: bool,
    pub ref_loading: TrackLoadingState,
    pub target_loading: TrackLoadingState,
    pub ref_cancel_token: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub target_cancel_token: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub timeline_viewport: TimelineViewport,
    pub navigator: Navigator,
    pub snapshot_output: Option<PathBuf>,
    pub zoom: f32,
    pub offset: f32,
    pub speed: PlaybackSpeed,
    // Playback state
    pub is_playing: bool,
    pub playback_mode: PlaybackMode,
    pub playback_pos: f32, // 0.0 to 1.0
    pub playback_start_instant: Option<std::time::Instant>,
    pub playback_start_pos: f32,
    pub last_move_left_instant: Option<std::time::Instant>,
    pub audio_sink_ref: Option<rodio::SpatialSink>,
    pub audio_sink_target: Option<rodio::SpatialSink>,
    pub _audio_stream: Option<rodio::OutputStream>,
}

impl Default for DubSyncGui {
    fn default() -> Self {
        let (stream, handle) = match rodio::OutputStream::try_default() {
            Ok((s, h)) => (Some(s), Some(h)),
            Err(_) => (None, None),
        };

        let (audio_sink_ref, audio_sink_target) = if let Some(h) = &handle {
            let left_ear = [-1.0, 0.0, 0.0];
            let right_ear = [1.0, 0.0, 0.0];
            (
                rodio::SpatialSink::try_new(h, [-1.0, 0.0, 0.0], left_ear, right_ear).ok(),
                rodio::SpatialSink::try_new(h, [1.0, 0.0, 0.0], left_ear, right_ear).ok(),
            )
        } else {
            (None, None)
        };

        Self {
            project: Project::default(),
            reference_data: None,
            target_data: None,
            reference_cache: None,
            target_cache: None,
            reference_stats: None,
            target_stats: None,
            is_loading: false,
            is_analyzing: false,
            ref_loading: TrackLoadingState::default(),
            target_loading: TrackLoadingState::default(),
            ref_cancel_token: None,
            target_cancel_token: None,
            timeline_viewport: TimelineViewport::new(),
            navigator: Navigator::new(),
            snapshot_output: None,
            zoom: 1.0,
            offset: 0.0,
            speed: PlaybackSpeed::Normal,
            is_playing: false,
            playback_mode: PlaybackMode::default(),
            playback_pos: 0.0,
            playback_start_instant: None,
            playback_start_pos: 0.0,
            last_move_left_instant: None,
            audio_sink_ref,
            audio_sink_target,
            _audio_stream: stream,
        }
    }
}

impl DubSyncGui {
    pub fn from_project(mut project: Project, output: Option<PathBuf>) -> (Self, Task<Message>) {
        let mut tasks = Vec::new();
        let ref_cancel_token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let target_cancel_token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        if let Some(p) = &project.reference_path {
            let expanded = ResourceManager::expand_path(p);
            project.reference_path = Some(expanded.clone());
            tasks.push(Task::run(load_audio_file(expanded, ref_cancel_token.clone()), |step| {
                match step {
                    LoadingStep::Meta(meta) => Message::ReferenceMeta(meta),
                    LoadingStep::Progress { name, step, total, percent } => {
                        Message::ReferenceLoadingProgress { name, step, total, percent }
                    }
                    LoadingStep::Result(res) => {
                        Message::ReferenceLoaded(res.map_err(|e| e.to_string()))
                    }
                }
            }));
        }

        if let Some(p) = &project.target_path {
            let expanded = ResourceManager::expand_path(p);
            project.target_path = Some(expanded.clone());
            tasks.push(Task::run(load_audio_file(expanded, target_cancel_token.clone()), |step| {
                match step {
                    LoadingStep::Meta(meta) => Message::TargetMeta(meta),
                    LoadingStep::Progress { name, step, total, percent } => {
                        Message::TargetLoadingProgress { name, step, total, percent }
                    }
                    LoadingStep::Result(res) => {
                        Message::TargetLoaded(res.map_err(|e| e.to_string()))
                    }
                }
            }));
        }

        let task = if tasks.is_empty() && output.is_some() {
            window::get_oldest().map(Message::WindowIdFetched)
        } else {
            Task::batch(tasks)
        };

        let is_loading = project.reference_path.is_some() || project.target_path.is_some();

        (
            Self {
                project,
                is_loading,
                snapshot_output: output,
                ref_cancel_token: Some(ref_cancel_token),
                target_cancel_token: Some(target_cancel_token),
                ..Default::default()
            },
            task,
        )
    }

    pub fn from_state(
        state: DubSyncProjectState,
        output: Option<PathBuf>,
    ) -> (Self, Task<Message>) {
        let project = if let Some(path) = &state.app_state.project_path {
            let content = std::fs::read_to_string(path).expect("Failed to read project file");
            serde_json::from_str(&content).expect("Failed to parse project JSON")
        } else {
            Project::default()
        };

        let (mut gui, task) = Self::from_project(project, output);
        gui.zoom = state.app_state.zoom;
        gui.offset = state.app_state.offset;
        gui.speed = state.app_state.speed;
        gui.playback_mode = state.app_state.playback_mode;

        // Sync waveform views
        gui.timeline_viewport.zoom = gui.zoom;
        gui.timeline_viewport.offset = gui.offset;
        gui.navigator.zoom = gui.zoom;
        gui.navigator.offset = gui.offset;

        (gui, task)
    }

    pub fn subscription(&self) -> iced::Subscription<Message> {
        let tick = if self.is_playing {
            iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::Tick)
        } else {
            iced::Subscription::none()
        };

        let keyboard = iced::event::listen().map(|event| match event {
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                let shift = modifiers.shift();
                let alt = modifiers.alt();
                let ctrl = modifiers.control() || modifiers.command();
                let cmd = modifiers.command();

                let mode = if ctrl && !cmd {
                    MoveMode::Boundary
                } else if alt {
                    MoveMode::Major
                } else if shift {
                    MoveMode::Minor
                } else {
                    MoveMode::Small
                };

                match key {
                    iced::keyboard::Key::Named(iced::keyboard::key::Named::Space) => {
                        Some(Message::TogglePlay)
                    }
                    iced::keyboard::Key::Character(c) if c == "l" || c == "L" => {
                        Some(Message::TogglePlaybackMode)
                    }
                    iced::keyboard::Key::Character(c)
                        if (c == "+" || c == "=") && (ctrl || cmd) =>
                    {
                        Some(Message::SetZoom(1000.0)) // Will be clamped to dynamic max
                    }
                    iced::keyboard::Key::Character(c)
                        if (c == "-" || c == "_") && (ctrl || cmd) =>
                    {
                        Some(Message::SetZoom(1.0)) // 0%
                    }
                    iced::keyboard::Key::Character(c) if (c == "+" || c == "=") && shift => {
                        Some(Message::StepZoom(true))
                    }
                    iced::keyboard::Key::Character(c) if (c == "-" || c == "_") && shift => {
                        Some(Message::StepZoom(false))
                    }
                    iced::keyboard::Key::Character(c) if c == "+" || c == "=" => {
                        Some(Message::ZoomIn)
                    }
                    iced::keyboard::Key::Character(c) if c == "-" || c == "_" => {
                        Some(Message::ZoomOut)
                    }
                    iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowLeft) => {
                        if cmd {
                            Some(Message::MoveCursorLeft(MoveMode::Boundary))
                        } else {
                            Some(Message::MoveCursorLeft(mode))
                        }
                    }
                    iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowRight) => {
                        if cmd {
                            Some(Message::MoveCursorRight(MoveMode::Boundary))
                        } else {
                            Some(Message::MoveCursorRight(mode))
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        });

        iced::Subscription::batch(vec![tick, keyboard.map(|m| m.unwrap_or(Message::None))])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::LoadProject => {
                return Task::perform(
                    rfd::AsyncFileDialog::new().add_filter("Project", &["json"]).pick_file(),
                    |handle| Message::ProjectFilePicked(handle.map(|h| h.path().to_path_buf())),
                );
            }
            Message::ProjectFilePicked(Some(p)) => {
                return Task::perform(load_project_file(p), |res| {
                    Message::ProjectLoaded(res.map_err(|e| e.to_string()))
                });
            }
            Message::ProjectFilePicked(None) => {}
            Message::ProjectLoaded(Ok(project)) => {
                let (new_gui, task) = Self::from_project(project, self.snapshot_output.clone());
                self.project = new_gui.project;
                self.is_loading = new_gui.is_loading;
                // Reset caches and data
                self.reference_data = None;
                self.target_data = None;
                self.reference_cache = None;
                self.target_cache = None;
                self.reference_stats = None;
                self.target_stats = None;
                self.timeline_viewport.reference.cache = None;
                self.timeline_viewport.target.cache = None;
                self.navigator.reference = None;
                self.navigator.target = None;
                self.timeline_viewport.cache.clear();
                self.navigator.cache.clear();

                return task;
            }
            Message::ProjectLoaded(Err(e)) => {
                eprintln!("Error loading project: {}", e);
            }
            Message::UploadReference => {
                return Task::perform(
                    rfd::AsyncFileDialog::new()
                        .add_filter("Audio", Codec::all_extensions())
                        .pick_file(),
                    |handle| Message::ReferenceFilePicked(handle.map(|h| h.path().to_path_buf())),
                );
            }
            Message::ReferenceFilePicked(Some(p)) => {
                self.project.reference_path = Some(p.clone());
                self.is_loading = true;
                self.ref_loading = TrackLoadingState { is_active: true, ..Default::default() };
                let cancel_token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                self.ref_cancel_token = Some(cancel_token.clone());
                return Task::run(load_audio_file(p, cancel_token), |step| match step {
                    LoadingStep::Meta(meta) => Message::ReferenceMeta(meta),
                    LoadingStep::Progress { name, step, total, percent } => {
                        Message::ReferenceLoadingProgress { name, step, total, percent }
                    }
                    LoadingStep::Result(res) => {
                        Message::ReferenceLoaded(res.map_err(|e| e.to_string()))
                    }
                });
            }
            Message::ReferenceFilePicked(None) => {}
            Message::UploadTarget => {
                return Task::perform(
                    rfd::AsyncFileDialog::new()
                        .add_filter("Audio", Codec::all_extensions())
                        .pick_file(),
                    |handle| Message::TargetFilePicked(handle.map(|h| h.path().to_path_buf())),
                );
            }
            Message::TargetFilePicked(Some(p)) => {
                self.project.target_path = Some(p.clone());
                self.is_loading = true;
                self.target_loading = TrackLoadingState { is_active: true, ..Default::default() };
                let cancel_token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                self.target_cancel_token = Some(cancel_token.clone());
                return Task::run(load_audio_file(p, cancel_token), |step| match step {
                    LoadingStep::Meta(meta) => Message::TargetMeta(meta),
                    LoadingStep::Progress { name, step, total, percent } => {
                        Message::TargetLoadingProgress { name, step, total, percent }
                    }
                    LoadingStep::Result(res) => {
                        Message::TargetLoaded(res.map_err(|e| e.to_string()))
                    }
                });
            }
            Message::TargetFilePicked(None) => {}
            Message::ReferenceMeta(stats) => {
                let filename = self
                    .project
                    .reference_path
                    .as_ref()
                    .and_then(|p| p.file_stem())
                    .and_then(|s| s.to_str())
                    .unwrap_or("Reference");
                self.timeline_viewport.reference.label = filename.to_string();
                self.timeline_viewport.reference.sub_label = Self::format_sub_label(&stats);
                self.reference_stats = Some(stats);
            }
            Message::TargetMeta(stats) => {
                let filename = self
                    .project
                    .target_path
                    .as_ref()
                    .and_then(|p| p.file_stem())
                    .and_then(|s| s.to_str())
                    .unwrap_or("Target");
                self.timeline_viewport.target.label = filename.to_string();
                self.timeline_viewport.target.sub_label = Self::format_sub_label(&stats);
                self.target_stats = Some(stats);
            }
            Message::ReferenceLoadingProgress { name, step, total, percent } => {
                self.ref_loading = TrackLoadingState {
                    step_name: name,
                    step,
                    total,
                    percent,
                    is_active: true,
                    error_message: None,
                };
                self.timeline_viewport.is_loading = true;
                self.navigator.is_loading = true;
            }
            Message::TargetLoadingProgress { name, step, total, percent } => {
                self.target_loading = TrackLoadingState {
                    step_name: name,
                    step,
                    total,
                    percent,
                    is_active: true,
                    error_message: None,
                };
                self.timeline_viewport.is_loading = true;
                self.navigator.is_loading = true;
            }
            Message::UnloadReference => {
                if let Some(token) = self.ref_cancel_token.take() {
                    token.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.project.reference_path = None;
                self.reference_data = None;
                self.reference_cache = None;
                self.reference_stats = None;
                self.ref_loading = TrackLoadingState::default();
                self.timeline_viewport.reference.cache = None;
                self.navigator.reference = None;
                self.timeline_viewport.cache.clear();
                self.navigator.cache.clear();
                if self.target_data.is_none() {
                    self.timeline_viewport.total_duration = 0.0;
                    self.navigator.total_duration = 0.0;
                }
                self.timeline_viewport.is_loading = self.target_loading.is_active;
                self.navigator.is_loading = self.target_loading.is_active;
            }
            Message::UnloadTarget => {
                if let Some(token) = self.target_cancel_token.take() {
                    token.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.project.target_path = None;
                self.target_data = None;
                self.target_cache = None;
                self.target_stats = None;
                self.target_loading = TrackLoadingState::default();
                self.timeline_viewport.target.cache = None;
                self.navigator.target = None;
                self.timeline_viewport.cache.clear();
                self.navigator.cache.clear();
                if self.reference_data.is_none() {
                    self.timeline_viewport.total_duration = 0.0;
                    self.navigator.total_duration = 0.0;
                }
                self.timeline_viewport.is_loading = self.ref_loading.is_active;
                self.navigator.is_loading = self.ref_loading.is_active;
            }
            Message::ReferenceLoaded(Ok((data, cache, stats))) => {
                self.reference_data = Some(data);
                self.reference_cache = Some(cache.clone());
                self.reference_stats = Some(stats);
                self.ref_loading.is_active = false;
                self.timeline_viewport.is_loading = self.target_loading.is_active;
                self.navigator.is_loading = self.target_loading.is_active;

                if let Some(s) = &self.reference_stats {
                    let filename = self
                        .project
                        .reference_path
                        .as_ref()
                        .and_then(|p| p.file_stem())
                        .and_then(|s| s.to_str())
                        .unwrap_or("Reference");
                    self.timeline_viewport.reference.label = filename.to_string();
                    self.timeline_viewport.reference.sub_label = Self::format_sub_label(s);
                }

                self.timeline_viewport.reference.cache = Some(cache.clone());
                self.navigator.reference = Some(cache);
                let duration =
                    self.reference_stats.as_ref().map(|s| s.duration_secs as f32).unwrap_or(0.0);
                self.timeline_viewport.total_duration = duration;
                self.navigator.total_duration = duration;
                self.timeline_viewport.cache.clear();
                self.navigator.cache.clear();
                self.is_loading = false;

                if self.snapshot_output.is_some() {
                    if let Some(t_data) = &self.target_data {
                        if self.project.alignment_report.is_some() {
                            return window::get_oldest().map(Message::WindowIdFetched);
                        } else {
                            self.is_analyzing = true;
                            let r = self.reference_data.as_ref().unwrap().clone();
                            let t = t_data.clone();
                            return Task::perform(perform_analysis(r, t), |res| {
                                Message::AnalysisCompleted(res.map_err(|e| e.to_string()))
                            });
                        }
                    }
                    if self.project.target_path.is_none() {
                        return window::get_oldest().map(Message::WindowIdFetched);
                    }
                }
            }
            Message::TargetLoaded(Ok((data, cache, stats))) => {
                self.target_data = Some(data);
                self.target_cache = Some(cache.clone());
                self.target_stats = Some(stats);
                self.target_loading.is_active = false;
                self.timeline_viewport.is_loading = self.ref_loading.is_active;
                self.navigator.is_loading = self.ref_loading.is_active;

                if let Some(s) = &self.target_stats {
                    let filename = self
                        .project
                        .target_path
                        .as_ref()
                        .and_then(|p| p.file_stem())
                        .and_then(|s| s.to_str())
                        .unwrap_or("Target");
                    self.timeline_viewport.target.label = filename.to_string();
                    self.timeline_viewport.target.sub_label = Self::format_sub_label(s);
                }

                self.timeline_viewport.target.cache = Some(cache.clone());
                self.navigator.target = Some(cache);
                let duration =
                    self.target_stats.as_ref().map(|s| s.duration_secs as f32).unwrap_or(0.0);
                if self.timeline_viewport.total_duration == 0.0 {
                    self.timeline_viewport.total_duration = duration;
                    self.navigator.total_duration = duration;
                }
                self.timeline_viewport.cache.clear();
                self.navigator.cache.clear();
                self.is_loading = false;

                if self.snapshot_output.is_some() {
                    if let Some(r_data) = &self.reference_data {
                        if self.project.alignment_report.is_some() {
                            return window::get_oldest().map(Message::WindowIdFetched);
                        } else {
                            self.is_analyzing = true;
                            let r = r_data.clone();
                            let t = self.target_data.as_ref().unwrap().clone();
                            return Task::perform(perform_analysis(r, t), |res| {
                                Message::AnalysisCompleted(res.map_err(|e| e.to_string()))
                            });
                        }
                    }
                    if self.project.reference_path.is_none() {
                        return window::get_oldest().map(Message::WindowIdFetched);
                    }
                }
            }
            Message::ReferenceLoaded(Err(e)) => {
                if e != "Cancelled" {
                    eprintln!("Error loading reference audio: {}", e);
                    self.ref_loading.error_message = Some(e);
                } else {
                    // It was intentionally cancelled, ensure it stays inactive/cleared
                    self.ref_loading = TrackLoadingState::default();
                }
                self.is_loading = self.target_loading.is_active;
                self.ref_loading.is_active = false;
                self.timeline_viewport.is_loading = self.target_loading.is_active;
                self.navigator.is_loading = self.target_loading.is_active;
                if self.snapshot_output.is_some() {
                    std::process::exit(1);
                }
            }
            Message::TargetLoaded(Err(e)) => {
                if e != "Cancelled" {
                    eprintln!("Error loading target audio: {}", e);
                    self.target_loading.error_message = Some(e);
                } else {
                    // It was intentionally cancelled, ensure it stays inactive/cleared
                    self.target_loading = TrackLoadingState::default();
                }
                self.is_loading = self.ref_loading.is_active;
                self.target_loading.is_active = false;
                self.timeline_viewport.is_loading = self.ref_loading.is_active;
                self.navigator.is_loading = self.ref_loading.is_active;
                if self.snapshot_output.is_some() {
                    std::process::exit(1);
                }
            }
            Message::Analyze => {
                if let (Some(r), Some(t)) = (&self.reference_data, &self.target_data) {
                    self.is_analyzing = true;
                    return Task::perform(perform_analysis(r.clone(), t.clone()), |res| {
                        Message::AnalysisCompleted(res.map_err(|e| e.to_string()))
                    });
                }
            }
            Message::AnalysisCompleted(Ok(report)) => {
                self.is_analyzing = false;
                self.project.alignment_report = Some(report);
                if self.snapshot_output.is_some() {
                    return window::get_oldest().map(Message::WindowIdFetched);
                }
            }
            Message::AnalysisCompleted(Err(e)) => {
                eprintln!("Analysis error: {}", e);
                self.is_analyzing = false;
            }
            Message::SaveProject => {
                return Task::perform(
                    rfd::AsyncFileDialog::new()
                        .add_filter("Project", &["json"])
                        .set_file_name("project.json")
                        .save_file(),
                    |handle| Message::ProjectSaveFilePicked(handle.map(|h| h.path().to_path_buf())),
                );
            }
            Message::ProjectSaveFilePicked(Some(path)) => {
                return Task::perform(save_project_file(path, self.project.clone()), |res| {
                    Message::ProjectSaved(res.map_err(|e| e.to_string()))
                });
            }
            Message::ProjectSaveFilePicked(None) => {}
            Message::ProjectSaved(Ok(())) => {
                println!("Project saved successfully.");
            }
            Message::ProjectSaved(Err(e)) => {
                eprintln!("Error saving project: {}", e);
            }
            Message::WindowIdFetched(Some(id)) => {
                return window::screenshot(id).map(Message::ScreenshotTaken);
            }
            Message::WindowIdFetched(None) => {
                if self.snapshot_output.is_some() {
                    eprintln!("Error: No window found for screenshot");
                    std::process::exit(1);
                }
            }
            Message::ScreenshotTaken(screenshot) => {
                if let Some(output) = &self.snapshot_output {
                    let bytes = screenshot.bytes;
                    let width = screenshot.size.width;
                    let height = screenshot.size.height;

                    let image = image::RgbaImage::from_raw(width, height, bytes.into())
                        .expect("Failed to create image from screenshot");
                    image.save(output).expect("Failed to save screenshot");
                    println!("Screenshot saved to {:?}", output);
                    std::process::exit(0);
                }
            }
            Message::ZoomChanged(steps) => {
                let max_z = (self.timeline_viewport.total_duration / 5.0).max(1.0);
                let z_linear = steps as f32 / 100.0;
                let new_zoom = 1.0 * max_z.powf(z_linear);
                return Task::perform(async move { new_zoom }, Message::SetZoom);
            }
            Message::SetZoom(z) => {
                let max_z = (self.timeline_viewport.total_duration / 5.0).max(1.0);
                self.zoom = z.clamp(1.0, max_z);
                self.offset = self.offset.clamp(0.0, (1.0 - 1.0 / self.zoom).max(0.0));
                self.timeline_viewport.zoom = self.zoom;
                self.timeline_viewport.offset = self.offset;
                self.navigator.zoom = self.zoom;
                self.navigator.offset = self.offset;
                self.timeline_viewport.cache.clear();
                self.navigator.cache.clear();
            }
            Message::OffsetChanged(o) => {
                self.offset = o.clamp(0.0, (1.0 - 1.0 / self.zoom).max(0.0));
                self.timeline_viewport.offset = self.offset;
                self.navigator.offset = self.offset;
                self.timeline_viewport.cache.clear();
                self.navigator.cache.clear();
            }
            Message::ZoomAndOffsetChanged(z, o) => {
                let max_z = (self.timeline_viewport.total_duration / 5.0).max(1.0);
                self.zoom = z.clamp(1.0, max_z);
                self.offset = o.clamp(0.0, (1.0 - 1.0 / self.zoom).max(0.0));
                self.timeline_viewport.zoom = self.zoom;
                self.timeline_viewport.offset = self.offset;
                self.navigator.zoom = self.zoom;
                self.navigator.offset = self.offset;
                self.timeline_viewport.cache.clear();
                self.navigator.cache.clear();
            }
            Message::SpeedChanged(s) => self.speed = s,
            Message::ToggleTimelineMode => {
                self.timeline_viewport.mode = match self.timeline_viewport.mode {
                    TimelineMode::Split => TimelineMode::Mirrored,
                    TimelineMode::Mirrored => TimelineMode::Split,
                };
                self.navigator.mode = self.timeline_viewport.mode;
                self.timeline_viewport.cache.clear();
                self.navigator.cache.clear();
            }
            Message::TogglePlay => {
                if self.is_playing {
                    if let Some(sink) = &self.audio_sink_ref {
                        sink.pause();
                    }
                    if let Some(sink) = &self.audio_sink_target {
                        sink.pause();
                    }
                    self.is_playing = false;
                    self.playback_start_instant = None;
                } else {
                    let has_ref = self.audio_sink_ref.is_some();
                    let has_tgt = self.audio_sink_target.is_some();

                    if has_ref || has_tgt {
                        if let (Some(sink), Some(data)) =
                            (&self.audio_sink_ref, &self.reference_data)
                        {
                            if sink.empty() {
                                sink.append(rodio::buffer::SamplesBuffer::new(
                                    data.channels,
                                    data.sample_rate,
                                    data.samples.clone(),
                                ));
                            }
                        }
                        if let (Some(sink), Some(data)) =
                            (&self.audio_sink_target, &self.target_data)
                        {
                            if sink.empty() {
                                sink.append(rodio::buffer::SamplesBuffer::new(
                                    data.channels,
                                    data.sample_rate,
                                    data.samples.clone(),
                                ));
                            }
                        }

                        if self.playback_mode == PlaybackMode::Loop {
                            let window_start = self.offset;
                            let window_end = (self.offset + 1.0 / self.zoom).min(1.0);
                            if self.playback_pos < window_start || self.playback_pos >= window_end {
                                self.playback_pos = window_start;
                                if let Some(stats) = &self.reference_stats {
                                    let seek_time = std::time::Duration::from_secs_f32(
                                        window_start * stats.duration_secs as f32,
                                    );
                                    if let Some(sink) = &self.audio_sink_ref {
                                        let _ = sink.try_seek(seek_time);
                                    }
                                    if let Some(sink) = &self.audio_sink_target {
                                        let _ = sink.try_seek(seek_time);
                                    }
                                }
                            }
                        }

                        if let Some(sink) = &self.audio_sink_ref {
                            sink.play();
                        }
                        if let Some(sink) = &self.audio_sink_target {
                            sink.play();
                        }

                        self.is_playing = true;
                        self.playback_start_instant = Some(std::time::Instant::now());
                        self.playback_start_pos = self.playback_pos;
                    }
                }
                self.timeline_viewport.is_playing = self.is_playing;
                self.navigator.is_playing = self.is_playing;
                self.timeline_viewport.playback_pos = self.playback_pos;
                self.navigator.playback_pos = self.playback_pos;
                self.timeline_viewport.cursor_cache.clear();
                self.navigator.cursor_cache.clear();
            }
            Message::TogglePlaybackMode => {
                self.playback_mode = match self.playback_mode {
                    PlaybackMode::Follow => PlaybackMode::Loop,
                    PlaybackMode::Loop => PlaybackMode::Follow,
                };
                self.timeline_viewport.cursor_cache.clear();
                self.navigator.cursor_cache.clear();
            }
            Message::Pause => {
                if self.is_playing {
                    if let Some(sink) = &self.audio_sink_ref {
                        sink.pause();
                    }
                    if let Some(sink) = &self.audio_sink_target {
                        sink.pause();
                    }
                    self.is_playing = false;
                    self.playback_start_instant = None;
                    self.timeline_viewport.is_playing = false;
                    self.navigator.is_playing = false;
                }
            }
            Message::Resume => {
                if !self.is_playing {
                    let mut started = false;
                    if let Some(sink) = &self.audio_sink_ref {
                        if !sink.empty() {
                            sink.play();
                            started = true;
                        }
                    }
                    if let Some(sink) = &self.audio_sink_target {
                        if !sink.empty() {
                            sink.play();
                            started = true;
                        }
                    }
                    if started {
                        self.is_playing = true;
                        self.playback_start_instant = Some(std::time::Instant::now());
                        self.playback_start_pos = self.playback_pos;
                        self.timeline_viewport.is_playing = true;
                        self.navigator.is_playing = true;
                    }
                }
            }
            Message::Stop => {
                if let Some(sink) = &self.audio_sink_ref {
                    sink.stop();
                }
                if let Some(sink) = &self.audio_sink_target {
                    sink.stop();
                }
                let (stream, handle) = match rodio::OutputStream::try_default() {
                    Ok((s, h)) => (Some(s), Some(h)),
                    Err(_) => (None, None),
                };
                let (audio_sink_ref, audio_sink_target) = if let Some(h) = &handle {
                    let le = [-1.0, 0.0, 0.0];
                    let re = [1.0, 0.0, 0.0];
                    (
                        rodio::SpatialSink::try_new(h, [-1.0, 0.0, 0.0], le, re).ok(),
                        rodio::SpatialSink::try_new(h, [1.0, 0.0, 0.0], le, re).ok(),
                    )
                } else {
                    (None, None)
                };
                self.audio_sink_ref = audio_sink_ref;
                self.audio_sink_target = audio_sink_target;
                self._audio_stream = stream;
                self.is_playing = false;
                self.playback_pos = 0.0;
                self.playback_start_instant = None;
                self.playback_start_pos = 0.0;
                self.timeline_viewport.playback_pos = 0.0;
                self.navigator.playback_pos = 0.0;
                self.timeline_viewport.is_playing = false;
                self.navigator.is_playing = false;
                self.timeline_viewport.cache.clear();
                self.navigator.cache.clear();
            }
            Message::Tick => {
                if self.is_playing {
                    if let (Some(start_instant), Some(stats)) =
                        (self.playback_start_instant, &self.reference_stats)
                    {
                        let elapsed = start_instant.elapsed().as_secs_f32();
                        let duration = stats.duration_secs as f32;
                        if duration > 0.0 {
                            let new_pos = (self.playback_start_pos + elapsed / duration).min(1.0);
                            match self.playback_mode {
                                PlaybackMode::Loop => {
                                    let window_end = (self.offset + 1.0 / self.zoom).min(1.0);
                                    if new_pos >= window_end {
                                        let start_offset = self.offset;
                                        return Task::perform(
                                            async move { start_offset },
                                            Message::SeekPlayback,
                                        );
                                    }
                                }
                                PlaybackMode::Follow => {
                                    let window_width = 1.0 / self.zoom;
                                    let window_center = self.offset + (window_width / 2.0);
                                    if new_pos > window_center {
                                        let new_offset = (new_pos - (window_width / 2.0))
                                            .clamp(0.0, (1.0 - window_width).max(0.0));
                                        if (new_offset - self.offset).abs() > 0.00001 {
                                            self.offset = new_offset;
                                            self.timeline_viewport.offset = self.offset;
                                            self.navigator.offset = self.offset;
                                            self.timeline_viewport.cache.clear();
                                            self.navigator.cache.clear();
                                        }
                                    }
                                }
                            }
                            self.playback_pos = new_pos;
                            self.timeline_viewport.playback_pos = new_pos;
                            self.navigator.playback_pos = new_pos;
                            self.timeline_viewport.cursor_cache.clear();
                            self.navigator.cursor_cache.clear();
                        }
                    }
                }
            }
            Message::SeekPlayback(pos) => {
                self.playback_pos = pos;
                self.timeline_viewport.playback_pos = pos;
                self.navigator.playback_pos = pos;
                self.timeline_viewport.cursor_cache.clear();
                self.navigator.cursor_cache.clear();
                if let Some(stats) = &self.reference_stats {
                    let seek_time =
                        std::time::Duration::from_secs_f32(pos * stats.duration_secs as f32);
                    if let Some(sink) = &self.audio_sink_ref {
                        let _ = sink.try_seek(seek_time);
                    }
                    if let Some(sink) = &self.audio_sink_target {
                        let _ = sink.try_seek(seek_time);
                    }
                    if self.is_playing {
                        self.playback_start_instant = Some(std::time::Instant::now());
                        self.playback_start_pos = pos;
                    }
                }
            }
            Message::ZoomIn => {
                let max_z = (self.timeline_viewport.total_duration / 5.0).max(1.0);
                let new_zoom = (self.zoom * 1.1).clamp(1.0, max_z);
                let middle_view = self.offset + 0.5 / self.zoom;
                let new_offset =
                    (middle_view - 0.5 / new_zoom).clamp(0.0, (1.0 - 1.0 / new_zoom).max(0.0));
                return Task::perform(async move { (new_zoom, new_offset) }, |(z, o)| {
                    Message::ZoomAndOffsetChanged(z, o)
                });
            }
            Message::ZoomOut => {
                let max_z = (self.timeline_viewport.total_duration / 5.0).max(1.0);
                let new_zoom = (self.zoom / 1.1).clamp(1.0, max_z);
                let middle_view = self.offset + 0.5 / self.zoom;
                let new_offset =
                    (middle_view - 0.5 / new_zoom).clamp(0.0, (1.0 - 1.0 / new_zoom).max(0.0));
                return Task::perform(async move { (new_zoom, new_offset) }, |(z, o)| {
                    Message::ZoomAndOffsetChanged(z, o)
                });
            }
            Message::StepZoom(zoom_in) => {
                let max_z = (self.timeline_viewport.total_duration / 5.0).max(1.0);
                let current_linear =
                    if max_z > 1.0 { self.zoom.log10() / max_z.log10() } else { 0.0 };
                let steps = [0.0, 0.25, 0.5, 0.75, 1.0];
                let target_linear = if zoom_in {
                    steps.iter().find(|&&s| s > current_linear + 0.01).copied().unwrap_or(1.0)
                } else {
                    steps.iter().rev().find(|&&s| s < current_linear - 0.01).copied().unwrap_or(0.0)
                };
                let target_zoom = if max_z > 1.0 { 1.0 * max_z.powf(target_linear) } else { 1.0 };
                return Task::perform(async move { target_zoom }, Message::SetZoom);
            }
            Message::MoveCursorLeft(mode) => {
                if let Some(stats) = &self.reference_stats {
                    let current_secs = if self.is_playing {
                        if let Some(start_instant) = self.playback_start_instant {
                            self.playback_start_pos * stats.duration_secs as f32
                                + start_instant.elapsed().as_secs_f32()
                        } else {
                            self.playback_pos * stats.duration_secs as f32
                        }
                    } else {
                        self.playback_pos * stats.duration_secs as f32
                    };
                    let new_pos_secs;
                    let now = std::time::Instant::now();
                    let quick_press = if let Some(last) = self.last_move_left_instant {
                        now.duration_since(last).as_millis() < 300
                    } else {
                        false
                    };
                    self.last_move_left_instant = Some(now);
                    match mode {
                        MoveMode::Boundary => new_pos_secs = 0.0,
                        MoveMode::Major => {
                            let tick_calc = crate::widgets::waveform::TimelineRuleTick::new(
                                self.timeline_viewport.total_duration,
                                self.zoom,
                            );
                            let interval = tick_calc.get_tick_interval();
                            let target_tick = (current_secs / interval).floor() * interval;
                            if quick_press || (current_secs - target_tick).abs() < 0.001 {
                                new_pos_secs = target_tick - interval;
                            } else {
                                new_pos_secs = target_tick;
                            }
                        }
                        MoveMode::Minor => {
                            let tick_calc = crate::widgets::waveform::TimelineRuleTick::new(
                                self.timeline_viewport.total_duration,
                                self.zoom,
                            );
                            let interval = tick_calc.get_sub_tick_interval();
                            let target_tick = (current_secs / interval).floor() * interval;
                            if quick_press || (current_secs - target_tick).abs() < 0.001 {
                                new_pos_secs = target_tick - interval;
                            } else {
                                new_pos_secs = target_tick;
                            }
                        }
                        MoveMode::Small => {
                            let visible_duration = stats.duration_secs as f32 / self.zoom;
                            new_pos_secs = current_secs - (visible_duration * 0.01);
                        }
                    }
                    let new_pos = (new_pos_secs.max(0.0) / stats.duration_secs as f32).min(1.0);
                    return Task::perform(async move { new_pos }, Message::SeekPlayback);
                }
            }
            Message::MoveCursorRight(mode) => {
                if let Some(stats) = &self.reference_stats {
                    let current_secs = if self.is_playing {
                        if let Some(start_instant) = self.playback_start_instant {
                            self.playback_start_pos * stats.duration_secs as f32
                                + start_instant.elapsed().as_secs_f32()
                        } else {
                            self.playback_pos * stats.duration_secs as f32
                        }
                    } else {
                        self.playback_pos * stats.duration_secs as f32
                    };
                    let new_pos_secs;
                    match mode {
                        MoveMode::Boundary => new_pos_secs = stats.duration_secs as f32,
                        MoveMode::Major => {
                            let tick_calc = crate::widgets::waveform::TimelineRuleTick::new(
                                self.timeline_viewport.total_duration,
                                self.zoom,
                            );
                            let interval = tick_calc.get_tick_interval();
                            let target_tick = (current_secs / interval).ceil() * interval;
                            if target_tick - current_secs < interval * 0.15 {
                                new_pos_secs = target_tick + interval;
                            } else {
                                new_pos_secs = target_tick;
                            }
                        }
                        MoveMode::Minor => {
                            let tick_calc = crate::widgets::waveform::TimelineRuleTick::new(
                                self.timeline_viewport.total_duration,
                                self.zoom,
                            );
                            let interval = tick_calc.get_sub_tick_interval();
                            let target_tick = (current_secs / interval).ceil() * interval;
                            if target_tick - current_secs < interval * 0.15 {
                                new_pos_secs = target_tick + interval;
                            } else {
                                new_pos_secs = target_tick;
                            }
                        }
                        MoveMode::Small => {
                            let visible_duration = stats.duration_secs as f32 / self.zoom;
                            new_pos_secs = current_secs + (visible_duration * 0.01);
                        }
                    }
                    let new_pos = (new_pos_secs.max(0.0) / stats.duration_secs as f32).min(1.0);
                    return Task::perform(async move { new_pos }, Message::SeekPlayback);
                }
            }
            Message::Batch(messages) => {
                let mut tasks = Vec::new();
                for msg in messages {
                    tasks.push(self.update(msg));
                }
                return Task::batch(tasks);
            }
            Message::None => {}
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let bold_font = Font { weight: font::Weight::Bold, ..Default::default() };
        let both_loaded = self.reference_data.is_some() && self.target_data.is_some();

        let header = container(
            row![
                column![
                    text("DubSync").size(24).font(bold_font),
                    text("SYNCHRONIZATION SUITE").size(10).color(Color::from_rgb8(150, 150, 150)),
                ],
                horizontal_space(),
                button(text(char::from(Icon::Settings).to_string()).font(LUCIDE).size(20))
                    .padding([10, 15])
                    .style(|_, _| button::Style {
                        background: Some(iced::Background::Color(Color::TRANSPARENT)),
                        text_color: Color::from_rgb8(150, 150, 150),
                        ..Default::default()
                    }),
            ]
            .align_y(Alignment::Center),
        )
        .padding(20)
        .width(Length::Fill);

        let toolbar = container(
            row![
                row![
                    button(text(char::from(Icon::FolderOpen).to_string()).font(LUCIDE).size(20))
                        .on_press(Message::LoadProject)
                        .padding(8)
                        .style(|_, status| button::Style {
                            background: match status {
                                button::Status::Hovered =>
                                    Some(iced::Background::Color(Color::from_rgb8(45, 45, 45))),
                                _ => None,
                            },
                            text_color: Color::WHITE,
                            ..Default::default()
                        }),
                    button(text(char::from(Icon::Save).to_string()).font(LUCIDE).size(20))
                        .on_press(Message::SaveProject)
                        .padding(8)
                        .style(|_, status| button::Style {
                            background: match status {
                                button::Status::Hovered =>
                                    Some(iced::Background::Color(Color::from_rgb8(45, 45, 45))),
                                _ => None,
                            },
                            text_color: Color::WHITE,
                            ..Default::default()
                        }),
                ]
                .spacing(5),
                container(horizontal_space())
                    .width(Length::Fixed(1.0))
                    .height(Length::Fixed(24.0))
                    .style(|_| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgb8(40, 40, 40))),
                        ..Default::default()
                    }),
                button(text(char::from(Icon::Zap).to_string()).font(LUCIDE).size(20))
                    .on_press_maybe((!self.is_analyzing && both_loaded).then_some(Message::Analyze))
                    .padding(8)
                    .style(move |_, status| button::Style {
                        background: match status {
                            button::Status::Hovered =>
                                Some(iced::Background::Color(Color::from_rgb8(45, 45, 45))),
                            _ => None,
                        },
                        text_color: if both_loaded {
                            Color::from_rgb8(0, 200, 200)
                        } else {
                            Color::from_rgb8(100, 100, 100)
                        },
                        ..Default::default()
                    }),
                horizontal_space(),
                button(
                    text(
                        char::from(match self.timeline_viewport.mode {
                            TimelineMode::Split => Icon::AudioLines,
                            TimelineMode::Mirrored => Icon::ZodiacAquarius,
                        })
                        .to_string()
                    )
                    .font(LUCIDE)
                    .size(20)
                )
                .on_press(Message::ToggleTimelineMode)
                .padding(8)
                .style(|_, status| button::Style {
                    background: match status {
                        button::Status::Hovered =>
                            Some(iced::Background::Color(Color::from_rgb8(45, 45, 45))),
                        button::Status::Pressed =>
                            Some(iced::Background::Color(Color::from_rgb8(30, 30, 30))),
                        _ => None,
                    },
                    text_color: Color::from_rgb8(180, 180, 180),
                    border: Border { radius: 4.0.into(), ..Default::default() },
                    ..Default::default()
                })
            ]
            .padding([0, 10])
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(48.0))
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb8(20, 20, 20))),
            border: Border {
                color: Color::from_rgb8(30, 30, 30),
                width: 1.0,
                ..Default::default()
            },
            ..Default::default()
        });

        let timeline_canvas = container(
            Canvas::new(&self.timeline_viewport).width(Length::Fill).height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb8(15, 15, 15))),
            ..Default::default()
        });

        let reference_overlay = self.render_track_overlay(
            "REFERENCE",
            self.project.reference_path.as_deref(),
            self.reference_stats.as_ref(),
            Message::UploadReference,
            Message::UnloadReference,
            self.ref_loading.clone(),
        );

        let target_overlay = self.render_track_overlay(
            "TARGET",
            self.project.target_path.as_deref(),
            self.target_stats.as_ref(),
            Message::UploadTarget,
            Message::UnloadTarget,
            self.target_loading.clone(),
        );

        let overlays = column![
            reference_overlay,
            container(horizontal_space()).height(Length::Fixed(28.0)), // Matches ruler + margins
            target_overlay,
        ]
        .width(Length::Fill)
        .height(Length::Fill);

        let any_loaded = self.reference_cache.is_some() || self.target_cache.is_some();

        let main_content: Element<'_, Message> = column![
            container(toolbar).padding(Padding { top: 10.0, right: 0.0, bottom: 0.0, left: 0.0 }),
            stack![timeline_canvas, overlays,].width(Length::Fill).height(Length::Fill),
            container(horizontal_space()).width(Length::Fill).height(Length::Fixed(1.0)).style(
                |_| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgb8(30, 30, 30))),
                    ..Default::default()
                }
            ),
            if any_loaded {
                container(Canvas::new(&self.navigator).width(Length::Fill).height(Length::Fill))
                    .width(Length::Fill)
                    .height(Length::Fixed(60.0))
                    .padding(0)
                    .style(|_| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgb8(15, 15, 15))),
                        ..Default::default()
                    })
            } else {
                container(horizontal_space()).height(Length::Shrink)
            }
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into();

        let main_row = row![column![main_content].width(Length::Fill).height(Length::Fill)]
            .height(Length::Fill);

        let current_time_content =
            if let Some(stats) = self.reference_stats.as_ref().or(self.target_stats.as_ref()) {
                let ts = self.playback_pos * stats.duration_secs as f32;
                let h = (ts / 3600.0) as u32;
                let m = ((ts % 3600.0) / 60.0) as u32;
                let s = (ts % 60.0) as u32;
                let c = ((ts % 1.0) * 100.0) as u32;

                let dark_grey = Color::from_rgb8(100, 100, 100);
                let light_grey = Color::from_rgb8(200, 200, 200);

                row![
                    text(format!("{:02}:", h)).color(dark_grey).font(GEIST_MONO).size(18),
                    text(format!("{:02}:{:02}", m, s)).color(light_grey).font(GEIST_MONO).size(18),
                    text(format!(":{:02}", c)).color(dark_grey).font(GEIST_MONO).size(18),
                ]
                .spacing(0)
            } else {
                let dark_grey = Color::from_rgb8(100, 100, 100);
                row![text("00:00:00:00").color(dark_grey).font(GEIST_MONO).size(18)]
            };

        let footer = container(
            row![
                container(current_time_content)
                    .padding([8, 0])
                    .style(|_| container::Style::default()), // Transparent background
                horizontal_space(),
                row![
                    button(text(char::from(Icon::Repeat).to_string()).font(LUCIDE).size(18).color(
                        if self.playback_mode == PlaybackMode::Loop {
                            Color::from_rgb8(0, 200, 200)
                        } else {
                            Color::WHITE
                        }
                    ))
                    .on_press(Message::TogglePlaybackMode)
                    .padding(10)
                    .style(|_, _| button::Style::default()),
                    button(
                        text(
                            char::from(if self.is_playing { Icon::Pause } else { Icon::Play })
                                .to_string()
                        )
                        .font(LUCIDE)
                        .size(32)
                        .color(Color::from_rgb8(0, 200, 200))
                    )
                    .on_press(Message::TogglePlay)
                    .padding(10)
                    .style(|_, status| button::Style {
                        background: None,
                        text_color: match status {
                            button::Status::Hovered => Color::from_rgb8(0, 255, 255),
                            _ => Color::from_rgb8(0, 200, 200),
                        },
                        ..Default::default()
                    }),
                    button(
                        text(char::from(Icon::Square).to_string())
                            .font(LUCIDE)
                            .size(18)
                            .color(Color::WHITE)
                    )
                    .on_press(Message::Stop)
                    .padding(10)
                    .style(|_, _| button::Style::default()),
                ]
                .align_y(Alignment::Center)
                .spacing(15),
                container(horizontal_space())
                    .width(Length::Fixed(1.0))
                    .height(Length::Fixed(30.0))
                    .style(|_| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgb8(40, 40, 40))),
                        ..Default::default()
                    }),
                horizontal_space(),
                row![
                    button(
                        text(char::from(Icon::ZoomOut).to_string())
                            .font(LUCIDE)
                            .size(20)
                            .color(Color::from_rgb8(150, 150, 150))
                    )
                    .on_press(Message::ZoomOut)
                    .style(|_, _| button::Style::default()),
                    {
                        let max_z = (self.timeline_viewport.total_duration / 5.0).max(1.0);
                        let zl = if max_z > 1.0 { self.zoom.log10() / max_z.log10() } else { 0.0 };
                        slider(0..=100, (zl * 100.0).round() as u32, Message::ZoomChanged)
                            .width(Length::Fixed(150.0))
                            .style(|_, _| slider::Style {
                                rail: slider::Rail {
                                    backgrounds: (
                                        iced::Background::Color(Color::from_rgb8(0, 200, 200)),
                                        iced::Background::Color(Color::from_rgb8(40, 40, 40)),
                                    ),
                                    width: 4.0,
                                    border: Border::default(),
                                },
                                handle: slider::Handle {
                                    shape: slider::HandleShape::Circle { radius: 7.0 },
                                    background: iced::Background::Color(Color::WHITE),
                                    border_width: 1.0,
                                    border_color: Color::from_rgb8(0, 200, 200),
                                },
                            })
                    },
                    button(
                        text(char::from(Icon::ZoomIn).to_string())
                            .font(LUCIDE)
                            .size(20)
                            .color(Color::from_rgb8(150, 150, 150))
                    )
                    .on_press(Message::ZoomIn)
                    .style(|_, _| button::Style::default()),
                    {
                        let max_z = (self.timeline_viewport.total_duration / 5.0).max(1.0);
                        let zl = if max_z > 1.0 { self.zoom.log10() / max_z.log10() } else { 0.0 };
                        text(format!("{}%", (zl * 100.0).round() as i32))
                            .size(16)
                            .font(GEIST_MONO)
                            .width(Length::Fixed(50.0))
                            .color(Color::from_rgb8(150, 150, 150))
                    },
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                container(horizontal_space())
                    .width(Length::Fixed(1.0))
                    .height(Length::Fixed(30.0))
                    .style(|_| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgb8(40, 40, 40))),
                        ..Default::default()
                    }),
                row![
                    self.speed_option("Slow", PlaybackSpeed::Slow, Color::from_rgb8(100, 150, 255)),
                    self.speed_option(
                        "1.0x",
                        PlaybackSpeed::Normal,
                        Color::from_rgb8(150, 150, 150)
                    ),
                    self.speed_option("Fast", PlaybackSpeed::Fast, Color::from_rgb8(255, 150, 50))
                ]
                .spacing(15)
                .align_y(Alignment::Center),
            ]
            .align_y(Alignment::Center)
            .padding([10, 20]),
        )
        .width(Length::Fill)
        .height(Length::Shrink)
        .style(|_| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb8(10, 10, 10))),
            border: Border {
                color: Color::from_rgb8(30, 30, 30),
                width: 1.0,
                ..Default::default()
            },
            ..Default::default()
        });

        column![header, main_row, footer].width(Length::Fill).height(Length::Fill).into()
    }

    fn render_track_overlay<'a>(
        &self,
        label: &'a str,
        path: Option<&'a std::path::Path>,
        stats: Option<&'a AudioStats>,
        on_upload: Message,
        on_unload: Message,
        loading: TrackLoadingState,
    ) -> Element<'a, Message> {
        let bold_font = Font { weight: font::Weight::Bold, ..Default::default() };
        let regular_font = crate::theme::GEIST_REGULAR;

        let content: Element<'a, Message> = if let Some(p) = path {
            let filename = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Unknown");
            let stats_text = if let Some(err) = &loading.error_message {
                format!("Error: {}", err)
            } else if loading.is_active {
                loading.step_name.clone()
            } else if let Some(s) = stats {
                Self::format_sub_label(s)
            } else {
                "Loading...".to_string()
            };

            let card = container(
                column![
                    row![
                        stack![
                            container(
                                text(filename)
                                    .size(14)
                                    .font(bold_font)
                                    .color(Color::from_rgba8(0, 0, 0, 0.8))
                                    .width(Length::Shrink)
                            )
                            .padding([1, 1]),
                            text(filename)
                                .size(14)
                                .font(bold_font)
                                .color(Color::WHITE)
                                .width(Length::Fill)
                        ],
                        button(text(char::from(Icon::X).to_string()).font(LUCIDE).size(12))
                            .on_press(on_unload.clone())
                            .padding(5)
                            .style(|_, status| button::Style {
                                background: None,
                                text_color: match status {
                                    button::Status::Hovered => Color::from_rgb8(220, 220, 220),
                                    _ => Color::WHITE,
                                },
                                ..Default::default()
                            })
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                    stack![
                        container(
                            text(stats_text.clone())
                                .size(10)
                                .font(regular_font)
                                .color(Color::from_rgba8(0, 0, 0, 0.8))
                                .width(Length::Shrink)
                        )
                        .padding([1, 1]),
                        text(stats_text)
                            .size(10)
                            .font(regular_font)
                            .color(Color::from_rgb8(200, 200, 200))
                            .width(Length::Shrink)
                    ]
                ]
                .spacing(2),
            )
            .padding(10);

            let is_reference = label == "REFERENCE";
            let alignment = if is_reference {
                iced::alignment::Vertical::Top
            } else {
                iced::alignment::Vertical::Bottom
            };

            let info_layer = container(card)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(10)
                .align_x(iced::alignment::Horizontal::Left)
                .align_y(alignment);

            if loading.is_active || loading.error_message.is_some() {
                let is_target = label == "TARGET";

                if let Some(err) = loading.error_message {
                    let error_layer = container(
                        column![
                            text("Error Loading Audio")
                                .size(14)
                                .font(bold_font)
                                .color(Color::from_rgb8(255, 100, 100)),
                            text(err).size(12).color(Color::from_rgb8(220, 180, 180)),
                            button(text("Dismiss").size(12)).on_press(on_unload).padding(6).style(
                                |_, _| button::Style {
                                    background: Some(iced::Background::Color(Color::from_rgb8(
                                        100, 40, 40
                                    ))),
                                    text_color: Color::WHITE,
                                    border: Border { radius: 4.0.into(), ..Default::default() },
                                    ..Default::default()
                                }
                            ),
                        ]
                        .spacing(8)
                        .align_x(Alignment::Center),
                    )
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .padding(20)
                    .align_x(iced::alignment::Horizontal::Center)
                    .align_y(iced::alignment::Vertical::Center)
                    .style(move |_| container::Style {
                        background: Some(iced::Background::Color(Color::from_rgba8(
                            40, 20, 20, 0.9,
                        ))),
                        ..Default::default()
                    });

                    stack![info_layer, error_layer].into()
                } else {
                    let overall_progress =
                        (loading.step as f32 - 1.0 + loading.percent) / loading.total as f32;
                    let step_text = format!(
                        "{} · Step {}/{} · {:.0}%",
                        loading.step_name,
                        loading.step,
                        loading.total,
                        loading.percent * 100.0
                    );

                    let progress_color =
                        if is_target { Color::from_rgb8(0, 255, 255) } else { Color::WHITE };

                    let progress_layer = container(
                        column![
                            stack![
                                container(
                                    text(step_text.clone())
                                        .size(12)
                                        .font(bold_font)
                                        .color(Color::from_rgba8(0, 0, 0, 0.8))
                                        .width(Length::Shrink)
                                )
                                .padding([1, 1]),
                                text(step_text)
                                    .size(12)
                                    .font(bold_font)
                                    .color(progress_color)
                                    .width(Length::Shrink)
                            ],
                            progress_bar(0.0..=1.0, overall_progress)
                                .height(Length::Fixed(10.0))
                                .style(move |_| progress_bar::Style {
                                    background: iced::Background::Color(Color::from_rgba8(
                                        20, 20, 20, 0.8
                                    )),
                                    bar: iced::Background::Color(progress_color),
                                    border: Border { radius: 5.0.into(), ..Default::default() },
                                })
                        ]
                        .spacing(12)
                        .align_x(Alignment::Center),
                    )
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .padding(Padding { top: 10.0, bottom: 30.0, left: 10.0, right: 10.0 })
                    .align_x(iced::alignment::Horizontal::Center)
                    .align_y(iced::alignment::Vertical::Center);

                    stack![info_layer, progress_layer].into()
                }
            } else {
                info_layer.into()
            }
        } else {
            let is_target = label == "TARGET";
            let dropzone = button(
                container(
                    column![
                        text(char::from(Icon::Plus).to_string()).font(LUCIDE).size(42).color(
                            if is_target { Color::from_rgb8(0, 255, 255) } else { Color::WHITE }
                        ),
                        column![
                            text(format!("Click to load {} track", label.to_lowercase()))
                                .size(14)
                                .font(bold_font)
                                .color(Color::WHITE),
                            text("or drag and drop audio file here")
                                .size(11)
                                .color(Color::from_rgb8(200, 200, 200)),
                        ]
                        .spacing(4)
                        .align_x(Alignment::Center)
                    ]
                    .spacing(16)
                    .align_x(Alignment::Center),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced::alignment::Horizontal::Center)
                .align_y(iced::alignment::Vertical::Center)
                .style(move |_theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgba8(40, 40, 40, 0.3))),
                    border: Border {
                        color: if is_target {
                            Color::from_rgba8(0, 200, 200, 0.4)
                        } else {
                            Color::from_rgba8(150, 150, 150, 0.4)
                        },
                        width: 1.0,
                        radius: 12.0.into(),
                    },
                    ..Default::default()
                }),
            )
            .on_press(on_upload)
            .style(move |_, status| button::Style {
                background: match status {
                    button::Status::Hovered => {
                        Some(iced::Background::Color(Color::from_rgba8(60, 60, 60, 0.2)))
                    }
                    _ => None,
                },
                ..Default::default()
            });

            container(dropzone)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(0)
                .align_x(iced::alignment::Horizontal::Center)
                .align_y(iced::alignment::Vertical::Center)
                .into()
        };

        container(content).width(Length::Fill).height(Length::Fill).into()
    }

    pub fn speed_option<'a>(
        &self,
        label: &'a str,
        speed: PlaybackSpeed,
        dot_color: Color,
    ) -> Element<'a, Message> {
        let is_selected = self.speed == speed;
        button(
            row![
                container(horizontal_space())
                    .width(Length::Fixed(8.0))
                    .height(Length::Fixed(8.0))
                    .style(move |_| container::Style {
                        background: Some(iced::Background::Color(dot_color)),
                        border: Border { radius: 4.0.into(), ..Default::default() },
                        ..Default::default()
                    }),
                text(label).size(12).color(if is_selected {
                    Color::WHITE
                } else {
                    Color::from_rgb8(150, 150, 150)
                })
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .on_press(Message::SpeedChanged(speed))
        .style(|_, _| button::Style {
            background: Some(iced::Background::Color(Color::TRANSPARENT)),
            ..Default::default()
        })
        .into()
    }

    fn format_sub_label(stats: &AudioStats) -> String {
        let bit_depth = if stats.bit_depth > 0 {
            format!("{} bit . ", stats.bit_depth)
        } else {
            "".to_string()
        };
        let channel_label = match stats.channels {
            ChannelLayout::Mono => "Mono",
            ChannelLayout::Stereo => "Stereo",
            ChannelLayout::Surround5_1 => "5.1",
            ChannelLayout::Surround7_1 => "7.1",
            ChannelLayout::Other(_) => "Multichannel",
        };
        format!(
            "{} . {}Hz . {}{} . {}",
            stats.format_duration(),
            stats.sample_rate,
            bit_depth,
            channel_label,
            stats.codec
        )
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }
}
