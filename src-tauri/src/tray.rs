//! The menu bar icon: shows whether Earshot Notes is recording, and starts or
//! stops a recording, opens the window, or quits.

use std::sync::{Arc, Mutex};

use tauri::image::Image;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{App, AppHandle, Manager, Wry};

use crate::commands::recording;
use crate::recording::{Recorder, SourceProblem};
use crate::store::{Source, Store};
use crate::window;

const TOGGLE: &str = "toggle-recording";
const OPEN: &str = "open-window";
const QUIT: &str = "quit";

const IDLE_ICON: Image<'static> = tauri::include_image!("icons/tray/idle.png");
const RECORDING_ICON: Image<'static> = tauri::include_image!("icons/tray/recording.png");

/// What the menu bar icon reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Idle,
    /// Recording, without the sources in `missing`, which say why each one couldn't start.
    Recording {
        missing: Vec<SourceProblem>,
    },
    /// The last attempt to start recording from the menu bar failed.
    Failed(String),
}

/// The text and icon the menu shows for a status.
#[derive(Debug, PartialEq)]
struct View {
    status: String,
    /// Why something is missing or failed, on its own line under the status.
    detail: Option<String>,
    toggle: &'static str,
    recording: bool,
}

/// Messages read as sentence fragments elsewhere in the UI; the menu shows them as sentences.
fn sentence(message: &str) -> String {
    let mut chars = message.chars();
    let first = chars
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    let mut text = first + chars.as_str();
    if !text.ends_with('.') {
        text.push('.');
    }
    text
}

fn view(status: &Status) -> View {
    match status {
        Status::Idle => View {
            status: "Not recording".into(),
            detail: None,
            toggle: "Start recording",
            recording: false,
        },
        Status::Recording { missing } => {
            let lacks = |source| missing.iter().any(|p: &SourceProblem| p.source == source);
            View {
                status: if lacks(Source::Them) {
                    "Recording your microphone only".into()
                } else if lacks(Source::Me) {
                    "Recording system audio only".into()
                } else {
                    "Recording".into()
                },
                detail: (!missing.is_empty()).then(|| {
                    missing
                        .iter()
                        .map(|p| sentence(&p.message))
                        .collect::<Vec<_>>()
                        .join(" ")
                }),
                toggle: "Stop recording",
                recording: true,
            }
        }
        Status::Failed(message) => View {
            status: "Couldn't start recording".into(),
            detail: Some(sentence(message)),
            toggle: "Start recording",
            recording: false,
        },
    }
}

/// The menu bar icon and the menu items that change.
struct Tray {
    icon: TrayIcon,
    menu: Menu<Wry>,
    status: MenuItem<Wry>,
    /// Shown under the status only when there's something to explain.
    detail: MenuItem<Wry>,
    detail_shown: bool,
    toggle: MenuItem<Wry>,
}

/// Adds the menu bar icon. Call once, during setup.
pub fn create(app: &App) -> tauri::Result<()> {
    let initial = view(&Status::Idle);
    let status = MenuItem::with_id(app, "status", &initial.status, false, None::<&str>)?;
    let detail = MenuItem::with_id(app, "detail", "", false, None::<&str>)?;
    let toggle = MenuItem::with_id(app, TOGGLE, initial.toggle, true, None::<&str>)?;
    let open = MenuItem::with_id(app, OPEN, "Open Earshot Notes", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, "Quit Earshot Notes", true, Some("CmdOrCtrl+Q"))?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &toggle,
            &open,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let icon = TrayIconBuilder::with_id("main")
        .icon(IDLE_ICON)
        .icon_as_template(true)
        .tooltip("Earshot Notes")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event)
        .build(app)?;
    app.manage(Mutex::new(Tray {
        icon,
        menu,
        status,
        detail,
        detail_shown: false,
        toggle,
    }));
    Ok(())
}

/// Shows a recording status in the menu bar. Safe to call from any thread.
pub fn show_status(app: &AppHandle, status: Status) {
    let Some(tray) = app.try_state::<Mutex<Tray>>() else {
        return;
    };
    let mut tray = tray.lock().unwrap_or_else(|e| e.into_inner());
    let view = view(&status);
    if let Err(error) = tray.show(&view) {
        log::warn!("couldn't update the menu bar icon: {error}");
    }
}

