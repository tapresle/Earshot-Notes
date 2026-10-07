//! The main window and the Dock icon. Closing the window hides it, and
//! Earshot Notes keeps running in the menu bar without a Dock icon until the
//! window opens again.

use tauri::{AppHandle, Manager, Runtime, Window};

const MAIN: &str = "main";

/// Shows the main window and the Dock icon, and brings them to the front.
pub fn show<R: Runtime>(app: &AppHandle<R>) {
    on_main_thread(app, macos_audio::show_dock_icon);
    restore_dev_icon();
    let Some(window) = app.get_webview_window(MAIN) else {
        log::error!("the main window is missing");
        return;
    };
    let result = window
        .show()
        .and_then(|()| window.unminimize())
        .and_then(|()| window.set_focus());
    if let Err(error) = result {
        log::warn!("couldn't show the window: {error}");
    }
}

/// A development build isn't bundled, so Tauri sets its Dock icon once at
/// startup, and macOS shows a generic "exec" icon whenever the Dock icon comes
/// back. Setting it again fixes that. A bundled app's icon comes from its bundle.
#[cfg(dev)]
fn restore_dev_icon() {
    if let Err(error) = macos_audio::set_app_icon(include_bytes!("../icons/icon.png")) {
        log::warn!("couldn't set the development Dock icon: {error}");
    }
}

#[cfg(not(dev))]
fn restore_dev_icon() {}

/// Hides a window instead of closing it, and hides the Dock icon. The page
/// stays loaded, so it keeps receiving events and opens again where it was.
pub fn hide<R: Runtime>(window: &Window<R>) {
    if let Err(error) = window.hide() {
        log::warn!("couldn't hide the window: {error}");
    }
    hide_dock_icon(window.app_handle());
}

/// Hides the Dock icon and app switcher entry, leaving the menu bar icon.
/// The app steps aside first, so macOS switches to the next app as usual
/// instead of redrawing the whole menu bar.
pub fn hide_dock_icon<R: Runtime>(app: &AppHandle<R>) {
    on_main_thread(app, macos_audio::hide_dock_icon);
}

/// AppKit calls must run on the main thread; this keeps their order too.
fn on_main_thread<R: Runtime>(app: &AppHandle<R>, work: fn()) {
    if let Err(error) = app.run_on_main_thread(work) {
        log::warn!("couldn't change the Dock icon: {error}");
    }
}
