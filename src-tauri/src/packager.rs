//! Empacotamento do que sai do encoder para o que o consumidor recebe.
//!
//! Duas responsabilidades que não são conversão de formato:
//!
//! - **política**: o `avcC` só é emitido quando muda. O decoder guarda a
//!   configuração recebida; reenviar a mesma a cada keyframe é desperdício, e
//!   omitir uma nova deixa o decoder com a antiga.
//! - **contrato de fio**: o primeiro byte de cada mensagem diz o que vem
//!   depois. É o mesmo contrato que o `ScreenShareView.tsx` lê.

use crate::bitstream::{self, AccessUnit};

/// Primeiro byte de cada mensagem.
const KIND_DELTA: u8 = 0;
const KIND_KEY: u8 = 1;
const KIND_CONFIG: u8 = 2;

/// Uma mensagem pronta para o consumidor.
pub enum Packet {
    /// `avcC` — configuração do decoder.
    Config(Vec<u8>),
    /// Access unit em AVCC de um keyframe.
    Key(Vec<u8>),
    /// Access unit em AVCC de um frame delta.
    Delta(Vec<u8>),
}

impl Packet {
    /// Serializa como `[kind][payload]`.
    pub fn into_payload(self) -> Vec<u8> {
        let (kind, data) = match self {
            Packet::Config(data) => (KIND_CONFIG, data),
            Packet::Key(data) => (KIND_KEY, data),
            Packet::Delta(data) => (KIND_DELTA, data),
        };

        let mut out = Vec::with_capacity(data.len() + 1);
        out.push(kind);
        out.extend_from_slice(&data);
        out
    }
}

/// Transforma access units em pacotes, guardando a última config enviada.
#[derive(Default)]
pub struct Packager {
    last_config: Option<Vec<u8>>,
}

impl Packager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pacotes de um access unit. Vazio quando o AU não tem NAL nenhum.
    pub fn push(&mut self, au: &AccessUnit<'_>, is_key: bool) -> Vec<Packet> {
        let avcc = bitstream::to_avcc(au);
        if avcc.is_empty() {
            return Vec::new();
        }

        let mut packets = Vec::with_capacity(2);

        if is_key {
            if let Some(config) = bitstream::avcc_config(au) {
                if self.last_config.as_deref() != Some(config.as_slice()) {
                    self.last_config = Some(config.clone());
                    packets.push(Packet::Config(config));
                }
            }
        }

        packets.push(if is_key {
            Packet::Key(avcc)
        } else {
            Packet::Delta(avcc)
        });

        packets
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPS: &[u8] = &[
        0x67, 0x42, 0xc0, 0x1f, 0x8c, 0x68, 0x05, 0x00, 0x5b, 0xa0, 0x1e, 0x11, 0x08, 0xd4,
    ];
    const PPS: &[u8] = &[0x68, 0xce, 0x3c, 0x80];
    const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x00, 0x21];
    const P: &[u8] = &[0x41, 0x9a, 0x00];
    /// Mesmo SPS com outro level, para simular mudança de configuração.
    const SPS_NOVO: &[u8] = &[
        0x67, 0x42, 0xc0, 0x20, 0x8c, 0x68, 0x05, 0x00, 0x5b, 0xa0, 0x1e, 0x11, 0x08, 0xd4,
    ];

    fn annexb(nals: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in nals {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(nal);
        }
        out
    }

    fn kinds(packets: &[Packet]) -> Vec<u8> {
        packets
            .iter()
            .map(|packet| match packet {
                Packet::Config(_) => KIND_CONFIG,
                Packet::Key(_) => KIND_KEY,
                Packet::Delta(_) => KIND_DELTA,
            })
            .collect()
    }

    #[test]
    fn keyframe_emite_config_antes_do_key() {
        let data = annexb(&[SPS, PPS, IDR]);
        let au = bitstream::parse_annexb(&data);

        let packets = Packager::new().push(&au, true);

        assert_eq!(kinds(&packets), [KIND_CONFIG, KIND_KEY]);
    }

    #[test]
    fn delta_nao_emite_config() {
        let data = annexb(&[P]);
        let au = bitstream::parse_annexb(&data);

        let packets = Packager::new().push(&au, false);

        assert_eq!(kinds(&packets), [KIND_DELTA]);
    }

    #[test]
    fn config_nao_repete_enquanto_nao_muda() {
        let data = annexb(&[SPS, PPS, IDR]);
        let au = bitstream::parse_annexb(&data);
        let mut packager = Packager::new();

        assert_eq!(kinds(&packager.push(&au, true)), [KIND_CONFIG, KIND_KEY]);
        assert_eq!(kinds(&packager.push(&au, true)), [KIND_KEY]);
    }

    #[test]
    fn config_muda_quando_o_sps_muda() {
        let primeiro = annexb(&[SPS, PPS, IDR]);
        let segundo = annexb(&[SPS_NOVO, PPS, IDR]);
        let au1 = bitstream::parse_annexb(&primeiro);
        let au2 = bitstream::parse_annexb(&segundo);
        let mut packager = Packager::new();

        assert_eq!(kinds(&packager.push(&au1, true)), [KIND_CONFIG, KIND_KEY]);
        assert_eq!(kinds(&packager.push(&au2, true)), [KIND_CONFIG, KIND_KEY]);
    }

    #[test]
    fn au_vazio_nao_emite_nada() {
        let au = bitstream::parse_annexb(&[1, 2, 3]);
        assert!(Packager::new().push(&au, false).is_empty());
    }

    #[test]
    fn payload_comeca_com_o_kind() {
        let data = annexb(&[P]);
        let au = bitstream::parse_annexb(&data);
        let packet = Packager::new().push(&au, false).pop().unwrap();

        let payload = packet.into_payload();

        assert_eq!(payload[0], KIND_DELTA);
        assert_eq!(&payload[1..], bitstream::to_avcc(&au));
    }
}
