//! MP4 edit lists affect native playback timing but are not applied by Symphonia 0.6.1.

use super::BoundedSource;
use std::io::{Read, Seek, SeekFrom};
use symphonia::core::io::MediaSource;

#[derive(Default)]
struct TrackTiming {
    audio: bool,
    edits: Vec<u8>,
}

/// Returns audio edit lists and their movie timescale, excluding tags, artwork tracks and creation dates.
/// Non-MP4 files return an empty supplement; malformed MP4 boxes disable caching.
pub(super) fn read(source: &mut BoundedSource<'_>) -> Option<(bool, Vec<u8>)> {
    let mut header = [0; 8];
    source.read_exact(&mut header).ok()?;
    if !matches!(
        &header[4..],
        b"ftyp" | b"free" | b"wide" | b"moov" | b"mdat"
    ) {
        return Some((false, Vec::new()));
    }
    let end = source.byte_len()?;
    let mut bytes = Vec::new();
    let mut budget = 4096;
    scan(
        source,
        0,
        end,
        0,
        &mut budget,
        &mut bytes,
        &mut TrackTiming::default(),
    )?;
    Some((true, bytes))
}

/// Walks only structural containers; bounded box count/depth and checked sizes prevent allocation/seek abuse.
fn scan(
    source: &mut BoundedSource<'_>,
    start: u64,
    end: u64,
    depth: u8,
    budget: &mut usize,
    bytes: &mut Vec<u8>,
    track: &mut TrackTiming,
) -> Option<()> {
    if depth > 4 {
        return None;
    }
    let mut position = start;
    while position < end {
        *budget = budget.checked_sub(1)?;
        if end - position < 8 {
            return None;
        }
        source.seek(SeekFrom::Start(position)).ok()?;
        let mut header = [0; 8];
        source.read_exact(&mut header).ok()?;
        let mut size = u64::from(u32::from_be_bytes(header[..4].try_into().ok()?));
        let mut header_size = 8;
        if size == 1 {
            let mut extended = [0; 8];
            source.read_exact(&mut extended).ok()?;
            size = u64::from_be_bytes(extended);
            header_size = 16;
        } else if size == 0 {
            size = end - position;
        }
        let next = position.checked_add(size)?;
        if size < header_size || next > end {
            return None;
        }
        let payload = position + header_size;
        match &header[4..] {
            b"moov" | b"mdia" | b"edts" => {
                scan(source, payload, next, depth + 1, budget, bytes, track)?
            }
            b"trak" => {
                let mut child = TrackTiming::default();
                scan(source, payload, next, depth + 1, budget, bytes, &mut child)?;
                if child.audio {
                    bytes.extend_from_slice(&(child.edits.len() as u64).to_le_bytes());
                    bytes.extend_from_slice(&child.edits);
                }
            }
            b"hdlr" if next - payload >= 12 => {
                let mut handler = [0; 12];
                source.read_exact(&mut handler).ok()?;
                track.audio = &handler[8..] == b"soun";
            }
            b"elst" => {
                let length = next - payload;
                if length > 65536 || track.edits.len() + length as usize > 65536 {
                    return None;
                }
                let mut edits = vec![0; length as usize];
                source.read_exact(&mut edits).ok()?;
                track.edits.extend(edits);
            }
            b"mvhd" => {
                let mut version = [0; 4];
                source.read_exact(&mut version).ok()?;
                let offset = match version[0] {
                    0 => 12,
                    1 => 20,
                    _ => return None,
                };
                if payload + offset + 4 > next {
                    return None;
                }
                source.seek(SeekFrom::Start(payload + offset)).ok()?;
                let mut scale = [0; 4];
                source.read_exact(&mut scale).ok()?;
                bytes.extend_from_slice(&scale);
            }
            _ => {}
        }
        if bytes.len() > 65536 {
            return None;
        }
        position = next;
    }
    Some(())
}
