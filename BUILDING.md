# Building

[English](BUILDING.md) · [Русский](BUILDING.ru.md)

Both platforms need pnpm, Node 22 and stable Rust; everything else is libmpv,
which must be **client API 2.x** (`libmpv.so.2`, mpv ≥ 0.36).

## Linux

```sh
./scripts/deps.sh --install   # system libraries; --check just lists them
pnpm install
pnpm tauri build              # or `pnpm tauri dev` to run it straight away
```

`deps.sh` knows the package names for ALT, Debian, Ubuntu, Fedora and Arch.

## Windows

libmpv is not on any package manager here, so `scripts/deps.ps1` fetches it and
builds the import library the MSVC linker needs. That step wants the Visual
Studio build tools (for `lib.exe`) and 7-Zip; the Rust toolchain must be the
MSVC one.

```powershell
pwsh -File scripts/deps.ps1
$env:MPV_LIB_DIR = "$PWD\src-tauri\mpv\lib"
$env:PATH = "$PWD\src-tauri\mpv\bin;$env:PATH"   # dev builds load the DLL from here
pnpm install
pnpm tauri build --bundles nsis          # or `pnpm tauri dev`
```

`MPV_LIB_DIR` is what the build reads; without it the build stops and says so.
The installer bundles `libmpv-2.dll`, so an installed copy needs nothing on
`PATH`.

## Packages

Bundles land in `src-tauri/target/release/bundle/`.

| Channel | Build with |
|---|---|
| `.deb`, `.rpm` | `pnpm tauri build --bundles deb,rpm` |
| AppImage | `./scripts/appimage.sh` |
| Flatpak | [`packaging/flatpak/`](packaging/flatpak/README.md) |
| `.exe` (NSIS), `.msi` | `pnpm tauri build --bundles nsis,msi` |

Build the AppImage with the script, not with `tauri build --bundles appimage`:
Tauri's AppRun hook forces `GDK_BACKEND=x11`, which would put a video player on
XWayland in every Wayland session, and the script undoes that.

Package dependencies are declared by hand rather than derived; the reasoning is
in [src-tauri/PACKAGING.md](src-tauri/PACKAGING.md).

[← Back to README](README.md)
