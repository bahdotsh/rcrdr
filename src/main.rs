use eframe::{egui, App, Frame, NativeOptions};
use egui::{Color32, RichText, Ui};
use rfd::FileDialog;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// Use clap for CLI backward compatibility
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Record screen to video file
    Record {
        /// Output file path
        #[arg(short, long, default_value = "output.mp4")]
        output: String,

        /// Recording duration in seconds (0 for manual stop with Ctrl+C)
        #[arg(short, long, default_value_t = 0)]
        duration: u64,

        /// Frame rate
        #[arg(short, long, default_value_t = 30)]
        fps: u32,
    },

    /// Convert video to GIF
    ConvertToGif {
        /// Input video file
        #[arg(short, long)]
        input: String,

        /// Output GIF file
        #[arg(short, long, default_value = "output.gif")]
        output: String,
    },

    /// Run a test recording to verify everything works
    Test {
        /// Output file path
        #[arg(short, long, default_value = "test.mp4")]
        output: String,
    },
}

// App states
#[derive(PartialEq, Clone)]
enum AppState {
    Setup,
    Main,
    Recording,
    Converting,
    Testing,
}

// UI state
struct RcrdrApp {
    state: AppState,
    logs: Vec<String>,

    // Recording settings
    output_path: String,
    duration: u64,
    fps: u32,

    // Conversion settings
    input_video_path: String,
    output_gif_path: String,

    // Setup
    ffmpeg_installed: bool,
    installation_logs: Vec<String>,

    // Recording state
    recording_start_time: Option<Instant>,
    recording_stop_flag: Option<Arc<AtomicBool>>,
    recording_log_receiver: Option<Receiver<String>>,
    recording_output_path: Option<String>,

    // Converting state
    converting_log_receiver: Option<Receiver<String>>,
    converting_progress: f32,

