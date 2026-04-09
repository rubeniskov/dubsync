use crate::audio_loader::WaveformCache;
use crate::message::Message;
use crate::types::TimelineMode;
use iced::keyboard;
use iced::mouse::{self, Cursor};
use iced::widget::canvas::{self, Cache, Event, Frame, Geometry, Path, Program};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

/// Logic for calculating tick positions based on zoom and width.
pub struct TimelineRuleTick {
    pub total_duration: f32,
    pub zoom: f32,
}

impl TimelineRuleTick {
    pub fn new(total_duration: f32, zoom: f32) -> Self {
        Self { total_duration, zoom }
    }

    pub fn get_tick_interval(&self) -> f32 {
        let visible_duration = self.total_duration / self.zoom;
        let raw_interval = visible_duration / 10.0;

        if raw_interval < 0.001 {
            0.001
        } else if raw_interval < 0.002 {
            0.002
        } else if raw_interval < 0.005 {
            0.005
        } else if raw_interval < 0.01 {
            0.01
        } else if raw_interval < 0.02 {
            0.02
        } else if raw_interval < 0.05 {
            0.05
        } else if raw_interval < 0.1 {
            0.1
        } else if raw_interval < 0.2 {
            0.2
        } else if raw_interval < 0.5 {
            0.5
        } else if raw_interval < 1.0 {
            1.0
        } else if raw_interval < 2.0 {
            2.0
        } else if raw_interval < 5.0 {
            5.0
        } else if raw_interval < 10.0 {
            10.0
        } else if raw_interval < 30.0 {
            30.0
        } else if raw_interval < 60.0 {
            60.0
        } else if raw_interval < 300.0 {
            300.0
        } else {
            600.0
        }
    }

    pub fn get_sub_tick_interval(&self) -> f32 {
        self.get_tick_interval() / 5.0
    }

    pub fn snap_to_minor(&self, time_secs: f32) -> f32 {
        let interval = self.get_sub_tick_interval();
        (time_secs / interval).round() * interval
    }
}

/// A modular component representing a single audio track waveform.
pub struct WaveformTrack {
    pub cache: Option<WaveformCache>,
    pub color: Color,
    pub is_reference: bool,
    pub label: String,
    pub sub_label: String,
}

impl WaveformTrack {
    pub fn draw(
        &self,
        frame: &mut Frame,
        bounds: Rectangle,
        zoom: f32,
        offset: f32,
        mode: TimelineMode,
    ) {
        let Some(cache) = &self.cache else { return };
        if cache.peaks.is_empty() {
            return;
        };

        let mid_y = bounds.y + (bounds.height / 2.0);
        let bar_width = 3.0f32;
        let gap = 2.0f32;
        let step = bar_width + gap;

        // Use a few extra bars to cover sub-pixel sliding at edges
        let num_bars = (bounds.width / step).floor() as usize + 2;

        let total_peaks = cache.peaks.len() as f32;
        let visible_peaks = total_peaks / zoom;
        let peaks_per_bar = visible_peaks / (bounds.width / step);

        // JITTER-FREE SAMPLING:
        // Anchor bars to the data's grid, then use a fractional visual offset
        let total_offset_in_bars = offset * total_peaks / peaks_per_bar;
        let first_bar_data_idx = total_offset_in_bars.floor();
        let sub_bar_px_offset = (total_offset_in_bars - first_bar_data_idx) * step;

        let is_mirrored = mode == TimelineMode::Mirrored;

        let path = Path::new(|builder| {
            for i in 0..num_bars {
                let data_idx = first_bar_data_idx + i as f32;
                let current_bar_start = data_idx * peaks_per_bar;
                let start_idx = current_bar_start.floor() as usize;
                let end_idx = (current_bar_start + peaks_per_bar).ceil() as usize;

                if start_idx >= cache.peaks.len() {
                    break;
                }

                let mut min_val = 0.0f32;
                let mut max_val = 0.0f32;

                if start_idx == end_idx || peaks_per_bar < 1.0 {
                    let (_, p_max) = cache.peaks[start_idx.min(cache.peaks.len() - 1)];
                    max_val = p_max;
                    min_val = -p_max; // Assume symmetry for upscaling
                } else {
                    for j in start_idx..end_idx.min(cache.peaks.len()) {
                        let (p_min, p_max) = cache.peaks[j];
                        min_val = min_val.min(p_min);
                        max_val = max_val.max(p_max);
                    }
                }

                // Smooth horizontal position including fractional shift
                let x = (i as f32 * step) - sub_bar_px_offset + (bar_width / 2.0);

                let (y_start, y_end) = if is_mirrored {
                    if self.is_reference {
                        let base_y = bounds.y + bounds.height;
                        (base_y, base_y - max_val.abs() * bounds.height)
                    } else {
                        let base_y = bounds.y;
                        (base_y, base_y + max_val.abs() * bounds.height)
                    }
                } else {
                    (
                        mid_y + (min_val * (bounds.height / 2.0)),
                        mid_y + (max_val * (bounds.height / 2.0)),
                    )
                };

                builder.move_to(Point::new(x, y_start));
                builder.line_to(Point::new(x, y_end));
            }
        });

        frame.stroke(&path, canvas::Stroke::default().with_color(self.color).with_width(bar_width));
    }
}

