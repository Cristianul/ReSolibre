use super::*;
use resolibre_core::analysis::TrackAnalyzer;
use resolibre_storage::audio_identity::AudioIdentity;
use std::time::{Duration, Instant};

/// Produces a valid full-sized matrix with small musical/technical facts and no private media.
fn report() -> Arc<TrackAnalysis> {
    let mut analyzer = TrackAnalyzer::new(8000, 1).unwrap();
    analyzer.push(&[0.0; 8000]).unwrap();
    let mut report = analyzer.finish().unwrap();
    report.columns = resolibre_core::analysis::MAX_COLUMNS;
    report.power_db = vec![-37.0; report.columns * resolibre_core::analysis::ROWS];
    report.seconds_per_column = report.duration_seconds / report.columns as f64;
    Arc::new(report)
}

/// Matrix eviction releases its allocation, preserves all other measurements and respects view recency.
#[test]
fn five_plots_do_not_evict_small_measurements() {
    let mut store = Reports::default();
    let first = report();
    let weak = Arc::downgrade(&first);
    let expected = TrackAnalysisSummary::from(first.as_ref());
    store.remember(0, first, Some([42; 32]));
    let other = report();
    for id in 1..5 {
        store.remember(id, Arc::clone(&other), None);
    }
    assert!(store.plot(0).is_some());
    store.remember(5, Arc::clone(&other), None);
    assert!(store.plot(1).is_none(), "opening a plot updates recency");
    for id in 6..10_000 {
        store.remember(id, Arc::clone(&other), None);
    }
    assert_eq!(store.plots.len(), 5);
    assert_eq!(store.entries.len(), 10_000);
    assert!(
        weak.upgrade().is_none(),
        "small facts must not keep the matrix alive"
    );
    assert_eq!(store.entries[&0].summary, expected);
    store.remove(0);
    assert!(!store.request(0));
    store.retain(&BTreeSet::from([9999]));
    assert_eq!(store.entries.len(), 1);
    assert!(Reports::default().plot(9999).is_none());
}

/// Polls only the cache reader; no decoder or original-file path exists in this workflow.
fn read_plot(store: &mut Reports, config: &Arc<CacheConfig>) -> Option<Arc<TrackAnalysis>> {
    let started = Instant::now();
    loop {
        if let Some((_, report)) = store.poll(Some(config)) {
            assert!(started.elapsed() < Duration::from_millis(500));
            eprintln!(
                "evicted matrix loaded in {:.3} s",
                started.elapsed().as_secs_f64()
            );
            return report;
        }
        assert!(started.elapsed() < Duration::from_secs(2));
        thread::sleep(Duration::from_millis(1));
    }
}

/// Historical reload is fast, profile-scoped, validated and cannot silently replace remembered facts.
#[test]
fn evicted_plot_reloads_without_original_audio_and_rejects_wrong_data() {
    let root = tempfile::tempdir().unwrap();
    let config = Arc::new(CacheConfig::new(root.path().join("cache")));
    let other_profile = Arc::new(CacheConfig::new(root.path().join("other")));
    let identity = AudioIdentity {
        digest: [42; 32],
        artist: "Artist".into(),
        title: "Title".into(),
    };
    let original = report();
    config
        .database()
        .unwrap()
        .save_track(&identity, &original)
        .unwrap();
    let mut store = Reports::default();
    store.remember(0, Arc::clone(&original), Some(identity.digest));
    for id in 1..6 {
        store.remember(id, report(), None);
    }
    assert!(store.plot(0).is_none());
    assert!(store.request(0));
    let loaded = read_plot(&mut store, &config).unwrap();
    assert_eq!(loaded.power_db, original.power_db);
    assert_eq!(
        store.entries[&0].summary,
        TrackAnalysisSummary::from(original.as_ref())
    );
    assert!(store.request(0));
    assert!(read_plot(&mut store, &other_profile).is_none());
    let mut different = (*original).clone();
    different.sample_rate_hz = 16000;
    config
        .database()
        .unwrap()
        .save_track(&identity, &different)
        .unwrap();
    assert!(store.request(0));
    assert!(read_plot(&mut store, &config).is_none());
    assert_eq!(store.entries[&0].summary.sample_rate_hz, 8000);
}

/// Forced refresh and closing the view reject an already-running lookup, even for the same occurrence.
#[test]
fn stale_disk_completion_never_revives_old_analysis() {
    for close in [false, true] {
        let mut store = Reports::default();
        let original = report();
        store.remember(0, Arc::clone(&original), Some([42; 32]));
        assert!(store.request(0));
        let ticket = store.requested.unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        store.lookup = Some(Lookup {
            ticket,
            thread: thread::spawn(move || receiver.recv().unwrap()),
        });
        if close {
            store.cancel_view();
        } else {
            store.remove(0);
            store.remember(0, report(), Some([42; 32]));
        }
        sender.send(Some((*original).clone())).unwrap();
        while !store.lookup.as_ref().unwrap().thread.is_finished() {
            thread::yield_now();
        }
        assert!(store.poll(None).is_none());
        assert!(store.lookup.is_none());
        assert!(store.requested.is_none());
    }
}

/// Measures full native analysis followed by matrix eviction/reload for one explicitly authorized recording.
#[test]
#[ignore = "requires RESOUL_ANALYSIS_PROBE pointing to an authorized recording"]
fn authorized_recording_reloads_evicted_plot_within_half_a_second() {
    use crate::track_analysis::{TrackInput, TrackJob};
    let path = std::env::var_os("RESOUL_ANALYSIS_PROBE").expect("authorized input required");
    let root = tempfile::tempdir().unwrap();
    let config = Arc::new(CacheConfig::new(root.path().join("cache")));
    let mut job = TrackJob::start_input(
        TrackInput::Path(path.into()),
        Some(Arc::clone(&config)),
        false,
    )
    .unwrap();
    let started = Instant::now();
    let original = loop {
        if let Some(result) = job.poll() {
            break Arc::new(result.unwrap());
        }
        assert!(started.elapsed() < Duration::from_secs(90));
        thread::sleep(Duration::from_millis(5));
    };
    let digest = job
        .digest()
        .expect("authorized recording must support persistence");
    let mut store = Reports::default();
    store.remember(0, Arc::clone(&original), Some(digest));
    for id in 1..6 {
        store.remember(id, report(), None);
    }
    assert!(store.plot(0).is_none());
    assert!(store.request(0));
    let loaded = read_plot(&mut store, &config).unwrap();
    assert_eq!(loaded.power_db, original.power_db);
    assert_eq!(loaded.sample_rate_hz, original.sample_rate_hz);
}
