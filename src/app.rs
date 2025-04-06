use crate::recorder::is_command_available;
use eframe::{egui, App, Frame};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Instant;

// App states
#[derive(PartialEq, Clone)]
pub enum AppState {
    Setup,
    Main,
    Recording,
    Converting,
    Testing,
}

// UI state
pub struct RcrdrApp {
    pub state: AppState,
    pub logs: Vec<String>,

    // Recording settings
    pub output_path: String,
    pub duration: u64,
    pub fps: u32,

    // Conversion settings
    pub input_video_path: String,
    pub output_gif_path: String,

    // Setup
    pub ffmpeg_installed: bool,
    pub installation_logs: Vec<String>,

    // Recording state
    pub recording_start_time: Option<Instant>,
    pub recording_stop_flag: Option<Arc<AtomicBool>>,
    pub recording_log_receiver: Option<Receiver<String>>,
    pub recording_output_path: Option<String>,

    // Converting state
    pub converting_log_receiver: Option<Receiver<String>>,
    pub converting_progress: f32,

    // Testing state
    pub testing_log_receiver: Option<Receiver<String>>,
}

impl Default for RcrdrApp {
    fn default() -> Self {
        let ffmpeg_installed = is_command_available("ffmpeg");

        let default_output = format!(
            "recording_{}.mp4",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );

        Self {
            state: if ffmpeg_installed {
                AppState::Main
            } else {
                AppState::Setup
            },
            logs: Vec::new(),
            output_path: default_output,
            duration: 0,
            fps: 30,
            input_video_path: String::new(),
            output_gif_path: String::new(),
            ffmpeg_installed,
            installation_logs: Vec::new(),
            recording_start_time: None,
            recording_stop_flag: None,
            recording_log_receiver: None,
            recording_output_path: None,
            converting_log_receiver: None,
            converting_progress: 0.0,
            testing_log_receiver: None,
        }
    }
}

impl App for RcrdrApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        egui::CentralPanel::default().show(ctx, |ui| match self.state {
            AppState::Setup => {
                self.show_setup_screen(ui);
            }
            AppState::Main => {
                self.show_main_screen(ui, ctx);
            }
            AppState::Recording => {
                self.show_recording_screen(ui, ctx);
            }
            AppState::Converting => {
                self.show_converting_screen(ui);
            }
            AppState::Testing => {
                self.show_testing_screen(ui);
            }
        });

        // Request continuous repainting while in active states
        match self.state {
            AppState::Recording | AppState::Converting | AppState::Testing => {
                ctx.request_repaint();
            }
            _ => {}
        }
    }
}
