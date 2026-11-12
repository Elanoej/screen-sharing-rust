use xcap::{Monitor, image::RgbaImage};

fn main() {
    println!("Hello, world!");

    screen_record();
}

fn screen_shot() {
    let monitors = Monitor::all().expect("Monitors not found");

    let monitor = monitors
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .expect("No primary monitor found");

    let image = monitor.capture_image().unwrap();

    image.save("./test.png").unwrap();
    println!("Screenshot!")
}

fn screen_record() {
    let monitors = Monitor::all().expect("Monitors not found");

    let monitor = monitors
        .into_iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .expect("No primary monitor found");

    let (video_recorder, sx) = monitor.video_recorder().unwrap();

    video_recorder.start().expect("aham");
    let mut frame_index: u64 = 0;

    loop {
        match sx.recv() {
            Ok(frame) => {
                let image = RgbaImage::from_raw(frame.width, frame.height, frame.raw)
                    .expect("failed to create image from frame");

                let path = format!("./video/frame_{frame_index:06}.png");
                image.save(path).expect("Erro ao gravar no disco");
                frame_index += 1;
            }
            Err(_) => break,
        }
    }
    video_recorder.stop().ok();
    println!("Screen recorded")
}
