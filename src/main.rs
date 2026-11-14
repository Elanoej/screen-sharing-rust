mod monitors;

use std::{
    fs::File,
    io::{Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use openh264::{
    encoder::Encoder,
    formats::{RgbaSliceU8, YUVBuffer},
};
use xcap::Frame;

struct Nvenc {
    child: Child,
    stdin: ChildStdin,
    enc_bytes: Arc<AtomicUsize>,
    writer: JoinHandle<()>,
}

impl Nvenc {
    fn new(w: u32, h: u32, fps: u32, path: &str) -> Self {
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            // timestamp de cada frame = hora real em que chegou ao pipe
            .args(["-use_wallclock_as_timestamps", "1"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
            .arg("-s")
            .arg(format!("{w}x{h}"))
            .args(["-i", "-"])
            // saída com fps constante: repete frames onde a tela não mudou
            .args(["-fps_mode", "cfr", "-r"])
            .arg(fps.to_string())
            .args(["-c:v", "h264_nvenc", "-preset", "p4", "-tune", "ll"])
            .args(["-bf", "0", "-b:v", "8M"])
            .arg("-g")
            .arg((fps * 2).to_string())
            // contêiner que guarda tempo (o .h264 cru não guarda)
            .args(["-f", "matroska", "pipe:1"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("não consegui iniciar o ffmpeg (está no PATH?)");

        let stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let enc_bytes = Arc::new(AtomicUsize::new(0));
        let counter = enc_bytes.clone();
        let mut file = File::create(path).expect("falha ao criar arquivo");

        let writer = std::thread::spawn(move || {
            let mut buf = [0u8; 64 * 1024];
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        counter.fetch_add(n, Ordering::Relaxed);
                        file.write_all(&buf[..n]).expect("falha escrevendo");
                    }
                }
            }
        });

        Self {
            child,
            stdin,
            enc_bytes,
            writer,
        }
    }

    fn push(&mut self, rgba: &[u8]) {
        self.stdin.write_all(rgba).expect("ffmpeg fechou o pipe");
    }

    fn take_bytes(&self) -> usize {
        self.enc_bytes.swap(0, Ordering::Relaxed)
    }

    fn finish(mut self) {
        drop(self.stdin);
        self.writer.join().ok();
        self.child.wait().ok();
    }
}

fn main() {
    println!("Starting recording");
    screen_record();
}

fn screen_record() {
    let monitor = monitors::get_main_monitor();
    let (video_recorder, sx) = monitor.video_recorder().unwrap();

    let mut nvenc: Option<Nvenc> = None;

    // let mut encoder = Encoder::new().expect("Error initiating encoder");
    // let mut out = File::create("./saida.h264").expect("falha ao criar arquivo");

    let mut window_start = Instant::now();
    let mut frames: u32 = 0;
    let mut raw_bytes: usize = 0;
    let mut enc_bytes: usize = 0;
    let mut push_time = Duration::ZERO;
    // let mut conv_time = Duration::ZERO;
    // let mut enc_time = Duration::ZERO;

    video_recorder.start().expect("Fail trying to record");

    while let Ok(frame) = sx.recv() {
        let enc = nvenc
            .get_or_insert_with(|| Nvenc::new(frame.width, frame.height, 60, "./saida_nvenc.mkv"));

        let t0 = Instant::now();
        enc.push(&frame.raw);
        push_time += t0.elapsed();

        frames += 1;
        raw_bytes += frame.raw.len();

        let elapsed = window_start.elapsed();
        if elapsed >= Duration::from_secs(1) {
            enc_bytes += enc.take_bytes();
            let secs = elapsed.as_secs_f64();
            let n = frames as f64;
            println!(
                "fps: {:.1} | bitrate: {:.0} kbps | push: {:.1} ms | compressão: {:.0}:1 | ocupação: {:.0}%",
                n / secs,
                enc_bytes as f64 * 8.0 / secs / 1000.0,
                push_time.as_secs_f64() * 1000.0 / n,
                raw_bytes as f64 / enc_bytes.max(1) as f64,
                push_time.as_secs_f64() / secs * 100.0,
            );
            window_start = Instant::now();
            frames = 0;
            raw_bytes = 0;
            enc_bytes = 0;
            push_time = Duration::ZERO;
        }
    }

    video_recorder.stop().ok();
    if let Some(e) = nvenc {
        e.finish();
    }
    println!("Screen recorded");
}

fn from_rgba_to_yuv(frame: &Frame) -> YUVBuffer {
    let dims = (frame.width as usize, frame.height as usize);
    let rgba = RgbaSliceU8::new(&frame.raw, dims);
    YUVBuffer::from_rgba8_source(rgba)
}

fn my_encode(encoder: &mut Encoder, yuv_source: &YUVBuffer) -> Vec<u8> {
    let bitstream = encoder.encode(yuv_source).expect("Fail enconding");
    bitstream.to_vec()
}
