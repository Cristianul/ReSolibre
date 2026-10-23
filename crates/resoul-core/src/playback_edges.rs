//! One source-time boundary contract for playback; analysis always reads the original audio.

/// Shared -55 dBFS threshold: slightly stronger edge trimming than the original -60 dBFS policy.
pub const SILENCE_AMPLITUDE: f32 = 0.001_778_279_4;
pub const MIN_SILENCE_SECONDS: f64 = 0.5;
pub const EDGE_PADDING_SECONDS: f64 = 0.05;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FadeMode {
    #[default]
    Off,
    In,
    Out,
    Both,
}

impl FadeMode {
    /// Advances the panel's single four-state control.
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::In,
            Self::In => Self::Out,
            Self::Out => Self::Both,
            Self::Both => Self::Off,
        }
    }

    /// Reports whether the source-time envelope includes its opening edge.
    pub fn fades_in(self) -> bool {
        matches!(self, Self::In | Self::Both)
    }

    /// Reports whether the source-time envelope includes its closing edge.
    pub fn fades_out(self) -> bool {
        matches!(self, Self::Out | Self::Both)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaybackEdges {
    pub trim: bool,
    pub fade: FadeMode,
}

#[derive(Clone, Copy, Debug)]
pub enum EdgeAction {
    ToggleTrim,
    CycleFade,
    Set(PlaybackEdges),
}

impl PlaybackEdges {
    /// Changes authoritative intent; views and future shortcuts share this entry point.
    pub fn apply(&mut self, action: EdgeAction) {
        match action {
            EdgeAction::ToggleTrim => self.trim = !self.trim,
            EdgeAction::CycleFade => self.fade = self.fade.cycle(),
            EdgeAction::Set(settings) => *self = settings,
        }
    }

    /// Processing requires reliable source bounds; disabled playback needs no analysis.
    pub fn needs_analysis(self) -> bool {
        self.trim || self.fade != FadeMode::Off
    }
}

/// Validated decoded source extent; boundaries use all channels before any playback DSP.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeAnalysis {
    pub duration: f64,
    pub first_sound: Option<f64>,
    pub last_sound: Option<f64>,
}

impl EdgeAnalysis {
    /// Rejects malformed measurements, including incomplete or reversed sound boundaries.
    pub fn new(duration: f64, first: Option<f64>, last: Option<f64>) -> Option<Self> {
        if !duration.is_finite() || duration <= 0.0 {
            return None;
        }
        match (first, last) {
            (None, None) => {}
            (Some(first), Some(last))
                if first.is_finite()
                    && last.is_finite()
                    && first >= 0.0
                    && first < last
                    && last <= duration => {}
            _ => return None,
        }
        Some(Self {
            duration,
            first_sound: first,
            last_sound: last,
        })
    }

    /// Applies only qualified edge silence; internal pauses and all-silent files stay intact.
    pub fn region(self, settings: PlaybackEdges) -> PlaybackRegion {
        let mut region = PlaybackRegion {
            start: 0.0,
            end: self.duration,
        };
        if settings.trim
            && let (Some(first), Some(last)) = (self.first_sound, self.last_sound)
        {
            if first >= MIN_SILENCE_SECONDS {
                region.start = (first - EDGE_PADDING_SECONDS).max(0.0);
            }
            if self.duration - last >= MIN_SILENCE_SECONDS {
                region.end = (last + EDGE_PADDING_SECONDS).min(self.duration);
            }
        }
        region
    }
}

/// Effective source-time interval used by seek, scan, EOF and every playback envelope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaybackRegion {
    pub start: f64,
    pub end: f64,
}

impl PlaybackRegion {
    /// Returns the visible, trimmed duration.
    pub fn duration(self) -> f64 {
        self.end - self.start
    }

    /// Maps an effective timeline seek to the shared source interval.
    pub fn source_time(self, elapsed: f64) -> f64 {
        (self.start + elapsed).clamp(self.start, self.end)
    }

    /// Maps a native source position to the visible playback timeline.
    pub fn elapsed(self, source: f64) -> f64 {
        (source - self.start).clamp(0.0, self.duration())
    }

    /// Caps one-second ramps to the available interval, avoiding overlap on short tracks.
    pub fn fade_seconds(self, mode: FadeMode) -> f64 {
        let divisor = if mode == FadeMode::Both { 2.0 } else { 1.0 };
        (self.duration() / divisor).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Silence decisions preserve interior pauses, short quiet edges and all-silent tracks.
    #[test]
    fn boundaries_and_fades_share_one_timeline() {
        let analysis = EdgeAnalysis::new(20.0, Some(5.0), Some(10.0)).unwrap();
        let settings = PlaybackEdges {
            trim: true,
            fade: FadeMode::Both,
        };
        let region = analysis.region(settings);
        assert_eq!(
            region,
            PlaybackRegion {
                start: 4.95,
                end: 10.05
            }
        );
        assert_eq!(region.source_time(0.0), 4.95);
        assert_eq!(region.source_time(99.0), 10.05);
        assert_eq!(region.elapsed(3.0), 0.0);
        assert_eq!(region.fade_seconds(settings.fade), 1.0);
        assert_eq!(analysis.region(PlaybackEdges::default()).duration(), 20.0);
        let short = EdgeAnalysis::new(0.8, Some(0.1), Some(0.7))
            .unwrap()
            .region(settings);
        assert_eq!(short.start, 0.0);
        assert_eq!(short.end, 0.8);
        assert_eq!(short.fade_seconds(FadeMode::Both), 0.4);
        assert_eq!(short.fade_seconds(FadeMode::In), 0.8);
        assert_eq!(
            EdgeAnalysis::new(10.0, None, None)
                .unwrap()
                .region(settings)
                .duration(),
            10.0
        );
        for (duration, first, last) in [
            (f64::NAN, None, None),
            (0.0, None, None),
            (1.0, Some(0.5), None),
            (1.0, Some(0.8), Some(0.2)),
            (1.0, Some(0.0), Some(2.0)),
        ] {
            assert!(EdgeAnalysis::new(duration, first, last).is_none());
        }
        let mut mode = FadeMode::Off;
        for expected in [FadeMode::In, FadeMode::Out, FadeMode::Both, FadeMode::Off] {
            mode = mode.cycle();
            assert_eq!(mode, expected);
        }
    }
}
