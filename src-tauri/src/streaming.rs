use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tauri::ipc::{Channel, InvokeResponseBody};
use xcap::{Frame, VideoRecorder};

use crate::{
    conversions,
    encoder::{self, EncoderConfig, VideoEncoder},
    monitors,
};

#[derive(serde::Deserialize)]
pub struct ShareConfig {
    monitor_id: u32,
}

pub struct Recording {
    recorder: VideoRecorder,
    stop_flag: Arc<AtomicBool>,
    consumer: JoinHandle<()>,
}

pub fn start_recording(
    config: ShareConfig,
    on_chunk: Channel<InvokeResponseBody>,
) -> Result<Recording, String> {
    let monitor = monitors::get_monitor_by_id(config.monitor_id)?;
    let (recorder, rx) = monitor.video_recorder().map_err(|e| e.to_string())?;

    let encoder = encoder::build_encoder(&EncoderConfig {
        width: 0,
        height: 0,
        fps: 30,
        bitrate: 2_500_000,
    })?;
    recorder.start().map_err(|e| e.to_string())?;

    let stop_flag = Arc::new(AtomicBool::new(false));
    let flag = stop_flag.clone();

    let consumer = thread::spawn(move || encode_loop(rx, encoder, on_chunk, flag));

    Ok(Recording {
        recorder,
        stop_flag,
        consumer,
    })
}

impl Recording {
    pub fn stop(self) -> Result<(), String> {
        self.stop_flag.store(true, Ordering::Relaxed);
        self.recorder.stop().map_err(|e| e.to_string())?;
        self.consumer
            .join()
            .map_err(|_| "thread de encode panicou".to_string())
    }
}

fn encode_loop(
    rx: Receiver<Frame>,
    mut encoder: Box<dyn VideoEncoder>,
    on_chunk: Channel<InvokeResponseBody>,
    stop_flag: Arc<AtomicBool>,
) {
    println!("Encoder: {}", encoder.name());

    let mut stats = StreamStats::default();
    let mut converter: Option<((u32, u32), conversions::Converter)> = None;

    while !stop_flag.load(Ordering::Relaxed) {
        let frame = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => continue, // volta a checar o stop_flag
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let (frame, dropped) = take_latest(&rx, frame);
        stats.add_dropped(dropped);

        let src = (frame.width, frame.height);
        let target = conversions::encode_size(src.0, src.1);
        let raw_len = target.0 as usize * target.1 as usize * 4;

        if converter.as_ref().map(|(s, _)| *s) != Some(src) {
            converter = Some((src, conversions::Converter::new(target)));
        }
        let (_, conv) = converter.as_mut().unwrap();

        let (yuv, timings) = conv.convert(&frame);
        stats.add_conversion(timings.resize, timings.yuv);

        let t1 = Instant::now();

        let chunk = match encoder.encode(&yuv) {
            Ok(Some(chunk)) => chunk,
            Ok(None) => {
                stats.add_skipped();
                continue;
            }
            Err(e) => {
                eprintln!("falha no encode, encerrando stream: {e}");
                break;
            }
        };
        let enc_time = t1.elapsed();
        stats.add_frame(raw_len, chunk.data.len(), enc_time);

        let mut payload = Vec::with_capacity(chunk.data.len() + 1);
        payload.push(chunk.is_key as u8);
        payload.extend_from_slice(&chunk.data);

        if on_chunk.send(InvokeResponseBody::Raw(payload)).is_err() {
            // frontend sumiu (janela fechada ou componente desmontado)
            break;
        }

        stats.report_if_due();
    }
}

/// O encode é mais lento que a captura: fica só com o frame mais recente.
/// Sem isso a fila cresce sem limite e a transmissão atrasa cada vez mais.
/// Retorna o frame retido e quantos foram descartados no caminho.
fn take_latest(rx: &Receiver<Frame>, mut frame: Frame) -> (Frame, u32) {
    let mut dropped = 0;
    while let Ok(newer) = rx.try_recv() {
        frame = newer;
        dropped += 1;
    }
    (frame, dropped)
}

/// Métricas da janela de 1s, impressas em `report_if_due`.
#[derive(Default)]
struct StreamStats {
    window_start: Option<Instant>,
    frames: u32,
    raw_bytes: usize,
    enc_bytes: usize,
    resize_time: Duration,
    yuv_time: Duration,
    enc_time: Duration,
    skipped: u32,
    dropped: u32,
}

impl StreamStats {
    fn add_dropped(&mut self, n: u32) {
        self.dropped += n;
    }

    fn add_skipped(&mut self) {
        self.skipped += 1;
    }

    fn add_conversion(&mut self, resize: Duration, yuv: Duration) {
        self.resize_time += resize;
        self.yuv_time += yuv;
    }

    fn add_frame(&mut self, raw_bytes: usize, enc_bytes: usize, enc: Duration) {
        self.frames += 1;
        self.raw_bytes += raw_bytes;
        self.enc_bytes += enc_bytes;
        self.enc_time += enc;
    }

    /// Imprime as métricas se a janela de 1s completou e zera os contadores.
    fn report_if_due(&mut self) {
        let start = *self.window_start.get_or_insert_with(Instant::now);
        let elapsed = start.elapsed();
        if elapsed < Duration::from_secs(1) {
            return;
        }

        let secs = elapsed.as_secs_f64();
        let n = self.frames as f64;
        let busy = (self.resize_time + self.yuv_time + self.enc_time).as_secs_f64() / secs * 100.0;
        println!(
            "fps: {:.1} | bitrate: {:.0} kbps | resize: {:.1} ms | yuv: {:.1} ms | encode: {:.1} ms | compressão: {:.0}:1 | ocupação: {:.0}% | skip: {} | drop: {}",
            n / secs,
            self.enc_bytes as f64 * 8.0 / secs / 1000.0,
            self.resize_time.as_secs_f64() * 1000.0 / n,
            self.yuv_time.as_secs_f64() * 1000.0 / n,
            self.enc_time.as_secs_f64() * 1000.0 / n,
            self.raw_bytes as f64 / self.enc_bytes as f64,
            busy,
            self.skipped,
            self.dropped,
        );

        *self = StreamStats::default();
        // zera a janela mas preserva o instante de início da próxima
        self.window_start = Some(Instant::now());
    }
}
