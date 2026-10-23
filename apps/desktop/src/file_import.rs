//! One bounded drop batch and background expansion job; UI decides Add or Replace after scanning.

#[cfg(all(test, unix))]
mod native_tests;
pub mod scan;
#[cfg(all(test, unix))]
mod tests;

use crate::window_events::WindowEvents;
use resolibre_audio::session::MAX_SELECTION_BYTES;
use resolibre_core::queue::MAX_QUEUE_ITEMS;
use slint::winit_030::winit::event::WindowEvent;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Target {
    #[default]
    Player,
    Playlist,
}

#[derive(Default)]
pub struct Inbox {
    paths: Vec<PathBuf>,
    bytes: usize,
    last_drop: Option<Instant>,
    target: Target,
    position: (f32, f32),
    busy: bool,
    error: Option<&'static str>,
    rejected: bool,
}

impl Inbox {
    /// Retains one bounded native batch; overflow rejects the whole batch, never a partial Replace.
    pub fn receive(&mut self, path: PathBuf, target: Target, position: (f32, f32)) {
        if self.busy {
            self.error = Some("Finish or cancel the current import first");
            return;
        }
        self.last_drop = Some(Instant::now());
        if self.rejected {
            return;
        }
        let length = path.as_os_str().len();
        if !path.is_absolute()
            || length > 4096
            || self.paths.len() >= MAX_QUEUE_ITEMS
            || self.bytes.saturating_add(length) > MAX_SELECTION_BYTES
        {
            self.paths.clear();
            self.bytes = 0;
            self.rejected = true;
            self.error = Some("Dropped selection is too large or contains an invalid path");
            return;
        }
        self.bytes += length;
        self.paths.push(path);
        self.target = target;
        self.position = position;
    }

    /// Releases pending input after confirmation, cancellation or failure.
    fn clear(&mut self) {
        self.paths.clear();
        self.bytes = 0;
        self.last_drop = None;
        self.busy = false;
        self.rejected = false;
    }
}

struct Job {
    worker: JoinHandle<Result<scan::Selection, &'static str>>,
    cancel: Arc<AtomicBool>,
}

pub struct Ready {
    pub selection: scan::Selection,
    pub target: Target,
    pub position: (f32, f32),
}

#[derive(Default)]
pub struct Importer {
    pub inbox: Rc<RefCell<Inbox>>,
    job: Option<Job>,
}

impl Importer {
    /// Coalesces winit's per-file events, then starts one worker; polling never waits on filesystem I/O.
    pub fn poll(&mut self) -> Option<Result<Ready, &'static str>> {
        if let Some(job) = self.job.as_ref() {
            if !job.worker.is_finished() {
                return None;
            }
            let job = self.job.take()?;
            let result = job
                .worker
                .join()
                .unwrap_or(Err("Folder import worker failed"));
            let inbox = self.inbox.borrow();
            return Some(result.map(|selection| Ready {
                selection,
                target: inbox.target,
                position: inbox.position,
            }));
        }
        let mut inbox = self.inbox.borrow_mut();
        if inbox.busy
            || inbox
                .last_drop
                .is_none_or(|last| last.elapsed() < Duration::from_millis(100))
        {
            return None;
        }
        if inbox.rejected {
            inbox.clear();
            return Some(Err(inbox
                .error
                .take()
                .unwrap_or("Cannot import dropped files")));
        }
        let paths = std::mem::take(&mut inbox.paths);
        inbox.busy = true;
        inbox.last_drop = None;
        let cancel = Arc::new(AtomicBool::new(false));
        let observed = cancel.clone();
        match std::thread::Builder::new()
            .name("resolibre-import".into())
            .spawn(move || scan::expand(paths, &observed))
        {
            Ok(worker) => {
                self.job = Some(Job { worker, cancel });
                None
            }
            Err(_) => {
                inbox.clear();
                Some(Err("Cannot start folder import"))
            }
        }
    }

    /// Clears menu/batch state after the owner has handled the result.
    pub fn finish(&mut self) {
        self.inbox.borrow_mut().clear();
    }

    /// Reports duplicate-drop feedback without changing the in-progress selection.
    pub fn take_notice(&self) -> Option<&'static str> {
        let mut inbox = self.inbox.borrow_mut();
        if inbox.rejected {
            return None;
        }
        inbox.error.take()
    }

    /// Reserves file-selection UI while collecting, scanning or awaiting a menu decision.
    pub fn reserved(&self) -> bool {
        let inbox = self.inbox.borrow();
        inbox.busy || inbox.last_drop.is_some()
    }

    /// Indicates active scanning; menus keep input reserved but do not run a worker.
    pub fn scanning(&self) -> bool {
        self.job.is_some()
    }

    /// Cooperatively cancels work and joins only once finished; close remains responsive.
    pub fn close(&mut self) -> bool {
        if let Some(job) = self.job.as_ref() {
            job.cancel.store(true, Ordering::Relaxed);
            if !job.worker.is_finished() {
                return false;
            }
        }
        if let Some(job) = self.job.take() {
            let _ = job.worker.join();
        }
        self.finish();
        true
    }
}

impl Drop for Importer {
    /// Reclaims the owned worker on exceptional teardown; normal close polls before destruction.
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel.store(true, Ordering::Relaxed);
            let _ = job.worker.join();
        }
    }
}

/// Shares the native hook with snapping and accepts only OS-delivered local paths.
pub fn watch(events: &WindowEvents, inbox: &Rc<RefCell<Inbox>>, target: Target) {
    let owner = Rc::downgrade(inbox);
    let mut position = (24.0, 48.0);
    events.listen(move |window, event| match event {
        WindowEvent::CursorMoved {
            position: physical, ..
        } => {
            let scale = window.scale_factor() as f64;
            position = ((physical.x / scale) as f32, (physical.y / scale) as f32);
        }
        WindowEvent::DroppedFile(path) => {
            if let Some(inbox) = owner.upgrade() {
                inbox.borrow_mut().receive(path.clone(), target, position);
            }
        }
        _ => {}
    });
}
