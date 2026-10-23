//! Narrow, privacy-safe format queries. Native strings never leave this module.

use crate::{LocalPlayer, PlaybackError, check_native, ffi};
use resolibre_core::track_info::{AudioCodec, StreamInfo};
use resolibre_core::{AudioFormat, AudioFormats, SampleFormat};
use std::ffi::{CStr, c_char};

impl LocalPlayer {
    /// Reads selected codec and decoded source dimensions without probing or decoding again.
    /// Native failures are returned for the caller to handle as optional information.
    pub fn stream_info(&mut self) -> Result<Option<StreamInfo>, PlaybackError> {
        if !self.loaded {
            return Ok(None);
        }

        let codec = self
            .read_identifier(c"current-tracks/audio/codec", audio_codec)?
            .unwrap_or_default();
        let sample_rate_hz = self.read_positive_integer(c"audio-params/samplerate")?;
        let channels = self.read_positive_integer(c"audio-params/channel-count")?;

        Ok(Some(StreamInfo {
            codec,
            sample_rate_hz,
            channels,
            bits_per_sample: self
                .encoded_info
                .filter(|info| {
                    info.codec == codec
                        && info.sample_rate_hz == sample_rate_hz
                        && info.channels == channels
                })
                .and_then(|info| info.bits_per_sample),
        }))
    }

    /// Reads decoder and audio API formats separately without changing playback.
    /// Unavailable properties are normal; other native failures retain safe codes.
    /// These synchronous readings neither measure DAC output nor prove audibility.
    pub fn audio_formats(&mut self) -> Result<AudioFormats, PlaybackError> {
        if !self.loaded {
            return Ok(AudioFormats::default());
        }

        let source = self.read_audio_format(
            c"audio-params/samplerate",
            c"audio-params/channel-count",
            c"audio-params/format",
        )?;
        let output = self.read_audio_format(
            c"audio-out-params/samplerate",
            c"audio-out-params/channel-count",
            c"audio-out-params/format",
        )?;

        Ok(AudioFormats { source, output })
    }

    /// Returns no format when any required field is temporarily unavailable.
    fn read_audio_format(
        &mut self,
        rate_property: &CStr,
        channels_property: &CStr,
        sample_property: &CStr,
    ) -> Result<Option<AudioFormat>, PlaybackError> {
        let Some(sample_rate_hz) = self.read_positive_integer(rate_property)? else {
            return Ok(None);
        };
        let Some(channels) = self.read_positive_integer(channels_property)? else {
            return Ok(None);
        };
        let Some(sample_format) = self.read_sample_format(sample_property)? else {
            return Ok(None);
        };

        Ok(Some(AudioFormat {
            sample_rate_hz,
            channels,
            sample_format,
        }))
    }

    /// Reads one native integer into aligned storage and validates its public range.
    fn read_positive_integer(&mut self, property: &CStr) -> Result<Option<u32>, PlaybackError> {
        let mut value = 0_i64;
        // SAFETY: the live handle and NUL-terminated name outlive the call;
        // MPV_FORMAT_INT64 writes exactly one i64 into this aligned local value.
        let result = unsafe {
            ffi::mpv_get_property(
                self.handle.as_ptr(),
                property.as_ptr(),
                ffi::FORMAT_INT64,
                (&mut value as *mut i64).cast(),
            )
        };
        if result == ffi::ERROR_PROPERTY_UNAVAILABLE {
            return Ok(None);
        }
        check_native("audio format", result)?;

        Ok(Some(positive_integer(value)?))
    }

    /// Maps a native allocation to a closed enum and frees it before returning.
    fn read_sample_format(
        &mut self,
        property: &CStr,
    ) -> Result<Option<SampleFormat>, PlaybackError> {
        self.read_identifier(property, sample_format)
    }

    /// Converts an engine identifier to a closed value and releases its allocation.
    fn read_identifier<T>(
        &mut self,
        property: &CStr,
        parse: fn(&[u8]) -> T,
    ) -> Result<Option<T>, PlaybackError> {
        let mut value: *mut c_char = std::ptr::null_mut();
        // SAFETY: MPV_FORMAT_STRING writes an allocated char pointer. This live
        // pointer slot, property name and handle remain valid throughout the call.
        let result = unsafe {
            ffi::mpv_get_property(
                self.handle.as_ptr(),
                property.as_ptr(),
                ffi::FORMAT_STRING,
                (&mut value as *mut *mut c_char).cast(),
            )
        };
        if result == ffi::ERROR_PROPERTY_UNAVAILABLE {
            return Ok(None);
        }
        check_native("audio format", result)?;
        if value.is_null() {
            return Err(PlaybackError::InvalidFormat);
        }

        // SAFETY: successful MPV_FORMAT_STRING returns a NUL-terminated allocation.
        // Only engine codec/sample-format identifiers are read, never media tags or paths.
        let bytes = unsafe { CStr::from_ptr(value) }.to_bytes();
        let format = parse(bytes);

        // SAFETY: this allocation belongs to mpv and is freed exactly once. The
        // closed enum holds no borrow; no fallible operation intervenes above.
        unsafe { ffi::mpv_free(value.cast()) };
        Ok(Some(format))
    }
}

