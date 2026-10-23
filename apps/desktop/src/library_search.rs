//! Bounded Library filtering; presentation only, never edits the session queue.

use crate::{PlaylistWindow, TrackRow};
use slint::{Model, ModelRc, VecModel};

/// Connects the same filter and stable-ID lookup to every Library host.
pub fn connect(window: &PlaylistWindow) {
    window.on_analysis_state(|states, id| {
        states
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.status)
            .unwrap_or(0)
    });
    window.on_limit_query(limit_query);
    window.on_filter_tracks(filter_tracks);
    window.on_find_track(|tracks, id| {
        tracks
            .iter()
            .position(|track| track.id == id)
            .map(|index| index as i32)
            .unwrap_or(-1)
    });
}

/// Bounds typed or pasted text without splitting a Unicode character.
fn limit_query(query: slint::SharedString) -> slint::SharedString {
    query.chars().take(256).collect::<String>().into()
}

/// Matches every whitespace-separated term as a literal, case-insensitive substring across allowed fields.
fn filter_tracks(tracks: ModelRc<TrackRow>, query: slint::SharedString) -> ModelRc<TrackRow> {
    let query = limit_query(query).to_lowercase();
    let terms: Vec<_> = query.split_whitespace().collect();
    if terms.is_empty() {
        return tracks;
    }
    let rows: Vec<_> = tracks
        .iter()
        .filter(|track| {
            let title = track.title.to_lowercase();
            terms
                .iter()
                .all(|term| track.search_text.contains(term) || title.contains(term))
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

#[cfg(test)]
mod tests;
