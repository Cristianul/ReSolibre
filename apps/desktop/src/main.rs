//! Local desktop composition; core/native modules own playback and queue decisions.

mod analysis_window;
mod artwork;
mod equalizer;
mod file_import;
mod interface_state;
mod library_duration;
mod library_search;
mod live_spectrum;
mod picker;
mod playback_edges;
mod playlist_window;
mod presentation;
mod preset_store;
mod profile_reset;
mod profile_storage;
mod session_store;
mod themes;
mod track_info;
mod transport_scan;
mod window_events;
mod window_group;
mod window_snap;

#[cfg(test)]
mod ui_tests;

#[cfg(test)]
mod visual_tests;

#[cfg(target_env = "gnu")]
#[path = "../../../crates/resolibre-audio/examples/support/process_memory.rs"]
mod process_memory;

use resolibre_audio::AudioOutput;
use resolibre_audio::session::{Command, Session};
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::process::ExitCode;
use std::rc::Rc;

slint::include_modules!();

/// Applies process policy before toolkit threads, or runs the isolated chooser helper.
fn main() -> ExitCode {
    // SAFETY: this entry point has not started toolkit, picker or worker threads.
    #[cfg(target_env = "gnu")]
    if let Err(error) = unsafe { process_memory::configure() } {
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
    }
    let mut use_x11 = std::env::var_os("DISPLAY").is_some_and(|display| !display.is_empty());
    let mut arguments = std::env::args_os().skip(1);
    if let Some(argument) = arguments.next() {
        if let Some(kind) = picker::Kind::from_argument(&argument) {
            return if arguments.next().is_none() && picker::run_helper(kind).is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
        if (argument == "--x11" || argument == "--native-display") && arguments.next().is_none() {
            use_x11 = argument == "--x11";
        } else {
            eprintln!(
                "error: use Open files inside ReSolibre; only --x11 or --native-display is supported at launch"
            );
            return ExitCode::FAILURE;
        }
    }
    match run(use_x11) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Builds one UI/session and keeps close responsive until owned native resources exit.
fn run(use_x11: bool) -> Result<(), &'static str> {
    if use_x11 {
        window_snap::select_x11()?;
    }
    let window = PlayerWindow::new().map_err(|_| "Cannot create desktop window")?;
    slint::set_xdg_app_id(window.global::<Branding>().get_app_id())
        .map_err(|_| "Cannot register desktop application identity")?;
    let analysis_directory = profile_storage::default_directory()
        .ok()
        .map(|directory| directory.join("analysis"));
    let session = Session::start_with_analysis_cache(AudioOutput::Device, analysis_directory)
        .map_err(|_| "Cannot start playback worker")?;
    let controller = Rc::new(RefCell::new(presentation::Controller::new(session)));
    controller.borrow_mut().start_default_storage();
    window.set_initializing(true);
    presentation::connect(&window, &controller);
    let windows = window_group::SharedGroup::default();
    let playlist =
        playlist_window::connect(&window, controller.borrow().import_inbox(), windows.clone());
    let themes = themes::connect(&window, windows.clone());
    let analysis = analysis_window::connect(&window, &controller, windows);
    let close_analysis = Rc::clone(&analysis);
    let reset_themes = themes.clone();
    let weak = window.as_weak();
    let close_controller = Rc::clone(&controller);
    let close_playlist = Rc::clone(&playlist);
    window.window().on_close_requested(move || {
        let mut controller = close_controller.borrow_mut();
        controller.close();
        close_playlist.borrow_mut().close();
        themes.borrow_mut().close();
        close_analysis.borrow_mut().close();
        if let Some(window) = weak.upgrade() {
            window.set_closing(true);
            window.set_error_message("Stopping playback…".into());
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    let reset_controller = controller.clone();
    let reset_playlist = playlist.clone();
    let reset_analysis = analysis.clone();
    let reset_window = window.as_weak();
    window.on_reset_application(move || {
        if let Some(window) = reset_window.upgrade() {
            reset_controller.borrow_mut().request_reset(&window);
            reset_themes.borrow_mut().close();
            reset_playlist.borrow_mut().close();
            reset_analysis.borrow_mut().close();
        }
    });
    let mut startup_complete = false;
    let startup_failure = Rc::new(Cell::new(None));
    let failure = startup_failure.clone();
    let timer = slint::Timer::default();
    let weak = window.as_weak();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(33),
        move || {
            if let Some(window) = weak.upgrade() {
                controller.borrow_mut().refresh(&window);
                analysis
                    .borrow_mut()
                    .refresh(&controller.borrow().snapshot());
                if !startup_complete && !controller.borrow().startup_pending() {
                    let ready = if window.get_playlist_requested() {
                        playlist.borrow_mut().prepare_startup(&window, &playlist)
                    } else {
                        Ok(true)
                    };
                    match ready {
                        Ok(false) => return,
                        Err(error) => {
                            eprintln!("warn: {error}");
                            window.set_playlist_error(error.into());
                        }
                        Ok(true) => {}
                    }
                    if window.show().is_err() {
                        failure.set(Some("Cannot show desktop window"));
                        let _ = slint::quit_event_loop();
                        return;
                    }
                    if window.get_playlist_requested()
                        && let Err(error) = playlist.borrow_mut().show(&window, &playlist)
                    {
                        eprintln!("warn: {error}");
                        window.set_playlist_error(error.into());
                    }
                    window.set_initializing(false);
                    startup_complete = true;
                }
                playlist.borrow().refresh(&window);
            }
        },
    );
    slint::run_event_loop_until_quit().map_err(|_| "Desktop event loop failed")?;
    startup_failure.take().map_or(Ok(()), Err)
}

/// Submits a UI intent, displaying rejected controls without losing the current session.
fn submit(
    window: &PlayerWindow,
    controller: &Rc<RefCell<presentation::Controller>>,
    command: Command,
) {
    controller.borrow_mut().submit(window, command);
}

#[cfg(test)]
mod drawer_tests;

#[cfg(test)]
mod scrolling_text_tests;