/// Maps exact decoder identifiers, never extensions, descriptions or inferred provenance.
fn audio_codec(value: &[u8]) -> AudioCodec {
    match value {
        b"flac" => AudioCodec::Flac,
        b"alac" => AudioCodec::Alac,
        b"pcm_u8" | b"pcm_s8" | b"pcm_s16le" | b"pcm_s16be" | b"pcm_s24le" | b"pcm_s24be"
        | b"pcm_s32le" | b"pcm_s32be" | b"pcm_f32le" | b"pcm_f32be" | b"pcm_f64le"
        | b"pcm_f64be" => AudioCodec::Pcm,
        b"mp3" => AudioCodec::Mp3,
        b"mp2" => AudioCodec::Mp2,
        b"aac" => AudioCodec::Aac,
        b"vorbis" => AudioCodec::Vorbis,
        b"opus" => AudioCodec::Opus,
        b"ac3" => AudioCodec::Ac3,
        b"eac3" => AudioCodec::Eac3,
        b"dts" => AudioCodec::Dts,
        b"wavpack" => AudioCodec::WavPack,
        b"ape" => AudioCodec::Ape,
        b"truehd" => AudioCodec::TrueHd,
        _ => AudioCodec::Unknown,
    }
}

/// Rejects negative, zero and overflowing format numbers without a lossy cast.
fn positive_integer(value: i64) -> Result<u32, PlaybackError> {
    let value = u32::try_from(value).map_err(|_| PlaybackError::InvalidFormat)?;
    if value == 0 {
        return Err(PlaybackError::InvalidFormat);
    }

    Ok(value)
}

/// Allows known PCM representations; unknown native text is never displayed.
fn sample_format(value: &[u8]) -> SampleFormat {
    match value {
        b"u8" => SampleFormat::Unsigned8,
        b"s16" => SampleFormat::Signed16,
        b"s32" => SampleFormat::Signed32,
        b"s64" => SampleFormat::Signed64,
        b"float" => SampleFormat::Float32,
        b"double" => SampleFormat::Float64,
        b"u8p" => SampleFormat::Unsigned8Planar,
        b"s16p" => SampleFormat::Signed16Planar,
        b"s32p" => SampleFormat::Signed32Planar,
        b"s64p" => SampleFormat::Signed64Planar,
        b"floatp" => SampleFormat::Float32Planar,
        b"doublep" => SampleFormat::Float64Planar,
        _ => SampleFormat::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::{positive_integer, sample_format};
    use resolibre_core::SampleFormat;

    /// Unknown identifiers and disguised filenames cannot produce a codec/provenance claim.
    #[test]
    fn codec_identifiers_are_allowlisted() {
        use super::{AudioCodec, audio_codec};
        for value in [
            b"song.flac".as_slice(),
            b"FLAC",
            b"pcm_private",
            b"\xff",
            b"\x1b[31m",
        ] {
            assert_eq!(audio_codec(value), AudioCodec::Unknown);
        }
        assert_eq!(audio_codec(b"flac"), AudioCodec::Flac);
        assert_eq!(audio_codec(b"mp3"), AudioCodec::Mp3);
        assert_eq!(audio_codec(b"pcm_s24le"), AudioCodec::Pcm);
        assert_eq!(audio_codec(b"wavpack").label(), "WavPack");
    }

    /// Untrusted integers cannot wrap or masquerade as a valid zero-rate format.
    #[test]
    fn rejects_invalid_format_numbers() {
        for value in [i64::MIN, -1, 0, i64::from(u32::MAX) + 1, i64::MAX] {
            assert!(positive_integer(value).is_err());
        }
        assert_eq!(positive_integer(48000).expect("valid rate"), 48000);
    }

    /// Arbitrary text, invalid UTF-8 and terminal escapes never enter diagnostics.
    #[test]
    fn hides_unknown_native_format_text() {
        for value in [b"private-token".as_slice(), b"\xff", b"\x1b[31m", b""] {
            assert_eq!(sample_format(value), SampleFormat::Unknown);
        }
        assert_eq!(sample_format(b"floatp"), SampleFormat::Float32Planar);
        assert_eq!(sample_format(b"s16"), SampleFormat::Signed16);
    }
}
