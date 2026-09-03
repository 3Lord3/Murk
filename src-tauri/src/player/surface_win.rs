//! Windows: mpv draws into a child window that sits *under* the WebView2.
//!
//! # Why `--wid` here and the render API on Linux
//!
//! The Linux path exists because Wayland has no window id to hand over, so
//! there mpv has to give us frames and `surface_gtk` composites them itself
//! (see that module for the full argument). Win32 does have window ids, and
//! handing one over buys back everything the GL path costs:
//!
//! * no GL context of ours to create, make current, resize and destroy, and no
//!   frame callback on the UI thread — mpv renders on its own thread with its
//!   own D3D11 swap chain, so a slow frame cannot stall the interface;
//! * `hwdec=auto-safe` reaches d3d11va without an interop dance;
//! * libepoxy, which is a GTK dependency, is not needed at all.
//!
//! The leak that `wid` avoids on X11 — a second taskbar entry carrying a
//! filename — does not arise: a `WS_CHILD` window is never in the taskbar and
//! has no title of its own.
//!
//! # The stacking
//!
//! ```text
//! tao top-level window
//!   ├── MurkVideo   (WS_CHILD, bottom of the z-order: mpv renders here)
//!   └── WebView2    (above it, transparent background: the interface)
//! ```
//!
//! This is why `app.windows[0].transparent` in `tauri.conf.json` is not
//! decorative: an opaque webview would hide the video completely. The child is
//! created `WS_DISABLED`, so it swallows no input — every click and key stroke
//! belongs to the Vue layer, exactly as on Linux.

use anyhow::{anyhow, Context, Result};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, RegisterClassW, SetWindowPos, CS_HREDRAW,
    CS_VREDRAW, HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WINDOW_EX_STYLE, WNDCLASSW,
    WS_CHILD, WS_CLIPSIBLINGS, WS_DISABLED, WS_VISIBLE,
};

use crate::player::surface::PlayerSurface;

/// The window class is registered once per process; a second `RegisterClassW`
/// with the same name fails, and there is only ever one video window anyway.
static CLASS_REGISTERED: std::sync::Once = std::sync::Once::new();

const CLASS_NAME: windows::core::PCWSTR = w!("MurkVideo");

/// A child window handle that can be moved across threads.
///
/// `HWND` is deliberately not `Send`, because most of the API around it wants
/// the owning thread. The calls this module makes afterwards — `SetWindowPos`,
/// `GetClientRect` — are documented as callable from any thread, so the handle
/// travels as a bare `isize` and is rebuilt at the point of use.
#[derive(Clone, Copy)]
struct WindowHandle(isize);

impl WindowHandle {
    fn hwnd(self) -> HWND {
        HWND(self.0 as *mut std::ffi::c_void)
    }
}

pub struct WinSurface {
    video: WindowHandle,
    parent: WindowHandle,
    /// Cleared on `detach`, so a resize event arriving during shutdown does not
    /// move a window that is being destroyed.
    live: Arc<AtomicIsize>,
}

impl WinSurface {
    /// Create the video child window inside `window`.
    ///
    /// Must run **before** the mpv instance exists: [`hwnd`](Self::hwnd) is the
    /// `wid` mpv is initialised with, and mpv reads that option only while
    /// creating its video output.
    pub fn new(window: &tauri::WebviewWindow) -> Result<Self> {
        // Tauri returns an `HWND` from its own version of the `windows` crate,
        // which need not be the version this crate resolves. The raw pointer
        // inside it is the ABI, so it crosses that boundary and the handle is
        // rebuilt here.
        let parent = window.hwnd().context("no HWND behind the webview")?;
        let parent = WindowHandle(parent.0 as isize);

        register_class()?;

        let instance = unsafe { GetModuleHandleW(None) }.context("GetModuleHandleW failed")?;
        let (width, height) = client_size(parent);

        // WS_CLIPSIBLINGS keeps the child from painting over the webview that
        // is stacked above it; without it a video frame can flash on top of the
        // interface during a resize.
        let video = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                CLASS_NAME,
                // No title: nothing here should ever be able to name a file.
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_DISABLED | WS_CLIPSIBLINGS,
                0,
                0,
                width,
                height,
                Some(parent.hwnd()),
                None,
                Some(instance.into()),
                None,
            )
        }
        .context("creating the video child window failed")?;

        let video = WindowHandle(video.0 as isize);
        stack_below_webview(video);

        Ok(Self {
            video,
            parent,
            live: Arc::new(AtomicIsize::new(video.0)),
        })
    }

    /// The value to pass to mpv as `wid`.
    pub fn hwnd(&self) -> i64 {
        self.video.0 as i64
    }
}

