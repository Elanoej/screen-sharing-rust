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
    bitstream, conversions,
    encoder::{self, EncoderConfig, VideoEncoder},
    monitors,
    packager::{Packager, Packet},
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
    let mut packager = Packager::new();

    while !stop_flag.load(Ordering::Relaxed) {
        // Quanto tempo o consumidor fica PARADO esperando frame. Se isso domina
        // o tempo por frame, quem manda no ritmo é a captura, não este loop.
        let t_wait = Instant::now();
        let frame = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => continue, // volta a checar o stop_flag
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let wait = t_wait.elapsed();

        // Tudo o que o consumidor faz com o frame, do recv até o send terminar.
        let t_proc = Instant::now();

        let (frame, dropped) = take_latest(&rx, frame);
        stats.add_dropped(dropped);

        let src = (frame.width, frame.height);
        let target = conversions::encode_size(src.0, src.1);
        let raw_len = target.0 as usize * target.1 as usize * 4;
        stats.set_size(target);

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

        let t2 = Instant::now();
        let au = bitstream::parse_annexb(&chunk.data);
        stats.add_parse(t2.elapsed());

        let t3 = Instant::now();
        let packets = packager.push(&au, chunk.is_key);
        stats.add_pack(t3.elapsed());
        if packets.is_empty() {
            eprintln!("frame sem NALs Annex-B, descartado");
            continue;
        }

        let t4 = Instant::now();
        let packet_count = packets.len() as u32;
        let sent = send_packets(packets, &on_chunk);
        stats.add_send(t4.elapsed(), packet_count);
        if !sent {
            // frontend sumiu (janela fechada ou componente desmontado)
            break;
        }

        stats.add_frame_timing(wait, t_proc.elapsed());
        stats.report_if_due();
    }
}

/// Entrega os pacotes ao frontend. `false` quando o canal fechou.
fn send_packets(packets: Vec<Packet>, on_chunk: &Channel<InvokeResponseBody>) -> bool {
    packets.into_iter().all(|packet| {
        on_chunk
            .send(InvokeResponseBody::Raw(packet.into_payload()))
            .is_ok()
    })
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
///
/// Dois blocos: o de sempre (throughput e custo do encoder) e um de diagnóstico
/// que separa "esperando frame" de "processando frame". A razão entre os dois
/// diz quem é o gargalo; sem ela, ocupação baixa é ambígua.
#[derive(Default)]
struct StreamStats {
    window_start: Option<Instant>,
    frames: u32,
    raw_bytes: usize,
    enc_bytes: usize,
    packets: u32,
    resize_time: Duration,
    yuv_time: Duration,
    enc_time: Duration,
    parse_time: Duration,
    pack_time: Duration,
    send_time: Duration,
    skipped: u32,
    dropped: u32,
    size: (u32, u32),
    /// Amostras por frame (ms). Vec basta: ~60 amostras por janela.
    waits: Vec<f32>,
    procs: Vec<f32>,
    sends: Vec<f32>,
}

/// Percentil de uma amostra já ordenada.
fn percentile(sorted: &[f32], p: f64) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx]
}

impl StreamStats {
    fn add_dropped(&mut self, n: u32) {
        self.dropped += n;
    }

    fn add_skipped(&mut self) {
        self.skipped += 1;
    }

    fn set_size(&mut self, size: (u32, u32)) {
        self.size = size;
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

    fn add_parse(&mut self, d: Duration) {
        self.parse_time += d;
    }

    fn add_pack(&mut self, d: Duration) {
        self.pack_time += d;
    }

    fn add_send(&mut self, d: Duration, packets: u32) {
        self.send_time += d;
        self.packets += packets;
        self.sends.push(d.as_secs_f64() as f32 * 1000.0);
    }

    fn add_frame_timing(&mut self, wait: Duration, proc: Duration) {
        self.waits.push(wait.as_secs_f64() as f32 * 1000.0);
        self.procs.push(proc.as_secs_f64() as f32 * 1000.0);
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

        // Diagnóstico: separa espera de processamento.
        self.waits.sort_by(|a, b| a.total_cmp(b));
        self.procs.sort_by(|a, b| a.total_cmp(b));
        self.sends.sort_by(|a, b| a.total_cmp(b));

        let wait_p50 = percentile(&self.waits, 0.50);
        let wait_p95 = percentile(&self.waits, 0.95);
        let wait_max = self.waits.last().copied().unwrap_or(0.0);
        let proc_p50 = percentile(&self.procs, 0.50);
        let proc_p95 = percentile(&self.procs, 0.95);
        let proc_max = self.procs.last().copied().unwrap_or(0.0);
        let send_p50 = percentile(&self.sends, 0.50);
        let send_p95 = percentile(&self.sends, 0.95);

        // Capacidade do loop se a captura entregasse frames sem parar.
        let capacity = if proc_p50 > 0.0 { 1000.0 / proc_p50 } else { 0.0 };
        let verdict = if wait_p50 > proc_p50 { "captura" } else { "consumo" };

        // Histograma da espera: mostra se a captura é regular ou aos trancos.
        let mut buckets = [0u32; 5];
        for &w in &self.waits {
            let i = match w {
                w if w < 5.0 => 0,
                w if w < 15.0 => 1,
                w if w < 25.0 => 2,
                w if w < 40.0 => 3,
                _ => 4,
            };
            buckets[i] += 1;
        }

        println!(
            "  {}x{} | espera p50 {:.1} p95 {:.1} max {:.1} | proc p50 {:.1} p95 {:.1} max {:.1} | send p50 {:.2} p95 {:.2} | parse {:.2} pack {:.2} | pkt/frame {:.2} | capacidade ~{:.0} fps | gargalo: {} | espera<5 {} 5-15 {} 15-25 {} 25-40 {} >40 {}",
            self.size.0,
            self.size.1,
            wait_p50,
            wait_p95,
            wait_max,
            proc_p50,
            proc_p95,
            proc_max,
            send_p50,
            send_p95,
            self.parse_time.as_secs_f64() * 1000.0 / n,
            self.pack_time.as_secs_f64() * 1000.0 / n,
            self.packets as f64 / n,
            capacity,
            verdict,
            buckets[0],
            buckets[1],
            buckets[2],
            buckets[3],
            buckets[4],
        );

        *self = StreamStats::default();
        // zera a janela mas preserva o instante de início da próxima
        self.window_start = Some(Instant::now());
    }
}
