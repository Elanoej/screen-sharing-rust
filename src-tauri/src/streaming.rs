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
        BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod,
        UsageType,
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

    // perfil de tela compartilhada em tempo real: mais rápido, menos qualidade
    let encoder_config = EncoderConfig::new()
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
        .num_threads(0);

    let encoder = Encoder::with_api_config(OpenH264API::from_source(), encoder_config)
        .map_err(|e| e.to_string())?;

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
    mut encoder: Encoder,
    on_chunk: Channel<InvokeResponseBody>,
    stop_flag: Arc<AtomicBool>,
) {
    let mut window_start = Instant::now();
    let mut frames: u32 = 0;
    let mut raw_bytes: usize = 0;
    let mut enc_bytes: usize = 0;
    let mut conv_time = Duration::ZERO;
    let mut enc_time = Duration::ZERO;
    let mut skipped: u32 = 0;
    let mut dropped: u32 = 0;

    while !stop_flag.load(Ordering::Relaxed) {
        let mut frame = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => continue, // volta a checar o stop_flag
            Err(RecvTimeoutError::Disconnected) => break,
        };

        // o encode é mais lento que a captura: fica só com o frame mais recente.
        // sem isso a fila cresce sem limite e a transmissão atrasa cada vez mais.
        while let Ok(newer) = rx.try_recv() {
            frame = newer;
            dropped += 1;
        }

        let target = conversions::encode_size(frame.width, frame.height);
        let raw_len = target.0 as usize * target.1 as usize * 4;

        let t0 = Instant::now();
        let yuv = conversions::frame_to_yuv(frame, target);
        conv_time += t0.elapsed();

        let t1 = Instant::now();
        let bitstream = encoder.encode(&yuv).expect("Fail encoding");
        enc_time += t1.elapsed();

        if matches!(bitstream.frame_type(), FrameType::Skip | FrameType::Invalid) {
            skipped += 1;
            continue;
        }

        let is_key = matches!(bitstream.frame_type(), FrameType::IDR | FrameType::I);

        let mut payload = Vec::new();
        payload.push(is_key as u8);
        bitstream.write_vec(&mut payload);

        frames += 1;
        raw_bytes += raw_len;
        enc_bytes += payload.len() - 1;

        if on_chunk.send(InvokeResponseBody::Raw(payload)).is_err() {
            // frontend sumiu (janela fechada ou componente desmontado)
            break;
        }

        let elapsed = window_start.elapsed();
        if elapsed >= Duration::from_secs(1) {
            let secs = elapsed.as_secs_f64();
            let n = frames as f64;
            let busy = (conv_time + enc_time).as_secs_f64() / secs * 100.0;
            println!(
                "fps: {:.1} | bitrate: {:.0} kbps | converter: {:.1} ms | encode: {:.1} ms | compressão: {:.0}:1 | ocupação: {:.0}% | skip: {} | drop: {}",
                n / secs,
                enc_bytes as f64 * 8.0 / secs / 1000.0,
                conv_time.as_secs_f64() * 1000.0 / n,
                enc_time.as_secs_f64() * 1000.0 / n,
                raw_bytes as f64 / enc_bytes as f64,
                busy,
                skipped,
                dropped,
            );
            window_start = Instant::now();
            frames = 0;
            raw_bytes = 0;
            enc_bytes = 0;
            conv_time = Duration::ZERO;
            enc_time = Duration::ZERO;
            skipped = 0;
            dropped = 0;
        }
    }
}

