//! Getting OpenGL entry points to mpv, and getting a frame back.
//!
//! libmpv does not link against GL itself and will not look symbols up on its
//! own; it asks the host through a callback. Routing that callback through
//! libepoxy means the same code serves EGL (Wayland) and GLX (X11) with no
//! branch of ours: epoxy already dispatches to whichever is live.
//!
//! # Why libepoxy is opened by hand
//!
//! The obvious route is the `epoxy` crate, but it is unbuildable: its build
//! dependency `gl_generator 0.9` requires `xml-rs 0.7`, and both `xml-rs`
//! releases in that range have been yanked, so the version cannot be resolved
//! at all. What that crate does for us here is two dlsym calls and one
//! constant, so it is done directly. That also drops a build-time dependency
//! on libepoxy's headers: the library is opened at runtime, and GTK 3 already
//! links it, so it is present wherever Murk can run.

use anyhow::{anyhow, Context, Result};
use libloading::Library;
use std::ffi::c_void;
use std::sync::OnceLock;

static EPOXY: OnceLock<Library> = OnceLock::new();

/// `GL_FRAMEBUFFER_BINDING`, from the OpenGL registry.
const GL_FRAMEBUFFER_BINDING: u32 = 0x8CA6;

type GetIntegervFn = unsafe extern "C" fn(u32, *mut i32);
static GET_INTEGERV: OnceLock<GetIntegervFn> = OnceLock::new();

/// The `ctx` value handed to mpv alongside the loader function.
///
/// `OpenGLInitParams::get_proc_address` is a plain `fn` pointer, not a closure,
/// so it cannot capture the loaded library. mpv passes this value back on every
/// call instead; the library itself lives in the static above.
pub struct GlLoader;

/// Open libepoxy and cache the one entry point Murk itself calls.
///
/// Call once, before any GL context exists. The soname is `libepoxy.so.0`
/// across all five target distributions; the unversioned name is tried second
/// only for a development tree that has just the `-devel` symlink.
pub fn init_gl_loader() -> Result<()> {
    if EPOXY.get().is_some() {
        return Ok(());
    }

    let mut errors = Vec::new();
    let library = ["libepoxy.so.0", "libepoxy.so"]
        .iter()
        .find_map(|name| match unsafe { Library::new(*name) } {
            Ok(lib) => Some(lib),
            Err(e) => {
                errors.push(format!("{name}: {e}"));
                None
            }
        })
        .ok_or_else(|| {
            anyhow!(
                "could not load libepoxy ({}). Install libepoxy, see scripts/deps.sh.",
                errors.join("; ")
            )
        })?;

    let library = EPOXY.get_or_init(|| library);

    // Every entry point is a *function-pointer variable* named `epoxy_glFoo`,
    // initially pointing at a resolver stub. Caching the stub before a context
    // exists is fine: calling it later performs the real lookup and forwards.
    let ptr = unsafe { read_dispatch_pointer(library, "epoxy_glGetIntegerv") }
        .context("libepoxy has no usable epoxy_glGetIntegerv")?;
    // SAFETY: the symbol is libepoxy's glGetIntegerv dispatch slot, whose type
    // is fixed by the OpenGL ABI.
    let get_integerv: GetIntegervFn = unsafe { std::mem::transmute(ptr) };
    let _ = GET_INTEGERV.set(get_integerv);

    Ok(())
}

/// Resolve a GL/EGL/GLX entry point for mpv.
///
/// Hardware decoding interop asks for platform functions (`eglGetCurrentDisplay`
/// and friends) as well as core GL, and if they come back null mpv silently
/// drops to software decoding. Going through epoxy's dispatch variables covers
/// all three families with one lookup rule.
/// Read the function pointer *stored in* a libepoxy dispatch variable.
///
/// This needs one dereference more than it looks like it should.
/// `libloading::Symbol<T>` derefs by reinterpreting the symbol's **address**
/// as a `T`:
///
/// ```ignore
/// fn deref(&self) -> &T { &*(&self.pointer as *const *mut _ as *const T) }
/// ```
///
/// which is right for a *function* symbol, where the address is the thing you
/// call. But `epoxy_glFoo` is a **data** symbol (`nm -D libepoxy.so.0` shows it
/// as `D`) holding a pointer to the real entry point. Taking the address and
/// calling it hands the CPU a pointer to a pointer, which segfaults deep inside
/// mpv with no hint as to why. So: take the address, then read through it.
unsafe fn read_dispatch_pointer(library: &Library, symbol: &str) -> Option<*mut c_void> {
    let sym = unsafe { library.get::<*mut c_void>(symbol.as_bytes()) }.ok()?;
    let address = unsafe { sym.try_as_raw_ptr() }?;
    let value = unsafe { *(address as *const *mut c_void) };
    (!value.is_null()).then_some(value)
}

