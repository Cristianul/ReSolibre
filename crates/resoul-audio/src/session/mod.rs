//! Bounded native session worker. No native work runs while mailbox locks are held.

mod analysis;
mod rhythm;
pub use analysis::{AnalysisStatus, LibraryAnalysisStatus};
pub use rhythm::{PreparedRhythm, RhythmStatus};

mod edges;
mod metadata;
mod saved;
mod worker;

pub use saved::SavedSession;

use resolibre_core::equalizer::{EqAction, Equalizer};
use resolibre_core::playback_edges::{EdgeAction, PlaybackEdges};
use resolibre_core::queue::{MAX_QUEUE_ITEMS, QueueAction, RepeatMode};
use resolibre_core::session::{ScanDirection, SessionState, Volume};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use crate::AudioOutput;

const MAX_COMMANDS: usize = 32;
pub const MAX_SELECTION_BYTES: usize = 1024 * 1024;

/// UI-visible occurrence information; never exposes the original source path.
#[derive(Clone, Debug)]
pub struct TrackLabel {
    pub id: u64,
    pub title: String,
    pub search_text: String,
    pub duration_seconds: Option<u32>,
}

/// Whether original-audio boundaries are available for the current playback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeStatus {
    Off,
    Pending,
    Ready,
    Unavailable,
}

/// Honest transition lifecycle; Preparing does not claim an audible or beat-aligned overlap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MixStatus {
    #[default]
    Off,
    Preparing,
    Ready,
    ReadyCrossfade,
    Crossfading,
    TempoMixing,
    BeatMixing,
    Unavailable,
}

/// Latest session observation, replacing rather than accumulating old events.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub saved: SavedSession,
    pub edges: PlaybackEdges,
    pub mix_enabled: bool,
    pub mix_status: MixStatus,
    pub mix_beat_ratio: Option<resolibre_core::transition::BeatRatio>,
    /// Prepared overlap bounds in original source seconds; absent until ready.
    pub mix_region: Option<resolibre_core::playback_edges::PlaybackRegion>,
    pub edge_status: EdgeStatus,
    pub trim_start: f64,
    pub trim_end: Option<f64>,
    pub revision: u64,
    pub generation: u64,
    pub queue_revision: u64,
    pub tracks: Arc<[TrackLabel]>,
    pub cover: Option<Arc<crate::metadata::Cover>>,
    pub cover_revision: u64,
    pub cover_pending: bool,
    pub current: Option<usize>,
    pub state: SessionState,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub seekable: bool,
    pub analysis: AnalysisStatus,
    pub library_analysis: Arc<std::collections::BTreeMap<u64, LibraryAnalysisStatus>>,
    pub analysis_pending: usize,
    pub rhythm: RhythmStatus,
    pub next_rhythm: Option<PreparedRhythm>,
    pub stream_info: Option<resolibre_core::track_info::StreamInfo>,
    pub live_spectrum: Option<resolibre_core::spectrum::live::LiveLevels>,
    pub scanning: Option<ScanDirection>,
    pub volume: Volume,
    pub equalizer: Equalizer,
    pub eq_sample_rate: u32,
    pub error: Option<String>,
    pub finished: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub can_next: bool,
    pub can_previous: bool,
    /// Back will navigate to an available predecessor rather than restart this track.
    pub previous_navigates: bool,
}

impl Default for Snapshot {
    /// Describes a fresh, silent session without a selected track.
    fn default() -> Self {
        Self {
            saved: SavedSession::default(),
            edges: PlaybackEdges::default(),
            mix_enabled: false,
            mix_status: MixStatus::Off,
            mix_region: None,
            mix_beat_ratio: None,
            edge_status: EdgeStatus::Off,
            trim_start: 0.0,
            trim_end: None,
            revision: 0,
            generation: 0,
            queue_revision: 0,
            tracks: Arc::from([]),
            cover: None,
            cover_revision: 0,
            cover_pending: false,
            current: None,
            state: SessionState::Idle,
            position: None,
            duration: None,
            seekable: false,
            analysis: AnalysisStatus::Idle,
            library_analysis: Arc::default(),
            analysis_pending: 0,
            rhythm: RhythmStatus::Detecting,
            next_rhythm: None,
            stream_info: None,
            live_spectrum: None,
            scanning: None,
            volume: Volume::default(),
            equalizer: Equalizer::default(),
            eq_sample_rate: 0,
            error: None,
            finished: false,
            shuffle: false,
            repeat: RepeatMode::Off,
            can_next: false,
            can_previous: false,
            previous_navigates: false,
        }
    }
}

