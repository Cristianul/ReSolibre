//! Only fixed numeric graph metadata crosses the native allocation boundary.

use super::graph::Group;
use crate::ffi;
use std::collections::BTreeMap;
use std::ffi::{CStr, c_char};
use std::ptr::NonNull;

const MAX_METADATA_BYTES: usize = 16 * 1024;

pub(super) struct Measurement {
    pub(super) frames: u64,
    pub(super) power: Vec<(usize, f64)>,
}

struct NativeString(*mut c_char);

impl Drop for NativeString {
    /// Releases exactly the mpv allocation, including parsing/error paths.
    fn drop(&mut self) {
        // SAFETY: this pointer has one owner and came from MPV_FORMAT_STRING.
        unsafe { ffi::mpv_free(self.0.cast()) };
    }
}

/// Reads the negotiated layout without allowing unchecked identifiers into filter syntax.
pub(super) fn layout(handle: NonNull<ffi::Handle>) -> Result<String, &'static str> {
    read_string(handle, c"audio-params/channels", 64)?
        .ok_or("Live spectrum channel layout unavailable")
}

/// Reads one bounded snapshot; empty metadata is normal before the first decoded frame.
pub(super) fn read(
    handle: NonNull<ffi::Handle>,
    group: &Group,
) -> Result<Option<Measurement>, &'static str> {
    let Some(text) = read_string(handle, &group.property, MAX_METADATA_BYTES)? else {
        return Ok(None);
    };
    parse(&text, group)
}

/// Checks exact counters and finite channel energy; negative infinity explicitly represents silence.
fn parse(text: &str, group: &Group) -> Result<Option<Measurement>, &'static str> {
    if text.len() > MAX_METADATA_BYTES {
        return Err("Oversized live spectrum metadata");
    }
    let values: BTreeMap<String, String> =
        serde_json::from_str(text).map_err(|_| "Invalid live spectrum metadata")?;
    if values.len() > 65 {
        return Err("Oversized live spectrum metadata");
    }
    let Some(counter) = values.get("lavfi.astats.Overall.Number_of_samples") else {
        return Ok(None);
    };
    let frames: f64 = counter
        .parse()
        .map_err(|_| "Invalid live spectrum counter")?;
    if !frames.is_finite()
        || !(0.0..=(1_u64 << 52) as f64).contains(&frames)
        || frames.fract() != 0.0
    {
        return Err("Invalid live spectrum counter");
    }
    let mut power = Vec::with_capacity(group.bands.len());
    for (index, band) in group.bands.iter().enumerate() {
        let mut sum = 0.0;
        for channel in 0..group.channels {
            let native_channel = group.channels as usize * (index + 1) + channel as usize + 1;
            let key = format!("lavfi.astats.{native_channel}.RMS_level");
            let value: f64 = values
                .get(&key)
                .ok_or("Missing live spectrum channel")?
                .parse()
                .map_err(|_| "Invalid live spectrum power")?;
            if value == f64::NEG_INFINITY {
                continue;
            }
            if !value.is_finite() {
                return Err("Invalid live spectrum power");
            }
            sum += 10_f64.powf(value / 10.0);
        }
        if !sum.is_finite() {
            return Err("Invalid live spectrum power");
        }
        power.push((*band, sum / f64::from(group.channels)));
    }
    Ok(Some(Measurement {
        frames: frames as u64,
        power,
    }))
}

/// Copies a bounded string while its native allocation and owning player's handle remain alive.
fn read_string(
    handle: NonNull<ffi::Handle>,
    property: &CStr,
    limit: usize,
) -> Result<Option<String>, &'static str> {
    let mut pointer = std::ptr::null_mut::<c_char>();
    // SAFETY: LocalPlayer keeps the handle alive; FORMAT_STRING writes one owned pointer.
    let result = unsafe {
        ffi::mpv_get_property(
            handle.as_ptr(),
            property.as_ptr(),
            ffi::FORMAT_STRING,
            (&mut pointer as *mut *mut c_char).cast(),
        )
    };
    if pointer.is_null() {
        return if matches!(
            result,
            ffi::ERROR_PROPERTY_UNAVAILABLE
                | ffi::ERROR_PROPERTY_NOT_FOUND
                | ffi::ERROR_PROPERTY_ERROR
        ) {
            Ok(None)
        } else {
            Err("Live spectrum metadata unavailable")
        };
    }
    let owned = NativeString(pointer);
    if result < 0 {
        return Err("Live spectrum metadata unavailable");
    }
    // Scan to the native terminator or the limit, without inventing an oversized allocation slice.
    for length in 0..=limit {
        // SAFETY: mpv guarantees termination; each byte before its terminator belongs to the allocation.
        if unsafe { *owned.0.add(length) } == 0 {
            // SAFETY: the scan proved these bytes precede the owned allocation's terminator.
            let bytes = unsafe { std::slice::from_raw_parts(owned.0.cast::<u8>(), length) };
            let text = std::str::from_utf8(bytes).map_err(|_| "Invalid live spectrum metadata")?;
            return Ok(Some(text.to_owned()));
        }
    }
    Err("Oversized live spectrum metadata")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keeps parsing tests independent of a native player while using the real channel-energy schema.
    fn group() -> Group {
        Group {
            label: "test".into(),
            property: std::ffi::CString::new("af-metadata/test").unwrap(),
            bands: vec![4],
            channels: 2,
        }
    }

    /// Both channels contribute energy; silence is valid, but malformed/oversized native data fails closed.
    #[test]
    fn validates_counters_power_and_payload_bounds() {
        let group = group();
        let values = |counter: &str, left: &str, right: &str| {
            serde_json::json!({
                "lavfi.astats.Overall.Number_of_samples": counter,
                "lavfi.astats.3.RMS_level": left,
                "lavfi.astats.4.RMS_level": right,
            })
            .to_string()
        };
        let measured = parse(&values("1200", "-20", "-40"), &group)
            .unwrap()
            .unwrap();
        assert_eq!(measured.frames, 1200);
        assert!((measured.power[0].1 - 0.00505).abs() < 1e-10);
        assert_eq!(
            parse(&values("1200", "-inf", "-inf"), &group)
                .unwrap()
                .unwrap()
                .power[0]
                .1,
            0.0
        );
        for counter in ["NaN", "inf", "-1", "1.5", "9007199254740992"] {
            assert!(parse(&values(counter, "-20", "-20"), &group).is_err());
        }
        for energy in ["NaN", "inf", "10000", "invalid"] {
            assert!(parse(&values("1200", energy, "-20"), &group).is_err());
        }
        assert!(parse(&" ".repeat(MAX_METADATA_BYTES + 1), &group).is_err());
        assert!(parse("{}", &group).unwrap().is_none());
    }
}