pub fn get_proc_address(_ctx: &GlLoader, name: &str) -> *mut c_void {
    let Some(library) = EPOXY.get() else {
        return std::ptr::null_mut();
    };

    // `epoxy_<name>` is a variable holding the real pointer, so read through it …
    let dispatch = format!("epoxy_{name}");
    if let Some(ptr) = unsafe { read_dispatch_pointer(library, &dispatch) } {
        return ptr;
    }

    // … whereas a plain `<name>`, where libepoxy exports one, is the function
    // itself, so its address is what we want.
    if let Ok(symbol) = unsafe { library.get::<*mut c_void>(name.as_bytes()) } {
        if let Some(ptr) = unsafe { symbol.try_as_raw_ptr() } {
            return ptr;
        }
    }

    std::ptr::null_mut()
}

/// The framebuffer GTK wants us to draw into.
///
/// `GtkGLArea` does not always render to framebuffer 0: with multisampling, or
/// on a compositor that hands out an offscreen target, it binds its own. Asking
/// GL what is currently bound, right after `attach_buffers()`, is the only
/// reliable answer.
pub fn current_framebuffer() -> i32 {
    let mut fbo: i32 = 0;
    if let Some(get_integerv) = GET_INTEGERV.get() {
        unsafe { get_integerv(GL_FRAMEBUFFER_BINDING, &mut fbo) };
    }
    fbo
}

/// Framebuffer size in **device pixels**.
///
/// GTK reports widget geometry in logical units. On a Wayland session with
/// fractional or 2× scaling those differ from the pixels mpv must fill, and
/// passing the logical numbers renders the video at the wrong resolution: soft
/// on a HiDPI screen, and wrong again the moment the window moves to a monitor
/// with a different scale.
pub fn surface_size_px(width: i32, height: i32, scale_factor: i32) -> (i32, i32) {
    let scale = scale_factor.max(1);
    ((width * scale).max(1), (height * scale).max(1))
}

pub fn require_loader() -> Result<()> {
    EPOXY
        .get()
        .map(|_| ())
        .context("GL loader was not initialised before creating the render context")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dispatch variables must be read *through*, not taken by address.
    ///
    /// Getting this wrong resolves a plausible non-null pointer that segfaults
    /// the moment mpv calls it, so a null check proves nothing. This compares
    /// against the symbol's own address, which is precisely the wrong answer.
    #[test]
    fn resolves_entry_points_to_functions_and_not_to_pointer_slots() {
        init_gl_loader().expect("libepoxy should be loadable");
        let library = EPOXY.get().unwrap();

        for name in ["glGetString", "glGetIntegerv", "glClear"] {
            let resolved = get_proc_address(&GlLoader, name);
            assert!(!resolved.is_null(), "{name} did not resolve");

            let slot = unsafe {
                library
                    .get::<*mut c_void>(format!("epoxy_{name}").as_bytes())
                    .ok()
                    .and_then(|s| s.try_as_raw_ptr())
            };
            if let Some(slot) = slot {
                assert_ne!(
                    resolved, slot,
                    "{name} resolved to the address of libepoxy's dispatch variable \
                     instead of the function it holds; calling that segfaults"
                );
            }
        }
    }

    #[test]
    fn surface_size_multiplies_by_the_scale_factor() {
        assert_eq!(surface_size_px(1280, 720, 1), (1280, 720));
        assert_eq!(surface_size_px(1280, 720, 2), (2560, 1440));
        // A zero or negative scale would ask mpv for an empty framebuffer.
        assert_eq!(surface_size_px(1280, 720, 0), (1280, 720));
        assert_eq!(surface_size_px(0, 0, 2), (1, 1));
    }
}