    // Testing state
    testing_log_receiver: Option<Receiver<String>>,
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

impl RcrdrApp {
    fn show_setup_screen(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.heading("Welcome to Screen Recorder");
            ui.add_space(20.0);

            ui.label("This application requires FFmpeg to work properly.");
            ui.label("FFmpeg is not currently detected on your system.");

            ui.add_space(10.0);

            if ui.button("Install FFmpeg").clicked() {
                self.installation_logs
                    .push("Starting FFmpeg installation...".to_string());
                self.install_ffmpeg();
            }

            if ui.button("Check again").clicked() {
                self.ffmpeg_installed = is_command_available("ffmpeg");
                if self.ffmpeg_installed {
                    self.state = AppState::Main;
                } else {
                    self.installation_logs
                        .push("FFmpeg still not found.".to_string());
                }
            }

            ui.add_space(10.0);
            ui.label("Manual installation:");

            #[cfg(target_os = "windows")]
            {
                if ui.button("Visit FFmpeg download page").clicked() {
                    open::that("https://ffmpeg.org/download.html").ok();
                }
                ui.hyperlink_to(
                    "Download FFmpeg",
                    "https://www.gyan.dev/ffmpeg/builds/ffmpeg-git-full.7z",
                );
                ui.label("1. Extract the archive");
                ui.label("2. Add the bin folder to your PATH environment variable");
                ui.label("3. Restart this application");
            }

            #[cfg(target_os = "macos")]
            {
                ui.label("Install with Homebrew:");
                ui.monospace("brew install ffmpeg");

                if ui.button("Install with Homebrew").clicked() {
                    thread::spawn(|| {
                        let output = Command::new("brew").args(["install", "ffmpeg"]).output();

                        match output {
                            Ok(_) => {}
                            Err(_) => {}
                        }
                    });
                }
            }

            #[cfg(target_os = "linux")]
            {
                ui.label("Ubuntu/Debian:");
                ui.monospace("sudo apt update && sudo apt install ffmpeg");

                ui.label("Fedora:");
                ui.monospace("sudo dnf install ffmpeg");

                ui.label("Arch Linux:");
                ui.monospace("sudo pacman -S ffmpeg");
            }

            ui.add_space(10.0);

            // Display installation logs
            if !self.installation_logs.is_empty() {
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .show(ui, |ui| {
                        ui.heading("Installation Log");
                        for log in &self.installation_logs {
                            ui.label(log);
                        }
                    });
            }
        });
    }

    fn show_main_screen(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.heading("Screen Recorder");

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Record & Convert").clicked() {
                    self.state = AppState::Main;
                }
            });
        });

        ui.add_space(10.0);

        egui::CollapsingHeader::new("Screen Recording")
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Output File:");
                    ui.text_edit_singleline(&mut self.output_path);

                    if ui.button("Browse").clicked() {
                        if let Some(path) = FileDialog::new()
                            .set_file_name(&self.output_path)
                            .add_filter("MP4 Video", &["mp4"])
                            .save_file()
                        {
                            self.output_path = path.to_string_lossy().to_string();
                        }
                    }
                });

                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Duration (seconds, 0 for manual stop):");
                    ui.add(egui::DragValue::new(&mut self.duration).speed(1.0));
                });

                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Frame Rate (FPS):");
                    ui.add(
                        egui::DragValue::new(&mut self.fps)
                            .speed(1.0)
                            .clamp_range(10..=60),
                    );
                });

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("Start Recording").clicked() {
                        self.start_recording();
                    }

                    if ui.button("Test Recording (3s)").clicked() {
                        self.start_test_recording();
                    }
                });
            });

        ui.add_space(20.0);

        egui::CollapsingHeader::new("Convert Video to GIF")
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Input Video:");
                    ui.text_edit_singleline(&mut self.input_video_path);

                    if ui.button("Browse").clicked() {
                        if let Some(path) = FileDialog::new()
                            .add_filter("Video Files", &["mp4", "avi", "mov", "mkv"])
                            .pick_file()
                        {
                            self.input_video_path = path.to_string_lossy().to_string();

                            // Auto-suggest output gif name
                            if let Some(input_path) = Path::new(&self.input_video_path).file_stem()
                            {
                                let parent = Path::new(&self.input_video_path)
                                    .parent()
                                    .unwrap_or(Path::new(""));
                                self.output_gif_path = parent
                                    .join(format!("{}.gif", input_path.to_string_lossy()))
                                    .to_string_lossy()
                                    .to_string();
                            }
                        }
                    }
                });

                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Output GIF:");
                    ui.text_edit_singleline(&mut self.output_gif_path);

                    if ui.button("Browse").clicked() {
                        if let Some(path) = FileDialog::new()
                            .set_file_name(&self.output_gif_path)
                            .add_filter("GIF Images", &["gif"])
                            .save_file()
                        {
                            self.output_gif_path = path.to_string_lossy().to_string();
                        }
                    }
                });

                ui.add_space(10.0);

                let convert_enabled = !self.input_video_path.is_empty()
                    && !self.output_gif_path.is_empty()
                    && Path::new(&self.input_video_path).exists();

                ui.add_enabled_ui(convert_enabled, |ui| {
                    if ui.button("Convert to GIF").clicked() {
                        self.start_gif_conversion();
                    }
                });

                if !convert_enabled
                    && !self.input_video_path.is_empty()
                    && !Path::new(&self.input_video_path).exists()
                {
                    ui.colored_label(Color32::RED, "Input video file does not exist!");
                }
            });

        // Status log area at bottom
        if !self.logs.is_empty() {
            ui.add_space(20.0);
            ui.separator();
            ui.heading("Log");

            egui::ScrollArea::vertical()
                .max_height(150.0)
                .show(ui, |ui| {
                    for log in &self.logs {
                        ui.label(log);
                    }
                });
        }
    }

    fn show_recording_screen(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        // Process any incoming logs if we have a receiver
        if let Some(log_receiver) = &self.recording_log_receiver {
            while let Ok(log) = log_receiver.try_recv() {
                self.logs.push(log);
            }
        }

        let start_time = self.recording_start_time.unwrap_or_else(Instant::now);
        let elapsed = start_time.elapsed();
        let elapsed_secs = elapsed.as_secs();
        let elapsed_str = format!(
            "{:02}:{:02}:{:02}",
            elapsed_secs / 3600,
            (elapsed_secs % 3600) / 60,
            elapsed_secs % 60
        );

        ui.vertical_centered(|ui| {
            ui.heading("Recording in Progress");

            ui.add_space(20.0);
            ui.heading(RichText::new(elapsed_str).size(30.0));
            ui.add_space(20.0);

            // Pulsating record icon
            let time = ui.input(|i| i.time);
            let pulse = (time.sin() * 0.5 + 0.5) as f32;
            let color = Color32::from_rgba_premultiplied(
                255,
                (40.0 + 60.0 * (1.0 - pulse)) as u8,
                (40.0 + 60.0 * (1.0 - pulse)) as u8,
                255,
            );

            ui.label(RichText::new("⚫ RECORDING").color(color).size(24.0));

            ui.add_space(30.0);
            if ui.button("Stop Recording").clicked() {
                if let Some(stop_flag) = &self.recording_stop_flag {
                    stop_flag.store(false, Ordering::SeqCst);
                    self.logs.push("Stopping recording...".to_string());
                }
            }

            ui.add_space(10.0);
            if let Some(output_path) = &self.recording_output_path {
                ui.label(format!("Output file: {}", output_path));
            }

            // Show the most recent logs
            ui.add_space(20.0);
            egui::ScrollArea::vertical()
                .max_height(150.0)
                .show(ui, |ui| {
                    ui.heading("Status");
                    for log in self.logs.iter().rev().take(10).rev() {
                        ui.label(log);
                    }
                });

            // Check if the recording has stopped
            if let Some(stop_flag) = &self.recording_stop_flag {
                if !stop_flag.load(Ordering::SeqCst) {
                    // Add a small delay to allow the final messages to come through
                    thread::sleep(Duration::from_millis(100));

                    if let Some(log_receiver) = &self.recording_log_receiver {
                        while let Ok(log) = log_receiver.try_recv() {
                            self.logs.push(log);
                        }
                    }

                    // Check if recording is truly done
                    if let Some(output_path) = &self.recording_output_path {
                        if verify_video_file(output_path) {
                            self.logs
                                .push(format!("Recording completed successfully: {}", output_path));
                            self.state = AppState::Main;

                            // Set the input video path to the recording for easy conversion
                            self.input_video_path = output_path.clone();

                            // Update output gif path
                            if let Some(input_path) = Path::new(output_path).file_stem() {
                                let parent =
                                    Path::new(output_path).parent().unwrap_or(Path::new(""));
                                self.output_gif_path = parent
                                    .join(format!("{}.gif", input_path.to_string_lossy()))
                                    .to_string_lossy()
                                    .to_string();
                            }

                            // Clean up recording state
                            self.recording_start_time = None;
                            self.recording_stop_flag = None;
                            self.recording_log_receiver = None;
                            self.recording_output_path = None;

                            // Request context update to refresh UI immediately
                            ctx.request_repaint();
                        }
                    }
                }
            }
        });
    }

    fn show_converting_screen(&mut self, ui: &mut Ui) {
        // Flag to track if we should transition to main screen
        let mut should_transition = false;
        let time = ui.input(|i| i.time);

        // Process any incoming logs
        if let Some(log_receiver) = &self.converting_log_receiver {
            while let Ok(log) = log_receiver.try_recv() {
                self.logs.push(log.clone());

                // Try to parse progress information from ffmpeg output
                if log.contains("time=") && log.contains("bitrate=") {
                    // Extract time information
                    if let Some(time_pos) = log.find("time=") {
                        let time_str = &log[time_pos + 5..];
                        if let Some(end_pos) = time_str.find(' ') {
                            let time_val = &time_str[..end_pos];
                            // Parse the timestamp HH:MM:SS.MS
                            let parts: Vec<&str> = time_val.split(':').collect();
                            if parts.len() == 3 {
                                if let (Ok(hours), Ok(minutes), Ok(seconds_f)) = (
                                    parts[0].parse::<f32>(),
                                    parts[1].parse::<f32>(),
                                    parts[2].parse::<f32>(),
                                ) {
                                    let current_secs = hours * 3600.0 + minutes * 60.0 + seconds_f;
                                    // Assuming input is around 30 seconds for progress calculation
                                    // This is a rough estimate
                                    self.converting_progress = (current_secs / 30.0).min(0.95);
                                }
                            }
                        }
                    }
                }

                // Check for completion messages
                if log.contains("GIF conversion completed successfully") {
                    self.converting_progress = 1.0;
                    should_transition = true;
                }
            }
        }

        // Check if we should transition after processing logs
        if self.converting_progress >= 1.0 && time > 1.0 {
            // Allow a few seconds for user to see the completion message
            if !should_transition {
                should_transition = true;
            }
        }

        ui.vertical_centered(|ui| {
            ui.heading("Converting Video to GIF");
            ui.add_space(20.0);

            // Progress bar
            ui.add(
                egui::ProgressBar::new(self.converting_progress)
                    .show_percentage()
                    .animate(true),
            );

            ui.add_space(20.0);

            if self.converting_progress >= 1.0 {
                ui.label(
                    RichText::new("Conversion complete!")
                        .color(Color32::GREEN)
                        .size(18.0),
                );
                ui.label("Returning to main screen...");
            } else {
                ui.label("This may take a while depending on the video length...");
            }

            // Show the most recent logs
            ui.add_space(20.0);
            egui::ScrollArea::vertical()
                .max_height(150.0)
                .show(ui, |ui| {
                    ui.heading("Status");
                    for log in self.logs.iter().rev().take(10).rev() {
                        ui.label(log);
                    }
                });
        });

        // Handle transition after UI is drawn
        if should_transition {
            // Add a delay before transitioning to ensure user sees completion
            thread::sleep(Duration::from_millis(1500));

            self.state = AppState::Main;
            self.converting_log_receiver = None;
            self.converting_progress = 0.0;
        }
    }

    fn show_testing_screen(&mut self, ui: &mut Ui) {
        // Process any incoming logs
        let mut test_complete = false;

        if let Some(log_receiver) = &self.testing_log_receiver {
            while let Ok(log) = log_receiver.try_recv() {
                self.logs.push(log.clone());
                if log.contains("Test recording completed") {
                    test_complete = true;
                }
            }
        }

        ui.vertical_centered(|ui| {
            ui.heading("Testing Recording Capabilities");
            ui.add_space(20.0);

            ui.label("Running a 3-second test recording to verify system compatibility...");

            if !test_complete {
                // Show spinner animation
                let time = ui.input(|i| i.time);
                let angle = time * 5.0;
                let points = 12;
                let radius = 20.0;
                let center = ui.cursor().center();

                ui.add_space(radius * 2.0 + 10.0);

                for i in 0..points {
                    let angle_i =
                        (i as f64 * 2.0 * std::f64::consts::PI / points as f64 + angle) as f32;
                    let (sin, cos) = angle_i.sin_cos();
                    let pos = egui::Pos2::new(center.x + radius * cos, center.y + radius * sin);
                    let alpha = 0.2 + 0.8 * (1.0 - (i as f32 / points as f32)).powi(2);
                    ui.painter().circle_filled(
                        pos,
                        3.0,
                        Color32::from_white_alpha((alpha * 255.0) as u8),
                    );
                }

                ui.add_space(20.0);
            } else {
                ui.add_space(10.0);
                ui.label(
                    RichText::new("✓ Test completed successfully!")
                        .color(Color32::GREEN)
                        .size(18.0),
                );
                ui.add_space(20.0);

                if ui.button("Return to Main Screen").clicked() {
                    self.state = AppState::Main;
                    self.testing_log_receiver = None;
                }
            }

            // Show the most recent logs
            ui.add_space(20.0);
            egui::ScrollArea::vertical()
                .max_height(150.0)
                .show(ui, |ui| {
                    ui.heading("Status");
                    for log in self.logs.iter().rev().take(10).rev() {
                        ui.label(log);
                    }
                });
        });
    }

    fn start_recording(&mut self) {
        let output_path = self.output_path.clone();
        let duration = self.duration;
        let fps = self.fps;

        // Set up log channel
        let (tx, rx) = channel();

        // Set up stop flag
        let stop_flag = Arc::new(AtomicBool::new(true));
        let stop_flag_clone = stop_flag.clone();

        // Start recording in a background thread
        let output_path_clone = output_path.clone();
        thread::spawn(move || {
            let result = record_screen_gui(&output_path_clone, duration, fps, stop_flag_clone, tx);
            if let Err(e) = result {
                eprintln!("Recording error: {}", e);
            }
        });

        // Update app state
        self.state = AppState::Recording;
        self.recording_start_time = Some(Instant::now());
        self.recording_stop_flag = Some(stop_flag);
        self.recording_log_receiver = Some(rx);
        self.recording_output_path = Some(output_path);
    }

    fn start_gif_conversion(&mut self) {
        let input_path = self.input_video_path.clone();
        let output_path = self.output_gif_path.clone();

        // Set up log channel
        let (tx, rx) = channel();

        // Start conversion in a background thread
        thread::spawn(move || {
            let result = convert_to_gif_gui(&input_path, &output_path, tx);
            if let Err(e) = result {
                eprintln!("Conversion error: {}", e);
            }
        });

        // Update app state
        self.state = AppState::Converting;
        self.converting_log_receiver = Some(rx);
        self.converting_progress = 0.0;
    }

    fn start_test_recording(&mut self) {
        // Create a temp file for test output
        let temp_dir = std::env::temp_dir();
        let test_output = temp_dir
            .join("rcrdr_test.mp4")
            .to_string_lossy()
            .to_string();

        // Set up log channel
        let (tx, rx) = channel();

        // Start test in a background thread
        thread::spawn(move || {
            let result = test_recording_gui(&test_output, tx);
            if let Err(e) = result {
                eprintln!("Test recording error: {}", e);
            }
        });

        // Update app state
        self.state = AppState::Testing;
        self.testing_log_receiver = Some(rx);
    }

    fn install_ffmpeg(&mut self) {
        #[cfg(target_os = "windows")]
        {
            self.installation_logs
                .push("Installing FFmpeg on Windows...".to_string());
            self.installation_logs
                .push("Please install manually following these steps:".to_string());
            self.installation_logs
                .push("1. Download from https://ffmpeg.org/download.html".to_string());
            self.installation_logs
                .push("2. Extract the archive".to_string());
            self.installation_logs
                .push("3. Add the bin folder to your PATH environment variable".to_string());
            self.installation_logs
                .push("4. Restart this application".to_string());

            // Open the download page
            if let Err(e) = open::that("https://ffmpeg.org/download.html") {
                self.installation_logs
                    .push(format!("Failed to open browser: {}", e));
            }
        }

        #[cfg(target_os = "macos")]
        {
            self.installation_logs
                .push("Installing FFmpeg on macOS...".to_string());

            // First check if Homebrew is installed
            let brew_installed = is_command_available("brew");

            if brew_installed {
                self.installation_logs
                    .push("Homebrew detected, installing FFmpeg...".to_string());

                // Spawn a thread to run the brew install command
                let logs = Arc::new(Mutex::new(self.installation_logs.clone()));
                let logs_clone = logs.clone();

                thread::spawn(move || {
                    let output = Command::new("brew").args(["install", "ffmpeg"]).output();

                    match output {
                        Ok(output) => {
                            let stdout = String::from_utf8_lossy(&output.stdout);
                            let stderr = String::from_utf8_lossy(&output.stderr);

                            let mut logs = logs_clone.lock().unwrap();
                            logs.push("Homebrew install output:".to_string());

                            for line in stdout.lines() {
                                logs.push(line.to_string());
                            }

                            if !stderr.is_empty() {
                                logs.push("Errors:".to_string());
                                for line in stderr.lines() {
                                    logs.push(line.to_string());
                                }
                            }

                            if output.status.success() {
                                logs.push("FFmpeg installed successfully!".to_string());
                            } else {
                                logs.push("Failed to install FFmpeg.".to_string());
                            }
                        }
                        Err(e) => {
                            let mut logs = logs_clone.lock().unwrap();
                            logs.push(format!("Failed to run Homebrew: {}", e));
                        }
                    }
                });
            } else {
                self.installation_logs
                    .push("Homebrew not found. Please install Homebrew first:".to_string());
                self.installation_logs.push("/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"".to_string());

                // Open the Homebrew website
                if let Err(e) = open::that("https://brew.sh/") {
                    self.installation_logs
                        .push(format!("Failed to open browser: {}", e));
                }
            }
        }

        #[cfg(target_os = "linux")]
        {
            self.installation_logs
                .push("Installing FFmpeg on Linux...".to_string());

            // Try to detect package manager
            let (pkg_mgr, install_cmd) = if is_command_available("apt") {
                (
                    "apt",
                    vec![
                        "sudo", "apt", "update", "&&", "sudo", "apt", "install", "-y", "ffmpeg",
                    ],
                )
            } else if is_command_available("dnf") {
                ("dnf", vec!["sudo", "dnf", "install", "-y", "ffmpeg"])
            } else if is_command_available("pacman") {
                (
                    "pacman",
                    vec!["sudo", "pacman", "-S", "--noconfirm", "ffmpeg"],
                )
            } else {
                ("unknown", vec![])
            };

            if pkg_mgr != "unknown" {
                self.installation_logs
                    .push(format!("Detected package manager: {}", pkg_mgr));
                self.installation_logs
                    .push(format!("Please run this command in terminal:"));
                self.installation_logs.push(install_cmd.join(" "));
            } else {
                self.installation_logs
                    .push("Could not detect package manager.".to_string());
                self.installation_logs.push(
                    "Please install FFmpeg using your distribution's package manager.".to_string(),
                );
            }
        }
    }
}

