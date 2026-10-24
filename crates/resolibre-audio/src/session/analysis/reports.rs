//! All small Library measurements plus five recent plots; disk inspection never reads original audio.

use crate::analysis_cache::CacheConfig;
use resolibre_core::analysis::{TrackAnalysis, TrackAnalysisSummary};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

const PLOT_LIMIT: usize = 5;

type Ticket = (u64, u64);

struct Entry {
    summary: TrackAnalysisSummary,
    digest: Option<[u8; 32]>,
    version: u64,
}

struct Lookup {
    ticket: Ticket,
    thread: JoinHandle<Option<TrackAnalysis>>,
}

#[derive(Default)]
pub(super) struct Reports {
    entries: HashMap<u64, Entry>,
    plots: VecDeque<(u64, Arc<TrackAnalysis>)>,
    version: u64,
    requested: Option<Ticket>,
    lookup: Option<Lookup>,
}

impl Reports {
    /// Retains small facts for every occurrence and admits the completed plot to the five-entry cache.
    pub(super) fn remember(
        &mut self,
        id: u64,
        report: Arc<TrackAnalysis>,
        digest: Option<[u8; 32]>,
    ) {
        self.version = self.version.wrapping_add(1);
        self.entries.insert(
            id,
            Entry {
                summary: TrackAnalysisSummary::from(report.as_ref()),
                digest,
                version: self.version,
            },
        );
        self.keep_plot(id, report);
    }

    /// Shares a recent plot and promotes its recency without cloning its matrix or beat vectors.
    pub(super) fn plot(&mut self, id: u64) -> Option<Arc<TrackAnalysis>> {
        let index = self.plots.iter().position(|(saved, _)| *saved == id)?;
        let entry = self.plots.remove(index)?;
        let report = Arc::clone(&entry.1);
        self.plots.push_back(entry);
        Some(report)
    }

    /// Coalesces disk opens to the latest inspection; unknown/unpersisted reports require ordinary analysis.
    pub(super) fn request(&mut self, id: u64) -> bool {
        let Some(entry) = self.entries.get(&id).filter(|entry| entry.digest.is_some()) else {
            return false;
        };
        self.requested = Some((id, entry.version));
        true
    }

    /// Closing or switching a view invalidates its pending publication without blocking on disk I/O.
    pub(super) fn cancel_view(&mut self) {
        self.requested = None;
    }

    /// Explicit fresh analysis invalidates both the old matrix and any late historical lookup.
    pub(super) fn remove(&mut self, id: u64) {
        self.entries.remove(&id);
        self.plots.retain(|(saved, _)| *saved != id);
        if self.requested.is_some_and(|(saved, _)| saved == id) {
            self.requested = None;
        }
    }

    /// Queue removal releases small facts and plots; IDs cannot leak across Library replacement.
    pub(super) fn retain(&mut self, ids: &BTreeSet<u64>) {
        self.entries.retain(|id, _| ids.contains(id));
        self.plots.retain(|(id, _)| ids.contains(id));
        if self.requested.is_some_and(|(id, _)| !ids.contains(&id)) {
            self.requested = None;
        }
    }

    /// Runs at most one bounded cache reader independently of the decoder; late/replaced results are discarded.
    pub(super) fn poll(
        &mut self,
        config: Option<&Arc<CacheConfig>>,
    ) -> Option<(u64, Option<Arc<TrackAnalysis>>)> {
        if self
            .lookup
            .as_ref()
            .is_some_and(|lookup| lookup.thread.is_finished())
        {
            let lookup = self.lookup.take()?;
            let report = match lookup.thread.join() {
                Ok(report) => report,
                Err(_) => {
                    if let Some(config) = config {
                        config.warn();
                    }
                    None
                }
            };
            if self.requested == Some(lookup.ticket) {
                self.requested = None;
                let (id, version) = lookup.ticket;
                let entry = self
                    .entries
                    .get(&id)
                    .filter(|entry| entry.version == version)?;
                // The digest/version identify the report; tolerate JSON's last-bit float rounding.
                let report = report.filter(|report| {
                    report.sample_rate_hz == entry.summary.sample_rate_hz
                        && report.channels == entry.summary.channels
                        && report.columns == entry.summary.columns
                        && (report.duration_seconds - entry.summary.duration_seconds).abs() < 1e-6
                });
                let report = report.map(Arc::new);
                if let Some(report) = &report {
                    if let Some(entry) = self.entries.get_mut(&id) {
                        entry.summary = TrackAnalysisSummary::from(report.as_ref());
                    }
                    self.keep_plot(id, Arc::clone(report));
                }
                return Some((id, report));
            }
        }
        if self.lookup.is_some() {
            return None;
        }
        let ticket = self.requested?;
        let digest = self.entries.get(&ticket.0)?.digest?;
        let Some(config) = config.cloned() else {
            self.requested = None;
            return Some((ticket.0, None));
        };
        let worker_config = Arc::clone(&config);
        match thread::Builder::new()
            .name("resolibre-analysis-view".into())
            .spawn(move || {
                let database = worker_config.database()?;
                match database.track_snapshot(&digest) {
                    Ok(report) => report,
                    Err(_) => {
                        worker_config.warn();
                        None
                    }
                }
            }) {
            Ok(thread) => self.lookup = Some(Lookup { ticket, thread }),
            Err(_) => {
                config.warn();
                self.requested = None;
                return Some((ticket.0, None));
            }
        }
        None
    }

    /// Evicts only the least recently used matrix; its BPM, beats and technical measurements survive.
    fn keep_plot(&mut self, id: u64, report: Arc<TrackAnalysis>) {
        self.plots.retain(|(saved, _)| *saved != id);
        self.plots.push_back((id, report));
        while self.plots.len() > PLOT_LIMIT {
            self.plots.pop_front();
        }
    }
}

impl Drop for Reports {
    /// Shutdown reaps the single owned reader; ordinary view changes never wait on its thread.
    fn drop(&mut self) {
        if let Some(lookup) = self.lookup.take() {
            let _ = lookup.thread.join();
        }
    }
}

#[cfg(test)]
mod tests;
