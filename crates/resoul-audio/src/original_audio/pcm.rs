//! Streaming floating-point WAV reader for the owned analysis decoder, with fixed memory bounds.
use resolibre_core::playback_edges::{EdgeAnalysis, SILENCE_AMPLITUDE};
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Reads bounded frames across all channels; opposite phase channels must never cancel in detection.
pub(crate) fn decode(
    reader: &mut impl Read,
    cancel: &AtomicBool,
    started: Instant,
    deadline: Duration,
) -> Result<EdgeAnalysis, ()> {
    let mut first = None;
    let mut last = None;
    let mut frame_count = 0_u64;
    let (rate, frames) = stream(reader, cancel, started, deadline, |_, channels, samples| {
        for frame in samples.chunks_exact(channels) {
            if frame.iter().any(|sample| sample.abs() >= SILENCE_AMPLITUDE) {
                first.get_or_insert(frame_count);
                last = Some(frame_count + 1);
            }
            frame_count += 1;
        }
        Ok(())
    })?;
    EdgeAnalysis::new(
        frames as f64 / f64::from(rate),
        first.map(|v| v as f64 / f64::from(rate)),
        last.map(|v| v as f64 / f64::from(rate)),
    )
    .ok_or(())
}

/// Streams validated complete interleaved frames in fixed blocks, initializing the consumer with an empty block.
pub(crate) fn stream(
    reader: &mut impl Read,
    cancel: &AtomicBool,
    started: Instant,
    deadline: Duration,
    consume: impl FnMut(u32, usize, &[f32]) -> Result<(), ()>,
) -> Result<(u32, u64), ()> {
    stream_prefix(reader, cancel, started, deadline, None, consume)
}

/// Stops cleanly at an optional source-time limit; full consumers still require decoder EOF.
pub(crate) fn stream_prefix(
    reader: &mut impl Read,
    cancel: &AtomicBool,
    started: Instant,
    deadline: Duration,
    max_seconds: Option<u32>,
    mut consume: impl FnMut(u32, usize, &[f32]) -> Result<(), ()>,
) -> Result<(u32, u64), ()> {
    let mut input = Input {
        reader,
        cancel,
        started,
        deadline,
    };
    let mut header = [0_u8; 12];
    input.exact(&mut header)?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
        return Err(());
    }
    let mut format = None;
    let mut header_bytes = 12;
    loop {
        let mut chunk = [0_u8; 8];
        input.exact(&mut chunk)?;
        let size = u32::from_le_bytes(chunk[4..].try_into().map_err(|_| ())?) as usize;
        if &chunk[..4] == b"data" {
            break;
        }
        header_bytes += 8 + size + size % 2;
        if header_bytes > 4096 {
            return Err(());
        }
        let mut bytes = vec![0; size + size % 2];
        input.exact(&mut bytes)?;
        if &chunk[..4] == b"fmt " {
            format = Some(parse_format(&bytes)?);
        }
    }
    let (channels, rate) = format.ok_or(())?;
    let channels = usize::from(channels);
    consume(rate, channels, &[])?;
    let frame_bytes = channels * 4;
    let mut buffer = [0_u8; 65536 + 128];
    let mut decoded = [0_f32; 16384];
    let mut pending = 0;
    let mut frames = 0_u64;
    loop {
        let count = input.read(&mut buffer[pending..65536])?;
        if count == 0 {
            break;
        }
        let length = pending + count;
        let available_frames = length / frame_bytes;
        let remaining = max_seconds.map(|seconds| u64::from(seconds) * u64::from(rate) - frames);
        let complete_frames =
            remaining.map_or(available_frames, |left| available_frames.min(left as usize));
        let complete = complete_frames * frame_bytes;
        let samples = complete / 4;
        for (destination, bytes) in decoded[..samples]
            .iter_mut()
            .zip(buffer[..complete].as_chunks::<4>().0)
        {
            let value = f32::from_le_bytes(*bytes);
            if !value.is_finite() {
                return Err(());
            }
            *destination = value;
        }
        consume(rate, channels, &decoded[..samples])?;
        frames = frames.checked_add((samples / channels) as u64).ok_or(())?;
        if max_seconds.is_some_and(|seconds| frames >= u64::from(seconds) * u64::from(rate)) {
            return Ok((rate, frames));
        }
        pending = length - complete;
        buffer.copy_within(complete..length, 0);
    }
    if pending != 0 || frames == 0 {
        return Err(());
    }
    Ok((rate, frames))
}