fn is_command_available(command: &str) -> bool {
    let output = if cfg!(target_os = "windows") {
        Command::new("where").arg(command).output()
    } else {
        Command::new("which").arg(command).output()
    };

    match output {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}

fn verify_video_file(file_path: &str) -> bool {
    // Check if file exists and is not empty
    match fs::metadata(file_path) {
        Ok(metadata) => {
            if metadata.len() == 0 {
                println!("Warning: The video file is empty.");
                return false;
            }
        }
        Err(_) => {
            println!("Warning: Could not access the video file.");
            return false;
        }
    }

    // Use ffprobe to verify the file is a valid video container
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            file_path,
        ])
        .output();

    match output {
        Ok(output) => {
            if !output.status.success() {
                println!("Warning: File does not appear to be a valid video file.");
                return false;
            }

            // Try to parse the duration
            let duration_str = String::from_utf8_lossy(&output.stdout);
            match duration_str.trim().parse::<f64>() {
                Ok(duration) => {
                    if duration <= 0.0 {
                        println!("Warning: Video file has zero duration.");
                        return false;
                    }
                }
                Err(_) => {
                    println!("Warning: Could not determine video duration.");
                    return false;
                }
            }

            true
        }
        Err(_) => {
            println!("Warning: Failed to verify video file.");
            false
        }
    }
}

