use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check if ffmpeg is installed
    if !is_command_available("ffmpeg") {
        return Err("FFmpeg is not installed. Please install FFmpeg first.".into());
    }

    let cli = Cli::parse();

    match &cli.command {
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

    Ok(())
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
            use std::os::unix::process::CommandExt;
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
