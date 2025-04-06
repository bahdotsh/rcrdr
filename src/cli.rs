use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
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
