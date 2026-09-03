//! Murk, a player for watching without spoilers.
//!
//! Hiding happens at the IPC boundary, not in CSS: hidden fields are absent
//! from the JSON rather than blanked out in the DOM, so a layout bug, an open
//! devtools window or a careless `v-if` cannot reveal them.

pub mod commands;
pub mod library;
pub mod player;
pub mod privacy;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Manager, RunEvent};

use library::{poster, Library};
use player::PlayerHandle;
use privacy::HidingProfile;

pub struct AppState {
    pub player: PlayerHandle,
    pub library: Library,
    /// Where covers end up: either extracted from episode files or copied from
    /// the user's pick. Local posters stay in the series folder.
    pub covers_dir: std::path::PathBuf,
    /// Rendered poster data URLs, so a library refresh does not re-read and
    /// re-encode every cover from disk each time.
    pub poster_cache: Mutex<poster::DataUrlCache>,
    /// The episode queued by auto-advance, waiting for the countdown to finish
    /// or be cancelled.
    pub pending_next: Mutex<Option<i64>>,
    pub shutdown: Arc<AtomicBool>,
}

/// Fix the window identity that window managers match against desktop files.
///
/// On X11 GTK builds the WM_CLASS out of `g_get_prgname()` (instance) and its
/// capitalized form (class); on Wayland GDK uses the same prgname as the xdg
/// toplevel app_id. The default value of `g_get_prgname()` is the basename of
/// `argv[0]`, which is only reliable when the binary is launched by that exact
/// path. Making it explicit keeps the window matching the `.desktop` files
/// (`StartupWMClass=murk`, both the ones Tauri generates and the Flatpak one)
/// no matter how the app was started: a symlink, `cargo run`, an AppImage.
#[cfg(target_os = "linux")]
fn set_wm_identity() {
    gtk::glib::set_prgname(Some("murk"));
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    // Paths are logged at `trace` only, which is off unless someone asks for
    // it: a debug log printing filenames would undo the point of the program.
    let filter = EnvFilter::try_from_env("MURK_LOG").unwrap_or_else(|_| EnvFilter::new("debug"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

/// Keep one known-benign GLib critical out of the console.
///
/// GTK's portal file chooser, GtkRecentManager and WebKitGTK all pass a NULL
/// string into `g_variant_new_string` at unpredictable moments (opening the
/// folder picker, a webview navigation, …). The assertion only logs and
/// returns, but it prints on every run and reads like a crash. It is not our
/// code, so this installs a GLib critical handler that swallows exactly that
/// one message and forwards everything else to the default handler.
///
/// Must run before the first offender does; there is no ordering constraint
/// with `init_tracing`, only with the GTK main loop.
#[cfg(target_os = "linux")]
fn silence_benign_glib_critical() {
    use gtk::glib;
    use std::ffi::{c_char, c_void, CStr};

    unsafe extern "C" fn filter(
        domain: *const c_char,
        level: glib::ffi::GLogLevelFlags,
        message: *const c_char,
        _user_data: *mut c_void,
    ) {
        let is_known_noise = if message.is_null() {
            false
        } else {
            CStr::from_ptr(message)
                .to_str()
                .is_ok_and(|m| m.contains("g_variant_new_string") && m.contains("string != NULL"))
        };
        if !is_known_noise {
            glib::ffi::g_log_default_handler(domain, level, message, std::ptr::null_mut());
        }
    }

    unsafe {
        glib::ffi::g_log_set_handler(
            c"GLib".as_ptr(),
            glib::ffi::G_LOG_LEVEL_CRITICAL,
            Some(filter),
            std::ptr::null_mut(),
        );
    }
}

/// `embed_window` is the native handle mpv should render into, on platforms
/// where mpv owns the drawing; see [`player::PlayerHandle::new`]. It has to be
/// known here, before the mpv instance is created, which is why the video
/// window is made first and the surface only attached afterwards.
fn build_state(app: &tauri::App, embed_window: Option<i64>) -> Result<AppState> {
    let data_dir = app
        .path()
        .app_data_dir()
        .context("no application data directory")?;
    let library = Library::open(&data_dir.join("library.sqlite3"))?;

    let covers_dir = data_dir.join("covers");
    std::fs::create_dir_all(&covers_dir).context("creating the covers directory")?;

    let profile = library
        .get_setting("hiding_profile")
        .ok()
        .flatten()
        .and_then(|id| HidingProfile::preset(&id))
        // Standard: everything hidden, peeking available.
        .unwrap_or_default();
    tracing::info!("hiding profile: {}", profile.id);

    tracing::debug!(?embed_window, "creating mpv player");
    let player = PlayerHandle::new(profile, embed_window)?;
    tracing::info!("mpv player created");

    Ok(AppState {
        player,
        library,
        covers_dir,
        poster_cache: Mutex::new(poster::DataUrlCache::new()),
        pending_next: Mutex::new(None),
        shutdown: Arc::new(AtomicBool::new(false)),
    })
}

pub fn run() {
    // Before gtk is initialised inside `Builder::build`, so the WM_CLASS and
    // the Wayland app_id are derived from the intended program name.
    #[cfg(target_os = "linux")]
    set_wm_identity();

    init_tracing();
    tracing::info!("murk starting");
    // Before the GTK main loop exists, so no message can slip past the filter.
    #[cfg(target_os = "linux")]
    silence_benign_glib_critical();

    // libmpv resolves GL entry points through a callback, so the loader has to
    // be standing before any render context is created. Windows hands mpv a
    // window instead of a framebuffer, so there is no loader to stand up.
    #[cfg(target_os = "linux")]
    if let Err(e) = player::render::init_gl_loader() {
        eprintln!("Murk: {e:#}");
        std::process::exit(1);
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let window = app
                .get_webview_window("main")
                .context("the main window is missing from tauri.conf.json")?;
            window.set_title(privacy::leaks::WINDOW_TITLE)?;

            // Windows only: the video child window must exist before mpv does,
            // because its handle is the `wid` mpv is initialised with.
            #[cfg(target_os = "windows")]
            let win_surface = player::surface_win::WinSurface::new(&window)?;
            #[cfg(target_os = "windows")]
            let embed_window = Some(win_surface.hwnd());
            #[cfg(not(target_os = "windows"))]
            let embed_window = None;

            tracing::info!(?embed_window, "main window ready, building player state");
            let state = build_state(app, embed_window)?;
            let shutdown = Arc::clone(&state.shutdown);
            app.manage(state);

            #[cfg(target_os = "linux")]
            {
                use player::surface::PlayerSurface;
                let mpv = app.state::<AppState>().player.mpv();
                let surface = player::surface_gtk::GtkSurface::new(mpv);
                surface.attach(&window)?;
            }

            #[cfg(target_os = "windows")]
            {
                use player::surface::PlayerSurface;
                tracing::debug!("attaching win32 video surface");
                win_surface.attach(&window)?;
                // Managed rather than leaked: the surface has to outlive setup
                // because it owns the resize hook, and handing it to Tauri
                // keeps `detach` reachable for a caller that wants to stop the
                // video before the window goes away.
                app.manage(win_surface);
            }

            // This thread owns the only `wait_event` call in the process.
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("murk-mpv-events".into())
                .spawn(move || player::events::run(handle, shutdown))?;

            tracing::info!("setup complete");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_playback,
            commands::play_pause,
            commands::seek_relative,
            commands::set_volume,
            commands::set_track,
            commands::stop,
            commands::toggle_fullscreen,
            commands::set_fullscreen,
            commands::list_series,
            commands::add_series,
            commands::rescan_series,
            commands::remove_series,
            commands::reset_progress,
            commands::set_series_poster,
            commands::clear_series_poster,
            commands::continue_series,
            commands::play_next,
            commands::cancel_next,
            commands::get_profile,
            commands::list_profiles,
            commands::set_profile,
            commands::peek_remaining,
            commands::can_finish_within,
            commands::peek_episode_identity,
            commands::watched_fraction,
            commands::system_languages,
            commands::get_locale,
            commands::set_locale,
        ])
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(e) => {
            eprintln!("Murk failed to start: {e:#}");
            std::process::exit(1);
        }
    };

    app.run(|handle, event| {
        if let RunEvent::ExitRequested { .. } = event {
            player::events::save_progress(handle, false);
            handle
                .state::<AppState>()
                .shutdown
                .store(true, Ordering::SeqCst);
        }
    });
}