impl Tray {
    fn show(&mut self, view: &View) -> tauri::Result<()> {
        self.status.set_text(&view.status)?;
        match (&view.detail, self.detail_shown) {
            (Some(detail), _) => {
                self.detail.set_text(detail)?;
                if !self.detail_shown {
                    self.menu.insert(&self.detail, 1)?;
                    self.detail_shown = true;
                }
            }
            (None, true) => {
                self.menu.remove(&self.detail)?;
                self.detail_shown = false;
            }
            (None, false) => {}
        }
        self.toggle.set_text(view.toggle)?;
        let icon = if view.recording {
            RECORDING_ICON
        } else {
            IDLE_ICON
        };
        self.icon.set_icon(Some(icon))?;
        // Setting the icon resets the template flag.
        self.icon.set_icon_as_template(true)
    }
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        TOGGLE => {
            let app = app.clone();
            // Starting can wait on a permission prompt, and stopping on the
            // audio pipelines, so neither runs on the main thread.
            tauri::async_runtime::spawn_blocking(move || toggle_recording(&app));
        }
        OPEN => window::show(app),
        // Exiting stops any recording first (see `RunEvent::Exit` in lib.rs).
        QUIT => app.exit(0),
        _ => {}
    }
}

fn toggle_recording(app: &AppHandle) {
    if app.state::<Arc<Recorder>>().active_meeting().is_some() {
        if let Err(error) = recording::stop(app) {
            log::error!(
                "couldn't stop recording from the menu bar: {}",
                error.message
            );
        }
        return;
    }
    if let Err(message) = record_new_meeting(app) {
        log::error!("couldn't start recording from the menu bar: {message}");
        show_status(app, Status::Failed(message));
    }
}

/// Creates an untitled meeting and records it. A meeting that couldn't start
/// recording is deleted again, so a failed attempt leaves nothing behind.
fn record_new_meeting(app: &AppHandle) -> Result<(), String> {
    let store = app.state::<Arc<Store>>();
    let meeting = store.create_meeting("").map_err(|e| e.to_string())?;
    if let Err(error) = recording::start(app, meeting.id) {
        if let Err(cleanup) = store.delete_meeting(meeting.id) {
            log::warn!("couldn't delete the meeting that failed to record: {cleanup}");
        }
        return Err(error.message);
    }
    log::info!("started recording meeting {} from the menu bar", meeting.id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(source: Source, message: &str) -> SourceProblem {
        SourceProblem {
            source,
            message: message.into(),
        }
    }

    #[test]
    fn offers_to_start_when_idle() {
        assert_eq!(
            view(&Status::Idle),
            View {
                status: "Not recording".into(),
                detail: None,
                toggle: "Start recording",
                recording: false,
            }
        );
    }

    #[test]
    fn offers_to_stop_and_shows_the_recording_icon_while_recording() {
        let both = view(&Status::Recording { missing: vec![] });
        assert_eq!(
            (
                both.status.as_str(),
                both.detail,
                both.toggle,
                both.recording
            ),
            ("Recording", None, "Stop recording", true)
        );
    }

    #[test]
    fn says_which_side_is_missing_and_why() {
        let without_system = view(&Status::Recording {
            missing: vec![problem(
                Source::Them,
                "system audio recording is off, so only your side is transcribed",
            )],
        });
        assert_eq!(without_system.status, "Recording your microphone only");
        assert_eq!(
            without_system.detail.as_deref(),
            Some("System audio recording is off, so only your side is transcribed.")
        );
        let without_mic = view(&Status::Recording {
            missing: vec![problem(Source::Me, "no microphone is available")],
        });
        assert_eq!(without_mic.status, "Recording system audio only");
        assert_eq!(
            without_mic.detail.as_deref(),
            Some("No microphone is available.")
        );
    }

    #[test]
    fn explains_a_failed_start_and_offers_to_try_again() {
        let failed = view(&Status::Failed("microphone access is off".into()));
        assert_eq!(failed.status, "Couldn't start recording");
        assert_eq!(failed.detail.as_deref(), Some("Microphone access is off."));
        assert_eq!(
            (failed.toggle, failed.recording),
            ("Start recording", false)
        );
    }
}
