use std::time::{Duration, Instant};

use fast_image_resize::{
    FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer,
    images::{Image, ImageRef},
};
use xcap::Frame;

use crate::encoder::YuvFrame;
use yuv::{
    YuvChromaSubsampling, YuvConversionMode, YuvPlanarImageMut, YuvRange, YuvStandardMatrix,
    rgba_to_yuv420,
};

const TARGET_HEIGHT: u32 = 720;

pub fn encode_size(w: u32, h: u32) -> (u32, u32) {
    if h <= TARGET_HEIGHT {
        return (w, h);
    }

    let th = TARGET_HEIGHT;
    let tw = ((w as f32 * th as f32 / h as f32) / 16.0).round() as u32 * 16;

    (tw.max(16), th)
}

pub struct Timings {
    pub resize: Duration,
    pub yuv: Duration,
}

pub struct Converter {
    resizer: Resizer,
    opts: ResizeOptions,
    scaled: Image<'static>,
    planes: YuvPlanarImageMut<'static, u8>,
    size: (u32, u32),
}

impl Converter {
    pub fn new(target: (u32, u32)) -> Self {
        let (tw, th) = target;
        Self {
            resizer: Resizer::new(),
            opts: ResizeOptions::new()
                .resize_alg(ResizeAlg::Convolution(FilterType::Bilinear))
                .use_alpha(false),
            scaled: Image::new(tw, th, PixelType::U8x4),
            planes: YuvPlanarImageMut::alloc(tw, th, YuvChromaSubsampling::Yuv420),
            size: target,
        }
    }

    pub fn convert(&mut self, frame: &Frame) -> (YuvFrame<'_>, Timings) {
        let (tw, th) = self.size;
        let t = Instant::now();

        let rgba: &[u8] = if (tw, th) == (frame.width, frame.height) {
            &frame.raw
        } else {
            let src = ImageRef::new(frame.width, frame.height, &frame.raw, PixelType::U8x4)
                .expect("frame RGBA com tamanho inconsistente");

            self.resizer
                .resize(&src, &mut self.scaled, &self.opts)
                .expect("resize falhou");
            self.scaled.buffer()
        };
        let resize = t.elapsed();

        let t = Instant::now();
        rgba_to_yuv420(
            &mut self.planes,
            rgba,
            tw * 4,
            YuvRange::Limited,
            YuvStandardMatrix::Bt601,
            YuvConversionMode::Balanced,
        )
        .expect("conversão YUV falhou");
        let yuv = t.elapsed();

        let yuv_frame = YuvFrame {
            y: self.planes.y_plane.borrow(),
            u: self.planes.u_plane.borrow(),
            v: self.planes.v_plane.borrow(),
            y_stride: self.planes.y_stride as usize,
            u_stride: self.planes.u_stride as usize,
            v_stride: self.planes.v_stride as usize,
            width: tw,
            height: th,
        };

        (yuv_frame, Timings { resize, yuv })
    }
}
