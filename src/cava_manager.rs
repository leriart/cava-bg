use anyhow::{Context, Result};
use log::{info, warn};
use std::io::{BufReader, Read, Write};
use std::process::{Child, Command, Stdio, ChildStdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::config::Config;

pub struct CavaManager {
    process: Option<Child>,
    reader: Option<BufReader<ChildStdout>>,
    bar_count: u32,
    running: Arc<AtomicBool>,
}

impl CavaManager {
    pub fn new(config: &Config) -> Result<Self> {
        let mut manager = Self {
            process: None,
            reader: None,
            bar_count: config.bars.amount,
            running: Arc::new(AtomicBool::new(true)),
        };
        manager.start(config)?;
        Ok(manager)
    }

    pub fn start(&mut self, config: &Config) -> Result<()> {
        self.stop();
        let cava_config = config.to_cava_raw_config();
        info!("Starting cava process with raw output...");
        let mut child = Command::new("cava")
            .arg("-p")
            .arg("/dev/stdin")
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .context("Failed to spawn cava process")?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(cava_config.as_bytes())
                .context("Failed to write config to cava stdin")?;
            stdin.flush()
                .context("Failed to flush cava stdin")?;
        }
        if let Some(stdout) = child.stdout.take() {
            self.reader = Some(BufReader::new(stdout));
            self.process = Some(child);
            info!("cava started successfully");
        } else {
            return Err(anyhow::anyhow!("Failed to get cava stdout"));
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(mut process) = self.process.take() {
            info!("Stopping cava process...");
            let _ = process.kill();
            let _ = process.wait();
            self.reader = None;
            info!("cava stopped");
        }
    }

    pub fn take_reader(&mut self) -> Result<BufReader<ChildStdout>> {
        self.reader.take().ok_or_else(|| anyhow::anyhow!("No reader available"))
    }

    pub fn is_running(&self) -> bool {
        self.process.is_some()
    }

    pub fn bar_count(&self) -> u32 {
        self.bar_count
    }
}

impl Drop for CavaManager {
    fn drop(&mut self) {
        self.stop();
        self.running.store(false, Ordering::SeqCst);
    }
}