fn record_screen_gui(
    output: &str,
    duration: u64,
    fps: u32,
    running: Arc<AtomicBool>,
    log_sender: Sender<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    log_sender.send("Initializing recording...".to_string())?;

    // Set up the FFmpeg command based on the platform
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y"); // Overwrite output file if it exists

    #[cfg(target_os = "windows")]
    {
        cmd.args([
            "-f",
            "gdigrab",
            "-framerate",
            &fps.to_string(),
            "-i",
            "desktop",
        ]);
    }

    #[cfg(target_os = "linux")]
    {
        cmd.args([
            "-f",
            "x11grab",
            "-framerate",
            &fps.to_string(),
            "-i",
            ":0.0",
        ]);
    }

    #[cfg(target_os = "macos")]
    {
        cmd.args([
            "-f",
            "avfoundation",
            "-framerate",
            &fps.to_string(),
            "-i",
            "1:none", // Capture screen 1, no audio
            "-pix_fmt",
            "uyvy422", // Needed for macOS avfoundation
        ]);
    }

    // Common output options
    cmd.args([
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "medium", "-crf", "23",
    ]);

    if duration > 0 {
        cmd.args(["-t", &duration.to_string()]);
    }

    // Add output file
    cmd.arg(output);

    // Capture stderr to provide better error messages
    cmd.stderr(Stdio::piped());

    if duration > 0 {
        // For fixed duration, just run and wait
        log_sender.send(format!("Recording for {} seconds...", duration))?;

        let output_result = cmd.output()?;
        if !output_result.status.success() {
            let error = String::from_utf8_lossy(&output_result.stderr);
            log_sender.send(format!("FFmpeg recording failed: {}", error))?;
            return Err(format!("FFmpeg recording failed: {}", error).into());
        }
    } else {
        // For manual stop, run in background and wait for stop signal
        let mut child = cmd.spawn()?;
        let stderr = child.stderr.take().expect("Failed to capture stderr");
        let mut stderr_reader = io::BufReader::new(stderr);

        log_sender.send("Recording started. Press Stop button when ready.".to_string())?;

        // Thread to monitor stderr and send logs
        let log_sender_clone = log_sender.clone();
        thread::spawn(move || {
            let mut buffer = [0; 1024];

            while let Ok(n) = stderr_reader.read(&mut buffer) {
                if n == 0 {
                    break;
                }

                let output = String::from_utf8_lossy(&buffer[..n]).to_string();
                log_sender_clone.send(output).unwrap_or_default();
            }
        });

        // Wait for the stop signal
        while running.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(100));

            // Check if ffmpeg exited on its own
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        log_sender.send("FFmpeg recording failed unexpectedly.".to_string())?;
                        return Err("FFmpeg recording failed unexpectedly.".into());
                    }
                    break;
                }
                Ok(None) => continue,
                Err(e) => return Err(e.into()),
            }
        }

        log_sender.send("Stopping recording...".to_string())?;

        // Gracefully terminate FFmpeg with SIGINT for proper file finalization
        #[cfg(unix)]
        {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGINT);
            }
            // Give FFmpeg a moment to clean up
            thread::sleep(Duration::from_millis(500));
        }

        // Then kill if still running
        let _ = child.kill();
        let _ = child.wait();

        log_sender.send("Recording stopped.".to_string())?;
        log_sender.send(format!("Saved to {}", output))?;
    }

    Ok(())
}

