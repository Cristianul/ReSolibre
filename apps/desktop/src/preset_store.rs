//! Named preset catalog and owned background I/O, independent of UI and audio threads.

mod files;
mod legacy;
#[cfg(test)]
mod tests;

use resolibre_core::equalizer::preset::Preset;
use std::path::PathBuf;
use std::thread::{self, JoinHandle};

pub const MAX_PRESETS: usize = 256;

#[derive(Clone, Debug)]
pub struct Entry {
    pub file: String,
    pub preset: Preset,
}

pub enum Completion {
    Catalog {
        entries: Vec<Entry>,
        selected: Option<String>,
        skipped: bool,
    },
    Imported(Preset),
    Exported,
}

type Job = JoinHandle<Result<Completion, &'static str>>;

pub struct PresetStore {
    directory: PathBuf,
    pub entries: Vec<Entry>,
    job: Option<Job>,
}

impl PresetStore {
    /// Loads the bounded catalog and migrates legacy slots inside one owned storage job.
    pub fn start(directory: PathBuf) -> Result<Self, &'static str> {
        let path = directory.clone();
        let job = spawn(move || files::load(&path))?;
        Ok(Self {
            directory,
            entries: Vec::new(),
            job: Some(job),
        })
    }

    /// Reports whether a load, save or transfer still owns the storage adapter.
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Starts a named save; replacement requires an explicit caller confirmation.
    pub fn save(&mut self, preset: Preset, replace: bool) -> Result<(), &'static str> {
        self.ready()?;
        let directory = self.directory.clone();
        self.job = Some(spawn(move || files::save(&directory, preset, replace))?);
        Ok(())
    }

    /// Reads only the selected preset, without applying it or changing the catalog yet.
    pub fn import(&mut self, path: PathBuf) -> Result<(), &'static str> {
        self.ready()?;
        self.job = Some(spawn(move || {
            files::read_preset(&path).map(Completion::Imported)
        })?);
        Ok(())
    }

    /// Writes a portable copy to a new explicit destination; existing files are never overwritten.
    pub fn export(&mut self, path: PathBuf, preset: Preset) -> Result<(), &'static str> {
        self.ready()?;
        self.job = Some(spawn(move || {
            files::export(&path, &preset)?;
            Ok(Completion::Exported)
        })?);
        Ok(())
    }

    /// Rejects overlapping jobs instead of silently replacing owned work.
    fn ready(&self) -> Result<(), &'static str> {
        if self.busy() {
            return Err("Preset storage is busy; please wait");
        }
        Ok(())
    }

    /// Reaps finished work without waiting; failed transactions retain the confirmed catalog.
    pub fn poll(&mut self) -> Result<Option<Completion>, &'static str> {
        if self.job.as_ref().is_none_or(|job| !job.is_finished()) {
            return Ok(None);
        }
        let Some(job) = self.job.take() else {
            return Ok(None);
        };
        let completed = job.join().map_err(|_| "Preset storage worker failed")??;
        if let Completion::Catalog { entries, .. } = &completed {
            self.entries = entries.clone();
        }
        Ok(Some(completed))
    }
}

impl Drop for PresetStore {
    /// Reaps owned I/O on abnormal exits; normal shutdown polls until the job finishes.
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            let _ = job.join();
        }
    }
}

/// Starts one bounded operation, keeping native thread failures recoverable.
fn spawn(
    work: impl FnOnce() -> Result<Completion, &'static str> + Send + 'static,
) -> Result<Job, &'static str> {
    thread::Builder::new()
        .name("eq-presets".into())
        .spawn(work)
        .map_err(|_| "Cannot start preset storage")
}