/// Accepts only bounded IEEE float output, including the WAV extensible float subtype.
fn parse_format(bytes: &[u8]) -> Result<(u16, u32), ()> {
    if bytes.len() < 16 {
        return Err(());
    }
    let code = u16::from_le_bytes([bytes[0], bytes[1]]);
    let channels = u16::from_le_bytes([bytes[2], bytes[3]]);
    let rate = u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| ())?);
    let bits = u16::from_le_bytes([bytes[14], bytes[15]]);
    let alignment = u16::from_le_bytes([bytes[12], bytes[13]]);
    let float = code == 3
        || (code == 65534
            && bytes.len() >= 40
            && bytes[24..40] == [3, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]);
    if !float
        || !(1..=32).contains(&channels)
        || !(8000..=384000).contains(&rate)
        || bits != 32
        || alignment != channels * 4
    {
        return Err(());
    }
    Ok((channels, rate))
}

struct Input<'a, R> {
    reader: &'a mut R,
    cancel: &'a AtomicBool,
    started: Instant,
    deadline: Duration,
}
impl<R: Read> Input<'_, R> {
    /// Polls cancellation during timed socket reads; neither empty input nor stalls can run forever.
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize, ()> {
        loop {
            if self.cancel.load(Ordering::Relaxed) || self.started.elapsed() >= self.deadline {
                return Err(());
            }
            match self.reader.read(bytes) {
                Ok(count) => return Ok(count),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(_) => return Err(()),
            }
        }
    }
    /// Rejects truncated metadata without allocating from untrusted lengths.
    fn exact(&mut self, mut bytes: &mut [u8]) -> Result<(), ()> {
        while !bytes.is_empty() {
            let count = self.read(bytes)?;
            if count == 0 {
                return Err(());
            }
            bytes = &mut bytes[count..];
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encodes the same bounded float PCM stream produced by the owned decoder.
    fn wav(samples: &[f32], channels: u16) -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend(u32::MAX.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16_u32.to_le_bytes());
        bytes.extend(3_u16.to_le_bytes());
        bytes.extend(channels.to_le_bytes());
        bytes.extend(8000_u32.to_le_bytes());
        bytes.extend((8000_u32 * u32::from(channels) * 4).to_le_bytes());
        bytes.extend((channels * 4).to_le_bytes());
        bytes.extend(32_u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(u32::MAX.to_le_bytes());
        for sample in samples {
            bytes.extend(sample.to_le_bytes());
        }
        bytes
    }

    /// Removes faint edge noise while retaining quiet single-channel music and its interior pause.
    #[test]
    fn faint_edges_and_quiet_music() {
        let mut samples = vec![0.0015; 4 * 8000 * 2];
        for frame in (8000..12000).chain(20000..24000) {
            samples[frame * 2 + 1] = -0.002;
        }
        let bytes = wav(&samples, 2);
        let analysis = decode(
            &mut &bytes[..],
            &AtomicBool::new(false),
            Instant::now(),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(
            analysis,
            EdgeAnalysis::new(4.0, Some(1.0), Some(3.0)).unwrap()
        );
        let region = analysis.region(resolibre_core::playback_edges::PlaybackEdges {
            trim: true,
            ..Default::default()
        });
        assert_eq!(region.start, 0.95);
        assert_eq!(region.end, 3.05);
    }

    /// Every channel participates; antiphase audio and short/quiet/interior sections are preserved.
    #[test]
    fn original_pcm_detection_and_rejection() {
        let cancel = AtomicBool::new(false);
        let mut samples = vec![0.0; 32000];
        for frame in 8000..12000 {
            samples[frame * 2] = 0.2;
            samples[frame * 2 + 1] = -0.2;
        }
        let bytes = wav(&samples, 2);
        let result = decode(
            &mut &bytes[..],
            &cancel,
            Instant::now(),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(
            result,
            EdgeAnalysis::new(2.0, Some(1.0), Some(1.5)).unwrap()
        );
        let silent = wav(&[0.0005; 8000], 1);
        assert_eq!(
            decode(
                &mut &silent[..],
                &cancel,
                Instant::now(),
                Duration::from_secs(1)
            )
            .unwrap()
            .first_sound,
            None
        );
        for invalid in [
            wav(&[f32::NAN], 1),
            wav(&[f32::INFINITY], 1),
            bytes[..bytes.len() - 1].to_vec(),
            bytes[..20].to_vec(),
            wav(&[], 1),
            wav(&[0.0], 0),
        ] {
            assert!(
                decode(
                    &mut &invalid[..],
                    &cancel,
                    Instant::now(),
                    Duration::from_secs(1)
                )
                .is_err()
            );
        }
        cancel.store(true, Ordering::Relaxed);
        assert!(
            decode(
                &mut &bytes[..],
                &cancel,
                Instant::now(),
                Duration::from_secs(1)
            )
            .is_err()
        );
    }
}