fn convert_to_gif_gui(
    input: &str,
    output: &str,
    log_sender: Sender<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    log_sender.send("Starting video to GIF conversion...".to_string())?;
    log_sender.send("This may take a while depending on video length.".to_string())?;

    // Use FFmpeg to convert video to GIF with reasonable quality
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-i",
        input,
        "-vf",
        "fps=10,scale=640:-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse",
        "-loop",
        "0",
        output,
    ]);

    // We want to capture stderr for progress updates
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn()?;

    if let Some(stderr) = child.stderr.take() {
        let mut reader = io::BufReader::new(stderr);
        let mut buffer = [0; 1024];

        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 {
                break;
            }

            let output = String::from_utf8_lossy(&buffer[..n]).to_string();
            log_sender.send(output)?;
        }
    }

    let status = child.wait()?;

    if !status.success() {
        log_sender.send("GIF conversion failed!".to_string())?;
        return Err("GIF conversion failed".into());
    }

    // Add a small delay to ensure file is properly written
    thread::sleep(Duration::from_millis(500));

    // Verify the output GIF file exists
    if let Ok(metadata) = fs::metadata(output) {
        if metadata.len() > 0 {
            log_sender.send("GIF conversion completed successfully!".to_string())?;
            log_sender.send(format!("Saved to {}", output))?;
        } else {
            log_sender.send("Warning: The output GIF file seems to be empty.".to_string())?;
        }
    } else {
        log_sender.send("Warning: Could not find the output GIF file.".to_string())?;
    }

    Ok(())
}

