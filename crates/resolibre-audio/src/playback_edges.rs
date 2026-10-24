//! Native source-time bounds and sample envelopes, independent of application volume and EQ.
use crate::{LocalPlayer, PlaybackError, check_native, ffi};
use resolibre_core::playback_edges::{FadeMode, PlaybackRegion};
use std::fs::File;

/// Persistent graph configuration is separate from live commands, which are lost on native resets.
#[derive(Default)]
pub(crate) struct EdgeFilter {
    configured: Option<(PlaybackRegion, FadeMode)>,
    pub dirty: bool,
}

impl LocalPlayer {
    /// Shares the retained read-only source with an original-audio analysis job.
    pub(crate) fn analysis_input(&self) -> Result<File, PlaybackError> {
        self.input
            .try_clone()
            .map_err(|error| PlaybackError::Input(error.kind()))
    }

    /// Selects the exact FFmpeg stream currently decoded by mpv, including default video audio.
    pub(crate) fn audio_stream_index(&mut self) -> Result<u32, PlaybackError> {
        let mut index = -1_i64;
        // SAFETY: INT64 writes into aligned live i64 storage; handle and static name stay valid.
        let code = unsafe {
            ffi::mpv_get_property(
                self.handle.as_ptr(),
                c"current-tracks/audio/ff-index".as_ptr(),
                ffi::FORMAT_INT64,
                (&mut index as *mut i64).cast(),
            )
        };
        check_native("audio stream", code)?;
        u32::try_from(index).map_err(|_| PlaybackError::InvalidFormat)
    }

    /// Changes edge processing without pausing or seeking an already-playing in-range track.
    /// Every subsequent seek uses these bounds. Analysis uses an independent, unprocessed decoder.
    pub(crate) fn apply_edges(
        &mut self,
        edges: Option<(PlaybackRegion, FadeMode)>,
        equalizer: resolibre_core::equalizer::Equalizer,
        preparing_start: bool,
    ) -> Result<(), PlaybackError> {
        if self.edges == edges {
            return Ok(());
        }
        let timing = self.timing()?;
        let position = timing.position.unwrap_or(0.0);
        if let Some((region, _)) = edges {
            self.playback_command(
                &["set", "start", &region.start.to_string()],
                "playback start",
            )?;
            self.playback_command(&["set", "end", &region.end.to_string()], "playback end")?;
        } else {
            self.playback_command(&["set", "start", "none"], "playback start")?;
            self.playback_command(&["set", "end", "none"], "playback end")?;
        }
        self.set_edge_envelope(edges)?;
        self.edges = edges;
        self.edge_filter.dirty = false;
        let outside =
            edges.is_some_and(|(region, _)| position < region.start || position > region.end);
        // Only startup or a newly excluded edge needs a flush. Paused edits also retain
        // queued samples; repeated seeks would accumulate the output clock's latency.
        if preparing_start || outside {
            let target = edges.map_or(position, |(region, _)| {
                position.clamp(region.start, region.end)
            });
            self.seek_with_equalizer(
                resolibre_core::SeekPosition::from_seconds(target)
                    .ok_or(PlaybackError::InvalidEvent)?,
                equalizer,
            )?;
        }
        Ok(())
    }

    /// Restores live parameters after format negotiation without replacing the audio graph.
    pub(crate) fn restore_edge_envelope(&mut self) -> Result<(), PlaybackError> {
        if self.edge_filter.dirty {
            // Explicit seeks already committed these parameters. A short file can finish
            // decoding into the output buffer before this event, leaving no graph to command.
            if self.edge_filter.configured != self.edges {
                self.set_edge_envelope(self.edges)?;
            }
            self.edge_filter.dirty = false;
        }
        Ok(())
    }

    /// Saves current fade intent only before an explicit seek can recreate the native graph.
    pub(crate) fn commit_edge_envelope(&mut self) -> Result<(), PlaybackError> {
        if self.edge_filter.configured != self.edges {
            self.playback_command(
                &["af", "add", &filter_graph(self.edges)],
                "playback envelope setup",
            )?;
            self.edge_filter.configured = self.edges;
        }
        Ok(())
    }

    /// Updates two fixed filters in place; disabling a fade never removes buffered samples.
    fn set_edge_envelope(
        &mut self,
        edges: Option<(PlaybackRegion, FadeMode)>,
    ) -> Result<(), PlaybackError> {
        let mode = edges.map_or(FadeMode::Off, |(_, mode)| mode);
        if let Some((region, mode)) = edges {
            let duration = region.fade_seconds(mode);
            for (target, start) in [
                ("afade@in", region.start),
                ("afade@out", region.end - duration),
            ] {
                self.edge_parameter(target, "start_time", &start.to_string())?;
                self.edge_parameter(target, "duration", &duration.to_string())?;
            }
        }
        self.edge_parameter(
            "afade@in",
            "enable",
            if mode.fades_in() { "1" } else { "0" },
        )?;
        self.edge_parameter(
            "afade@out",
            "enable",
            if mode.fades_out() { "1" } else { "0" },
        )
    }

    /// Sends only adapter-owned names and validated numeric arguments to libavfilter.
    fn edge_parameter(
        &mut self,
        target: &str,
        command: &str,
        value: &str,
    ) -> Result<(), PlaybackError> {
        self.playback_command(
            &["af-command", "resolibre_edges", command, value, target],
            "playback envelope",
        )
    }

    /// mpv's end boundary follows playback direction: reverse scans stop at the trimmed start.
    pub(crate) fn set_edge_direction(&mut self, backward: bool) -> Result<(), PlaybackError> {
        if let Some((region, _)) = self.edges {
            let end = if backward { region.start } else { region.end };
            self.playback_command(&["set", "end", &end.to_string()], "scan trimmed boundary")?;
        }
        Ok(())
    }
}

/// Installs both source-time ramps once; runtime enable/parameter commands preserve the active graph.
pub(crate) fn filter_graph(edges: Option<(PlaybackRegion, FadeMode)>) -> String {
    let (region, mode) = edges.unwrap_or((
        PlaybackRegion {
            start: 0.0,
            end: 1.0,
        },
        FadeMode::Off,
    ));
    let duration = region.fade_seconds(mode);
    let start = region.start;
    let end = region.end - duration;
    let fade_in = u8::from(mode.fades_in());
    let fade_out = u8::from(mode.fades_out());
    format!(
        "@resolibre_edges:lavfi=[afade@in=t=in:st={start}:d={duration}:curve=tri:enable={fade_in},afade@out=t=out:st={end}:d={duration}:curve=tri:enable={fade_out}]"
    )
}
