use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use openh264::{
    OpenH264API,
    encoder::{
        BitRate, Complexity, EncodedBitStream, Encoder, EncoderConfig, FrameRate, FrameType,
        IntraFramePeriod, UsageType,
    },
};
use tauri::ipc::{Channel, InvokeResponseBody};
use xcap::{Frame, VideoRecorder};

use crate::{conversions, monitors};

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

    let encoder = build_encoder()?;
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

/// Encoder com perfil de tela compartilhada em tempo real: mais rápido, menos
/// qualidade.
fn streaming_encoder_config() -> EncoderConfig {
    EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .complexity(Complexity::Low)
        .max_frame_rate(FrameRate::from_hz(30.0))
        .bitrate(BitRate::from_bps(2_500_000))
        // I-frame a cada 60 frames (~2s): sem isso o decoder não tem como se
        // recuperar de um frame perdido e a transmissão congela.
        .intra_frame_period(IntraFramePeriod::from_num_frames(60))
        // fatia o frame em NALs de até 4 KB; é o que permite o encoder usar
        // várias threads (com slice único, num_threads não paraleliza nada).
        .max_slice_len(4_000)
        .num_threads(0)
}

fn build_encoder() -> Result<Encoder, String> {
    Encoder::with_api_config(OpenH264API::from_source(), streaming_encoder_config())
        .map_err(|e| e.to_string())
}

fn encode_loop(
    rx: Receiver<Frame>,
    mut encoder: Encoder,
    on_chunk: Channel<InvokeResponseBody>,
    stop_flag: Arc<AtomicBool>,
) {
    let mut stats = StreamStats::default();

    while !stop_flag.load(Ordering::Relaxed) {
        let frame = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => continue, // volta a checar o stop_flag
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let (frame, dropped) = take_latest(&rx, frame);
        stats.add_dropped(dropped);

        let target = conversions::encode_size(frame.width, frame.height);
        let raw_len = target.0 as usize * target.1 as usize * 4;

        let t0 = Instant::now();
        let yuv = conversions::frame_to_yuv(frame, target);
        let conv_time = t0.elapsed();

        let t1 = Instant::now();
        let bitstream = match encoder.encode(&yuv) {
            Ok(bitstream) => bitstream,
            Err(e) => {
                // não derruba o processo: para o stream e reporta o motivo
                eprintln!("falha no encode, encerrando stream: {e}");
                break;
            }
        };
        let enc_time = t1.elapsed();

        let payload = match nal_payload(&bitstream) {
            Some(payload) => payload,
            None => {
                stats.add_skipped();
                continue;
            }
        };

        stats.add_frame(raw_len, payload.len() - 1, conv_time, enc_time);

        if on_chunk.send(InvokeResponseBody::Raw(payload)).is_err() {
            // frontend sumiu (janela fechada ou componente desmontado)
            break;
        }

        stats.report_if_due();
    }
}

/// Protocolo do chunk enviado ao frontend:
/// byte 0 = 1 se keyframe (IDR/I), 0 caso contrário; resto = NALs H.264.
/// Retorna `None` quando o encoder não gerou nada aproveitável (Skip/Invalid).
fn nal_payload(bitstream: &EncodedBitStream<'_>) -> Option<Vec<u8>> {
    if matches!(bitstream.frame_type(), FrameType::Skip | FrameType::Invalid) {
        return None;
    }

    let is_key = matches!(bitstream.frame_type(), FrameType::IDR | FrameType::I);

    let mut payload = Vec::new();
    payload.push(is_key as u8);
    bitstream.write_vec(&mut payload);
    Some(payload)
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
    conv_time: Duration,
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

    fn add_frame(&mut self, raw_bytes: usize, enc_bytes: usize, conv: Duration, enc: Duration) {
        self.frames += 1;
        self.raw_bytes += raw_bytes;
        self.enc_bytes += enc_bytes;
        self.conv_time += conv;
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
        let busy = (self.conv_time + self.enc_time).as_secs_f64() / secs * 100.0;
        println!(
            "fps: {:.1} | bitrate: {:.0} kbps | converter: {:.1} ms | encode: {:.1} ms | compressão: {:.0}:1 | ocupação: {:.0}% | skip: {} | drop: {}",
            n / secs,
            self.enc_bytes as f64 * 8.0 / secs / 1000.0,
            self.conv_time.as_secs_f64() * 1000.0 / n,
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
