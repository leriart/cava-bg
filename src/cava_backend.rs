// src/cava_backend.rs
use anyhow::{Context, Result};
use log::info;
use std::io::{BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use crate::app_config::Config;

pub struct CavaBackend {
    _process: Child,
}

impl CavaBackend {
    pub fn new(bar_count: usize, config: &Config) -> Result<(Self, Receiver<Vec<f32>>)> {
        let (tx, rx): (Sender<Vec<f32>>, Receiver<Vec<f32>>) = channel();
        let cava_config_str = config.to_cava_raw_config();

        let mut cmd = Command::new("cava")
            .arg("-p")
            .arg("/dev/stdin")
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .context("Failed to spawn cava process")?;

        if let Some(mut stdin) = cmd.stdin.take() {
            stdin.write_all(cava_config_str.as_bytes())
                .context("Failed to write config to cava stdin")?;
            stdin.flush().context("Failed to flush cava stdin")?;
        }

        let stdout = cmd.stdout.take()
            .context("Failed to get cava stdout")?;
        let mut reader = BufReader::new(stdout);

        info!("cava backend started, reading {} bars", bar_count);

        thread::spawn(move || {
            let mut buffer = vec![0u8; bar_count * 2];
            loop {
                match reader.read_exact(&mut buffer) {
                    Ok(_) => {
                        let mut unpacked = vec![0.0; bar_count];
                        for (i, chunk) in buffer.chunks_exact(2).enumerate() {
                            let num = u16::from_le_bytes([chunk[0], chunk[1]]);
                            unpacked[i] = (num as f32) / 65530.0;
                        }
                        if tx.send(unpacked).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            info!("cava reader thread finished");
        });

        Ok((Self { _process: cmd }, rx))
    }
}

impl Drop for CavaBackend {
    fn drop(&mut self) {
        let _ = self._process.kill();
        let _ = self._process.wait();
    }
}