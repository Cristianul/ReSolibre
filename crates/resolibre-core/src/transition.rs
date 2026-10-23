//! Bounded source-time transition policy; native playback owns clocks and rendering.

mod ratio;
mod tempo_return;
pub use ratio::BeatRatio;
pub use tempo_return::{
    TEMPO_RETURN_DURATION_SECONDS, TEMPO_RETURN_LEAD_SECONDS, TempoReturn, TempoReturnFrame,
};

use crate::analysis::tempo::{BeatSegment, TempoReport};
use crate::playback_edges::PlaybackRegion;

pub const MAX_EDGE_SKIP_SECONDS: f64 = 16.0;
pub const MAX_TEMPO_CHANGE: f64 = 0.20;
/// Approach the shared midpoint over six outgoing source seconds.
pub const TEMPO_MEET_DURATION_SECONDS: f64 = 6.0;
/// Hold the shared tempo before overlap so buffered audio can finish the approach.
pub const TEMPO_MEET_SETTLE_SECONDS: f64 = 1.0;

/// Shares the finite MIX speed limit between planning and native adapters.
pub fn is_supported_mix_speed(speed: f64) -> bool {
    speed.is_finite() && (1.0 - MAX_TEMPO_CHANGE..=1.0 + MAX_TEMPO_CHANGE).contains(&speed)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionPlan {
    pub outgoing_start: f64,
    pub outgoing_end: f64,
    pub incoming_start: f64,
    pub outgoing_bpm: Option<f64>,
    pub incoming_bpm: Option<f64>,
    /// True only when both overlap origins are supported by observed phase evidence.
    pub phase_matched: bool,
    pub beat_ratio: BeatRatio,
}

#[derive(Clone, Copy, Debug)]
pub struct TransitionFrame {
    pub outgoing_gain: f64,
    pub incoming_gain: f64,
    pub outgoing_speed: f64,
    pub incoming_speed: f64,
    pub complete: bool,
}

impl TransitionPlan {
    /// Chooses bounded MIX edge cuts independently of tempo matching, then aligns beats or crossfades.
    pub fn prepare(
        outgoing: PlaybackRegion,
        incoming: PlaybackRegion,
        current: Option<&TempoReport>,
        next: Option<&TempoReport>,
    ) -> Option<Self> {
        if !valid_region(outgoing) || !valid_region(incoming) {
            return None;
        }
        let maximum_overlap = (outgoing.duration() / 4.0).min(incoming.duration() / 4.0);
        let overlap = overlap_duration(outgoing.duration(), incoming.duration());
        if overlap < 0.5 {
            return None;
        }
        let tail_limit = MAX_EDGE_SKIP_SECONDS.min(outgoing.duration() * 0.1);
        let head_limit = MAX_EDGE_SKIP_SECONDS.min(incoming.duration() * 0.1);
        let earliest_end = outgoing.end - tail_limit;
        let latest_start = incoming.start + head_limit;
        let outgoing = cut_edge(outgoing, current, false);
        let incoming = cut_edge(incoming, next, true);
        let fallback = Self {
            outgoing_start: outgoing.end - overlap,
            outgoing_end: outgoing.end,
            incoming_start: incoming.start,
            outgoing_bpm: None,
            incoming_bpm: None,
            phase_matched: false,
            beat_ratio: BeatRatio::OneToOne,
        };
        let (Some(current), Some(next)) = (current, next) else {
            return Some(fallback);
        };
        let closing_segments = joined_segments(current);
        let opening_segments = joined_segments(next);
        for tail in closing_segments.iter().rev() {
            if tail.end_seconds < earliest_end || tail.start_seconds > outgoing.end {
                continue;
            }
            for head in &opening_segments {
                if head.start_seconds > latest_start || head.end_seconds < incoming.start {
                    continue;
                }
                let Some(beat_ratio) = BeatRatio::choose(tail.bpm, head.bpm) else {
                    continue;
                };
                let (outgoing_beats, incoming_beats) = beat_ratio.counts();
                let target_intervals = overlap * tail.bpm / 60.0;
                // Ratio plans end on complete shared cycles and never extend the preparation lead-in.
                let intervals = if beat_ratio == BeatRatio::OneToOne {
                    target_intervals.round() as usize
                } else {
                    let cycles = (target_intervals / outgoing_beats as f64).floor() as usize;
                    let Some(intervals) = cycles.checked_mul(outgoing_beats) else {
                        continue;
                    };
                    intervals
                };
                let Some(incoming_intervals) =
                    (intervals / outgoing_beats).checked_mul(incoming_beats)
                else {
                    continue;
                };
                // Missing attacks may span whole beats, but the complete overlap still needs phase evidence.
                if intervals < 8 || incoming_intervals < 8 {
                    continue;
                }
                let closing = observed_groups(tail, intervals).rev().find(|beats| {
                    beats.start >= outgoing.start
                        && beats.end <= outgoing.end
                        && beats.end >= earliest_end
                        && beats.end - beats.start <= maximum_overlap
                });
                let opening = observed_groups(head, incoming_intervals).find(|beats| {
                    beats.start >= incoming.start
                        && beats.start <= latest_start
                        && beats.end <= incoming.end
                        && beats.end - beats.start <= maximum_overlap
                });
                if let (Some(closing), Some(opening)) = (closing, opening) {
                    return Some(Self {
                        outgoing_start: closing.start,
                        outgoing_end: closing.end,
                        incoming_start: opening.start,
                        outgoing_bpm: Some(tail.bpm),
                        incoming_bpm: Some(head.bpm),
                        phase_matched: true,
                        beat_ratio,
                    });
                }
            }
        }
        Some(fallback.match_tempo(current, next))
    }

    /// Keeps independently supported speed matching when measured beat phase is unavailable.
    fn match_tempo(mut self, current: &TempoReport, next: &TempoReport) -> Self {
        let Some(current_bpm) = supported_tempo(current, self.outgoing_start, self.outgoing_end)
        else {
            return self;
        };
        // Incoming source time advances by the speed ratio, not by outgoing seconds.
        let Some(next_bpm) = supported_tempo(next, self.incoming_start, self.incoming_start) else {
            return self;
        };
        let Some(beat_ratio) = BeatRatio::choose(current_bpm, next_bpm) else {
            return self;
        };
        let speed = current_bpm / beat_ratio.comparable_incoming_bpm(next_bpm);
        let incoming_end = self.incoming_start + (self.outgoing_end - self.outgoing_start) * speed;
        if supported_tempo(next, self.incoming_start, incoming_end)
            .is_some_and(|bpm| (bpm / next_bpm - 1.0).abs() < 0.005)
        {
            self.outgoing_bpm = Some(current_bpm);
            self.incoming_bpm = Some(next_bpm);
            self.beat_ratio = beat_ratio;
        }
        self
    }

    /// Maps an outgoing source clock to the incoming clock for the retained shared pulse ratio.
    /// Both speeds follow the same ramp, so this source-time ratio stays constant throughout it.
    pub fn incoming_position(self, outgoing_position: f64) -> Option<f64> {
        let current = self.outgoing_bpm?;
        let next = self.beat_ratio.comparable_incoming_bpm(self.incoming_bpm?);
        let speed = current / next;
        if !outgoing_position.is_finite() || !is_supported_mix_speed(speed) {
            return None;
        }
        let position = self.incoming_start + (outgoing_position - self.outgoing_start) * speed;
        (position.is_finite() && position >= 0.0).then_some(position)
    }

    /// Allows the native load deadline and the outgoing tempo approach before the earliest overlap/cut.
    pub fn preparation_start(outgoing: PlaybackRegion) -> Option<f64> {
        if !valid_region(outgoing) {
            return None;
        }

        let duration = outgoing.duration();
        let overlap = overlap_duration(duration, duration);
        let cut = MAX_EDGE_SKIP_SECONDS.min(duration * 0.1);
        // Sixteen seconds cover native loading, beat rounding and muted preroll.
        let preparation_lead = 16.0 + TEMPO_MEET_DURATION_SECONDS + TEMPO_MEET_SETTLE_SECONDS;
        Some((outgoing.end - overlap - cut - preparation_lead).max(outgoing.start))
    }

    /// Anchors the return to the incoming clock so it can continue after the outgoing stream ends.
    pub fn tempo_return(self) -> Option<TempoReturn> {
        let current = self.outgoing_bpm?;
        let next = self.beat_ratio.comparable_incoming_bpm(self.incoming_bpm?);
        let speed = current / next;
        let overlap = self.outgoing_end - self.outgoing_start;
        if !overlap.is_finite() || overlap <= 0.0 || !is_supported_mix_speed(1.0 / speed) {
            return None;
        }
        let lead = TEMPO_RETURN_LEAD_SECONDS.min(overlap / 2.0);
        if !lead.is_finite() || lead < 0.0 {
            return None;
        }
        let start = self.incoming_position(self.outgoing_end - lead)?;
        // The geometric midpoint splits the proportional tempo difference
        // equally: incoming sqrt(speed), outgoing 1 / sqrt(speed).
        TempoReturn::new(start, speed.sqrt())
    }

    /// Prepares the outgoing midpoint before overlap, then shares the retained return curve.
    pub fn frame(self, position: f64) -> TransitionFrame {
        let progress = ((position - self.outgoing_start)
            / (self.outgoing_end - self.outgoing_start))
            .clamp(0.0, 1.0);
        let mut outgoing_speed = 1.0;
        let mut incoming_speed = 1.0;
        if let (Some(current), Some(next)) = (self.outgoing_bpm, self.incoming_bpm)
            && let Some(curve) = self.tempo_return()
        {
            let speed_ratio = current / self.beat_ratio.comparable_incoming_bpm(next);
            let midpoint_speed = speed_ratio.sqrt();
            if position < self.outgoing_start {
                // Finish before either stream overlaps audibly. Short introductions
                // retain time for both a smooth approach and a stable shared tempo.
                let settling = TEMPO_MEET_SETTLE_SECONDS.min(self.outgoing_start / 2.0);
                let reached = self.outgoing_start - settling;
                let duration = TEMPO_MEET_DURATION_SECONDS.min(reached);
                let start = reached - duration;
                let progress = ((position - start) / duration.max(f64::EPSILON)).clamp(0.0, 1.0);
                let blend = progress * progress * (3.0 - 2.0 * progress);
                outgoing_speed = 1.0 + (1.0 / midpoint_speed - 1.0) * blend;
                incoming_speed = midpoint_speed;
            } else if let Some(incoming_position) = self.incoming_position(position)
                && let Some(frame) = curve.frame(incoming_position)
            {
                incoming_speed = frame.speed;
                outgoing_speed = incoming_speed / speed_ratio;
            }
        }
        TransitionFrame {
            outgoing_gain: 1.0 - progress,
            incoming_gain: progress,
            outgoing_speed,
            incoming_speed,
            complete: progress >= 1.0,
        }
    }
}

/// Targets ten percent of the shorter effective track; short clips retain a four-second floor within the quarter cap.
fn overlap_duration(outgoing: f64, incoming: f64) -> f64 {
    let shorter = outgoing.min(incoming);
    (shorter * 0.1).max(4.0).min(shorter / 4.0)
}

/// Cuts only the joining edge, within one shared budget, even when beats cannot be synchronized.
fn cut_edge(
    mut region: PlaybackRegion,
    report: Option<&TempoReport>,
    opening: bool,
) -> PlaybackRegion {
    let Some(report) = report else {
        return region;
    };
    let limit = MAX_EDGE_SKIP_SECONDS.min(region.duration() * 0.1);
    let latest_start = region.start + limit;
    let earliest_end = region.end - limit;
    if let Some(activity) = report.activity.bounds() {
        if opening {
            region.start = (activity.start - 0.05).clamp(region.start, latest_start);
        } else if report.activity.analyzed_seconds + 0.01 >= region.end {
            // A 64-second prefix says nothing about a longer song's ending.
            region.end = (activity.end + 0.05).clamp(earliest_end, region.end);
        }
    }
    let mut groups = report.segments.iter().flat_map(beat_groups);
    if opening {
        if let Some(beats) = groups.find(|beats| {
            beats.start >= region.start && beats.start <= latest_start && beats.end <= region.end
        }) {
            region.start = beats.start;
        }
    } else if let Some(beats) = groups.rev().find(|beats| {
        beats.start >= region.start && beats.end >= earliest_end && beats.end <= region.end
    }) {
        region.end = (beats.end + 0.05).min(region.end);
    }
    region
}

/// Refuses malformed source regions before any arithmetic or native seek.
fn valid_region(region: PlaybackRegion) -> bool {
    region.start.is_finite()
        && region.end.is_finite()
        && region.start >= 0.0
        && region.end > region.start
}

/// Joins adjacent analysis windows only when their observed attacks form one continuous, stable pulse.
fn joined_segments(report: &TempoReport) -> Vec<BeatSegment> {
    let mut joined: Vec<BeatSegment> = Vec::new();
    for segment in &report.segments {
        if let Some(previous) = joined.last_mut() {
            let trusted = previous.beat_confidence.is_some_and(|value| value >= 0.55)
                && segment.beat_confidence.is_some_and(|value| value >= 0.55);
            let agrees = (segment.bpm / previous.bpm - 1.0).abs() < 0.005;
            let continuous = previous
                .beats
                .last()
                .zip(segment.beats.first())
                .is_some_and(|(last, first)| {
                    let period = 60.0 / previous.bpm;
                    let count = ((first - last) / period).round();
                    (1.0..=4.0).contains(&count) && (first - last - count * period).abs() <= 0.025
                });
            if trusted && agrees && continuous {
                previous.end_seconds = segment.end_seconds;
                previous.beats.extend_from_slice(&segment.beats);
                previous.confidence = previous.confidence.min(segment.confidence);
                previous.beat_confidence = previous
                    .beat_confidence
                    .zip(segment.beat_confidence)
                    .map(|(left, right)| left.min(right));
                continue;
            }
        }
        joined.push(segment.clone());
    }
    joined
}

/// Requires nine consecutive measured attacks before cutting a beat-free joining edge.
fn beat_groups(segment: &BeatSegment) -> impl DoubleEndedIterator<Item = PlaybackRegion> + '_ {
    observed_groups(segment, 8)
}

