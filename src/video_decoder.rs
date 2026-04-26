#![allow(dead_code)]

use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender, TryRecvError};
use ffmpeg::format;
use ffmpeg::frame;
use ffmpeg::media::Type;
use ffmpeg::software::scaling::{context::Context as ScalingContext, flag::Flags};
use ffmpeg::util::format::pixel::Pixel;
use ffmpeg_next as ffmpeg;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub pts_seconds: f64,
    pub duration_seconds: f64,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct VideoDecoderConfig {
    pub target_width: u32,
    pub target_height: u32,
    pub looping: bool,
    pub max_buffered_frames: usize,
    pub frame_cache_size: usize,
}

impl Default for VideoDecoderConfig {
    fn default() -> Self {
        Self {
            target_width: 1920,
            target_height: 1080,
            looping: true,
            max_buffered_frames: 6,
            frame_cache_size: 120,
        }
    }
}

#[derive(Debug)]
pub struct VideoDecoder {
    rx: Receiver<VideoFrame>,
    cached_frames: Arc<Mutex<VecDeque<VideoFrame>>>,
    last_frame: Option<VideoFrame>,
    stream_duration_seconds: Option<f64>,
    source_path: PathBuf,
}

impl VideoDecoder {
    pub fn new(path: impl AsRef<Path>, cfg: VideoDecoderConfig) -> Result<Self> {
        ffmpeg::init().context("Failed to initialize FFmpeg")?;
        let source_path = path.as_ref().to_path_buf();
        let stream_duration_seconds = inspect_duration(&source_path).ok().flatten();

        let (tx, rx) = bounded::<VideoFrame>(cfg.max_buffered_frames.max(2));
        let cached_frames = Arc::new(Mutex::new(VecDeque::with_capacity(
            cfg.frame_cache_size.max(8),
        )));
        let cache_ref = cached_frames.clone();
        let source_for_thread = source_path.clone();

        thread::Builder::new()
            .name(format!("ffmpeg-decode:{}", source_for_thread.display()))
            .spawn(move || {
                if let Err(e) = decode_loop(source_for_thread, cfg, tx, cache_ref) {
                    log::error!("FFmpeg decode loop ended with error: {e:#}");
                }
            })
            .context("Failed to spawn FFmpeg decode thread")?;

        Ok(Self {
            rx,
            cached_frames,
            last_frame: None,
            stream_duration_seconds,
            source_path,
        })
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub fn stream_duration_seconds(&self) -> Option<f64> {
        self.stream_duration_seconds
    }

    pub fn last_frame(&self) -> Option<VideoFrame> {
        self.last_frame.clone()
    }

    pub fn cached_frames(&self) -> Vec<VideoFrame> {
        self.cached_frames.lock().iter().cloned().collect()
    }

    pub fn poll_latest_for_time(&mut self, playback_time_seconds: f64) -> Option<VideoFrame> {
        // Intelligent frame skipping: consume all late frames and keep
        // the one closest to the target playback time.
        let mut selected: Option<VideoFrame> = None;

        loop {
            match self.rx.try_recv() {
                Ok(frame) => {
                    if frame.pts_seconds <= playback_time_seconds + 0.020 {
                        selected = Some(frame);
                        continue;
                    }

                    // Future frame: keep the latest valid frame and preserve this one as fallback
                    if selected.is_none() {
                        selected = Some(frame);
                    }
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        if let Some(frame) = selected {
            self.last_frame = Some(frame.clone());
            return Some(frame);
        }

        self.last_frame.clone()
    }
}

fn inspect_duration(path: &Path) -> Result<Option<f64>> {
    let ictx =
        format::input(path).with_context(|| format!("Failed opening media: {}", path.display()))?;
    let stream = ictx
        .streams()
        .best(Type::Video)
        .ok_or_else(|| anyhow!("No video stream found in {}", path.display()))?;

    let dur = stream.duration();
    if dur > 0 {
        let tb: f64 = stream.time_base().into();
        Ok(Some(dur as f64 * tb))
    } else {
        Ok(None)
    }
}

fn decode_loop(
    path: PathBuf,
    cfg: VideoDecoderConfig,
    tx: Sender<VideoFrame>,
    cache: Arc<Mutex<VecDeque<VideoFrame>>>,
) -> Result<()> {
    let mut sequence: u64 = 0;
    loop {
        let status = decode_one_pass(&path, cfg, &tx, &cache, &mut sequence);
        match status {
            Ok(()) => {
                if cfg.looping {
                    continue;
                }
                break;
            }
            Err(e) => {
                log::error!("Decode pass failed for {}: {e:#}", path.display());
                if cfg.looping {
                    thread::sleep(Duration::from_millis(250));
                    continue;
                }
                return Err(e);
            }
        }
    }

    Ok(())
}

fn decode_one_pass(
    path: &Path,
    cfg: VideoDecoderConfig,
    tx: &Sender<VideoFrame>,
    cache: &Arc<Mutex<VecDeque<VideoFrame>>>,
    sequence: &mut u64,
) -> Result<()> {
    let mut ictx =
        format::input(path).with_context(|| format!("Failed opening media: {}", path.display()))?;
    let input_stream = ictx
        .streams()
        .best(Type::Video)
        .ok_or_else(|| anyhow!("No video stream found in {}", path.display()))?;

    let stream_index = input_stream.index();
    let time_base: f64 = input_stream.time_base().into();

    let context_decoder =
        ffmpeg::codec::context::Context::from_parameters(input_stream.parameters())
            .context("Failed creating codec context")?;
    let mut decoder = context_decoder
        .decoder()
        .video()
        .context("Failed creating video decoder")?;

    let src_w = decoder.width();
    let src_h = decoder.height();
    let dst_w = cfg.target_width.max(1);
    let dst_h = cfg.target_height.max(1);

    let mut scaler = ScalingContext::get(
        decoder.format(),
        src_w,
        src_h,
        Pixel::RGBA,
        dst_w,
        dst_h,
        Flags::BILINEAR,
    )
    .context("Failed creating software scaling context")?;

    let mut decoded = frame::Video::empty();
    for (stream, packet) in ictx.packets() {
        if stream.index() != stream_index {
            continue;
        }

        decoder
            .send_packet(&packet)
            .context("Failed sending packet to decoder")?;
        while decoder.receive_frame(&mut decoded).is_ok() {
            let frame = convert_frame(&decoded, &mut scaler, dst_w, dst_h, time_base, *sequence)?;
            *sequence += 1;

            // Bounded queue + local cache
            if tx.send(frame.clone()).is_err() {
                return Ok(());
            }

            let mut cache_lock = cache.lock();
            cache_lock.push_back(frame);
            while cache_lock.len() > cfg.frame_cache_size.max(8) {
                cache_lock.pop_front();
            }
        }
    }

    decoder.send_eof().ok();
    while decoder.receive_frame(&mut decoded).is_ok() {
        let frame = convert_frame(&decoded, &mut scaler, dst_w, dst_h, time_base, *sequence)?;
        *sequence += 1;
        if tx.send(frame.clone()).is_err() {
            return Ok(());
        }
        let mut cache_lock = cache.lock();
        cache_lock.push_back(frame);
        while cache_lock.len() > cfg.frame_cache_size.max(8) {
            cache_lock.pop_front();
        }
    }

    Ok(())
}

fn convert_frame(
    decoded: &frame::Video,
    scaler: &mut ScalingContext,
    dst_w: u32,
    dst_h: u32,
    time_base: f64,
    sequence: u64,
) -> Result<VideoFrame> {
    let mut rgb_frame = frame::Video::new(Pixel::RGBA, dst_w, dst_h);
    scaler
        .run(decoded, &mut rgb_frame)
        .context("Scaling/conversion to RGBA failed")?;

    let stride = rgb_frame.stride(0);
    let src = rgb_frame.data(0);
    let mut rgba = vec![0u8; (dst_w * dst_h * 4) as usize];

    let row_bytes = (dst_w as usize) * 4;
    for y in 0..dst_h as usize {
        let src_off = y * stride;
        let dst_off = y * row_bytes;
        rgba[dst_off..dst_off + row_bytes].copy_from_slice(&src[src_off..src_off + row_bytes]);
    }

    let pts = decoded.pts().unwrap_or(0);
    let duration = decoded.packet().duration;

    let pts_seconds = pts as f64 * time_base;
    let duration_seconds = if duration > 0 {
        duration as f64 * time_base
    } else {
        1.0 / 60.0
    };

    Ok(VideoFrame {
        rgba,
        width: dst_w,
        height: dst_h,
        pts_seconds,
        duration_seconds,
        sequence,
    })
}
