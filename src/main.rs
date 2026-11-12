use std::{fs::File, io::Write};

use openh264::{
    encoder::Encoder,
    formats::{RgbaSliceU8, YUVBuffer},
};
use xcap::Monitor;

fn main() {
    println!("Starting recording");
    screen_record();
}

fn screen_record() {
    let monitors = Monitor::all().expect("Monitors not found");

    let monitor = monitors
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .expect("No primary monitor found");

    let (video_recorder, sx) = monitor.video_recorder().unwrap();

    let mut encoder = Encoder::new().expect("Error initiating encoder");
    let mut out = File::create("./saida.h264").expect("falha ao criar arquivo");

    video_recorder.start().expect("aham");

    loop {
        match sx.recv() {
            Ok(frame) => {
                let dims = (frame.width as usize, frame.height as usize);
                let rgba = RgbaSliceU8::new(&frame.raw, dims);
                let yuv = YUVBuffer::from_rgba8_source(rgba);

                let bitstream = encoder.encode(&yuv).expect("fail encoding");
                let bytes = bitstream.to_vec();

                println!("Bytes: {}", bytes.len());
                println!("Original: {}", frame.raw.len());
                out.write_all(&bytes).expect("Fail writing in disc");
            }
            Err(_) => break,
        }
    }
    video_recorder.stop().ok();
    println!("Screen recorded");
}