/// The main synchronized viewport containing tracks and the central ruler.
pub struct TimelineViewport {
    pub reference: WaveformTrack,
    pub target: WaveformTrack,
    pub cache: Cache,
    pub cursor_cache: Cache,
    pub zoom: f32,
    pub offset: f32,
    pub mode: TimelineMode,
    pub playback_pos: f32,
    pub total_duration: f32,
    pub is_playing: bool,
    pub is_loading: bool,
}

impl TimelineViewport {
    pub fn new() -> Self {
        Self {
            reference: WaveformTrack {
                cache: None,
                color: Color::from_rgb8(0, 200, 200),
                is_reference: true,
                label: "Reference Track".to_string(),
                sub_label: "".to_string(),
            },
            target: WaveformTrack {
                cache: None,
                color: Color::from_rgb8(200, 200, 200),
                is_reference: false,
                label: "Target Track".to_string(),
                sub_label: "".to_string(),
            },
            cache: Cache::default(),
            cursor_cache: Cache::default(),
            zoom: 1.0,
            offset: 0.0,
            mode: TimelineMode::Mirrored,
            playback_pos: 0.0,
            total_duration: 0.0,
            is_playing: false,
            is_loading: false,
        }
    }
}

#[derive(Default)]
pub struct State {
    pub interaction: Option<WaveformInteraction>,
    pub keyboard_modifiers: keyboard::Modifiers,
    pub phantom_pos: Option<Point>,
}

pub enum WaveformInteraction {
    Panning {
        start_x: f32,
        start_offset: f32,
        press_position: Point,
        press_time: std::time::Instant,
        was_playing: bool,
    },
    Scrubbing {
        was_playing: bool,
    },
}

impl Program<Message> for TimelineViewport {
    type State = State;

