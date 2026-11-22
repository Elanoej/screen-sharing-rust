use openh264::{
    OpenH264API,
    encoder::{
        BitRate, Complexity, Encoder, EncoderConfig as H264Config, FrameRate, FrameType,
        IntraFramePeriod, UsageType,
    },
    formats::YUVSlices,
};

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
    Ok(Box::new(OpenH264Encoder::new(config)?))
}

pub struct OpenH264Encoder {
    inner: Encoder,
}

impl OpenH264Encoder {
    pub fn new(config: &EncoderConfig) -> Result<Self, String> {
        let h264_config = H264Config::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .complexity(Complexity::Low)
            .max_frame_rate(FrameRate::from_hz(config.fps as f32))
            .bitrate(BitRate::from_bps(config.bitrate))
            .intra_frame_period(IntraFramePeriod::from_num_frames(60))
            .max_slice_len(4_000)
            .num_threads(0);

        let inner = Encoder::with_api_config(OpenH264API::from_source(), h264_config)
            .map_err(|e| e.to_string())?;
        Ok(Self { inner })
    }
}

impl VideoEncoder for OpenH264Encoder {
    fn name(&self) -> &str {
        "openh264"
    }

    fn encode(&mut self, frame: &YuvFrame) -> Result<Option<EncodedChunk>, String> {
        let yuv = YUVSlices::new(
            (frame.y, frame.u, frame.v),
            (frame.width as usize, frame.height as usize),
            (frame.y_stride, frame.u_stride, frame.v_stride),
        );

        let bitstream = self.inner.encode(&yuv).map_err(|e| e.to_string())?;
        let frame_type = bitstream.frame_type();

        if matches!(frame_type, FrameType::Skip | FrameType::Invalid) {
            return Ok(None);
        }

        let mut data = Vec::new();
        bitstream.write_vec(&mut data);

        Ok(Some(EncodedChunk {
            data,
            is_key: matches!(frame_type, FrameType::IDR | FrameType::I),
        }))
    }

    fn request_keyframe(&mut self) {
        self.inner.force_intra_frame();
    }
}
