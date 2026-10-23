//! Shared source-duration presentation for Library rows and its unfiltered total.

use resolibre_audio::session::TrackLabel;

/// Formats known whole seconds; unavailable metadata is never presented as a zero-length track.
pub fn track(seconds: Option<u32>) -> String {
    match seconds {
        Some(seconds) => clock(u64::from(seconds)),
        None => "--:--".into(),
    }
}

/// Sums every occurrence in the bounded queue, marking incomplete totals with a trailing plus.
pub fn total(tracks: &[TrackLabel]) -> String {
    let mut seconds = 0_u64;
    let mut known = 0;
    for track in tracks {
        if let Some(duration) = track.duration_seconds {
            // At most 1,024 u32 durations; u64 also preserves totals beyond 24 hours.
            seconds += u64::from(duration);
            known += 1;
        }
    }
    if !tracks.is_empty() && known == 0 {
        return "--:--".into();
    }
    let mut label = clock(seconds);
    if known < tracks.len() {
        label.push('+');
    }
    label
}

/// Shows hours only from one hour onward, without wrapping after one day.
fn clock(seconds: u64) -> String {
    if seconds < 3600 {
        return format!("{:02}:{:02}", seconds / 60, seconds % 60);
    }

    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates independent occurrences with only duration metadata populated.
    fn labels(durations: &[Option<u32>]) -> Vec<TrackLabel> {
        durations
            .iter()
            .enumerate()
            .map(|(id, duration)| TrackLabel {
                id: id as u64,
                title: String::new(),
                search_text: String::new(),
                duration_seconds: *duration,
            })
            .collect()
    }

    /// Unknown, zero, long, duplicate and incomplete durations retain honest totals.
    #[test]
    fn durations_and_totals() {
        assert_eq!(track(None), "--:--");
        assert_eq!(track(Some(0)), "00:00");
        assert_eq!(track(Some(3599)), "59:59");
        assert_eq!(track(Some(3600)), "01:00:00");
        assert_eq!(track(Some(3661)), "01:01:01");
        assert_eq!(total(&[]), "00:00");
        assert_eq!(total(&labels(&[None, None])), "--:--");
        assert_eq!(total(&labels(&[Some(61), None])), "01:01+");
        assert_eq!(total(&labels(&[Some(61), Some(61)])), "02:02");
        assert_eq!(total(&labels(&[Some(1800), Some(1800)])), "01:00:00");
        assert_eq!(total(&labels(&[Some(90061)])), "25:01:01");
        assert_eq!(
            total(&labels(&[Some(u32::MAX); 1024])),
            clock(u64::from(u32::MAX) * 1024)
        );
    }
}
