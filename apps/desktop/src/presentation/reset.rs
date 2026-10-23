//! Confirmed reset lifecycle: drain every writer before deleting application data.

use super::Controller;
use crate::{PlayerWindow, profile_reset};
use std::thread::JoinHandle;

pub(super) struct Job(Option<JoinHandle<Result<(), &'static str>>>);

impl Job {
    /// Joins a completed worker once; normal polling checks completion first.
    fn finish(&mut self) -> Result<(), &'static str> {
        self.0
            .take()
            .ok_or("Reset worker missing")?
            .join()
            .unwrap_or(Err("Profile reset worker failed"))
    }
}

impl Drop for Job {
    /// Keeps even abnormal shutdown from detaching a live deletion worker.
    fn drop(&mut self) {
        if let Some(job) = self.0.take() {
            let _ = job.join();
        }
    }
}

#[derive(Default)]
pub(super) enum ResetState {
    #[default]
    None,
    Requested,
    Working(Job),
    Failed,
    Done,
}

impl ResetState {
    /// Reports a confirmed reset waiting for playback and storage to stop.
    pub fn requested(&self) -> bool {
        matches!(self, Self::Requested)
    }
}

impl Controller {
    /// Accepts only an explicit UI confirmation, then uses the normal owned shutdown sequence.
    pub(crate) fn request_reset(&mut self, window: &PlayerWindow) {
        if self.close_started.is_some() || self.restoring() {
            return;
        }
        self.reset = ResetState::Requested;
        window.set_reset_confirm(false);
        window.set_closing(true);
        self.close();
    }

    /// Starts deletion only after audio, imports, presets and session writes have finished.
    pub(super) fn start_reset(&mut self, window: &PlayerWindow) {
        let result = self
            .storage
            .as_ref()
            .ok_or("Profile storage unavailable")
            .and_then(|store| store.reset_target())
            .and_then(|(directory, activity)| {
                std::thread::Builder::new()
                    .name("profile-reset".into())
                    .spawn(move || {
                        activity.make_exclusive()?;
                        profile_reset::clear(&directory)
                    })
                    .map_err(|_| "Cannot start profile reset")
            });
        match result {
            Ok(job) => self.reset = ResetState::Working(Job(Some(job))),
            Err(error) => self.reset_failed(window, error),
        }
    }

    /// Keeps the UI responsive during deletion; a failed reset never silently resumes old autosaves.
    pub(super) fn poll_reset(&mut self, window: &PlayerWindow) -> bool {
        match &self.reset {
            ResetState::None | ResetState::Requested => return false,
            ResetState::Failed | ResetState::Done => return true,
            ResetState::Working(job)
                if job.0.as_ref().is_some_and(|thread| !thread.is_finished()) =>
            {
                window.set_error_message("Resetting application data…".into());
                return true;
            }
            ResetState::Working(_) => {}
        }
        let ResetState::Working(mut job) = std::mem::take(&mut self.reset) else {
            return true;
        };
        match job.finish() {
            Ok(()) => {
                self.reset = ResetState::Done;
                let _ = slint::quit_event_loop();
            }
            Err(error) => self.reset_failed(window, error),
        }
        true
    }

    /// Leaves a redacted persistent error visible; the user can close and retry after reopening.
    fn reset_failed(&mut self, window: &PlayerWindow, error: &'static str) {
        eprintln!("error: {error}");
        self.reset = ResetState::Failed;
        window.set_reset_failure(
            format!("{error}. Some data may remain. Close and reopen ReSolibre.").into(),
        );
    }
}
