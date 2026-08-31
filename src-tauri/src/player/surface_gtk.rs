//! Linux: mpv renders into a `GtkGLArea` that sits *under* the webview.
//!
//! # Why the render API and not `--wid`
//!
//! mpv can either create a child window inside the host's window (`wid`) or
//! render into a framebuffer the host provides (`vo=libmpv`). `wid` needs a
//! window id to hand over, and Wayland has no such thing, so `wid` works only
//! on X11, win32 and macOS. Since Wayland is the priority, Murk uses the render
//! API, and gets three other things for free:
//!
//! * compositing stops being a trick with overlapping X windows: `GtkGLArea`
//!   is an ordinary widget and `GtkOverlay` stacks the webview above it through
//!   GTK's normal paint order;
//! * mpv creates no window of its own, so there is no second entry in the task
//!   bar with a filename in it, one metadata leak fewer;
//! * Windows, macOS and Android take the same GL path later, changing only
//!   where the context comes from.

use anyhow::{anyhow, Context, Result};
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use libmpv2::render::{OpenGLInitParams, RenderContext, RenderParam, RenderParamApiType};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

use crate::player::render::{self, GlLoader};
use crate::player::surface::PlayerSurface;

// `gdk_wayland_display_get_wl_display` and `gdk_x11_display_get_xdisplay` live
// in libgdk-3, which gtk-sys already links. There is no gtk-rs crate exposing
// the GTK 3 Wayland backend, so they are declared here rather than pulled in.
unsafe extern "C" {
    fn gdk_wayland_display_get_wl_display(display: *mut gdk::ffi::GdkDisplay) -> *mut c_void;
    fn gdk_x11_display_get_xdisplay(display: *mut gdk::ffi::GdkDisplay) -> *mut c_void;
}

/// Which display server this *process* ended up on.
///
/// Decided at runtime by asking GDK what kind of display it opened, not with
/// `#[cfg]`: one binary has to work in a Wayland session and in an X11 session,
/// and verification step 7 logs back into X11 and runs the same executable.
enum DisplayServer {
    Wayland(*mut c_void),
    X11(*mut c_void),
    Unknown,
}

fn detect_display(display: &gdk::Display) -> DisplayServer {
    let type_name = display.type_().name().to_string();
    let raw = display.as_ptr();
    if type_name.contains("Wayland") {
        let handle = unsafe { gdk_wayland_display_get_wl_display(raw) };
        if !handle.is_null() {
            return DisplayServer::Wayland(handle);
        }
    }
    if type_name.contains("X11") {
        let handle = unsafe { gdk_x11_display_get_xdisplay(raw) };
        if !handle.is_null() {
            return DisplayServer::X11(handle);
        }
    }
    DisplayServer::Unknown
}

/// Holds the render context on the GTK main thread.
///
/// `RenderContext` is neither `Send` nor `Sync` and must be created, used and
/// destroyed on the thread that owns the GL context, so it lives in an `Rc` the
/// signal handlers share, and nowhere else.
type SharedContext = Rc<RefCell<Option<RenderContext<'static>>>>;

pub struct GtkSurface {
    mpv: &'static libmpv2::Mpv,
}

impl GtkSurface {
    pub fn new(mpv: &'static libmpv2::Mpv) -> Self {
        Self { mpv }
    }
}

impl PlayerSurface for GtkSurface {
    fn attach(&self, window: &tauri::WebviewWindow) -> Result<()> {
        render::require_loader()?;
        attach_gl_area(window, self.mpv)
    }

    fn render_frame(&self, _fbo: i32, _width: i32, _height: i32) -> Result<()> {
        // On GTK the render call is driven by the widget's own `render` signal;
        // there is nothing for an outside caller to do.
        Ok(())
    }

    fn detach(&self) {}
}