/// Accepts measured phase across the full span, including gaps of at most four whole beats.
/// No timestamps are invented: both endpoints and at least nine attacks must be observed.
fn observed_groups(
    segment: &BeatSegment,
    intervals: usize,
) -> impl DoubleEndedIterator<Item = PlaybackRegion> + '_ {
    let trusted = segment.bpm.is_finite()
        && (40.0..=240.0).contains(&segment.bpm)
        && segment
            .beat_confidence
            .is_some_and(|confidence| confidence.is_finite() && confidence >= 0.55);
    let period = 60.0 / segment.bpm;
    // Precompute invalid gaps so each candidate is checked without rescanning its entire span.
    let mut invalid_prefix = vec![0_usize; segment.beats.len() + 1];
    for (index, beat) in segment.beats.iter().enumerate() {
        let gap = if index == 0 {
            period
        } else {
            beat - segment.beats[index - 1]
        };
        let count = (gap / period).round();
        let regular = (1.0..=4.0).contains(&count) && (gap - count * period).abs() <= 0.025;
        invalid_prefix[index + 1] =
            invalid_prefix[index] + usize::from(!valid_beat(*beat, segment) || !regular);
    }
    (0..segment.beats.len()).filter_map(move |index| {
        if !trusted || !valid_beat(segment.beats[index], segment) {
            return None;
        }
        let start = segment.beats[index];
        let target = start + intervals as f64 * period;
        let remaining = &segment.beats[index..];
        let end = remaining.partition_point(|beat| *beat < target - 0.025);
        let last = *remaining.get(end)?;
        if end < 8
            || (last - target).abs() > 0.025
            || invalid_prefix[index + end + 1] != invalid_prefix[index + 1]
        {
            return None;
        }
        let beats = &remaining[..=end];
        // Local gaps alone could accumulate drift; every retained attack must share this phase.
        let same_phase = beats.iter().all(|beat| {
            let elapsed = beat - start;
            (elapsed - (elapsed / period).round() * period).abs() <= 0.025
        });
        same_phase.then_some(PlaybackRegion { start, end: last })
    })
}

