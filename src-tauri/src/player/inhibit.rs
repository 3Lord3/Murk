//! Keeping the screen awake.
//!
//! `vo=libmpv` has a side effect that is easy to discover only at minute
//! fourteen of an episode: mpv owns no window, so it cannot talk to the
//! screensaver, and its `stop-screensaver` option does nothing. In a normal
//! player this is free. Here it is the application's job.
//!
//! `gtk_application_inhibit` is the portable route on Linux: GTK forwards it to
//! xdg-desktop-portal, which works under Wayland, under X11 and inside Flatpak
//! alike, rather than talking to one specific screensaver's D-Bus name.
//!
//! Windows has the same hole for the same reason — mpv's `stop-screensaver`
//! acts on a window mpv would own, and here the window is ours — and
//! `SetThreadExecutionState` is its answer.

use std::sync::atomic::{AtomicU32, Ordering};
use tauri::AppHandle;

/// The inhibit cookie, or 0 for "not inhibiting".
static COOKIE: AtomicU32 = AtomicU32::new(0);

/// Inhibit while playing, release on pause and on stop.
pub fn set_inhibited(app: &AppHandle, inhibited: bool) {
    let app = app.clone();
    // GTK objects are not `Send`, so nothing is captured but the `AppHandle`;
    // the window is looked up again on the main thread.
    let _ = app.clone().run_on_main_thread(move || {
        apply(&app, inhibited);
    });
}

#[cfg(target_os = "linux")]
fn apply(app: &AppHandle, inhibited: bool) {
    use gtk::prelude::*;

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(gtk_window) = window.gtk_window() else {
        return;
    };
    let Some(application) = gtk_window.application() else {
        return;
    };

    let current = COOKIE.load(Ordering::SeqCst);

    if inhibited {
        if current != 0 {
            return;
        }
        let cookie = application.inhibit(
            Some(&gtk_window),
            gtk::ApplicationInhibitFlags::IDLE,
            // Shown by some desktops in their "an app is keeping the screen on"
            // list, so it must not name what is playing.
            Some("Playing"),
        );
        COOKIE.store(cookie, Ordering::SeqCst);
    } else if current != 0 {
        application.uninhibit(current);
        COOKIE.store(0, Ordering::SeqCst);
    }
}

/// Windows: the request is per *thread* and is dropped when that thread ends,
/// so it has to be made on a thread that outlives the episode. `set_inhibited`
/// hands this to the main thread, which does.
///
/// `ES_CONTINUOUS` makes the state stick rather than resetting the idle timer
/// once; `ES_DISPLAY_REQUIRED` covers the screen blanking that is the actual
/// symptom, and `ES_SYSTEM_REQUIRED` the sleep behind it. Releasing is
/// `ES_CONTINUOUS` on its own.
#[cfg(target_os = "windows")]
fn apply(_app: &AppHandle, inhibited: bool) {
    use windows::Win32::System::Power::{
        SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
    };

    let state = if inhibited {
        ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED
    } else {
        ES_CONTINUOUS
    };
    unsafe { SetThreadExecutionState(state) };
    // Nothing to hand back to a later release call, but the flag keeps the
    // meaning of COOKIE the same on both platforms: non-zero means inhibiting.
    COOKIE.store(u32::from(inhibited), Ordering::SeqCst);
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn apply(_app: &AppHandle, _inhibited: bool) {}