/// Rebuild the window's widget tree so the video sits under the webview.
///
/// # The shape of the tree is not free
///
/// Tauri implements resizing for undecorated windows by attaching a
/// button-press handler to the webview, and that handler assumes a specific
/// hierarchy (`tauri-runtime-wry/src/undecorated_resizing.rs`):
///
/// ```ignore
/// if let Some(window) = webview.parent().and_then(|w| w.parent()) {
///   let window: gtk::Window = window.downcast().unwrap();
/// ```
///
/// The webview's **grandparent must be the window**. Inserting the overlay
/// between the window and Tauri's default vbox makes the grandparent a
/// `GtkOverlay`, the downcast fails, and because the unwrap runs inside a GTK
/// signal trampoline (an `extern "C"` frame that cannot unwind) the panic
/// aborts the process on the first click.
///
/// So the overlay goes directly under the window and the webview directly
/// under the overlay:
///
/// ```text
/// GtkApplicationWindow
///   └── GtkOverlay
///         ├── GtkGLArea    (base layer: mpv renders here)
///         └── WebKitWebView (overlay layer: the interface)
/// ```
///
/// Tauri's default vbox is emptied and left out of the tree. It exists to hold
/// a menu bar, which Murk does not have; tao keeps its own reference to it, so
/// dropping ours does not free it.
fn attach_gl_area(window: &tauri::WebviewWindow, mpv: &'static libmpv2::Mpv) -> Result<()> {
    let gtk_window = window
        .gtk_window()
        .context("no GTK window behind the webview")?;
    let vbox = window
        .default_vbox()
        .context("no default vbox in the window")?;

    // The title is fixed here and never touched again. A player that renames
    // its window per episode announces the episode in the task bar, the window
    // switcher and the screenshot the user posts later.
    gtk_window.set_title(crate::privacy::leaks::WINDOW_TITLE);
    harden_recent_files();

    let gl_area = gtk::GLArea::new();
    gl_area.set_hexpand(true);
    gl_area.set_vexpand(true);
    // No depth or stencil: mpv draws a flat image, and allocating them per
    // frame costs memory bandwidth on integrated GPUs for nothing.
    gl_area.set_has_depth_buffer(false);
    gl_area.set_has_stencil_buffer(false);

    // The signal handlers must be connected *before* the widget is shown.
    // `show_all()` realizes the GLArea synchronously, and a `realize` handler
    // attached afterwards is never called: the render context is never built
    // and the video never appears, with nothing in the log to say so.
    let context: SharedContext = Rc::new(RefCell::new(None));
    connect_realize(&gl_area, mpv, Rc::clone(&context));
    connect_render(&gl_area, Rc::clone(&context));
    connect_unrealize(&gl_area, context);

    let overlay = gtk::Overlay::new();
    overlay.add(&gl_area);

    // Move the webview (and anything else tao put in the vbox) up one level,
    // out of the vbox and straight into the overlay, so that the parent chain
    // Tauri's resize handler walks stays two deep.
    gtk_window.remove(&vbox);
    for child in vbox.children() {
        vbox.remove(&child);
        overlay.add_overlay(&child);
        // The interface has to keep receiving clicks; it is kept from painting
        // over the video on the CSS side instead (see the `/watch` route).
        overlay.set_overlay_pass_through(&child, false);
    }

    gtk_window.add(&overlay);
    gtk_window.show_all();

    tracing::debug!(
        realized = gl_area.is_realized(),
        "video surface attached under the webview"
    );

    Ok(())
}

fn connect_realize(gl_area: &gtk::GLArea, mpv: &'static libmpv2::Mpv, context: SharedContext) {
    gl_area.connect_realize(move |area| {
        area.make_current();
        if let Some(error) = area.error() {
            tracing::error!("GLArea failed to create a context: {error}");
            return;
        }

        match create_render_context(area, mpv) {
            Ok(mut ctx) => {
                // Called from mpv's render thread, where touching the mpv API
                // is forbidden. All it may do is wake the main loop, which then
                // asks the widget to redraw.
                let area_weak = glib::SendWeakRef::from(area.downgrade());
                ctx.set_update_callback(move || {
                    let area_weak = area_weak.clone();
                    glib::MainContext::default().invoke(move || {
                        if let Some(area) = area_weak.upgrade() {
                            area.queue_render();
                        }
                    });
                });
                *context.borrow_mut() = Some(ctx);
                tracing::info!("mpv render context created");
            }
            Err(e) => tracing::error!("could not create the mpv render context: {e}"),
        }
    });
}