/// Uses covered local measurements, or a fully analyzed stable track with repeated independent support.
fn supported_tempo(report: &TempoReport, start: f64, end: f64) -> Option<f64> {
    let reliable = |segment: &&BeatSegment| {
        segment.bpm.is_finite()
            && (40.0..=240.0).contains(&segment.bpm)
            && segment.confidence.is_finite()
            && (0.45..=1.0).contains(&segment.confidence)
            && segment.start_seconds.is_finite()
            && segment.end_seconds.is_finite()
            && segment.start_seconds >= 0.0
            && segment.end_seconds > segment.start_seconds
    };
    let local = report
        .segments
        .iter()
        .filter(reliable)
        .find(|segment| segment.start_seconds <= start && segment.end_seconds >= start);
    if let Some(local) = local {
        let mut covered = local.end_seconds;
        for segment in report.segments.iter().filter(reliable) {
            if segment.end_seconds <= covered {
                continue;
            }
            if segment.start_seconds > covered + 0.01
                || (segment.bpm / local.bpm - 1.0).abs() > 0.005
            {
                break;
            }
            covered = segment.end_seconds;
        }
        if covered >= end {
            return Some(local.bpm);
        }
    }
    let bpm = report
        .bpm
        .filter(|bpm| bpm.is_finite() && (40.0..=240.0).contains(bpm))?;
    if report.variable
        || !report.activity.analyzed_seconds.is_finite()
        || report.activity.analyzed_seconds < end
    {
        return None;
    }
    if report
        .segments
        .iter()
        .filter(reliable)
        .any(|segment| (segment.bpm / bpm - 1.0).abs() >= 0.005)
    {
        return None;
    }
    let supported = report
        .segments
        .iter()
        .filter(reliable)
        .filter(|segment| segment.confidence >= 0.55 && (segment.bpm / bpm - 1.0).abs() < 0.005)
        .count();
    (supported >= 2).then_some(bpm)
}

