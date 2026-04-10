use anyhow::{Context, Result};
use log::{info, warn};
use std::io::{BufReader, Read};
use std::process::{Command, Stdio, Child, ChildStdout};
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
        match Command::new("cava")
            .arg("-p")
            .arg("/dev/stdin")
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
        {
            Ok(mut process) => {
                if let Some(stdin) = process.stdin.take() {
                    use std::io::Write;
                    let mut writer = std::io::BufWriter::new(stdin);
                    writer.write_all(cava_config.as_bytes())
                        .context("Failed to write config to cava stdin")?;
                    writer.flush()
                        .context("Failed to flush cava stdin")?;
                }
                if let Some(stdout) = process.stdout.take() {
                    self.reader = Some(BufReader::new(stdout));
                    self.process = Some(process);
                    info!("cava started successfully with raw output");
                    info!("cava config:\n{}", cava_config);
                } else {
                    return Err(anyhow::anyhow!("Failed to get cava stdout"));
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to start cava process: {}", e));
            }
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

    pub fn read_audio_data(&mut self) -> Result<Option<Vec<f32>>> {
        if let Some(reader) = &mut self.reader {
            let buffer_size = self.bar_count as usize * 2;
            let mut buffer = vec![0u8; buffer_size];
            match reader.read_exact(&mut buffer) {
                Ok(_) => {
                    let mut audio_data = Vec::with_capacity(self.bar_count as usize);
                    for chunk in buffer.chunks_exact(2) {
                        let value = u16::from_le_bytes([chunk[0], chunk[1]]);
                        let normalized = (value as f32) / 65530.0;
                        audio_data.push(normalized);
                    }
                    Ok(Some(audio_data))
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
                Err(e) => {
                    warn!("Error reading from cava: {}", e);
                    self.stop();
                    Err(anyhow::anyhow!("Failed to read audio data: {}", e))
                }
            }
        } else {
            Ok(None)
        }
    }

    pub fn is_running(&self) -> bool {
        self.process.is_some()
    }

    pub fn bar_count(&self) -> u32 {
        self.bar_count
    }

    pub fn start_monitor(&self, config: Config) {
        let running = self.running.clone();
        let _monitor_config = config.clone();
        thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_secs(10));
            }
        });
    }

    pub fn take_reader(&mut self) -> Result<BufReader<ChildStdout>> {
        if let Some(reader) = self.reader.take() {
            Ok(reader)
        } else {
            Err(anyhow::anyhow!("Cava reader not available"))
        }
    }
}

impl Drop for CavaManager {
    fn drop(&mut self) {
        self.stop();
        self.running.store(false, Ordering::SeqCst);
    }
}