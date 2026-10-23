//! Coalesced owned storage jobs: one write in flight and one latest pending session.

mod format;
#[cfg(test)]
mod tests;

use crate::{interface_state::InterfaceState, profile_storage};
use resolibre_audio::session::SavedSession;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const FILE_NAME: &str = "session-v1.bin";
type Job = JoinHandle<Result<Completion, &'static str>>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredSession {
    pub audio: SavedSession,
    pub interface: InterfaceState,
}

enum Completion {
    Loaded {
        activity: Arc<profile_storage::SessionLease>,
        state: Result<(Arc<profile_storage::SessionLease>, StoredSession), &'static str>,
    },
    Saved(StoredSession),
}

pub struct SessionStore {
    directory: PathBuf,
    lease: Option<Arc<profile_storage::SessionLease>>,
    activity: Option<Arc<profile_storage::SessionLease>>,
    job: Option<Job>,
    loading: bool,
    failed: bool,
    latest: Option<StoredSession>,
    saved: Option<StoredSession>,
    dirty_since: Option<Instant>,
}

impl SessionStore {
    /// Loads only this profile in a background job; the exclusive lease prevents competing writers.
    pub fn start(directory: PathBuf) -> Result<Self, &'static str> {
        let path = directory.clone();
        let job = thread::Builder::new()
            .name("session-load".into())
            .spawn(move || {
                let activity = Arc::new(profile_storage::share_profile(&path)?);
                let state = (|| {
                    let lease = Arc::new(profile_storage::lock_session(&path)?);
                    let saved = match profile_storage::read(&path, FILE_NAME, format::MAX_BYTES)? {
                        Some(bytes) => format::decode(&bytes)?,
                        None => StoredSession::default(),
                    };
                    Ok((lease, saved))
                })();
                Ok(Completion::Loaded { activity, state })
            })
            .map_err(|_| "Cannot start session storage")?;
        Ok(Self {
            directory,
            lease: None,
            activity: None,
            job: Some(job),
            loading: true,
            failed: false,
            latest: None,
            saved: None,
            dirty_since: None,
        })
    }

    /// Authorizes profile use only after the shared activity lease has been acquired.
    pub fn profile_available(&self) -> bool {
        self.activity.is_some()
    }

    /// Captures reset ownership after pending writes finish; corrupt settings may still be reset.
    pub fn reset_target(
        &self,
    ) -> Result<(PathBuf, Arc<profile_storage::SessionLease>), &'static str> {
        if self.busy() {
            return Err("Profile storage is busy");
        }
        let activity = self
            .activity
            .clone()
            .ok_or("Profile access unavailable; reopen ReSolibre")?;
        Ok((self.directory.clone(), activity))
    }

    /// Indicates that startup commands must wait so an old load cannot overwrite new intent.
    pub fn loading(&self) -> bool {
        self.loading
    }

    /// Reports live work or unsaved intent; close forces the coalescing deadline to expire.
    pub fn busy(&self) -> bool {
        self.job.is_some() || (!self.failed && self.latest != self.saved)
    }

    /// Retains confirmed audio and interface preferences, excluding timing and transient windows.
    pub fn observe(&mut self, saved: StoredSession) {
        if self.loading || self.failed || self.latest.as_ref() == Some(&saved) {
            return;
        }
        self.latest = Some(saved);
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    /// Reaps completed jobs and starts at most one write per 500 ms; close flushes immediately.
    pub fn poll(&mut self, closing: bool) -> Result<Option<StoredSession>, &'static str> {
        let result = self.poll_inner(closing);
        if result.is_err() {
            // Preserve unreadable/unknown files and avoid repeating errors or automatic overwrites.
            self.loading = false;
            self.failed = true;
            self.lease = None;
        }
        result
    }

    /// Keeps I/O on owned threads; successful completion confirms only the version actually written.
    fn poll_inner(&mut self, closing: bool) -> Result<Option<StoredSession>, &'static str> {
        if self.job.as_ref().is_some_and(JoinHandle::is_finished) {
            let Some(job) = self.job.take() else {
                return Ok(None);
            };
            match job.join().map_err(|_| "Session storage worker failed")?? {
                Completion::Loaded { activity, state } => {
                    self.activity = Some(activity);
                    let (lease, saved) = state?;
                    self.lease = Some(lease);
                    self.loading = false;
                    self.saved = Some(saved.clone());
                    self.latest = Some(saved.clone());
                    return Ok(Some(saved));
                }
                Completion::Saved(saved) => self.saved = Some(saved),
            }
        }
        if self.failed || self.loading || self.job.is_some() || self.latest == self.saved {
            return Ok(None);
        }
        if !closing
            && self
                .dirty_since
                .is_some_and(|time| time.elapsed() < Duration::from_millis(500))
        {
            return Ok(None);
        }
        let Some(saved) = self.latest.clone() else {
            return Ok(None);
        };
        let lease = self.lease.clone().ok_or("Session storage unavailable")?;
        let directory = self.directory.clone();
        self.job = Some(
            thread::Builder::new()
                .name("session-save".into())
                .spawn(move || {
                    let _lease = lease;
                    let bytes = format::encode(&saved)?;
                    profile_storage::write(&directory, FILE_NAME, &bytes)?;
                    Ok(Completion::Saved(saved))
                })
                .map_err(|_| "Cannot start session storage")?,
        );
        self.dirty_since = None;
        Ok(None)
    }
}

impl Drop for SessionStore {
    /// Reaps any in-flight file transaction; normal close first flushes all pending intent.
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            let _ = job.join();
        }
    }
}
