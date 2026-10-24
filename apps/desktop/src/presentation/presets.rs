//! One controller path for named EQ saves, loads and portable file transfers.

use super::*;
use crate::picker::Kind;
use crate::preset_store::Completion;
use resolibre_core::equalizer::{EqAction, Equalizer, PRESET_NAMES, preset::Preset};

impl Controller {
    /// Resolves the shared dropdown index, excluding its final Custom display entry.
    fn preset_at(&self, index: usize) -> Option<Preset> {
        let entries = self
            .presets
            .as_ref()
            .map(|store| store.entries.as_slice())
            .unwrap_or_default();
        if let Some(entry) = entries.get(index) {
            return Some(entry.preset.clone());
        }
        let built_in = index.checked_sub(entries.len())?;
        Preset::new(PRESET_NAMES.get(built_in)?, Equalizer::preset(built_in)?).ok()
    }

    /// Applies a confirmed catalog entry through the same audio command as built-in curves.
    pub fn load_preset(&mut self, window: &PlayerWindow, index: usize) {
        if let Some(preset) = self.preset_at(index) {
            self.submit(window, Command::Equalizer(EqAction::Load(preset.settings)));
            window.set_eq_notice("".into());
        }
    }

    /// Captures settled EQ settings before opening the name editor; later edits cannot change the draft.
    pub fn begin_preset_save(&mut self, window: &PlayerWindow) {
        if !self.presets_ready() {
            return;
        }
        let snapshot = self.session.snapshot();
        let name = self
            .preset_at(window.get_eq_preset_index() as usize)
            .filter(|preset| preset.settings == snapshot.equalizer)
            .map(|preset| preset.name)
            .unwrap_or_default();
        self.open_preset_editor(window, name, snapshot.equalizer);
    }

    /// Opens one embedded editor shared by Save and Import, without mutating current playback.
    fn open_preset_editor(&mut self, window: &PlayerWindow, name: String, settings: Equalizer) {
        self.preset_draft = Some(settings);
        window.set_eq_preset_name(name.into());
        window.set_eq_preset_message("".into());
        window.set_eq_replace_confirm(false);
        window.set_eq_dialog_open(true);
        window.set_eq_notice("".into());
    }

    /// Discards the editor only when no write is in progress; accepted writes finish before shutdown.
    pub fn cancel_preset_save(&mut self, window: &PlayerWindow) {
        if self.presets.as_ref().is_some_and(|store| store.busy()) {
            return;
        }
        self.preset_draft = None;
        window.set_eq_dialog_open(false);
    }

    /// Validates names and makes replacing a matching user preset an explicit second action.
    pub fn save_preset(&mut self, window: &PlayerWindow, name: &str, replace: bool) {
        if !self.presets_ready() {
            return;
        }
        let Some(settings) = self.preset_draft else {
            return;
        };
        let preset = match Preset::new(name, settings) {
            Ok(preset) => preset,
            Err(error) => {
                self.preset_error(window, error);
                return;
            }
        };
        let Some(store) = self.presets.as_mut() else {
            return;
        };
        let exists = store
            .entries
            .iter()
            .any(|entry| entry.preset.name.to_lowercase() == preset.name.to_lowercase());
        if exists && !replace {
            window.set_eq_replace_confirm(true);
            window.set_eq_preset_message(
                "This name already exists. Replace it or choose another name.".into(),
            );
            return;
        }
        match store.save(preset, replace) {
            Ok(()) => {
                window.set_eq_store_busy(true);
                window.set_eq_preset_message("Saving…".into());
            }
            Err(error) => self.preset_error(window, error),
        }
    }

    /// Allows one explicit preset operation only after restore and pending EQ commands settle.
    fn presets_ready(&self) -> bool {
        self.close_started.is_none()
            && !self.restoring()
            && self.preset_picker.is_none()
            && self.session.snapshot().revision >= self.expected_revision
            && self.presets.as_ref().is_some_and(|store| !store.busy())
    }

    /// Starts the shared native file chooser; export captures the current curve at this moment.
    pub fn pick_preset(&mut self, window: &PlayerWindow, export: bool) {
        if !self.presets_ready() || self.picker.is_some() {
            return;
        }
        let snapshot = self.session.snapshot();
        let export_preset = if export {
            let settings = self.preset_draft.unwrap_or(snapshot.equalizer);
            match Preset::new(window.get_eq_preset_name().as_str(), settings) {
                Ok(preset) => Some(preset),
                Err(error) => {
                    self.preset_error(window, error);
                    return;
                }
            }
        } else {
            None
        };
        let kind = if export {
            Kind::ExportPreset
        } else {
            Kind::ImportPreset
        };
        match Picker::start_for(kind, false) {
            Ok(picker) => {
                self.preset_picker = Some(picker);
                self.preset_export = export_preset;
                window.set_eq_store_busy(true);
                window.set_eq_notice("".into());
            }
            Err(error) => self.preset_error(window, error),
        }
    }