fn test_recording_gui(
    output: &str,
    log_sender: Sender<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    log_sender.send("Starting test recording...".to_string())?;
    log_sender.send(
        "This will record your screen for 3 seconds to verify everything works.".to_string(),
    )?;

    #[cfg(target_os = "macos")]
    log_sender
        .send("Note: On macOS, you may need to grant screen recording permissions".to_string())?;

    // Set up the FFmpeg command for a very short recording
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y"); // Overwrite output file if it exists

    #[cfg(target_os = "windows")]
    {
        cmd.args(["-f", "gdigrab", "-framerate", "30", "-i", "desktop"]);
    }

    #[cfg(target_os = "linux")]
    {
        cmd.args(["-f", "x11grab", "-framerate", "30", "-i", ":0.0"]);
    }

    #[cfg(target_os = "macos")]
    {
        // List available devices first to help diagnose issues
        let devices = Command::new("ffmpeg")
            .args(["-f", "avfoundation", "-list_devices", "true", "-i", ""])
            .output();

        match devices {
            Ok(output) => {
                let output_str = String::from_utf8_lossy(&output.stderr);
                log_sender.send("Available capture devices:".to_string())?;
                for line in output_str.lines() {
                    if line.contains("AVFoundation") || line.contains("capture") {
                        log_sender.send(line.to_string())?;
                    }
                }
            }
            Err(_) => log_sender.send("Failed to list capture devices.".to_string())?,
        }

        cmd.args([
            "-f",
            "avfoundation",
            "-framerate",
            "30",
            "-i",
            "1:none", // Default to screen 1, but this might need adjustment
            "-pix_fmt",
            "uyvy422", // Needed for macOS avfoundation
        ]);
    }

    // Common output options
    cmd.args([
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-preset",
        "ultrafast", // Use ultrafast for test
        "-crf",
        "28", // Lower quality for test
        "-t",
        "3", // 3 seconds
        output,
    ]);

    // Capture stderr to provide better error messages
    cmd.stderr(Stdio::piped());

    log_sender.send("Test recording in progress (3 seconds)...".to_string())?;

    let mut child = cmd.spawn()?;
    let stderr = child.stderr.take().expect("Failed to capture stderr");
    let mut stderr_reader = io::BufReader::new(stderr);
    let stderr_thread = std::thread::spawn(move || {
        let mut buffer = [0; 1024];
        let mut error_output = String::new();

        while let Ok(n) = stderr_reader.read(&mut buffer) {
            if n == 0 {
                break;
            }
            error_output.push_str(&String::from_utf8_lossy(&buffer[..n]));
        }

        error_output
    });

    // Wait for ffmpeg to finish
    let status = child.wait()?;
    let stderr_output = stderr_thread.join().unwrap_or_default();

    if !status.success() {
        log_sender.send(format!("Test recording failed: {}", stderr_output))?;
        return Err(format!("Test recording failed: {}", stderr_output).into());
    }

    if verify_video_file(output) {
        log_sender.send("Test recording completed successfully!".to_string())?;
        log_sender.send("Your system is configured correctly for screen recording.".to_string())?;
    } else {
        log_sender
            .send("Test recording completed but did not produce a valid video file.".to_string())?;
        log_sender.send("Please check your system configuration.".to_string())?;
        return Err("Test recording failed to produce a valid video file.".into());
    }

    Ok(())
}

