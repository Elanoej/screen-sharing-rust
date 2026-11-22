use xcap::{
    Frame,
    image::{RgbaImage, imageops, imageops::FilterType},
};

use openh264::formats::{RgbaSliceU8, YUVBuffer};

/// Altura máxima do stream. O monitor é 21:9, então a largura acompanha o
/// aspecto original em vez de virar 1280x720 (que esticaria/distorceria).
const TARGET_HEIGHT: u32 = 720;

/// Resolução de encode, mantendo o aspecto e alinhando a largura a múltiplo de
/// 16 (tamanho de macrobloco do H.264).
pub fn encode_size(w: u32, h: u32) -> (u32, u32) {
    if h <= TARGET_HEIGHT {
        return (w, h);
    }

    let th = TARGET_HEIGHT;
    let tw = ((w as f32 * th as f32 / h as f32) / 16.0).round() as u32 * 16;

    (tw.max(16), th)
}

/// Converte um frame RGBA para YUV, redimensionando apenas se a resolução de
/// encode for diferente da original.
pub fn frame_to_yuv(frame: Frame, target: (u32, u32)) -> YUVBuffer {
    let (tw, th) = target;

    if (tw, th) == (frame.width, frame.height) {
        return YUVBuffer::from_rgba8_source(RgbaSliceU8::new(
            &frame.raw,
            (tw as usize, th as usize),
        ));
    }

    // RgbaImage::from_raw move o buffer, sem copiar os ~11 MB do frame
    let img = RgbaImage::from_raw(frame.width, frame.height, frame.raw)
        .expect("frame RGBA com tamanho inconsistente");
    let resized = imageops::resize(&img, tw, th, FilterType::Triangle);

    YUVBuffer::from_rgba8_source(RgbaSliceU8::new(
        resized.as_raw(),
        (tw as usize, th as usize),
    ))
}
