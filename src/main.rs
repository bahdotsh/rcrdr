use crate::app::RcrdrApp;
use crate::cli::{Cli, Commands};
use crate::recorder::{
    convert_to_gif, is_command_available, record_screen, test_recording, verify_video_file,
};
use clap::Parser;
use eframe::{run_native, NativeOptions};
use std::error::Error;

mod app;
mod cli;
mod recorder;
mod ui;

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

fn main() -> Result<(), Box<dyn Error>> {
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

                let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
                let r = running.clone();

                ctrlc::set_handler(move || {
                    println!("\nStopping recording...");
                    r.store(false, std::sync::atomic::Ordering::SeqCst);
                })?;

                record_screen(output, *duration, *fps, running)?;

                // Verify the output file is valid
                if !verify_video_file(output) {
                    return Err(format!("Failed to create a valid video file: {}. Try running the 'test' command to diagnose issues.", output).into());
                }
            }
            Commands::ConvertToGif { input, output } => {
                // First, verify that the input file exists and is a valid video
                if !std::path::Path::new(input).exists() {
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

        run_native(
            "Screen Recorder",
            native_options,
            Box::new(|_cc| Box::new(app)),
        )?;
    }

    Ok(())
}
