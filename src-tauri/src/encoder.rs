mod ffmpeg;
mod h264;

/// Frame YUV420 planar, já redimensionado. Formato meutro: qualquer backend consome.
pub struct YuvFrame<'a> {
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub y_stride: usize,
    pub u_stride: usize,
    pub v_stride: usize,
    pub width: u32,
    pub height: u32,
}

/// Um frame codificado, pronto para enviar ao frontend.
pub struct EncodedChunk {
    pub data: Vec<u8>, // H.264 Annex-B
    pub is_key: bool,  // o frontend usa isso no primeiro byte
}

pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
}

pub trait VideoEncoder: Send {
    fn name(&self) -> &str;
    fn encode(&mut self, frame: &YuvFrame) -> Result<Option<EncodedChunk>, String>;
    fn request_keyframe(&mut self);
}

pub fn build_encoder(config: &EncoderConfig) -> Result<Box<dyn VideoEncoder>, String> {
    match ffmpeg::FfmpegH264::new(config) {
        Ok(enc) => return Ok(Box::new(enc)),
        Err(e) => eprintln!("encode por GPU indisponível ({e}); usando CPU"),
    }

    Ok(Box::new(h264::OpenH264Encoder::new(config)?))
}
