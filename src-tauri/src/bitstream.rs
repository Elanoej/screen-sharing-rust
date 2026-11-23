//! Serialização de bitstream H.264.
//!
//! NAL é a unidade do H.264: cada um tem 1 byte de header (`0x67` = SPS,
//! `0x68` = PPS, `0x65` = slice IDR) e o payload. Annex-B e AVCC não são
//! protocolos — são duas formas de escrever uma sequência de NALs em bytes:
//!
//! - **Annex-B** separa os NALs com start codes (`00 00 01` / `00 00 00 01`)
//!   e deixa SPS/PPS dentro do fluxo, como qualquer outro NAL. É o que sai do
//!   encoder (OpenH264 e FFmpeg).
//! - **AVCC** prefixa cada NAL com seu tamanho e tira SPS/PPS do fluxo, para
//!   um registro à parte, o `avcC` (AVCDecoderConfigurationRecord).
//!
//! O `AccessUnit` é o meio: a lista de NALs de uma imagem, sem saber de
//! formato. `parse_annexb` produz um; `to_avcc` e `avcc_config` consomem.
//!
//! No Linux o WebKitGTK usa GStreamer e fixa as caps de entrada em
//! `video/x-h264, stream-format=avc, alignment=au` — NALs prefixados por
//! tamanho, um access unit por buffer, com o `avcC` vindo de
//! `VideoDecoderConfig.description`. Annex-B puro não negocia e o decoder
//! nunca produz frames.

/// Tipo de NAL nos 5 bits baixos do header.
const NAL_TYPE_SPS: u8 = 7;
const NAL_TYPE_PPS: u8 = 8;

/// NALs de uma única imagem codificada, já sem os start codes.
pub struct AccessUnit<'a> {
    nals: Vec<&'a [u8]>,
}

impl<'a> AccessUnit<'a> {
    fn sps(&self) -> Option<&'a [u8]> {
        self.find(NAL_TYPE_SPS)
    }

    fn pps(&self) -> Option<&'a [u8]> {
        self.find(NAL_TYPE_PPS)
    }

    fn find(&self, nal_type: u8) -> Option<&'a [u8]> {
        self.nals
            .iter()
            .copied()
            .find(|nal| nal[0] & 0x1f == nal_type)
    }
}

/// Tamanho do start code em `i`, se houver um ali.
fn start_code_len(data: &[u8], i: usize) -> Option<usize> {
    if data.len() >= i + 4 && data[i..i + 4] == [0, 0, 0, 1] {
        Some(4)
    } else if data.len() >= i + 3 && data[i..i + 3] == [0, 0, 1] {
        Some(3)
    } else {
        None
    }
}

/// Separa um access unit Annex-B nos seus NALs.
pub fn parse_annexb(data: &[u8]) -> AccessUnit<'_> {
    let mut nals = Vec::new();
    let mut current: Option<usize> = None;
    let mut i = 0;

    while i < data.len() {
        if let Some(sc) = start_code_len(data, i) {
            if let Some(start) = current {
                nals.push(&data[start..i]);
            }
            i += sc;
            current = Some(i);
        } else {
            i += 1;
        }
    }

    if let Some(start) = current {
        nals.push(&data[start..]);
    }

    nals.retain(|nal| !nal.is_empty());
    AccessUnit { nals }
}

/// Serializa o access unit em AVCC: cada NAL vira `[u32 BE len][NAL]`.
///
/// Devolve só a amostra. O AVCC só é decodificável junto do `avcC`, que sai de
/// [`avcc_config`].
pub fn to_avcc(au: &AccessUnit<'_>) -> Vec<u8> {
    let capacity = au.nals.iter().map(|nal| nal.len() + 4).sum();
    let mut out = Vec::with_capacity(capacity);

    for nal in &au.nals {
        out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        out.extend_from_slice(nal);
    }

    out
}

