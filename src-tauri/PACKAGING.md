# Packaging notes

## Dependencies are declared, not derived

The plan for this project assumed that leaving `bundle.linux.*.depends` empty
would let `rpmbuild` and `dpkg-shlibdeps` derive `libmpv.so.2` from the ELF.
**That is not what Tauri does.** Tauri 2.11's bundler writes a fixed dependency
list of its own (`libwebkit2gtk-4.1-0`, `libgtk-3-0`) plus whatever `depends`
contains, and never invokes either tool. Verified by building and inspecting:

```
$ rpm -qpR Murk-0.1.0-1.x86_64.rpm
libwebkit2gtk-4.1.so.0()(64bit)
libgtk-3.so.0()(64bit)
rpmlib(...)
```

No libmpv. A package built that way installs cleanly and then fails to start.

## What is declared instead

```jsonc
"rpm":  { "depends": ["libmpv.so.2()(64bit)", "libepoxy.so.0()(64bit)"] },
"deb":  { "depends": ["libmpv2", "libepoxy0"] }
```

The **rpm** entries are sonames, not package names: rpm generates `Provides:`
from sonames automatically, so Fedora's `mpv-libs` satisfies
`libmpv.so.2()(64bit)` without the package ever naming it.

```
$ rpm -q --provides mpv-libs   # Fedora
libmpv.so.2()(64bit)
```

The rpm channel targets **Fedora only**. ALT was a target earlier, on the
assumption that soname dependencies would make one package serve both, but ALT
needs the package rebuilt against its own rpm regardless of how the
dependencies are written, so a Fedora-built rpm is not the artefact to hand an
ALT user. Building from source works there — `scripts/deps.sh` still knows
ALT's package names — and so does the Flatpak.

The **deb** entries are package names, because dpkg has no soname-level virtual
packages. That is acceptable here because deb targets only Debian and Ubuntu,
and both call the package `libmpv2`.

`libepoxy` is listed explicitly because Murk `dlopen`s it rather than linking it
(see `src/player/render.rs`), so it leaves no trace in the ELF for any automatic
scanner to find. GTK 3 pulls it in anyway, but depending on that is depending on
someone else's dependency.

## Verifying

`scripts/check-package-deps.sh` runs against the built bundles and fails if the
libmpv dependency is missing or if an rpm names a distribution-specific package
instead of a soname. It runs in CI after `tauri build`.

## AppImage

Build it with `scripts/appimage.sh`, not with `tauri build --bundles appimage`
directly. The bundler's GTK AppRun hook exports `GDK_BACKEND=x11`
unconditionally (a workaround for tauri-apps/tauri#8541), which would put a
video player on XWayland in every Wayland session and hand mpv an X11
`Display*` instead of a `wl_display`. The script builds, rewrites that line to
honour the session (an explicit `GDK_BACKEND` still wins), and repacks the
AppDir. It fails loudly if the hook stops forcing x11, so the patch cannot
outlive the bug it works around.

## Windows

Two files carry the Windows packaging, and they exist for one reason each.

`tauri.windows.conf.json` is a platform config Tauri merges into
`tauri.conf.json` on Windows only. It has to be a separate file because
`bundle.resources` is not per-platform: listing `mpv/bin/libmpv-2.dll` in the
main config would fail every Linux build, where that file does not exist. It
declares the NSIS and MSI targets, bundles the DLL next to `murk.exe`, and asks
the installer to fetch WebView2 if the machine has none (Windows 11 does).

`scripts/deps.ps1` is the counterpart to `deps.sh`: it downloads the upstream
libmpv build and derives `mpv.lib` from `mpv.def` with `lib.exe`, because
`libmpv2-sys` emits `cargo:rustc-link-lib=mpv` and MSVC cannot link a DLL
directly. `/name:libmpv-2.dll` is not optional — without it the import library
records the *def file's* name and the program starts looking for `mpv.dll`.

Unlike the Linux packages, nothing here is declared by hand and then verified:
the DLL either sits beside the executable or the program does not start, which
is a failure a smoke run catches immediately. `check-package-deps.sh` has no
Windows counterpart for that reason.
