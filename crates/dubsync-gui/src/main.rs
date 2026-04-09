mod app;
mod audio_loader;
mod cli;
mod message;
mod tasks;
mod theme;
mod types;
mod widgets;

use crate::app::DubSyncGui;
use crate::cli::{Args, Commands};
use crate::types::DubSyncProjectState;
use clap::Parser;
use dubsync_core::{Project, ResourceManager};
use iced::Task;
use lucide_icons::LUCIDE_FONT_BYTES;

pub fn main() -> iced::Result {
    let args = Args::parse();

    let (initial_state, initial_task) = match &args.command {
        Some(Commands::Snapshot { state, output }) => {
            let expanded_state = ResourceManager::expand_path(state);
            let content =
                std::fs::read_to_string(expanded_state).expect("Failed to read state file");
            let state: DubSyncProjectState =
                serde_json::from_str(&content).expect("Failed to parse state JSON");
            DubSyncGui::from_state(state, Some(output.clone()))
        }
        None => {
            if let Some(path) = &args.project {
                let expanded_project = ResourceManager::expand_path(path);
                let content =
                    std::fs::read_to_string(expanded_project).expect("Failed to read project file");
                let project: Project =
                    serde_json::from_str(&content).expect("Failed to parse project JSON");
                DubSyncGui::from_project(project, None)
            } else {
                (DubSyncGui::default(), Task::none())
            }
        }
    };

    let mut app = iced::application(
        "DubSync - Audio Synchronization Suite",
        DubSyncGui::update,
        DubSyncGui::view,
    )
    .subscription(DubSyncGui::subscription)
    .theme(DubSyncGui::theme)
    .default_font(theme::GEIST_REGULAR)
    .font(LUCIDE_FONT_BYTES)
    .font(include_bytes!("../../../assets/Geist-Regular.ttf").as_slice())
    .font(include_bytes!("../../../assets/GeistMono-Regular.ttf").as_slice());

    if let Some(Commands::Snapshot { .. }) = &args.command {
        app = app.window(iced::window::Settings { visible: false, ..Default::default() });
    }

    app.run_with(move || (initial_state, initial_task))
}
