//! Bounded in-player filter measurements; no second decoder or system capture device.

mod graph;
mod metadata;

use crate::ffi;
use graph::Graph;
use resolibre_core::spectrum::{
    AnalysisError,
    live::{BAND_COUNT, BandMeasurement, LiveConsumer, LiveLevels},
};
use std::ptr::NonNull;
use std::time::{Duration, Instant};

pub(crate) struct LiveSpectrum {
    // LocalPlayer owns the handle and stops polling before destruction. Drop performs no FFI.
    handle: NonNull<ffi::Handle>,
    pub(super) format: Option<(u32, u32)>,
    graph: Option<Graph>,
    consumer: LiveConsumer,
    instance: u32,
    last_end: u64,
    blocked: Vec<Option<u64>>,
    last_poll: Option<Instant>,
    last_progress_clock: Option<f64>,
}

#[cfg(test)]
mod tests;

impl LiveSpectrum {
    /// Borrows the owning player's handle, queried only by its existing worker.
    pub(super) fn new(handle: NonNull<ffi::Handle>) -> Result<Self, &'static str> {
        Ok(Self {
            handle,
            format: None,
            graph: None,
            consumer: LiveConsumer::default(),
            instance: 1,
            last_end: 0,
            blocked: Vec::new(),
            last_poll: None,
            last_progress_clock: None,
        })
    }

    /// Lists only this player's own filters before replacing a changed source format.
    pub(super) fn labels(&self) -> Vec<String> {
        self.graph.as_ref().map_or_else(Vec::new, |graph| {
            graph
                .groups
                .iter()
                .map(|group| group.label.clone())
                .collect()
        })
    }

    /// Restores the validated source layout so speaker positions cannot change for visualization.
    pub(super) fn graph(&mut self, rate: u32, channels: u32) -> Result<String, &'static str> {
        let layout = metadata::layout(self.handle)?;
        let graph = Graph::new(rate, channels, &layout)?;
        let filters = graph.filters.clone();
        self.blocked = vec![None; graph.groups.len()];
        self.graph = Some(graph);
        self.last_end = 0;
        self.last_poll = None;
        self.last_progress_clock = None;
        Ok(filters)
    }

    /// Rejects currently published native frames before the owner issues its exact seek.
    pub(crate) fn seek(&mut self, seconds: f64) -> Result<(), AnalysisError> {
        self.consumer.seek(self.instance, seconds)?;
        self.instance = self
            .instance
            .checked_add(1)
            .ok_or(AnalysisError::PositionOverflow)?;
        if let Some(graph) = &self.graph {
            for (group, blocked) in graph.groups.iter().zip(&mut self.blocked) {
                *blocked = metadata::read(self.handle, group)
                    .ok()
                    .flatten()
                    .map(|data| data.frames);
            }
        }
        self.last_end = 0;
        self.last_poll = None;
        self.last_progress_clock = Some(seconds);
        Ok(())
    }

    /// Reads at most twenty bounded snapshots per second; the shared queue follows the playback clock.
    pub(super) fn poll(
        &mut self,
        clock: Option<f64>,
        paused: bool,
        scanning: bool,
    ) -> Result<LiveLevels, &'static str> {
        let clock = clock.unwrap_or(self.consumer.offset());
        if !paused
            && !scanning
            && self
                .last_poll
                .is_none_or(|last| last.elapsed() >= Duration::from_millis(50))
        {
            self.last_poll = Some(Instant::now());
            self.last_progress_clock.get_or_insert(clock);
            self.measure(clock)?;
            if self
                .last_progress_clock
                .is_some_and(|last| clock - last > 3.0)
            {
                return Err("Live spectrum measurements unavailable");
            }
        }
        Ok(self.consumer.present(clock, paused, scanning))
    }

    /// Averages energy across channels without phase cancellation and rejects pre-seek measurements.
    fn measure(&mut self, clock: f64) -> Result<(), &'static str> {
        let graph = self
            .graph
            .as_ref()
            .ok_or("Live spectrum graph unavailable")?;
        if graph.groups.is_empty() {
            self.last_progress_clock = Some(clock);
            return Ok(());
        }
        let mut power = [0.0; BAND_COUNT];
        let mut end_frame = u64::MAX;
        for (index, group) in graph.groups.iter().enumerate() {
            let Some(data) = metadata::read(self.handle, group)? else {
                return Ok(());
            };
            if self.blocked[index] == Some(data.frames) {
                return Ok(());
            }
            end_frame = end_frame.min(data.frames);
            for (band, energy) in data.power {
                power[band] = energy;
            }
        }
        self.blocked.fill(None);
        if end_frame < u64::from(graph.window_frames) || end_frame == self.last_end {
            return Ok(());
        }
        if end_frame < self.last_end {
            self.instance = self
                .instance
                .checked_add(1)
                .ok_or("Live spectrum identity exhausted")?;
        }
        self.last_end = end_frame;
        self.last_progress_clock = Some(clock);
        self.consumer
            .accept_bands(
                BandMeasurement {
                    instance: self.instance,
                    rate: graph.rate,
                    channels: graph.channels as usize,
                    end_frame,
                    window_frames: graph.window_frames,
                    power,
                },
                clock,
            )
            .map_err(|_| "Invalid live spectrum measurement")
    }
}