// Original CLI functions that are still needed for CLI mode
#[cfg(target_os = "macos")]
fn print_macos_permission_guide() {
    println!("Note for macOS users:");
    println!("This app needs permission to record your screen.");
    println!("If this is your first time using the app, you may see a permission dialog.");
    println!("You might need to:");
    println!("1. Go to System Preferences > Security & Privacy > Privacy > Screen Recording");
    println!(
        "2. Make sure your terminal app (Terminal, iTerm2, etc.) is allowed to record the screen"
    );
    println!("3. You may need to restart your terminal after granting permissions");
    println!();
}

fn record_screen(
    output: &str,
    duration: u64,
    fps: u32,
    running: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Press Ctrl+C to stop recording.");

    // Set up the FFmpeg command based on the platform
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y"); // Overwrite output file if it exists

    #[cfg(target_os = "windows")]
    {
        cmd.args([
            "-f",
            "gdigrab",
            "-framerate",
            &fps.to_string(),
            "-i",
            "desktop",
        ]);
    }

    #[cfg(target_os = "linux")]
    {
        cmd.args([
            "-f",
            "x11grab",
            "-framerate",
            &fps.to_string(),
            "-i",
            ":0.0",
        ]);
    }

    #[cfg(target_os = "macos")]
    {
        cmd.args([
            "-f",
            "avfoundation",
            "-framerate",
            &fps.to_string(),
            "-i",
            "1:none", // Capture screen 1, no audio
            "-pix_fmt",
            "uyvy422", // Needed for macOS avfoundation
        ]);
    }

    // Common output options
    cmd.args([
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "medium", "-crf", "23",
    ]);

    if duration > 0 {
        cmd.args(["-t", &duration.to_string()]);
    }

    // Add output file
    cmd.arg(output);

    // Capture stderr to provide better error messages
    cmd.stderr(Stdio::piped());

    if duration > 0 {
        // For fixed duration, just run and wait
        print!("Recording for {} seconds... ", duration);
        io::stdout().flush()?;

        let output = cmd.output()?;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(format!("FFmpeg recording failed: {}", error).into());
        }
    } else {
        // For manual stop, run in background and wait for Ctrl+C
        let mut child = cmd.spawn()?;
        let stderr = child.stderr.take().expect("Failed to capture stderr");
        let mut stderr_reader = io::BufReader::new(stderr);
        let stderr_thread = std::thread::spawn(move || {
            let mut buffer = [0; 1024];
            let mut error_output = String::new();

            while let Ok(n) = stderr_reader.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                error_output.push_str(&String::from_utf8_lossy(&buffer[..n]));
            }

            error_output
        });

        // Give FFmpeg a moment to start up before allowing termination
        println!("Initializing recording...");
        thread::sleep(Duration::from_secs(1));

        // Print a simple animation to show recording is in progress
        let mut i = 0;
        let spinner = ['|', '/', '-', '\\'];

        // This flag helps ensure a minimum recording time to create a valid file
        let min_record_time = 2; // seconds
        let start_time = std::time::Instant::now();

        while running.load(Ordering::SeqCst) {
            let elapsed = start_time.elapsed().as_secs();
            print!("\rRecording {} ({}s) ", spinner[i], elapsed);
            io::stdout().flush()?;
            i = (i + 1) % spinner.len();

            std::thread::sleep(Duration::from_millis(100));

            // Check if ffmpeg exited on its own
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        let error_output = stderr_thread.join().unwrap_or_default();
                        return Err(format!("FFmpeg recording failed: {}", error_output).into());
                    }
                    break;
                }
                Ok(None) => continue,
                Err(e) => return Err(e.into()),
            }
        }

        // If the recording was very short, wait a bit more to ensure it's valid
        let elapsed = start_time.elapsed().as_secs();
        if elapsed < min_record_time {
            println!("\rEnsuring valid recording... Please wait.");
            thread::sleep(Duration::from_secs(min_record_time - elapsed));
        }

        println!("\rTerminating recording process...");

        // Gracefully terminate FFmpeg with SIGINT for proper file finalization
        #[cfg(unix)]
        {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGINT);
            }
            // Give FFmpeg a moment to clean up
            thread::sleep(Duration::from_millis(500));
        }

        // Then kill if still running
        let _ = child.kill();
        let _ = child.wait();

        // Check if there were any errors
        let error_output = stderr_thread.join().unwrap_or_default();
        if error_output.contains("Error") && error_output.contains("error") {
            return Err(format!("FFmpeg recording failed: {}", error_output).into());
        }
    }

    println!("\nRecording saved to {}", output);
    Ok(())
}

