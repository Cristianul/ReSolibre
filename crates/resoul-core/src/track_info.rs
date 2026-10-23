//! Observed stream facts, separate from decoder precision and recording provenance.

/// Known codec identities; arbitrary native text never enters presentation or logs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioCodec {
    Flac,
    Alac,
    Pcm,
    Mp3,
    Mp2,
    Aac,
    Vorbis,
    Opus,
    Ac3,
    Eac3,
    Dts,
    WavPack,
    Ape,
    TrueHd,
    #[default]
    Unknown,
}

/// Encoding mode is separate from recording provenance; mixed-mode streams need additional evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressionKind {
    Lossless,
    Lossy,
    Unknown,
}

impl AudioCodec {
    /// Classifies known encoding identities only; this never certifies the origin of decoded audio.
    pub fn compression(self) -> CompressionKind {
        match self {
            Self::Flac | Self::Alac | Self::Pcm | Self::Ape | Self::TrueHd => {
                CompressionKind::Lossless
            }
            Self::Mp3
            | Self::Mp2
            | Self::Aac
            | Self::Vorbis
            | Self::Opus
            | Self::Ac3
            | Self::Eac3 => CompressionKind::Lossy,
            Self::Dts | Self::WavPack | Self::Unknown => CompressionKind::Unknown,
        }
    }

    /// Returns a fixed codec label, never a claim about the original recording.
    pub fn label(self) -> &'static str {
        match self {
            Self::Flac => "FLAC",
            Self::Alac => "ALAC",
            Self::Pcm => "PCM",
            Self::Mp3 => "MP3",
            Self::Mp2 => "MP2",
            Self::Aac => "AAC",
            Self::Vorbis => "Vorbis",
            Self::Opus => "Opus",
            Self::Ac3 => "AC-3",
            Self::Eac3 => "E-AC-3",
            Self::Dts => "DTS",
            Self::WavPack => "WavPack",
            Self::Ape => "APE",
            Self::TrueHd => "TrueHD",
            Self::Unknown => "Unknown codec",
        }
    }
}

/// Current selected audio stream, observed before user EQ/output conversion.
/// Missing fields are normal during negotiation. Working PCM precision is not
/// encoded bit depth; none of these facts establishes original provenance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamInfo {
    pub codec: AudioCodec,
    pub sample_rate_hz: Option<u32>,
    pub channels: Option<u32>,
    /// Encoded integer precision, never the decoder's working sample format.
    pub bits_per_sample: Option<u8>,
}

/// Reads only a native FLAC STREAMINFO header; malformed, wrapped and unsupported formats stay unknown.
pub fn flac_stream_info(bytes: &[u8]) -> Option<StreamInfo> {
    if bytes.len() < 42
        || &bytes[..4] != b"fLaC"
        || bytes[4] & 0x7f != 0
        || bytes[5..8] != [0, 0, 34]
    {
        return None;
    }
    let minimum = u16::from_be_bytes(bytes[8..10].try_into().ok()?);
    let maximum = u16::from_be_bytes(bytes[10..12].try_into().ok()?);
    let packed = u64::from_be_bytes(bytes[18..26].try_into().ok()?);
    let rate = (packed >> 44) as u32;
    let channels = ((packed >> 41) & 7) as u32 + 1;
    let bits = ((packed >> 36) & 31) as u8 + 1;
    if minimum < 16 || maximum < minimum || rate == 0 || bits < 4 {
        return None;
    }
    Some(StreamInfo {
        codec: AudioCodec::Flac,
        sample_rate_hz: Some(rate),
        channels: Some(channels),
        bits_per_sample: Some(bits),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encoded precision is read from STREAMINFO, with truncation, wrong block types and invalid dimensions rejected.
    #[test]
    fn flac_precision_is_encoded_metadata() {
        for bits in [16_u8, 24, 32] {
            let mut bytes = [0_u8; 42];
            bytes[..8].copy_from_slice(b"fLaC\x80\0\0\x22");
            bytes[8..12].copy_from_slice(&[0x10, 0, 0x10, 0]);
            let packed = (44100_u64 << 44) | (1 << 41) | (u64::from(bits - 1) << 36);
            bytes[18..26].copy_from_slice(&packed.to_be_bytes());
            let info = flac_stream_info(&bytes).unwrap();
            assert_eq!(info.bits_per_sample, Some(bits));
            assert_eq!(info.channels, Some(2));
            assert_eq!(info.sample_rate_hz, Some(44100));
            for length in 0..42 {
                assert_eq!(flac_stream_info(&bytes[..length]), None);
            }
            bytes[4] = 4;
            assert_eq!(flac_stream_info(&bytes), None);
            bytes[4] = 0;
            bytes[18..26].fill(0);
            assert_eq!(flac_stream_info(&bytes), None);
        }
        assert_eq!(flac_stream_info(&[0; 42]), None);
    }

    /// Clients share conservative codec classification; hybrid/mixed modes cannot be certified by codec name.
    #[test]
    fn compression_mode_is_shared_and_conservative() {
        for codec in [
            AudioCodec::Flac,
            AudioCodec::Alac,
            AudioCodec::Pcm,
            AudioCodec::Ape,
            AudioCodec::TrueHd,
        ] {
            assert_eq!(codec.compression(), CompressionKind::Lossless);
        }
        for codec in [
            AudioCodec::Mp3,
            AudioCodec::Mp2,
            AudioCodec::Aac,
            AudioCodec::Vorbis,
            AudioCodec::Opus,
            AudioCodec::Ac3,
            AudioCodec::Eac3,
        ] {
            assert_eq!(codec.compression(), CompressionKind::Lossy);
        }
        for codec in [AudioCodec::Dts, AudioCodec::WavPack, AudioCodec::Unknown] {
            assert_eq!(codec.compression(), CompressionKind::Unknown);
        }
    }
}
