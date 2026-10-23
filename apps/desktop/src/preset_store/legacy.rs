//! Read-only migration of the original three slots.

use resolibre_core::equalizer::Equalizer;
type Slots = [Option<Equalizer>; 3];
const MAX_BYTES: u64 = 4096;

/// Rejects oversized, unknown-version, extra-field, non-finite or out-of-range stored data atomically.
pub(super) fn decode(bytes: &[u8]) -> Result<Slots, &'static str> {
    let invalid = "Invalid equalizer preset data";
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid)?;
    let mut lines = text.lines();
    if lines.next() != Some("ReSolibre-EQ-1") {
        return Err(invalid);
    }
    let mut slots = [None; 3];
    for slot in &mut slots {
        let line = lines.next().ok_or(invalid)?;
        if line == "-" {
            continue;
        }
        let mut fields = line.split_whitespace();
        let enabled = match fields.next() {
            Some("0") => false,
            Some("1") => true,
            _ => return Err(invalid),
        };
        let preamp = fields.next().ok_or(invalid)?.parse().map_err(|_| invalid)?;
        let mut bands = [0.0; 10];
        for band in &mut bands {
            *band = fields.next().ok_or(invalid)?.parse().map_err(|_| invalid)?;
        }
        if fields.next().is_some() {
            return Err(invalid);
        }
        *slot = Some(Equalizer::from_saved_v1(enabled, bands, preamp).map_err(|_| invalid)?);
    }
    if lines.next().is_some() {
        return Err(invalid);
    }
    Ok(slots)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Legacy data remains strict even though valid old preamps migrate into today's narrower range.
    #[test]
    fn legacy_corruption_is_rejected_before_migration() {
        let valid = "ReSolibre-EQ-1\n1 0 0 0 0 0 0 0 0 0 0 0\n-\n-\n";
        assert!(decode(valid.as_bytes()).unwrap()[0].is_some());
        for invalid in [
            valid.replace("EQ-1", "EQ-9"),
            valid.replace("1 0", "1 NaN"),
            valid.replace("1 0", "1 -25"),
            valid.replace("1 0 0", "1 0 13"),
            format!("{valid}extra"),
            "x".repeat(4097),
        ] {
            assert!(decode(invalid.as_bytes()).is_err());
        }
        assert!(decode(b"ReSolibre-EQ-1\n-\n-\n").is_err());
        assert!(decode(&[0xff]).is_err());
    }
}
