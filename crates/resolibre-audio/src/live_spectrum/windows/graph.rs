//! Numeric filter graphs keep original channels separate from measured branches.

use resolibre_core::spectrum::live::{BAND_COUNT, band_edges};
use std::ffi::CString;

pub(super) struct Group {
    pub(super) label: String,
    pub(super) property: CString,
    pub(super) bands: Vec<usize>,
    pub(super) channels: u32,
}

pub(super) struct Graph {
    pub(super) filters: String,
    pub(super) groups: Vec<Group>,
    pub(super) rate: u32,
    pub(super) channels: u32,
    pub(super) window_frames: u32,
}

impl Graph {
    /// Bounds each merge to 64 channels and restores the validated layout through a pure channel map.
    pub(super) fn new(rate: u32, channels: u32, source_layout: &str) -> Result<Self, &'static str> {
        if !(1..=1_536_000).contains(&rate) || !(1..=32).contains(&channels) {
            return Err("Live spectrum format unsupported");
        }
        let layout = layout(source_layout, channels)?;
        let edges = band_edges();
        let nyquist = f64::from(rate) / 2.0;
        let active: Vec<_> = (0..BAND_COUNT)
            .filter(|band| edges[*band] < nyquist)
            .collect();
        let capacity = (64 / channels - 1) as usize;
        let window_frames = (rate / 20).clamp(1, 8192);
        let mut groups = Vec::new();
        let mut filters = Vec::new();
        for bands in active.chunks(capacity) {
            let label = format!("resolibre_live_{}", bands[0]);
            let property = CString::new(format!("af-metadata/{label}"))
                .map_err(|_| "Invalid live spectrum property")?;
            let body = filter(bands, channels, rate, window_frames, layout);
            // Short audio windows must not create slice-worker wakeups for every band/channel.
            filters.push(format!("@{label}:lavfi=[{body}]:o=[threads=1]"));
            groups.push(Group {
                label,
                property,
                bands: bands.to_vec(),
                channels,
            });
        }
        Ok(Self {
            filters: filters.join(","),
            groups,
            rate,
            channels,
            window_frames,
        })
    }
}

/// Uses owned syntax and finite constants; no media paths, tags or unchecked identifiers enter the graph.
fn filter(bands: &[usize], channels: u32, rate: u32, frames: u32, layout: &str) -> String {
    let mut split = format!(
        "aformat=sample_fmts=dblp,asetnsamples=n={frames}:p=0,asplit={}[main]",
        bands.len() + 1
    );
    for index in 0..bands.len() {
        split.push_str(&format!("[b{index}]"));
    }
    let mut steps = vec![split];
    let edges = band_edges();
    for (index, band) in bands.iter().enumerate() {
        let low = edges[*band];
        let high = edges[*band + 1].min(f64::from(rate) / 2.0);
        let center = (low * high).sqrt();
        let width = (high / low).log2();
        steps.push(format!("[b{index}]bandpass=f={center:.9}:t=o:w={width:.9},bandpass=f={center:.9}:t=o:w={width:.9}[f{index}]"));
    }
    let mut merge = "[main]".to_owned();
    for index in 0..bands.len() {
        merge.push_str(&format!("[f{index}]"));
    }
    merge.push_str(&format!("amerge=inputs={},ametadata=mode=delete,astats=metadata=1:reset=1:length=0:measure_perchannel=RMS_level:measure_overall=none,pan={layout}", bands.len() + 1));
    for channel in 0..channels {
        merge.push_str(&format!("|c{channel}=c{channel}"));
    }
    // Count time only after restoring original channels, avoiding a second pass over every band.
    merge.push_str(",astats=metadata=1:reset=0:length=0:measure_perchannel=none:measure_overall=Number_of_samples");
    steps.push(merge);
    steps.join(";")
}

/// Accepts known layouts only, preventing graph injection and accidental speaker relabeling.
fn layout(value: &str, channels: u32) -> Result<&'static str, &'static str> {
    let (count, name) = match value {
        "mono" => (1, "mono"),
        "stereo" => (2, "stereo"),
        "2.1" => (3, "2.1"),
        "3.0" => (3, "3.0"),
        "quad" => (4, "quad"),
        "quad(side)" => (4, "quad(side)"),
        "3.1" => (4, "3.1"),
        "4.0" => (4, "4.0"),
        "4.1" => (5, "4.1"),
        "5.0" => (5, "5.0"),
        "5.0(side)" => (5, "5.0(side)"),
        "5.1" => (6, "5.1"),
        "5.1(side)" => (6, "5.1(side)"),
        "6.0" => (6, "6.0"),
        "hexagonal" => (6, "hexagonal"),
        "6.1" => (7, "6.1"),
        "6.1(back)" => (7, "6.1(back)"),
        "7.0" => (7, "7.0"),
        "7.1" => (8, "7.1"),
        "7.1(wide)" => (8, "7.1(wide)"),
        "octagonal" => (8, "octagonal"),
        "hexadecagonal" => (16, "hexadecagonal"),
        "22.2" => (24, "22.2"),
        _ => return Err("Live spectrum channel layout unsupported"),
    };
    if count != channels {
        return Err("Live spectrum channel layout mismatch");
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Graph size and merge dimensions stay bounded; missing/high-frequency bands cannot invent energy.
    #[test]
    fn bounded_graphs_preserve_layout_and_reject_injection() {
        for (channels, layout) in [
            (1, "mono"),
            (2, "stereo"),
            (6, "5.1"),
            (8, "7.1"),
            (24, "22.2"),
        ] {
            let graph = Graph::new(48000, channels, layout).unwrap();
            assert!(graph.filters.len() < 32768);
            for group in graph.groups {
                assert!(channels as usize * (group.bands.len() + 1) <= 64);
            }
        }
        let low_rate = Graph::new(8000, 1, "mono").unwrap();
        assert!(low_rate.groups[0].bands.len() < BAND_COUNT);
        for (rate, channels, layout) in [
            (0, 2, "stereo"),
            (48000, 0, "mono"),
            (48000, 1, "stereo"),
            (48000, 2, "stereo];amovie=untrusted"),
        ] {
            assert!(Graph::new(rate, channels, layout).is_err());
        }
    }
}