    fn update(
        &self,
        state: &mut Self::State,
        event: Event,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.keyboard_modifiers = modifiers;
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let cursor_position = cursor.position_in(bounds);
                if let Some(position) = cursor_position {
                    let ruler_height = 20.0;
                    let ruler_margin = 4.0;
                    let total_ruler_height = ruler_height + ruler_margin * 2.0;
                    let ruler_area_y = (bounds.height / 2.0) - (total_ruler_height / 2.0);

                    if position.y >= ruler_area_y && position.y <= ruler_area_y + total_ruler_height
                    {
                        state.interaction =
                            Some(WaveformInteraction::Scrubbing { was_playing: self.is_playing });
                        let click_x_norm = position.x / bounds.width;
                        let seek_pos = self.offset + (click_x_norm / self.zoom);
                        let mut messages = vec![Message::SeekPlayback(seek_pos.clamp(0.0, 1.0))];
                        if self.is_playing {
                            messages.push(Message::Pause);
                        }
                        return (canvas::event::Status::Captured, Some(Message::Batch(messages)));
                    } else {
                        state.interaction = Some(WaveformInteraction::Panning {
                            start_x: position.x,
                            start_offset: self.offset,
                            press_position: position,
                            press_time: std::time::Instant::now(),
                            was_playing: self.is_playing,
                        });
                        return (canvas::event::Status::Captured, None);
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let mut message = None;
                if let Some(interaction) = &state.interaction {
                    match interaction {
                        WaveformInteraction::Panning {
                            press_position,
                            press_time,
                            was_playing,
                            ..
                        } => {
                            if let Some(position) = cursor.position_in(bounds) {
                                let dx = position.x - press_position.x;
                                let dy = position.y - press_position.y;
                                if dx * dx + dy * dy < 25.0
                                    && press_time.elapsed().as_millis() < 200
                                {
                                    let mut seek_pos =
                                        self.offset + ((position.x / bounds.width) / self.zoom);
                                    if state.keyboard_modifiers.shift() {
                                        let tick_calc =
                                            TimelineRuleTick::new(self.total_duration, self.zoom);
                                        seek_pos = tick_calc
                                            .snap_to_minor(seek_pos * self.total_duration)
                                            / self.total_duration;
                                    }
                                    if *was_playing {
                                        message = Some(Message::Batch(vec![
                                            Message::SeekPlayback(seek_pos.clamp(0.0, 1.0)),
                                            Message::Resume,
                                        ]));
                                    } else {
                                        message =
                                            Some(Message::SeekPlayback(seek_pos.clamp(0.0, 1.0)));
                                    }
                                }
                            }
                        }
                        WaveformInteraction::Scrubbing { was_playing } => {
                            if *was_playing {
                                message = Some(Message::Resume);
                            }
                        }
                    }
                }
                state.interaction = None;
                return (canvas::event::Status::Captured, message);
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                state.phantom_pos = cursor.position_in(bounds);
                self.cursor_cache.clear();

                if let Some(interaction) = &state.interaction {
                    if let Some(position) = cursor.position() {
                        let position = Point::new(position.x - bounds.x, position.y - bounds.y);
                        match interaction {
                            WaveformInteraction::Panning { start_x, start_offset, .. } => {
                                let delta_x = position.x - start_x;
                                let offset_delta = -(delta_x / bounds.width) / self.zoom;
                                let new_offset = (start_offset + offset_delta)
                                    .clamp(0.0, (1.0 - 1.0 / self.zoom).max(0.0));
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::OffsetChanged(new_offset)),
                                );
                            }
                            WaveformInteraction::Scrubbing { .. } => {
                                let mut click_x_norm = position.x / bounds.width;
                                if state.keyboard_modifiers.shift() {
                                    let tick_calc =
                                        TimelineRuleTick::new(self.total_duration, self.zoom);
                                    let audio_pos = tick_calc.snap_to_minor(
                                        (self.offset + (click_x_norm / self.zoom))
                                            * self.total_duration,
                                    );
                                    click_x_norm =
                                        (audio_pos / self.total_duration - self.offset) * self.zoom;
                                }
                                let seek_pos = self.offset + (click_x_norm / self.zoom);
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::SeekPlayback(seek_pos.clamp(0.0, 1.0))),
                                );
                            }
                        }
                    }
                }
                return (canvas::event::Status::Captured, None);
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if let Some(position) = cursor.position_in(bounds) {
                    let max_zoom = (self.total_duration / 5.0).max(1.0);
                    match delta {
                        mouse::ScrollDelta::Lines { x, y } => {
                            if x.abs() > y.abs() {
                                let new_offset = (self.offset - x * 0.01)
                                    .clamp(0.0, (1.0 - 1.0 / self.zoom).max(0.0));
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::OffsetChanged(new_offset)),
                                );
                            } else if state.keyboard_modifiers.shift() {
                                let new_zoom = (self.zoom * (1.0 + y * 0.05)).clamp(1.0, max_zoom);
                                let cursor_x_norm = position.x / bounds.width;
                                let audio_pos = self.offset + cursor_x_norm / self.zoom;
                                let new_offset = (audio_pos - cursor_x_norm / new_zoom)
                                    .clamp(0.0, (1.0 - 1.0 / new_zoom).max(0.0));
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::ZoomAndOffsetChanged(new_zoom, new_offset)),
                                );
                            } else {
                                let new_offset = (self.offset - y * 0.01)
                                    .clamp(0.0, (1.0 - 1.0 / self.zoom).max(0.0));
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::OffsetChanged(new_offset)),
                                );
                            }
                        }
                        mouse::ScrollDelta::Pixels { x, y } => {
                            if x.abs() > y.abs() {
                                let new_offset = (self.offset - x * 0.0005)
                                    .clamp(0.0, (1.0 - 1.0 / self.zoom).max(0.0));
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::OffsetChanged(new_offset)),
                                );
                            } else if state.keyboard_modifiers.shift() {
                                let new_zoom =
                                    (self.zoom * (1.0 + (y / 20.0) * 0.05)).clamp(1.0, max_zoom);
                                let cursor_x_norm = position.x / bounds.width;
                                let audio_pos = self.offset + cursor_x_norm / self.zoom;
                                let new_offset = (audio_pos - cursor_x_norm / new_zoom)
                                    .clamp(0.0, (1.0 - 1.0 / new_zoom).max(0.0));
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::ZoomAndOffsetChanged(new_zoom, new_offset)),
                                );
                            } else {
                                let new_offset = (self.offset - y * 0.0005)
                                    .clamp(0.0, (1.0 - 1.0 / self.zoom).max(0.0));
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::OffsetChanged(new_offset)),
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        (canvas::event::Status::Ignored, None)
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> Vec<Geometry> {
        let ruler_height = 20.0;
        let ruler_margin = 4.0;
        let total_ruler_height = ruler_height + ruler_margin * 2.0;

        let mid_y = bounds.height / 2.0;
        let ruler_y = mid_y - (ruler_height / 2.0);
        let ruler_area_y = mid_y - (total_ruler_height / 2.0);

        let geom = self.cache.draw(renderer, bounds.size(), |frame| {
            // Background
            frame.fill(
                &Path::rectangle(Point::ORIGIN, bounds.size()),
                canvas::Fill::from(Color::from_rgb8(15, 15, 15)),
            );

            // 1. Draw central TimelineRule
            let ruler_rect =
                Path::rectangle(Point::new(0.0, ruler_y), Size::new(bounds.width, ruler_height));
            frame.fill(&ruler_rect, canvas::Fill::from(Color::from_rgb8(25, 25, 25)));

            let tick_calc = TimelineRuleTick::new(self.total_duration, self.zoom);
            let interval = tick_calc.get_tick_interval();
            let start_time = self.offset * self.total_duration;
            let visible_duration = self.total_duration / self.zoom;

            let mut current_tick = (start_time / interval).floor() * interval - interval;
            while current_tick <= start_time + visible_duration + interval {
                let x = ((current_tick - start_time) / visible_duration) * bounds.width;
                if x >= -100.0 && x <= bounds.width + 100.0 {
                    // Major tick (centered, brighter)
                    let tick_path =
                        Path::line(Point::new(x, ruler_y), Point::new(x, ruler_y + ruler_height));
                    frame.stroke(
                        &tick_path,
                        canvas::Stroke::default()
                            .with_color(Color::from_rgb8(100, 100, 100))
                            .with_width(1.0),
                    );

                    let mins = ((current_tick % 3600.0) / 60.0) as i32;
                    let secs = (current_tick % 60.0) as i32;

                    // Time label - centered vertically, aligned to the right of the tick (brighter)
                    let label = format!("{:02}:{:02}", mins, secs);
                    frame.fill_text(canvas::Text {
                        content: label,
                        position: Point::new(x + 4.0, ruler_y + (ruler_height / 2.0)),
                        color: Color::from_rgb8(160, 160, 160),
                        size: 10.0.into(),
                        horizontal_alignment: iced::alignment::Horizontal::Left,
                        vertical_alignment: iced::alignment::Vertical::Center,
                        ..Default::default()
                    });

                    // Centered Minor ticks (subtle but clear)
                    let sub_tick_interval = tick_calc.get_sub_tick_interval();
                    let sub_tick_height = 6.0;
                    let sub_tick_y = ruler_y + (ruler_height - sub_tick_height) / 2.0;
                    for j in 1..5 {
                        let sub_tick_time = current_tick + (j as f32 * sub_tick_interval);
                        let sub_x =
                            ((sub_tick_time - start_time) / visible_duration) * bounds.width;
                        if sub_x >= 0.0 && sub_x <= bounds.width {
                            let sub_tick_path = Path::line(
                                Point::new(sub_x, sub_tick_y),
                                Point::new(sub_x, sub_tick_y + sub_tick_height),
                            );
                            frame.stroke(
                                &sub_tick_path,
                                canvas::Stroke::default()
                                    .with_color(Color::from_rgb8(60, 60, 60))
                                    .with_width(1.0),
                            );
                        }
                    }
                }
                current_tick += interval;
            }

            // 2. Draw tracks
            self.reference.draw(
                frame,
                Rectangle { x: 0.0, y: 0.0, width: bounds.width, height: ruler_area_y },
                self.zoom,
                self.offset,
                self.mode,
            );
            self.target.draw(
                frame,
                Rectangle {
                    x: 0.0,
                    y: ruler_area_y + total_ruler_height,
                    width: bounds.width,
                    height: bounds.height - (ruler_area_y + total_ruler_height),
                },
                self.zoom,
                self.offset,
                self.mode,
            );
        });

        let cursor_geom = self.cursor_cache.draw(renderer, bounds.size(), |frame| {
            if self.is_loading {
                return;
            }
            let cursor_relative_x = (self.playback_pos - self.offset) * self.zoom;
            if (0.0..=1.0).contains(&cursor_relative_x) {
                let x = cursor_relative_x * bounds.width;
                frame.stroke(
                    &Path::line(Point::new(x, 0.0), Point::new(x, bounds.height)),
                    canvas::Stroke::default()
                        .with_color(Color::from_rgb8(0, 255, 255))
                        .with_width(2.0),
                );
            }
            if let Some(mut phantom) = state.phantom_pos {
                if state.keyboard_modifiers.shift() {
                    let tick_calc = TimelineRuleTick::new(self.total_duration, self.zoom);
                    let audio_pos = tick_calc.snap_to_minor(
                        (self.offset + (phantom.x / bounds.width) / self.zoom)
                            * self.total_duration,
                    );
                    phantom.x = ((audio_pos - (self.offset * self.total_duration))
                        / (self.total_duration / self.zoom))
                        * bounds.width;
                }
                frame.stroke(
                    &Path::line(Point::new(phantom.x, 0.0), Point::new(phantom.x, bounds.height)),
                    canvas::Stroke::default()
                        .with_color(Color::from_rgba8(150, 150, 150, 0.5))
                        .with_width(1.0),
                );
            }
        });

        vec![geom, cursor_geom]
    }
}

