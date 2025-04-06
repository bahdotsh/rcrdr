use std::fs;
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub fn is_command_available(command: &str) -> bool {
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

pub fn verify_video_file(file_path: &str) -> bool {
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

pub fn record_screen_gui(
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

pub fn convert_to_gif_gui(
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

pub fn test_recording_gui(
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

pub fn record_screen(
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

pub fn convert_to_gif(input: &str, output: &str) -> Result<(), Box<dyn std::error::Error>> {
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

pub fn test_recording(output: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("This will record your screen for 3 seconds to test if everything works.");
    println!("If you don't see any errors, then your system is properly configured.");

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