    /// Handles chooser results and owned I/O completions without blocking UI or audio.
    pub(super) fn poll_presets(&mut self, window: &PlayerWindow) {
        self.poll_preset_picker(window);
        let completion = self.presets.as_mut().map(|store| store.poll());
        match completion {
            Some(Ok(Some(Completion::Catalog {
                selected, skipped, ..
            }))) => {
                self.update_preset_names(window);
                if let Some(name) = selected {
                    if let Some(store) = self.presets.as_ref()
                        && let Some(index) = store
                            .entries
                            .iter()
                            .position(|entry| entry.preset.name == name)
                    {
                        window.set_eq_preset_index(index as i32);
                    }
                    self.preset_draft = None;
                    window.set_eq_dialog_open(false);
                    window.set_eq_notice("Saved".into());
                }
                if skipped {
                    self.preset_error(
                        window,
                        "Some invalid or duplicate EQ presets were not loaded",
                    );
                }
            }
            Some(Ok(Some(Completion::Imported(preset)))) => {
                self.open_preset_editor(window, preset.name, preset.settings);
                window.set_eq_preset_message(
                    "Choose a name to add this imported preset. Playback stays unchanged.".into(),
                );
            }
            Some(Ok(Some(Completion::Exported))) => {
                window.set_eq_notice("Exported".into());
                window.set_eq_preset_message(
                    "Exported. You can share the .resolibre-eq file.".into(),
                );
            }
            Some(Err(error)) => self.preset_error(window, error),
            _ => {}
        }
        let busy = self.preset_picker.is_some()
            || self.presets.as_ref().is_none_or(|store| store.busy())
            || self.session.snapshot().revision < self.expected_revision;
        window.set_eq_store_busy(busy);
        if !busy {
            self.show_preset_selection(window);
        }
    }

    /// Converts one completed chooser result into a storage job; cancellation discards only the transfer.
    fn poll_preset_picker(&mut self, window: &PlayerWindow) {
        let Some(picker) = self.preset_picker.as_mut() else {
            return;
        };
        let result = picker.poll();
        if matches!(result, Ok(None)) {
            return;
        }
        self.preset_picker = None;
        let export = self.preset_export.take();
        let paths = match result {
            Ok(Some(paths)) => paths,
            Err(error) => {
                self.preset_error(window, error);
                return;
            }
            Ok(None) => return,
        };
        if paths.is_empty() {
            return;
        }
        if paths.len() != 1 {
            self.preset_error(window, "Select one EQ preset");
            return;
        }
        let Some(store) = self.presets.as_mut() else {
            return;
        };
        let result = if let Some(preset) = export {
            store.export(paths[0].clone(), preset)
        } else {
            store.import(paths[0].clone())
        };
        if let Err(error) = result {
            self.preset_error(window, error);
        }
    }

    /// Reads only small copyable settings during refresh, avoiding name allocations on every tick.
    fn preset_settings(&self, index: usize) -> Option<Equalizer> {
        let entries = self
            .presets
            .as_ref()
            .map(|store| store.entries.as_slice())
            .unwrap_or_default();
        if let Some(entry) = entries.get(index) {
            return Some(entry.preset.settings);
        }
        Equalizer::preset(index.checked_sub(entries.len())?)
    }

    /// Rebuilds names only on catalog revisions, including Custom for unsaved curves.
    fn update_preset_names(&self, window: &PlayerWindow) {
        let mut names: Vec<slint::SharedString> = Vec::new();
        if let Some(store) = self.presets.as_ref() {
            names.extend(
                store
                    .entries
                    .iter()
                    .map(|entry| entry.preset.name.as_str().into()),
            );
        }
        names.extend(PRESET_NAMES.into_iter().map(Into::into));
        names.push("Custom".into());
        window.set_eq_presets(ModelRc::new(VecModel::from(names)));
    }

    /// Keeps the selector honest when sliders, reset, bypass or session restore change the curve.
    fn show_preset_selection(&self, window: &PlayerWindow) {
        let settings = self.session.snapshot().equalizer;
        if self
            .preset_settings(window.get_eq_preset_index() as usize)
            .is_some_and(|preset| preset == settings)
        {
            return;
        }
        let count =
            PRESET_NAMES.len() + self.presets.as_ref().map_or(0, |store| store.entries.len());
        let selected = (0..count)
            .find(|index| {
                self.preset_settings(*index)
                    .is_some_and(|preset| preset == settings)
            })
            .unwrap_or(count);
        window.set_eq_preset_index(selected as i32);
    }

    /// Reports handled failures once using safe messages, keeping editor errors next to the action.
    fn preset_error(&mut self, window: &PlayerWindow, message: &'static str) {
        eprintln!("warn: {message}");
        if window.get_eq_dialog_open() {
            window.set_eq_preset_message(message.into());
        } else {
            self.local_error = Some(message.into());
        }
    }
}

#[cfg(test)]
mod tests;