/// Typed controls; seek generations prevent dragging an old track's slider into a new one.
pub enum Command {
    Restore(SavedSession),
    Open(Vec<PathBuf>),
    Add(Vec<PathBuf>),
    Replace(Vec<PathBuf>),
    Queue(QueueAction),
    Analyze {
        generation: u64,
    },
    ViewAnalysis {
        generation: u64,
    },
    AnalyzeTrack {
        id: u64,
    },
    AnalyzeAll,
    CancelAllAnalysis,
    CancelAnalysis {
        generation: u64,
    },
    TogglePause,
    Stop,
    Next,
    Previous,
    Seek {
        generation: u64,
        seconds: f64,
    },
    Scan {
        generation: u64,
        direction: Option<ScanDirection>,
    },
    Volume(Volume),
    GlideVolume(Volume),
    Equalizer(EqAction),
    Edges(EdgeAction),
    SetMix(bool),
}

struct Request {
    revision: u64,
    command: Command,
}

#[derive(Default)]
struct Mailbox {
    requests: VecDeque<Request>,
    next_revision: u64,
    stop_barrier: u64,
    shutdown: bool,
}

struct Shared {
    mailbox: Mutex<Mailbox>,
    wake: Condvar,
    snapshot: Mutex<Snapshot>,
}

/// Owns the worker lifetime; explicit close remains nonblocking until it has finished.
pub struct Session {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl Session {
    /// Starts one worker; it constructs and destroys all native handles on that thread.
    pub fn start(output: AudioOutput) -> std::io::Result<Self> {
        Self::start_with_analysis_cache(output, None)
    }

    /// Starts playback with an optional platform-supplied private analysis-cache directory.
    /// Storage and hashing remain in background jobs; failures do not disable playback.
    pub fn start_with_analysis_cache(
        output: AudioOutput,
        directory: Option<PathBuf>,
    ) -> std::io::Result<Self> {
        let cache =
            directory.map(|directory| Arc::new(crate::analysis_cache::CacheConfig::new(directory)));
        let shared = Arc::new(Shared {
            mailbox: Mutex::new(Mailbox::default()),
            wake: Condvar::new(),
            snapshot: Mutex::new(Snapshot::default()),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("resolibre-playback".into())
            .spawn(move || {
                worker::run(worker_shared, output, cache);
            })?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }

    /// Submits without native I/O. Stop clears pending work and always has capacity.
    /// Rejected batches/queue floods leave the previous session intent unchanged.
    pub fn submit(&self, command: Command) -> Result<u64, &'static str> {
        if let Command::Restore(saved) = &command {
            saved.validate()?;
        }
        if let Command::Equalizer(action) = &command {
            action.validate()?;
        }
        if let Command::Open(paths) | Command::Add(paths) | Command::Replace(paths) = &command {
            validate_selection(paths)?;
        }
        if let Command::Seek { seconds, .. } = &command
            && (!seconds.is_finite() || *seconds < 0.0)
        {
            return Err("Invalid seek position");
        }
        let observed_generation = self.snapshot().generation;
        let mut mailbox = self
            .shared
            .mailbox
            .lock()
            .map_err(|_| "Session unavailable")?;
        if mailbox.shutdown {
            return Err("Session is closing");
        }

        if matches!(command, Command::Restore(_)) && mailbox.next_revision != 0 {
            return Err("Session restoration is only allowed at startup");
        }

        // Coalescing is limited to an adjacent request of the same kind. Never move
        // a slider request across a track change or other ordered command.
        let coalesce = mailbox.requests.back().is_some_and(|request| {
            if let (Command::Equalizer(previous), Command::Equalizer(next)) =
                (&request.command, &command)
            {
                return next.replaces(*previous);
            }
            matches!(
                (&request.command, &command),
                (Command::Seek { .. }, Command::Seek { .. })
                    | (
                        Command::Volume(_) | Command::GlideVolume(_),
                        Command::Volume(_) | Command::GlideVolume(_)
                    )
            )
        });
        let stop = matches!(command, Command::Stop);
        let release_scan = matches!(command,
            Command::Scan { generation, direction: None } if generation == observed_generation
        );
        if !stop && !release_scan && !coalesce && mailbox.requests.len() >= MAX_COMMANDS {
            return Err("Too many pending controls; try again");
        }
        if matches!(
            command,
            Command::Open(_) | Command::Add(_) | Command::Replace(_)
        ) && mailbox.requests.iter().any(|request| {
            matches!(
                request.command,
                Command::Open(_) | Command::Add(_) | Command::Replace(_)
            )
        }) {
            return Err("A file selection is already pending");
        }
        mailbox.next_revision = mailbox
            .next_revision
            .checked_add(1)
            .ok_or("Session revision exhausted")?;
        let revision = mailbox.next_revision;
        if stop {
            mailbox.requests.clear();
            mailbox.stop_barrier = revision;
        } else if release_scan {
            // Release always has room, even during a flood. At most one extra slot;
            // superseded scan requests cannot start after the pointer was released.
            mailbox.requests.retain(|request| {
                !matches!(request.command, Command::Scan { generation, .. }
                    if generation <= observed_generation)
            });
        } else if coalesce {
            mailbox.requests.pop_back();
        }
        mailbox.requests.push_back(Request { revision, command });
        self.shared.wake.notify_one();
        Ok(revision)
    }

    /// Copies the latest bounded observation; audio/native work never holds this lock.
    pub fn snapshot(&self) -> Snapshot {
        self.shared
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Requests cancellation/cleanup without blocking the UI thread.
    pub fn request_shutdown(&self) {
        let mut mailbox = self
            .shared
            .mailbox
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        mailbox.shutdown = true;
        mailbox.requests.clear();
        self.shared.wake.notify_one();
    }

    /// Checks native/thread cleanup, not merely whether shutdown was acknowledged.
    pub fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Joins only after completion; a worker panic becomes an observable safe error.
    pub fn finish(&mut self) -> Result<bool, &'static str> {
        if !self.is_finished() {
            return Ok(false);
        }
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| "Playback worker failed")?;
        }
        Ok(true)
    }
}