/// Monta o AVCDecoderConfigurationRecord (`avcC`) a partir dos NALs SPS/PPS.
///
/// `None` quando o access unit não traz os dois parâmetros — o que é erro de
/// encoder, não condição normal: sem `avcC` o decoder não inicia.
pub fn avcc_config(au: &AccessUnit<'_>) -> Option<Vec<u8>> {
    let sps = au.sps()?;
    let pps = au.pps()?;

    if sps.len() < 4 || sps.len() > u16::MAX as usize || pps.len() > u16::MAX as usize {
        return None;
    }

    let mut out = Vec::with_capacity(11 + sps.len() + pps.len());
    out.push(1); // configurationVersion
    out.extend_from_slice(&sps[1..4]); // profile, compatibilidade e level
    out.push(0xFF); // reserved + lengthSizeMinusOne = 3 (4 bytes por NAL)
    out.push(0xE1); // reserved + numOfSequenceParameterSets = 1
    out.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    out.extend_from_slice(sps);
    out.push(1); // numOfPictureParameterSets
    out.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    out.extend_from_slice(pps);

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPS: &[u8] = &[
        0x67, 0x42, 0xc0, 0x1f, 0x8c, 0x68, 0x05, 0x00, 0x5b, 0xa0, 0x1e, 0x11, 0x08, 0xd4,
    ];
    const PPS: &[u8] = &[0x68, 0xce, 0x3c, 0x80];
    const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x00, 0x21];

    fn annexb() -> Vec<u8> {
        let mut out = Vec::new();
        for nal in [SPS, PPS, IDR] {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(nal);
        }
        out
    }

    #[test]
    fn separa_os_nals_do_access_unit() {
        let data = annexb();
        let au = parse_annexb(&data);
        assert_eq!(au.nals.as_slice(), &[SPS, PPS, IDR]);
    }

    #[test]
    fn converte_annexb_para_avcc() {
        let data = annexb();
        let au = parse_annexb(&data);
        let mut expected = Vec::new();
        for nal in [SPS, PPS, IDR] {
            expected.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            expected.extend_from_slice(nal);
        }
        assert_eq!(to_avcc(&au), expected);
    }

    #[test]
    fn aceita_start_code_de_tres_e_quatro_bytes() {
        let mut data = vec![0, 0, 0, 1];
        data.extend_from_slice(SPS);
        data.extend_from_slice(&[0, 0, 1]);
        data.extend_from_slice(PPS);

        let avcc = to_avcc(&parse_annexb(&data));
        assert_eq!(&avcc[..4], &(SPS.len() as u32).to_be_bytes());
        assert_eq!(&avcc[4..4 + SPS.len()], SPS);
        assert_eq!(
            &avcc[4 + SPS.len()..8 + SPS.len()],
            &(PPS.len() as u32).to_be_bytes()
        );
        assert_eq!(&avcc[8 + SPS.len()..], PPS);
    }

    #[test]
    fn monta_avcc_record() {
        let data = annexb();
        let au = parse_annexb(&data);
        assert_eq!(
            avcc_config(&au).unwrap(),
            vec![
                0x01, 0x42, 0xc0, 0x1f, 0xff, 0xe1, 0x00, 0x0e, 0x67, 0x42, 0xc0, 0x1f, 0x8c, 0x68,
                0x05, 0x00, 0x5b, 0xa0, 0x1e, 0x11, 0x08, 0xd4, 0x01, 0x00, 0x04, 0x68, 0xce, 0x3c,
                0x80,
            ]
        );
    }

    #[test]
    fn sem_sps_nao_gera_config() {
        let mut data = vec![0, 0, 0, 1];
        data.extend_from_slice(IDR);
        assert_eq!(avcc_config(&parse_annexb(&data)), None);
    }

    #[test]
    fn stream_sem_start_code_nao_quebra() {
        assert!(to_avcc(&parse_annexb(&[1, 2, 3])).is_empty());
    }
}