impl PlayerSurface for WinSurface {
    fn attach(&self, window: &tauri::WebviewWindow) -> Result<()> {
        // The window already exists and mpv is already drawing into it by the
        // time this runs; what is left is keeping it the size of the client
        // area. tao does not resize a child window it does not know about, and
        // mpv resizes its swap chain from the window, not the other way round,
        // so an unhandled resize leaves the video at its startup size.
        let parent = self.parent;
        let live = Arc::clone(&self.live);
        window.on_window_event(move |event| match event {
            tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. } => {
                let handle = live.load(Ordering::SeqCst);
                if handle != 0 {
                    fit_to_parent(WindowHandle(handle), parent);
                }
            }
            // The window and its children are gone; anything still holding the
            // handle must stop using it. This is the same guard `detach` sets,
            // reached from the event side: whichever happens first wins and the
            // other is a no-op.
            tauri::WindowEvent::Destroyed => live.store(0, Ordering::SeqCst),
            _ => {}
        });
        Ok(())
    }

    fn render_frame(&self, _fbo: i32, _width: i32, _height: i32) -> Result<()> {
        // mpv presents on its own schedule here; there is no frame for an
        // outside caller to drive.
        Ok(())
    }

    fn detach(&self) {
        // The window is destroyed with its parent at shutdown. Forgetting the
        // handle first is what stops a late resize event from touching it; the
        // `Destroyed` branch above does the same thing when shutdown comes from
        // the window rather than from a caller.
        self.live.store(0, Ordering::SeqCst);
    }
}

fn register_class() -> Result<()> {
    let mut result = Ok(());
    CLASS_REGISTERED.call_once(|| {
        result = (|| {
            let instance = unsafe { GetModuleHandleW(None) }.context("GetModuleHandleW failed")?;
            let class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance.into(),
                lpszClassName: CLASS_NAME,
                // Black, not the default: the frame mpv has not drawn yet, and
                // the letterbox bars around a 2.39:1 film, are both this brush.
                hbrBackground: HBRUSH(unsafe { GetStockObject(BLACK_BRUSH) }.0),
                ..Default::default()
            };
            if unsafe { RegisterClassW(&class) } == 0 {
                // `from_win32` is `GetLastError`, which is what RegisterClassW
                // leaves behind when it returns the zero atom.
                return Err(anyhow!(
                    "registering the video window class failed: {}",
                    windows::core::Error::from_win32()
                ));
            }
            Ok(())
        })();
    });
    result
}

/// Nothing of ours happens in this window: mpv owns every pixel of it and the
/// window is disabled, so no input arrives either.
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn client_size(window: WindowHandle) -> (i32, i32) {
    let mut rect = RECT::default();
    if unsafe { GetClientRect(window.hwnd(), &mut rect) }.is_err() {
        return (1, 1);
    }
    (
        (rect.right - rect.left).max(1),
        (rect.bottom - rect.top).max(1),
    )
}

fn fit_to_parent(video: WindowHandle, parent: WindowHandle) {
    let (width, height) = client_size(parent);
    let _ = unsafe {
        SetWindowPos(
            video.hwnd(),
            Some(HWND_BOTTOM),
            0,
            0,
            width,
            height,
            SWP_NOACTIVATE,
        )
    };
}

/// Put the video window at the bottom of its siblings, i.e. under the webview.
///
/// A freshly created child goes to the *top* of the z-order, which would hide
/// the interface behind an opaque video surface.
fn stack_below_webview(video: WindowHandle) {
    let _ = unsafe {
        SetWindowPos(
            video.hwnd(),
            Some(HWND_BOTTOM),
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        )
    };
}
