use ffmpeg::{Dictionary, Rational, codec, encoder, format::Pixel, frame, picture};
use ffmpeg_next::{self as ffmpeg, Packet};

use crate::encoder::EncodedChunk;

use super::{EncoderConfig, VideoEncoder, YuvFrame};

const CANDIDATES: &[&str] = &["h264_nvenc", "h264_amf", "h264_videotoolbox"];
const PROBE_SIZE: (u32, u32) = (1280, 720);

struct Opened {
    enc: encoder::video::Encoder,
    frame: frame::Video,
    size: (u32, u32),
    pts: i64,
}

pub struct FfmpegH264 {
    name: &'static str,
    fps: u32,
    bitrate: u32,
    force_key: bool,
    opened: Opened,
}

fn open(name: &str, (w, h): (u32, u32), fps: u32, bitrate: u32) -> Result<Opened, String> {
    let codec = encoder::find_by_name(name)
        .ok_or_else(|| format!("{name}: não existe nesta build do FFmpeg"))?;

    let mut v = codec::context::Context::new()
        .encoder()
        .video()
        .map_err(|e| e.to_string())?;
    v.set_width(w);
    v.set_height(h);
    v.set_format(Pixel::YUV420P);
    v.set_time_base(Rational(1, fps as i32));
    v.set_frame_rate(Some(Rational(fps as i32, 1)));
    v.set_bit_rate(bitrate as usize);
    v.set_max_b_frames(0);
    v.set_gop(60);

    let mut opts = Dictionary::new();
    if name == "h264_nvenc" {
        opts.set("preset", "p1"); // o mais rápido
        opts.set("tune", "ll"); // baixa latência
        opts.set("rc", "cbr");
        opts.set("zerolatency", "1");
        opts.set("forced-idr", "1"); // keyframe forçado vira IDR de verdade
        opts.set("profile", "baseline"); // casa com o avc1.42... do frontend
    };

    // É AQUI que falha sem driver / sem GPU compatível
    let enc = v
        .open_as_with(codec, opts)
        .map_err(|e| format!("{name}: {e}"))?;

    Ok(Opened {
        enc,
        frame: frame::Video::new(Pixel::YUV420P, w, h),
        size: (w, h),
        pts: 0,
    })
}

impl FfmpegH264 {
    pub fn new(config: &EncoderConfig) -> Result<Self, String> {
        ffmpeg::init().map_err(|e| e.to_string())?;

        let size = if config.width > 0 && config.height > 0 {
            (config.width, config.height)
        } else {
            PROBE_SIZE
        };

        let mut last_err = String::from("nenhum encoder de GPU candidato");
        for &name in CANDIDATES {
            match open(name, size, config.fps, config.bitrate) {
                Ok(opened) => {
                    return Ok(Self {
                        name,
                        fps: config.fps,
                        bitrate: config.bitrate,
                        force_key: true,
                        opened,
                    });
                }
                Err(e) => {
                    eprintln!("{e}");
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }
}

fn copy_plane(
    dst: &mut frame::Video,
    plane: usize,
    src: &[u8],
    src_stride: usize,
    row_bytes: usize,
    rows: usize,
) {
    let dst_stride = dst.stride(plane);
    let data = dst.data_mut(plane);
    for y in 0..rows {
        data[y * dst_stride..y * dst_stride + row_bytes]
            .copy_from_slice(&src[y * src_stride..y * src_stride + row_bytes]);
    }
}

impl VideoEncoder for FfmpegH264 {
    fn name(&self) -> &str {
        self.name
    }

    fn encode(&mut self, f: &YuvFrame) -> Result<Option<super::EncodedChunk>, String> {
        let size = (f.width, f.height);
        if self.opened.size != size {
            // primeiro frame real (ou resolução mudou): reabre com o tamanho certo
            self.opened = open(self.name, size, self.fps, self.bitrate)?;
            self.force_key = true;
        }

        let o = &mut self.opened;

        let (w, h) = (f.width as usize, f.height as usize);
        let (cw, ch) = ((w + 1) / 2, (h + 1) / 2);
        copy_plane(&mut o.frame, 0, f.y, f.y_stride, w, h);
        copy_plane(&mut o.frame, 1, f.u, f.u_stride, cw, ch);
        copy_plane(&mut o.frame, 2, f.v, f.v_stride, cw, ch);

        o.frame.set_pts(Some(o.pts));
        o.pts += 1;
        o.frame.set_kind(if self.force_key {
            picture::Type::I
        } else {
            picture::Type::None
        });
        self.force_key = false;

        o.enc.send_frame(&o.frame).map_err(|e| e.to_string())?;

        let mut data = Vec::new();
        let mut is_key = false;
        let mut pkt = Packet::empty();
        while o.enc.receive_packet(&mut pkt).is_ok() {
            if let Some(bytes) = pkt.data() {
                data.extend_from_slice(bytes);
            }
            is_key |= pkt.is_key();
        }

        if data.is_empty() {
            Ok(None)
        } else {
            Ok(Some(EncodedChunk { data, is_key }))
        }
    }

    fn request_keyframe(&mut self) {
        self.force_key = true;
    }
}