pub struct Navigator {
    pub reference: Option<WaveformCache>,
    pub target: Option<WaveformCache>,
    pub reference_color: Color,
    pub target_color: Color,
    pub cache: Cache,
    pub cursor_cache: Cache,
    pub zoom: f32,
    pub offset: f32,
    pub playback_pos: f32,
    pub mode: TimelineMode,
    pub total_duration: f32,
    pub is_playing: bool,
    pub is_loading: bool,
}

impl Navigator {
    pub fn new() -> Self {
        Self {
            reference: None,
            target: None,
            reference_color: Color::from_rgba8(0, 200, 200, 0.3),
            target_color: Color::from_rgba8(200, 200, 200, 0.3),
            cache: Cache::default(),
            cursor_cache: Cache::default(),
            zoom: 1.0,
            offset: 0.0,
            mode: TimelineMode::Mirrored,
            playback_pos: 0.0,
            total_duration: 0.0,
            is_playing: false,
            is_loading: false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_full_peaks(
        &self,
        frame: &mut Frame,
        cache: &WaveformCache,
        color: Color,
        size: Size,
        y_min_norm: f32,
        y_max_norm: f32,
        is_reference: bool,
        x_offset: f32,
    ) {
        if cache.peaks.is_empty() {
            return;
        }
        let draw_height = size.height * (y_max_norm - y_min_norm);
        let mid_y = size.height * y_min_norm + (draw_height / 2.0);
        let step = 3.0; // Fixed step for nav
        let num_bars = (size.width / step).floor() as usize;
        let peaks_per_bar = cache.peaks.len() as f32 / num_bars as f32;

        let path = Path::new(|builder| {
            for i in 0..num_bars {
                let start_idx = (i as f32 * peaks_per_bar).floor() as usize;
                let (_, p_max) = cache.peaks[start_idx.min(cache.peaks.len() - 1)];
                let x = x_offset + i as f32 * step + 1.0;

                // FORCE Mirrored mode for Navigator
                let (y_start, y_end) = if is_reference {
                    (mid_y, mid_y - p_max.abs() * (draw_height / 2.0))
                } else {
                    (mid_y, mid_y + p_max.abs() * (draw_height / 2.0))
                };

                builder.move_to(Point::new(x, y_start));
                builder.line_to(Point::new(x, y_end));
            }
        });
        frame.stroke(&path, canvas::Stroke::default().with_color(color).with_width(2.0));
    }
}

#[derive(Default)]
pub struct NavState {
    pub interaction: Option<Interaction>,
}

pub enum Interaction {
    Dragging { start_x: f32, start_offset: f32 },
    ResizingLeft { start_x: f32, start_offset: f32, start_zoom: f32 },
    ResizingRight { start_x: f32, start_offset: f32, start_zoom: f32 },
}

const NAV_PADDING: f32 = 10.0;
const NAV_HANDLE_WIDTH: f32 = 6.0;

impl Program<Message> for Navigator {
    type State = NavState;

    fn update(
        &self,
        state: &mut Self::State,
        event: Event,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        let usable_width = bounds.width - 2.0 * NAV_PADDING;
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(position) = cursor.position_in(bounds) {
                    let vw = usable_width / self.zoom;
                    let vx = NAV_PADDING + self.offset * usable_width;

                    if position.x >= vx - NAV_HANDLE_WIDTH && position.x <= vx {
                        state.interaction = Some(Interaction::ResizingLeft {
                            start_x: position.x,
                            start_offset: self.offset,
                            start_zoom: self.zoom,
                        });
                    } else if position.x >= vx + vw && position.x <= vx + vw + NAV_HANDLE_WIDTH {
                        state.interaction = Some(Interaction::ResizingRight {
                            start_x: position.x,
                            start_offset: self.offset,
                            start_zoom: self.zoom,
                        });
                    } else if position.x > vx && position.x < vx + vw {
                        state.interaction = Some(Interaction::Dragging {
                            start_x: position.x,
                            start_offset: self.offset,
                        });
                    } else {
                        let new_offset = ((position.x - NAV_PADDING) / usable_width
                            - 0.5 / self.zoom)
                            .clamp(0.0, (1.0 - 1.0 / self.zoom).max(0.0));
                        state.interaction = Some(Interaction::Dragging {
                            start_x: position.x,
                            start_offset: new_offset,
                        });
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::OffsetChanged(new_offset)),
                        );
                    }
                    return (canvas::event::Status::Captured, None);
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.interaction = None;
                return (canvas::event::Status::Captured, None);
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                self.cursor_cache.clear();
                if let Some(interaction) = &state.interaction {
                    if let Some(pos) = cursor.position() {
                        let pos = Point::new(pos.x - bounds.x, pos.y - bounds.y);
                        match interaction {
                            Interaction::Dragging { start_x, start_offset } => {
                                let new_offset = (start_offset + (pos.x - start_x) / usable_width)
                                    .clamp(0.0, (1.0 - 1.0 / self.zoom).max(0.0));
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::OffsetChanged(new_offset)),
                                );
                            }
                            Interaction::ResizingLeft { start_x, start_offset, start_zoom } => {
                                let dx = pos.x - start_x;
                                let max_z = (self.total_duration / 5.0).max(1.0);
                                let min_vw = usable_width / max_z;

                                let right_edge = (start_offset + 1.0 / start_zoom) * usable_width;
                                let tentative_vx = (start_offset * usable_width + dx).max(0.0);
                                let new_vw = (right_edge - tentative_vx).clamp(min_vw, right_edge);
                                let final_vx = right_edge - new_vw;

                                let new_z = (usable_width / new_vw).clamp(1.0, max_z);
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::ZoomAndOffsetChanged(
                                        new_z,
                                        final_vx / usable_width,
                                    )),
                                );
                            }
                            Interaction::ResizingRight { start_x, start_offset, start_zoom } => {
                                let dx = pos.x - start_x;
                                let max_z = (self.total_duration / 5.0).max(1.0);
                                let min_vw = usable_width / max_z;

                                let left_edge = start_offset * usable_width;
                                let tentative_vw = usable_width / start_zoom + dx;
                                let new_vw = tentative_vw.clamp(min_vw, usable_width - left_edge);

                                let new_z = (usable_width / new_vw).clamp(1.0, max_z);
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::ZoomAndOffsetChanged(new_z, *start_offset)),
                                );
                            }
                        }
                    }
                }
                return (canvas::event::Status::Captured, None);
            }
            _ => {}
        }
        (canvas::event::Status::Ignored, None)
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> mouse::Interaction {
        let usable_width = bounds.width - 2.0 * NAV_PADDING;
        if let Some(interaction) = &state.interaction {
            match interaction {
                Interaction::Dragging { .. } => return mouse::Interaction::Grab,
                Interaction::ResizingLeft { .. } | Interaction::ResizingRight { .. } => {
                    return mouse::Interaction::ResizingHorizontally;
                }
            }
        }
        if let Some(p) = cursor.position_in(bounds) {
            let vw = usable_width / self.zoom;
            let vx = NAV_PADDING + self.offset * usable_width;

            if (p.x >= vx - NAV_HANDLE_WIDTH && p.x <= vx)
                || (p.x >= vx + vw && p.x <= vx + vw + NAV_HANDLE_WIDTH)
            {
                return mouse::Interaction::ResizingHorizontally;
            }
            if p.x > vx && p.x < vx + vw {
                return mouse::Interaction::Grab;
            }
        }
        mouse::Interaction::default()
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> Vec<Geometry> {
        let usable_width = bounds.width - 2.0 * NAV_PADDING;
        let geom = self.cache.draw(renderer, bounds.size(), |frame| {
            if let Some(c) = &self.reference {
                self.draw_full_peaks(
                    frame,
                    c,
                    self.reference_color,
                    Size::new(usable_width, bounds.height),
                    0.0,
                    1.0,
                    true,
                    NAV_PADDING,
                );
            }
            if let Some(c) = &self.target {
                self.draw_full_peaks(
                    frame,
                    c,
                    self.target_color,
                    Size::new(usable_width, bounds.height),
                    0.0,
                    1.0,
                    false,
                    NAV_PADDING,
                );
            }

            let vw = usable_width / self.zoom;
            let vx = NAV_PADDING + self.offset * usable_width;

            frame.fill(
                &Path::rectangle(Point::new(vx, 0.0), Size::new(vw, bounds.height)),
                canvas::Fill::from(Color::from_rgba8(0, 200, 200, 0.1)),
            );
            frame.stroke(
                &Path::rectangle(Point::new(vx, 0.0), Size::new(vw, bounds.height)),
                canvas::Stroke::default().with_color(Color::from_rgb8(0, 200, 200)).with_width(1.0),
            );

            // Handles completely outside the edges
            frame.fill(
                &Path::rectangle(
                    Point::new(vx - NAV_HANDLE_WIDTH, -1.0),
                    Size::new(NAV_HANDLE_WIDTH, bounds.height),
                ),
                canvas::Fill::from(Color::from_rgb8(0, 200, 200)),
            );
            frame.fill(
                &Path::rectangle(
                    Point::new(vx + vw, -1.0),
                    Size::new(NAV_HANDLE_WIDTH, bounds.height),
                ),
                canvas::Fill::from(Color::from_rgb8(0, 200, 200)),
            );
        });
        let cursor_geom = self.cursor_cache.draw(renderer, bounds.size(), |frame| {
            if self.is_loading {
                return;
            }
            let x = NAV_PADDING + self.playback_pos * usable_width;
            frame.stroke(
                &Path::line(Point::new(x, 0.0), Point::new(x, bounds.height)),
                canvas::Stroke::default().with_color(Color::from_rgb8(0, 255, 255)).with_width(2.0),
            );
        });
        vec![geom, cursor_geom]
    }
}
