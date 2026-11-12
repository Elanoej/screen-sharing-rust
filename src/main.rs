mod monitors;

use std::{
    fs::File,
    io::Write,
    time::{Duration, Instant},
};

use openh264::{
    encoder::Encoder,
    formats::{RgbaSliceU8, YUVBuffer},
};
use xcap::Frame;

fn main() {
    println!("Starting recording");
    screen_record();
}

fn screen_record() {
    let monitor = monitors::get_main_monitor();

    let (video_recorder, sx) = monitor.video_recorder().unwrap();

    let mut encoder = Encoder::new().expect("Error initiating encoder");
    let mut out = File::create("./saida.h264").expect("falha ao criar arquivo");

    let mut window_start = Instant::now();
    let mut frames: u32 = 0;
    let mut raw_bytes: usize = 0;
    let mut enc_bytes: usize = 0;
    let mut conv_time = Duration::ZERO;
    let mut enc_time = Duration::ZERO;

    video_recorder.start().expect("Fail trying to record");

    loop {
        match sx.recv() {
            Ok(frame) => {
                let t0 = Instant::now();
                let yuv = from_rgba_to_yuv(&frame);
                conv_time += t0.elapsed();

                let t1 = Instant::now();
                let bytes = my_encode(&mut encoder, &yuv);
                enc_time += t1.elapsed();

                frames += 1;
                raw_bytes += frame.raw.len();
                enc_bytes += bytes.len();

                out.write_all(&bytes).expect("Fail writing in disc");

                let elapsed = window_start.elapsed();
                if elapsed >= Duration::from_secs(1) {
                    let secs = elapsed.as_secs_f64();
                    let n = frames as f64;
                    let busy = (conv_time + enc_time).as_secs_f64() / secs * 100.0;
                    println!(
                        "fps: {:.1} | bitrate: {:.0} kbps | converter: {:.1} ms | encode: {:.1} ms | compressão: {:.0}:1 | ocupação: {:.0}%",
                        n / secs,
                        enc_bytes as f64 * 8.0 / secs / 1000.0,
                        conv_time.as_secs_f64() * 1000.0 / n,
                        enc_time.as_secs_f64() * 1000.0 / n,
                        raw_bytes as f64 / enc_bytes as f64,
                        busy,
                    );
                    window_start = Instant::now();
                    frames = 0;
                    raw_bytes = 0;
                    enc_bytes = 0;
                    conv_time = Duration::ZERO;
                    enc_time = Duration::ZERO;
                }
            }
            Err(_) => break,
        }
    }
    video_recorder.stop().ok();
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
