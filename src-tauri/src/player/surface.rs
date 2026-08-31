//! Where the video goes.
//!
//! Everything about *driving* mpv is platform-independent, and the render API
//! is identical on every platform mpv supports. What differs is only where the
//! OpenGL context comes from. That difference is this trait, and it is the only
//! thing a port to Windows, macOS or Android has to supply:
//!
//! | platform | implementation |
//! |---|---|
//! | Linux    | `surface_gtk`: `GtkGLArea` under the webview (Wayland and X11) |
//! | Windows  | a WGL context on the WebView2 host window |
//! | macOS    | `NSOpenGLContext`, or Metal interop |
//! | Android  | GLES on a `SurfaceView` |

use anyhow::Result;

pub trait PlayerSurface {
    /// Put the video surface underneath the webview of the given window and
    /// start rendering into it.
    fn attach(&self, window: &tauri::WebviewWindow) -> Result<()>;

    /// Render one frame into `fbo`, sized in device pixels.
    fn render_frame(&self, fbo: i32, width: i32, height: i32) -> Result<()>;

    /// Tear down the render context. Must run while the GL context is still
    /// current; see the note in `surface_gtk`.
    fn detach(&self);
}
