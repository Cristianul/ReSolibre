//! EQ presentation translates values/actions; no DSP or duplicate preset policy lives here.

use crate::{EqBand, PlayerWindow, presentation::Controller, submit};
use resolibre_audio::session::{Command, Snapshot};
use resolibre_core::equalizer::{
    EqAction, FREQUENCIES, PREAMP_MAX_DB, PREAMP_MIN_DB, PRESET_NAMES, active_bands,
};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

/// Binds every panel action to shared session or preset-store entry points.
pub fn connect(window: &PlayerWindow, controller: &Rc<RefCell<Controller>>) {
    initialize(window);
    let weak = window.as_weak();
    let owner = Rc::clone(controller);
    window.on_eq_band(move |index, db| {
        if let Some(window) = weak.upgrade() {
            submit(
                &window,
                &owner,
                Command::Equalizer(EqAction::Band {
                    index: index as usize,
                    db: f64::from(db),
                }),
            );
        }
    });
    let weak = window.as_weak();
    let owner = Rc::clone(controller);
    window.on_eq_gain(move |db| {
        if let Some(window) = weak.upgrade() {
            submit(
                &window,
                &owner,
                Command::Equalizer(EqAction::Preamp(f64::from(db))),
            );
        }
    });
    let weak = window.as_weak();
    let owner = Rc::clone(controller);
    window.on_eq_enable(move |enabled| {
        if let Some(window) = weak.upgrade() {
            submit(
                &window,
                &owner,
                Command::Equalizer(EqAction::Enabled(enabled)),
            );
        }
    });
    let weak = window.as_weak();
    let owner = Rc::clone(controller);
    window.on_eq_load(move |index| {
        if let Some(window) = weak.upgrade() {
            owner.borrow_mut().load_preset(&window, index as usize);
        }
    });
    let weak = window.as_weak();
    let owner = Rc::clone(controller);
    window.on_eq_save(move || {
        if let Some(window) = weak.upgrade() {
            owner.borrow_mut().begin_preset_save(&window);
        }
    });
    let weak = window.as_weak();
    let owner = Rc::clone(controller);
    window.on_eq_commit_preset(move |name, replace| {
        if let Some(window) = weak.upgrade() {
            owner
                .borrow_mut()
                .save_preset(&window, name.as_str(), replace);
        }
    });
    let weak = window.as_weak();
    let owner = Rc::clone(controller);
    window.on_eq_cancel_preset(move || {
        if let Some(window) = weak.upgrade() {
            owner.borrow_mut().cancel_preset_save(&window);
        }
    });
    let weak = window.as_weak();
    let owner = Rc::clone(controller);
    window.on_eq_transfer(move |export| {
        if let Some(window) = weak.upgrade() {
            owner.borrow_mut().pick_preset(&window, export);
        }
    });
}

/// Initializes the same presentation model for production and visual inspection.
pub fn initialize(window: &PlayerWindow) {
    window.set_eq_preamp_minimum(PREAMP_MIN_DB as f32);
    window.set_eq_preamp_maximum(PREAMP_MAX_DB as f32);
    show(window, &Snapshot::default());
    let names: Vec<slint::SharedString> = PRESET_NAMES
        .into_iter()
        .chain(["Custom"])
        .map(Into::into)
        .collect();
    window.set_eq_presets(ModelRc::new(VecModel::from(names)));
}

/// Shows worker-confirmed target gains; native smoothing may still be converging to that intent.
pub fn show(window: &PlayerWindow, snapshot: &Snapshot) {
    let gains = snapshot.equalizer.bands();
    let available = if snapshot.eq_sample_rate == 0 {
        [true; 10]
    } else {
        active_bands(snapshot.eq_sample_rate)
    };
    let rows: Vec<_> = FREQUENCIES
        .iter()
        .enumerate()
        .map(|(index, frequency)| EqBand {
            label: if *frequency >= 1000.0 {
                format!("{}k", frequency / 1000.0).into()
            } else {
                format!("{frequency}").into()
            },
            gain: gains[index] as f32,
            available: available[index],
        })
        .collect();
    // Retain the model while dragging; replacing it every refresh destroys focus/drag state.
    if let Some(model) = window
        .get_eq_bands()
        .as_any()
        .downcast_ref::<VecModel<EqBand>>()
    {
        for (index, row) in rows.into_iter().enumerate() {
            if model.row_data(index).as_ref() != Some(&row) {
                model.set_row_data(index, row);
            }
        }
    } else {
        window.set_eq_bands(ModelRc::new(VecModel::from(rows)));
    }
    window.set_eq_enabled(snapshot.equalizer.enabled());
    window.set_eq_preamp(snapshot.equalizer.preamp() as f32);
}