fn create_render_context(
    area: &gtk::GLArea,
    mpv: &'static libmpv2::Mpv,
) -> Result<RenderContext<'static>> {
    let display = area.display();

    let mut params = vec![
        RenderParam::ApiType(RenderParamApiType::OpenGl),
        RenderParam::InitParams(OpenGLInitParams {
            get_proc_address: render::get_proc_address,
            ctx: GlLoader,
        }),
    ];

    // Handing mpv the native display is what lets hardware decoding share
    // buffers with our GL context. Without it `hwdec=auto-safe` finds no
    // interop and falls back to software decoding, which melts a laptop on
    // 4K HEVC.
    match detect_display(&display) {
        DisplayServer::Wayland(handle) => {
            tracing::info!("wayland session: passing wl_display to mpv");
            params.push(RenderParam::WaylandDisplay(handle));
        }
        DisplayServer::X11(handle) => {
            tracing::info!("x11 session: passing Display* to mpv");
            params.push(RenderParam::X11Display(handle));
        }
        DisplayServer::Unknown => {
            tracing::warn!(
                "unrecognised display backend; continuing without hardware decoding interop"
            );
        }
    }

    tracing::debug!("calling mpv_render_context_create");
    let ctx = mpv
        .create_render_context(params)
        .map_err(|e| anyhow!("mpv_render_context_create failed: {e}"))?;
    tracing::debug!("mpv_render_context_create returned");
    Ok(ctx)
}

fn connect_render(gl_area: &gtk::GLArea, context: SharedContext) {
    gl_area.connect_render(move |area, _| {
        let borrowed = context.borrow();
        let Some(ctx) = borrowed.as_ref() else {
            return glib::Propagation::Proceed;
        };

        // GTK may render into its own framebuffer rather than 0.
        area.attach_buffers();
        let fbo = render::current_framebuffer();

        let (width, height) = render::surface_size_px(
            area.allocated_width(),
            area.allocated_height(),
            area.scale_factor(),
        );

        // `flip = true`: OpenGL's Y axis points up, video's points down.
        if let Err(e) = ctx.render::<GlLoader>(fbo, width, height, true) {
            tracing::warn!("mpv render failed: {e}");
        }
        ctx.report_swap();

        glib::Propagation::Proceed
    });
}

fn connect_unrealize(gl_area: &gtk::GLArea, context: SharedContext) {
    gl_area.connect_unrealize(move |area| {
        // Order matters: `mpv_render_context_free` deletes GL objects, so the
        // context has to still be alive and current. Dropping this at process
        // exit instead would free them against a context that is already gone.
        area.make_current();
        let taken = context.borrow_mut().take();
        drop(taken);
        tracing::info!("mpv render context destroyed");
    });
}

/// Stop GTK from recording the folders the user opens.
///
/// The folder picker would otherwise write every chosen path into
/// `recently-used.xbel`, where the desktop's launcher will show the series name
/// back to the user, outside Murk, where no hiding profile applies.
pub fn harden_recent_files() {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    let object: &glib::Object = settings.upcast_ref();

    // These properties have come and gone across GTK 3 point releases, so each
    // is set only if this build actually has it.
    for (name, value) in [
        ("gtk-recent-files-enabled", false.to_value()),
        ("gtk-recent-files-max-age", 0i32.to_value()),
        ("gtk-recent-files-limit", 0i32.to_value()),
    ] {
        if object.find_property(name).is_some() {
            object.set_property(name, &value);
        }
    }
}
