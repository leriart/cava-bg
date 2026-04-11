use anyhow::Result;
use std::io::{BufReader, Read};
use std::process::{Command, Stdio, Child, ChildStdout};
use std::thread;
use std::sync::mpsc::{channel, Receiver, Sender};

pub struct CavaBackend {
    _process: Child,
    reader: BufReader<ChildStdout>,
}

impl CavaBackend {
    pub fn new(bar_count: usize) -> Result<(Self, Receiver<Vec<f32>>)> {
        let (tx, rx): (Sender<Vec<f32>>, Receiver<Vec<f32>>) = channel();
        
        let mut cmd = Command::new("cava")
            .arg("-p")
            .arg("/dev/stdin")
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()?;

        // ... (código para escribir la configuración de cava en su stdin) ...
        let stdout = cmd.stdout.take().unwrap();
        let reader = BufReader::new(stdout);
        
        // Hilo separado para leer cava
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
                        if tx.send(unpacked).is_err() { break; }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok((Self { _process: cmd, reader }, rx))
    }
}