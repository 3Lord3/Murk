//! Keeping the screen awake.
//!
//! `vo=libmpv` has a side effect that is easy to discover only at minute
//! fourteen of an episode: mpv owns no window, so it cannot talk to the
//! screensaver, and its `stop-screensaver` option does nothing. In a normal
//! player this is free. Here it is the application's job.
//!
//! `gtk_application_inhibit` is the portable route: GTK forwards it to
//! xdg-desktop-portal, which works under Wayland, under X11 and inside Flatpak
//! alike, rather than talking to one specific screensaver's D-Bus name.

use std::sync::atomic::{AtomicU32, Ordering};
use tauri::{AppHandle, Manager};

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

#[cfg(not(target_os = "linux"))]
fn apply(_app: &AppHandle, _inhibited: bool) {}