fn convert_to_gif(input: &str, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Converting video to GIF (this may take a moment)...");

    // Use FFmpeg to convert video to GIF with reasonable quality
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-i",
        input,
        "-vf",
        "fps=10,scale=640:-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse",
        "-loop",
        "0",
        output,
    ]);

    cmd.stderr(Stdio::piped());

    let output_result = cmd.output()?;
    if !output_result.status.success() {
        let error = String::from_utf8_lossy(&output_result.stderr);
        return Err(format!("GIF conversion failed: {}", error).into());
    }

    Ok(())
}

fn test_recording(output: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("This will record your screen for 3 seconds to test if everything works.");
    println!("If you don't see any errors, then your system is properly configured.");

    #[cfg(target_os = "macos")]
    print_macos_permission_guide();

    println!("Starting test recording in 3 seconds...");
    thread::sleep(Duration::from_secs(3));

    // Set up the FFmpeg command for a very short recording
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y"); // Overwrite output file if it exists

    #[cfg(target_os = "windows")]
    {
        cmd.args(["-f", "gdigrab", "-framerate", "30", "-i", "desktop"]);
    }

    #[cfg(target_os = "linux")]
    {
        cmd.args(["-f", "x11grab", "-framerate", "30", "-i", ":0.0"]);
    }

    #[cfg(target_os = "macos")]
    {
        // List available devices first to help diagnose issues
        let devices = Command::new("ffmpeg")
            .args(["-f", "avfoundation", "-list_devices", "true", "-i", ""])
            .output();

        match devices {
            Ok(output) => {
                let output_str = String::from_utf8_lossy(&output.stderr);
                println!("Available capture devices:");
                for line in output_str.lines() {
                    if line.contains("AVFoundation") || line.contains("capture") {
                        println!("{}", line);
                    }
                }
            }
            Err(_) => println!("Failed to list capture devices."),
        }

        cmd.args([
            "-f",
            "avfoundation",
            "-framerate",
            "30",
            "-i",
            "1:none", // Default to screen 1, but this might need adjustment
            "-pix_fmt",
            "uyvy422", // Needed for macOS avfoundation
        ]);
    }

    // Common output options
    cmd.args([
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-preset",
        "ultrafast", // Use ultrafast for test
        "-crf",
        "28", // Lower quality for test
        "-t",
        "3", // 3 seconds
        output,
    ]);

    // Capture stderr to provide better error messages
    cmd.stderr(Stdio::piped());

    println!("Recording...");
    let output_result = cmd.output()?;

    if !output_result.status.success() {
        let error = String::from_utf8_lossy(&output_result.stderr);
        return Err(format!("Test recording failed: {}", error).into());
    }

    if verify_video_file(output) {
        println!("Success! Test recording completed without errors.");
        println!("You can view the test video at: {}", output);
    } else {
        return Err("Test recording completed but did not produce a valid video file.".into());
    }

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse CLI args first to maintain backward compatibility
    let cli = Cli::parse();

    // If there are CLI subcommands, run in CLI mode
    if let Some(command) = &cli.command {
        // Check if ffmpeg is installed
        if !is_command_available("ffmpeg") {
            return Err("FFmpeg is not installed. Please install FFmpeg first.".into());
        }

        match command {
            Commands::Record {
                output,
                duration,
                fps,
            } => {
                println!("Recording screen to {}...", output);

                #[cfg(target_os = "macos")]
                print_macos_permission_guide();

                let running = Arc::new(AtomicBool::new(true));
                let r = running.clone();

                ctrlc::set_handler(move || {
                    println!("\nStopping recording...");
                    r.store(false, Ordering::SeqCst);
                })?;

                record_screen(output, *duration, *fps, running)?;

                // Verify the output file is valid
                if !verify_video_file(output) {
                    return Err(format!("Failed to create a valid video file: {}. Try running the 'test' command to diagnose issues.", output).into());
                }
            }
            Commands::ConvertToGif { input, output } => {
                // First, verify that the input file exists and is a valid video
                if !Path::new(input).exists() {
                    return Err(format!("Input file does not exist: {}", input).into());
                }

                if !verify_video_file(input) {
                    return Err(format!("Input file is not a valid video file: {}", input).into());
                }

                println!("Converting {} to GIF {}...", input, output);
                convert_to_gif(input, output)?;
                println!("Conversion complete!");
            }
            Commands::Test { output } => {
                println!("Running test recording to {}...", output);
                test_recording(output)?;
            }
        }
    } else {
        // Launch GUI mode
        let app = RcrdrApp::default();
        let native_options = NativeOptions {
            initial_window_size: Some(egui::vec2(800.0, 600.0)),
            min_window_size: Some(egui::vec2(640.0, 480.0)),
            ..Default::default()
        };

        eframe::run_native(
            "Screen Recorder",
            native_options,
            Box::new(|_cc| Box::new(app)),
        )?;
    }

    Ok(())
}