impl Drop for Session {
    /// Never detaches native resources; normal UI close polls finish before dropping.
    fn drop(&mut self) {
        self.request_shutdown();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            eprintln!("error: playback worker failed during cleanup");
        }
    }
}

/// Validates the complete selection before retaining paths or submitting native work.
pub fn validate_selection(paths: &[PathBuf]) -> Result<(), &'static str> {
    if paths.is_empty() || paths.len() > MAX_QUEUE_ITEMS {
        return Err("Select 1 to 1024 files");
    }
    let mut bytes = 0_usize;
    for path in paths {
        let length = path.as_os_str().len();
        if length == 0 || length > 4096 {
            return Err("Invalid local file path");
        }
        bytes = bytes.saturating_add(length);
    }
    if bytes > MAX_SELECTION_BYTES {
        return Err("File selection is too large");
    }
    Ok(())
}

/// Makes a bounded plain-text filename without changing the original path bytes.
pub fn display_title(path: &std::path::Path) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut title = String::new();
    for character in name.chars().take(160) {
        if character.is_control()
            || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            title.push(' ');
        } else {
            title.push(character);
        }
    }
    if title.is_empty() {
        title.push_str("Local track");
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Holds the mailbox still so flooding and release capacity are tested deterministically.
    #[test]
    fn scan_release_has_bounded_reserved_capacity_and_stale_release_cannot_cancel() {
        let session = Session {
            shared: Arc::new(Shared {
                mailbox: Mutex::new(Mailbox::default()),
                wake: Condvar::new(),
                snapshot: Mutex::new(Snapshot::default()),
            }),
            worker: None,
        };
        session
            .submit(Command::Scan {
                generation: 0,
                direction: Some(ScanDirection::Forward),
            })
            .unwrap();
        session
            .submit(Command::Scan {
                generation: 9,
                direction: None,
            })
            .unwrap();
        assert_eq!(session.shared.mailbox.lock().unwrap().requests.len(), 2);
        session.submit(Command::Stop).unwrap();
        for _ in 1..MAX_COMMANDS {
            session.submit(Command::TogglePause).unwrap();
        }
        assert!(session.submit(Command::TogglePause).is_err());
        for _ in 0..100 {
            session
                .submit(Command::Scan {
                    generation: 0,
                    direction: None,
                })
                .unwrap();
            assert_eq!(
                session.shared.mailbox.lock().unwrap().requests.len(),
                MAX_COMMANDS + 1
            );
            assert!(
                session
                    .submit(Command::Scan {
                        generation: 9,
                        direction: None
                    })
                    .is_err()
            );
        }
        session.submit(Command::Stop).unwrap();
        assert_eq!(session.shared.mailbox.lock().unwrap().requests.len(), 1);
    }
}
