use clap::{Parser, ValueEnum};
use std::path::PathBuf;

/// Cavabg - Native CAVA visualizer for Hyprland
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Configuration file path
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Log level
    #[arg(short, long, value_enum, default_value_t = LogLevel::Info)]
    pub log_level: LogLevel,

    /// Show version information
    #[arg(short = 'V', long)]
    pub version: bool,

    /// List available monitors
    #[arg(short, long)]
    pub list_monitors: bool,

    /// Test configuration without running
    #[arg(long)]
    pub test_config: bool,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn to_filter(&self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

impl Cli {
    pub fn parse() -> Self {
        Parser::parse()
    }

    pub fn init_logging(&self) {
        let filter = self.log_level.to_filter();
        std::env::set_var("RUST_LOG", filter);
        env_logger::init();
    }

    pub fn show_version() {
        println!("Cavabg {}", env!("CARGO_PKG_VERSION"));
        println!("Authors: {}", env!("CARGO_PKG_AUTHORS"));
        println!("Repository: {}", env!("CARGO_PKG_REPOSITORY"));
        println!("License: {}", env!("CARGO_PKG_LICENSE"));
    }
}
