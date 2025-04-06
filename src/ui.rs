use eframe::egui::{self, Color32, RichText, Ui};
use egui::Context;
use rfd::FileDialog;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::app::{AppState, RcrdrApp};
use crate::recorder::{
    convert_to_gif_gui, is_command_available, record_screen_gui, test_recording_gui,
    verify_video_file,
};

impl RcrdrApp {
    pub fn show_setup_screen(&mut self, ui: &mut Ui) {
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

    pub fn show_main_screen(&mut self, ui: &mut Ui, ctx: &Context) {
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

    pub fn show_recording_screen(&mut self, ui: &mut Ui, ctx: &Context) {
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

    pub fn show_converting_screen(&mut self, ui: &mut Ui) {
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

    pub fn show_testing_screen(&mut self, ui: &mut Ui) {
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

    pub fn start_recording(&mut self) {
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

    pub fn start_gif_conversion(&mut self) {
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

    pub fn start_test_recording(&mut self) {
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

    pub fn install_ffmpeg(&mut self) {
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
                let logs = Arc::new(std::sync::Mutex::new(self.installation_logs.clone()));
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