/// Validates one observed timestamp against its measurement interval.
fn valid_beat(beat: f64, segment: &BeatSegment) -> bool {
    beat.is_finite() && beat >= segment.start_seconds && beat <= segment.end_seconds
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MIX cuts both joining edges without TRIM or beat evidence; coverage and the original cut budget stay authoritative.
    #[test]
    fn fallback_cuts_activity_edges_without_requiring_tempo() {
        use crate::analysis::activity::AudioActivity;
        let region = PlaybackRegion {
            start: 0.0,
            end: 100.0,
        };
        let current = TempoReport {
            activity: AudioActivity {
                analyzed_seconds: 100.0,
                first_sound: Some(4.0),
                last_sound: Some(94.0),
            },
            ..Default::default()
        };
        let next = TempoReport {
            activity: AudioActivity {
                analyzed_seconds: 64.0,
                first_sound: Some(5.0),
                last_sound: Some(62.0),
            },
            ..Default::default()
        };
        let plan = TransitionPlan::prepare(region, region, Some(&current), Some(&next)).unwrap();
        assert_eq!(plan.outgoing_end, 94.05);
        assert_eq!(plan.outgoing_start, 84.05);
        assert_eq!(plan.incoming_start, 4.95);
        assert!(plan.outgoing_bpm.is_none());
        let partial = TransitionPlan::prepare(region, region, Some(&next), None).unwrap();
        assert_eq!(
            partial.outgoing_end, 100.0,
            "prefix cannot cut a longer ending"
        );
        assert_eq!(partial.incoming_start, 0.0);
        let short_region = PlaybackRegion {
            start: 0.0,
            end: 30.0,
        };
        let capped =
            TransitionPlan::prepare(short_region, short_region, Some(&next), Some(&next)).unwrap();
        assert_eq!(capped.incoming_start, 3.0, "intro cut cannot exceed 10%");
        let mut excessive = current.clone();
        excessive.activity.last_sound = Some(20.0);
        let capped = TransitionPlan::prepare(region, region, Some(&excessive), None).unwrap();
        assert_eq!(capped.outgoing_end, 90.0);
        excessive.activity.last_sound = Some(f64::NAN);
        let invalid = TransitionPlan::prepare(region, region, Some(&excessive), None).unwrap();
        assert_eq!(invalid.outgoing_end, 100.0);
    }

    /// Four-minute songs overlap for 24 seconds, with monotonic opposite gains and early bounded preparation.
    #[test]
    fn proportional_overlap_keeps_music_and_uses_complete_beat_evidence() {
        let region = PlaybackRegion {
            start: 0.0,
            end: 240.0,
        };
        let fallback = TransitionPlan::prepare(region, region, None, None).unwrap();
        assert_eq!(fallback.outgoing_start, 216.0);
        assert_eq!(fallback.outgoing_end, 240.0);
        assert_eq!(fallback.incoming_start, 0.0);
        assert_eq!(TransitionPlan::preparation_start(region), Some(177.0));

        let mut previous = fallback.frame(fallback.outgoing_start);
        for step in 0..=24 {
            let frame = fallback.frame(216.0 + step as f64);
            assert!(frame.outgoing_gain <= previous.outgoing_gain);
            assert!(frame.incoming_gain >= previous.incoming_gain);
            assert!((frame.outgoing_gain + frame.incoming_gain - 1.0).abs() < 1e-10);
            previous = frame;
        }
        assert_eq!(fallback.frame(228.0).incoming_gain, 0.5);
        let shorter = PlaybackRegion {
            start: 0.0,
            end: 120.0,
        };
        let unequal = TransitionPlan::prepare(region, shorter, None, None).unwrap();
        assert_eq!(unequal.outgoing_start, 228.0);

        let current = report(120.0, 210.0, 57);
        let next = report(124.0, 1.0, 65);
        let synced = TransitionPlan::prepare(region, region, Some(&current), Some(&next)).unwrap();
        assert_eq!(synced.outgoing_end - synced.outgoing_start, 24.0);
        assert_eq!(synced.outgoing_bpm, Some(120.0));
        let mut split = current.clone();
        let later_beats = split.segments[0].beats.split_off(28);
        let boundary = later_beats[0];
        let mut later = split.segments[0].clone();
        later.start_seconds = boundary;
        later.beats = later_beats;
        split.segments[0].end_seconds = boundary;
        split.segments.push(later);
        let joined = TransitionPlan::prepare(region, region, Some(&split), Some(&next)).unwrap();
        assert_eq!(
            joined, synced,
            "analysis window boundaries must not shorten a stable mix"
        );
        let mut sparse = current;
        sparse.segments[0].beats.remove(20);
        let uncertain =
            TransitionPlan::prepare(region, region, Some(&sparse), Some(&next)).unwrap();
        assert_eq!(uncertain.outgoing_end - uncertain.outgoing_start, 24.0);
        assert!(
            uncertain.phase_matched,
            "one missed attack must not discard supported phase or shorten the transition"
        );
        assert!(
            TransitionPlan::preparation_start(PlaybackRegion {
                start: 0.0,
                end: f64::NAN
            })
            .is_none()
        );
    }

    /// Missing phase must not disable supported tempo matching or its smooth shared return ramp.
    #[test]
    fn measured_tempo_without_attacks_still_controls_both_speeds() {
        let region = PlaybackRegion {
            start: 0.0,
            end: 100.0,
        };
        let mut current = report(120.0, 80.0, 41);
        let mut next = report(124.0, 0.0, 61);
        for segment in current.segments.iter_mut().chain(&mut next.segments) {
            segment.beats.clear();
            segment.beat_confidence = None;
        }
        let plan = TransitionPlan::prepare(region, region, Some(&current), Some(&next)).unwrap();
        assert!(!plan.phase_matched);
        assert_eq!(plan.outgoing_bpm, Some(120.0));
        assert_eq!(plan.incoming_bpm, Some(124.0));
        assert!((plan.frame(90.0).incoming_speed - (120.0_f64 / 124.0).sqrt()).abs() < 1e-10);
        for step in 0..=100 {
            let frame = plan.frame(98.0 + step as f64 / 50.0);
            assert!((120.0 * frame.outgoing_speed - 124.0 * frame.incoming_speed).abs() < 1e-10);
        }
        assert_return_finishes_after_handover(plan);
        next.segments[0].bpm = 148.0;
        let incompatible =
            TransitionPlan::prepare(region, region, Some(&current), Some(&next)).unwrap();
        assert_eq!(incompatible.frame(90.0).incoming_speed, 1.0);
        next.segments[0].bpm = f64::NAN;
        assert!(
            TransitionPlan::prepare(region, region, Some(&current), Some(&next))
                .unwrap()
                .incoming_bpm
                .is_none()
        );
    }

    /// Sparse measured attacks can retain phase; off-grid attacks and extended gaps cannot authorize SYNC.
    #[test]
    fn sparse_phase_requires_consistent_observed_endpoints() {
        let region = PlaybackRegion {
            start: 0.0,
            end: 240.0,
        };
        let mut current = report(120.0, 200.0, 80);
        let next = report(124.0, 0.0, 80);
        current.segments[0].beats = current.segments[0]
            .beats
            .iter()
            .copied()
            .enumerate()
            .filter(|(index, _)| index % 7 != 3)
            .map(|(_, beat)| beat)
            .collect();
        let plan = TransitionPlan::prepare(region, region, Some(&current), Some(&next)).unwrap();
        assert!(plan.phase_matched);
        assert_eq!(plan.outgoing_end - plan.outgoing_start, 24.0);
        for (index, beat) in current.segments[0].beats.iter_mut().enumerate() {
            if index % 5 == 0 {
                *beat += 0.12;
            }
        }
        let irregular =
            TransitionPlan::prepare(region, region, Some(&current), Some(&next)).unwrap();
        assert!(!irregular.phase_matched);
        // A stable BPM can remain useful independently of attack phase.
        current.segments[0].end_seconds = 240.0;
        let tempo = TransitionPlan::prepare(region, region, Some(&current), Some(&next)).unwrap();
        assert_eq!(tempo.outgoing_bpm, Some(120.0));
        assert!(!tempo.phase_matched);
        current.segments[0].beats = vec![200.0, 239.5];
        assert!(
            !TransitionPlan::prepare(region, region, Some(&current), Some(&next))
                .unwrap()
                .phase_matched
        );
    }

    /// Global BPM requires repeated support and complete coverage; variable gaps and conflicting local tempos stay unknown.
    #[test]
    fn stable_context_is_bounded_and_variable_overlap_is_not_averaged() {
        let mut current = report(120.0, 0.0, 32);
        current.segments.extend(report(120.0, 32.0, 32).segments);
        current.activity.analyzed_seconds = 100.0;
        assert_eq!(supported_tempo(&current, 90.0, 100.0), Some(120.0));
        current.activity.analyzed_seconds = 64.0;
        assert_eq!(supported_tempo(&current, 90.0, 100.0), None);
        current.activity.analyzed_seconds = 100.0;
        current.variable = true;
        assert_eq!(supported_tempo(&current, 90.0, 100.0), None);
        current.segments[0].end_seconds = 32.0;
        current.segments[1].bpm = 130.0;
        assert_eq!(supported_tempo(&current, 20.0, 40.0), None);
    }

    /// Builds measured fixture attacks with an explicit bounded observation interval.
    fn report(bpm: f64, start: f64, count: usize) -> TempoReport {
        let beats: Vec<_> = (0..count)
            .map(|index| start + index as f64 * 60.0 / bpm)
            .collect();
        TempoReport {
            activity: Default::default(),
            bpm: Some(bpm),
            variable: false,
            segments: vec![BeatSegment {
                start_seconds: start,
                end_seconds: *beats.last().unwrap(),
                bpm,
                beats,
                confidence: 0.9,
                beat_confidence: Some(0.9),
                subdivision_ambiguous: false,
            }],
        }
    }

    /// Edge skipping is bounded and the return ramp preserves equal BPM while restoring incoming speed.
    #[test]
    fn mix_limits_and_final_ramp() {
        let region = PlaybackRegion {
            start: 0.0,
            end: 100.0,
        };
        let plan = TransitionPlan::prepare(
            region,
            region,
            Some(&report(120.0, 86.0, 21)),
            Some(&report(124.0, 3.0, 30)),
        )
        .unwrap();
        assert_eq!(plan.outgoing_end, 96.0);
        assert_eq!(plan.incoming_start, 3.0);
        for step in 0..=100 {
            let position = plan.outgoing_start
                + (plan.outgoing_end - plan.outgoing_start) * step as f64 / 100.0;
            let frame = plan.frame(position);
            assert!((frame.outgoing_gain + frame.incoming_gain - 1.0).abs() < 1e-10);
            assert!((120.0 * frame.outgoing_speed - 124.0 * frame.incoming_speed).abs() < 1e-10);
        }
        assert_return_finishes_after_handover(plan);
        assert!(plan.frame(plan.outgoing_end).complete);
        let distant = TransitionPlan::prepare(
            region,
            region,
            Some(&report(120.0, 70.0, 13)),
            Some(&report(124.0, 3.0, 30)),
        )
        .unwrap();
        assert!(distant.outgoing_bpm.is_none());
        assert_eq!(distant.incoming_start, 3.0);
        assert_eq!(distant.outgoing_end, 100.0);
    }

    /// Handover preserves the adjusted speed; the retained incoming-clock curve eventually reaches unity.
    fn assert_return_finishes_after_handover(plan: TransitionPlan) {
        let Some(curve) = plan.tempo_return() else {
            assert_eq!(plan.frame(plan.outgoing_end).incoming_speed, 1.0);
            return;
        };
        let position = plan.incoming_position(plan.outgoing_end).unwrap();
        let at_handover = curve.frame(position).unwrap();
        assert!(!at_handover.complete);
        assert_eq!(
            at_handover.speed,
            plan.frame(plan.outgoing_end).incoming_speed
        );
        assert_eq!(curve.frame(curve.end_position()).unwrap().speed, 1.0);
        assert!(curve.frame(curve.end_position()).unwrap().complete);
    }

    /// Large adjustments return over several seconds with bounded acceleration and gentle endpoints.
    #[test]
    fn tempo_return_is_gradual_in_both_directions() {
        for (current, next) in [(100.0, 120.0), (120.0, 100.0)] {
            let plan = TransitionPlan {
                outgoing_start: 0.0,
                outgoing_end: 24.0,
                incoming_start: 0.0,
                outgoing_bpm: Some(current),
                incoming_bpm: Some(next),
                phase_matched: true,
                beat_ratio: BeatRatio::OneToOne,
            };
            let initial_speed: f64 = current / next;
            let midpoint_speed = initial_speed.sqrt();
            assert_eq!(plan.frame(0.0).incoming_speed, midpoint_speed);
            for position in [6.0, 12.0, 23.0] {
                let frame = plan.frame(position);
                assert_eq!(frame.incoming_speed, midpoint_speed);
                assert!((frame.outgoing_speed - 1.0 / midpoint_speed).abs() < 1e-10);
                assert!((current * frame.outgoing_speed - (current * next).sqrt()).abs() < 1e-10);
            }
            assert!((plan.frame(23.9).incoming_speed - midpoint_speed).abs() > 0.001);
            assert!((plan.frame(24.0).incoming_speed - midpoint_speed).abs() < 0.015);

            // Integrate the source clock as the native scheduler does, using 10 ms wall steps.
            let mut position = 0.0;
            let mut previous = plan.frame(position);
            while position < plan.outgoing_end {
                position += previous.outgoing_speed * 0.01;
                let frame = plan.frame(position);
                assert!((frame.incoming_speed - previous.incoming_speed).abs() < 0.001);
                assert!((frame.outgoing_speed - previous.outgoing_speed).abs() < 0.001);
                assert!(
                    (frame.incoming_speed - 1.0).abs() <= (previous.incoming_speed - 1.0).abs()
                );
                assert!(
                    (current * frame.outgoing_speed - next * frame.incoming_speed).abs() < 1e-10
                );
                previous = frame;
            }
            assert_ne!(previous.incoming_speed, 1.0);
            assert!(previous.complete);
            assert_return_finishes_after_handover(plan);
        }
    }

    /// The outgoing approach is gradual and silent on the incoming side, even near the file start.
    #[test]
    fn midpoint_is_ready_before_audible_overlap() {
        for start in [0.5, 8.0, 30.0] {
            for (current, next) in [(100.0, 120.0), (120.0, 100.0)] {
                let plan = TransitionPlan {
                    outgoing_start: start,
                    outgoing_end: start + 12.0,
                    incoming_start: 0.0,
                    outgoing_bpm: Some(current),
                    incoming_bpm: Some(next),
                    phase_matched: true,
                    beat_ratio: BeatRatio::OneToOne,
                };
                let settling = 1.0_f64.min(start / 2.0);
                let reached = start - settling;
                let duration = TEMPO_MEET_DURATION_SECONDS.min(reached);
                let beginning = reached - duration;
                let midpoint = (current / next).sqrt();
                assert_eq!(plan.frame(beginning).outgoing_speed, 1.0);
                let mut previous = 1.0;
                for step in 0..=100 {
                    let position = beginning + duration * step as f64 / 100.0;
                    let frame = plan.frame(position);
                    assert_eq!(frame.incoming_gain, 0.0);
                    assert_eq!(frame.outgoing_gain, 1.0);
                    assert_eq!(frame.incoming_speed, midpoint);
                    assert!((frame.outgoing_speed - previous).abs() < 0.002);
                    assert!((frame.outgoing_speed - 1.0).abs() >= (previous - 1.0).abs());
                    previous = frame.outgoing_speed;
                }
                assert!((previous - 1.0 / midpoint).abs() < 1e-10);
                // Already-buffered outgoing samples must also have the shared BPM
                // when the incoming gain first rises, in either tempo direction.
                for delay in [0.0, settling / 2.0, settling] {
                    let outgoing = plan.frame(start - delay);
                    let incoming = plan.frame(start);
                    assert!(
                        (current * outgoing.outgoing_speed - next * incoming.incoming_speed).abs()
                            < 1e-10,
                        "both tempos must settle before overlap, including {delay}s of output delay"
                    );
                }
                assert_eq!(plan.frame(start + 3.0).incoming_speed, midpoint);
            }
        }
    }

    /// Short overlaps cap the lead at half their span without shortening the full return duration.
    #[test]
    fn short_overlap_limits_the_return_lead() {
        for duration in [0.5, 4.0, 10.0] {
            for (current, next) in [(100.0, 120.0), (120.0, 100.0)] {
                let plan = TransitionPlan {
                    outgoing_start: 10.0,
                    outgoing_end: 10.0 + duration,
                    incoming_start: 0.0,
                    outgoing_bpm: Some(current),
                    incoming_bpm: Some(next),
                    phase_matched: false,
                    beat_ratio: BeatRatio::OneToOne,
                };
                assert_eq!(
                    plan.frame(10.0 + duration / 2.0).incoming_speed,
                    (current / next).sqrt()
                );
                assert_ne!(
                    plan.frame(
                        plan.outgoing_end - TEMPO_RETURN_LEAD_SECONDS.min(duration / 2.0) / 2.0
                    )
                    .incoming_speed,
                    (current / next).sqrt()
                );
                assert_return_finishes_after_handover(plan);
            }
        }
    }

    /// Both directions accept the 20% boundary, including the final outgoing ramp, but reject larger or invalid rates.
    #[test]
    fn twenty_percent_limit_applies_to_phase_and_tempo_plans() {
        for speed in [0.8, 1.0, 1.2] {
            assert!(is_supported_mix_speed(speed));
        }
        for speed in [0.7999, 1.2001, 0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(!is_supported_mix_speed(speed));
        }
        let region = PlaybackRegion {
            start: 0.0,
            end: 100.0,
        };
        for (outgoing, incoming, accepted) in [
            (100.0, 120.0, true),
            (120.0, 100.0, true),
            (100.0, 120.01, false),
            (120.01, 100.0, false),
            (100.0, 124.0, false),
        ] {
            let mut current = report(outgoing, 0.0, 250);
            let mut next = report(incoming, 0.0, 250);
            for phase in [true, false] {
                if !phase {
                    current.segments[0].beat_confidence = None;
                    next.segments[0].beat_confidence = None;
                }
                let plan =
                    TransitionPlan::prepare(region, region, Some(&current), Some(&next)).unwrap();
                assert_eq!(
                    plan.outgoing_bpm.is_some(),
                    accepted,
                    "{outgoing} -> {incoming}, phase={phase}"
                );
                assert_eq!(plan.phase_matched, phase && accepted);
                for step in 0..=100 {
                    let position = plan.outgoing_start
                        + (plan.outgoing_end - plan.outgoing_start) * step as f64 / 100.0;
                    let frame = plan.frame(position);
                    assert!(is_supported_mix_speed(frame.outgoing_speed));
                    assert!(is_supported_mix_speed(frame.incoming_speed));
                    if accepted {
                        assert!(
                            (outgoing * frame.outgoing_speed - incoming * frame.incoming_speed)
                                .abs()
                                < 1e-10
                        );
                    }
                }
                assert_return_finishes_after_handover(plan);
            }
        }
    }

    /// Ratio plans align whole observed cycles and preserve their clock mapping throughout the shared return ramp.
    #[test]
    fn shared_cycles_preserve_rates_phase_and_original_bpm() {
        let region = PlaybackRegion {
            start: 0.0,
            end: 240.0,
        };
        for (outgoing, incoming) in [
            (120.0, 220.0),
            (220.0, 120.0),
            (80.0, 224.0),
            (224.0, 80.0),
            (120.0, 174.0),
            (174.0, 120.0),
        ] {
            let mut current = report(outgoing, 0.0, (240.0 * outgoing / 60.0) as usize + 1);
            let mut next = report(incoming, 0.0, (240.0 * incoming / 60.0) as usize + 1);
            current.segments[0].end_seconds = 240.0;
            next.segments[0].end_seconds = 240.0;
            for phase in [true, false] {
                if !phase {
                    current.segments[0].beats.clear();
                    current.segments[0].beat_confidence = None;
                    next.segments[0].beats.clear();
                    next.segments[0].beat_confidence = None;
                }
                let plan =
                    TransitionPlan::prepare(region, region, Some(&current), Some(&next)).unwrap();
                let ratio = BeatRatio::choose(outgoing, incoming).unwrap();
                let (outgoing_count, incoming_count) = ratio.counts();
                assert_eq!(plan.beat_ratio, ratio);
                assert_eq!(plan.phase_matched, phase);
                assert_eq!(plan.outgoing_bpm, Some(outgoing));
                assert_eq!(plan.incoming_bpm, Some(incoming));
                let expected_speed =
                    outgoing * incoming_count as f64 / (incoming * outgoing_count as f64);
                assert!(
                    (plan.frame(plan.outgoing_start).incoming_speed - expected_speed.sqrt()).abs()
                        < 1e-10
                );
                assert_eq!(plan.incoming_position(f64::NAN), None);
                for step in 0..=100 {
                    let position = plan.outgoing_start
                        + (plan.outgoing_end - plan.outgoing_start) * step as f64 / 100.0;
                    let frame = plan.frame(position);
                    assert!(is_supported_mix_speed(frame.outgoing_speed));
                    assert!(is_supported_mix_speed(frame.incoming_speed));
                    let outgoing_cycles = outgoing * frame.outgoing_speed / outgoing_count as f64;
                    let incoming_cycles = incoming * frame.incoming_speed / incoming_count as f64;
                    assert!((outgoing_cycles - incoming_cycles).abs() < 1e-10);
                    let expected_position =
                        plan.incoming_start + (position - plan.outgoing_start) * expected_speed;
                    assert!(
                        (plan.incoming_position(position).unwrap() - expected_position).abs()
                            < 1e-10
                    );
                }
                assert_return_finishes_after_handover(plan);
                if phase {
                    let cycles = (plan.outgoing_end - plan.outgoing_start) * outgoing
                        / (60.0 * outgoing_count as f64);
                    assert!((cycles - cycles.round()).abs() < 1e-10);
                    let incoming_end = plan.incoming_position(plan.outgoing_end).unwrap();
                    assert!(
                        next.segments[0]
                            .beats
                            .iter()
                            .any(|beat| (beat - incoming_end).abs() < 0.025)
                    );
                }
            }
        }
    }

    /// Malformed giant extents cannot overflow cycle counts or allocate work proportional to duration.
    #[test]
    fn excessive_cycle_counts_do_not_overflow() {
        let region = PlaybackRegion {
            start: 0.0,
            end: 1e100,
        };
        for (outgoing, incoming) in [(80.0, 224.0), (224.0, 80.0)] {
            let mut current = report(outgoing, 0.0, 16);
            let mut next = report(incoming, 0.0, 16);
            current.segments[0].end_seconds = region.end;
            next.segments[0].end_seconds = region.end;
            let plan =
                TransitionPlan::prepare(region, region, Some(&current), Some(&next)).unwrap();
            assert!(!plan.phase_matched);
            assert!(plan.incoming_position(f64::INFINITY).is_none());
        }
    }

    /// Variable songs mix with the measured closing/opening tempos, never with a global average or opposite edge.
    #[test]
    fn variable_tracks_use_local_boundary_tempos() {
        let region = PlaybackRegion {
            start: 0.0,
            end: 100.0,
        };
        let mut current = report(90.0, 0.0, 30);
        current.segments.extend(report(120.0, 86.0, 21).segments);
        current.variable = true;
        current.bpm = None;
        let mut next = report(124.0, 3.0, 30);
        next.segments.extend(report(180.0, 90.0, 28).segments);
        next.variable = true;
        next.bpm = None;
        let plan = TransitionPlan::prepare(region, region, Some(&current), Some(&next)).unwrap();
        assert_eq!(plan.outgoing_bpm, Some(120.0));
        assert_eq!(plan.incoming_bpm, Some(124.0));
        assert_eq!(plan.incoming_start, 3.0);
        assert_eq!(plan.outgoing_start, 86.0);
        let frame = plan.frame(plan.outgoing_end - 1.0);
        assert!((120.0 * frame.outgoing_speed - 124.0 * frame.incoming_speed).abs() < 1e-10);
        assert_return_finishes_after_handover(plan);
    }

    /// Uncertain alignment retains crossfade; only independently supported edges may be cut.
    #[test]
    fn uncertain_alignment_uses_only_supported_edge_cuts() {
        let region = PlaybackRegion {
            start: 0.0,
            end: 100.0,
        };
        let mut current = report(120.0, 90.0, 9);
        current.segments[0].beats.remove(4);
        for next in [report(120.0, 2.0, 30), report(174.0, 2.0, 30)] {
            let plan =
                TransitionPlan::prepare(region, region, Some(&current), Some(&next)).unwrap();
            assert!(plan.outgoing_bpm.is_none());
            assert_eq!(plan.incoming_start, 2.0);
        }
        assert!(
            TransitionPlan::prepare(
                PlaybackRegion {
                    start: 0.0,
                    end: f64::NAN
                },
                region,
                None,
                None
            )
            .is_none()
        );
        assert!(
            TransitionPlan::prepare(
                PlaybackRegion {
                    start: 0.0,
                    end: 1.0
                },
                region,
                None,
                None
            )
            .is_none()
        );
    }
}
