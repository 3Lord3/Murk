#!/bin/sh
# Build the AppImage, then undo the one thing the bundler gets wrong for Murk.
#
# Tauri's AppImage recipe ships an AppRun hook from linuxdeploy-plugin-gtk that
# does `export GDK_BACKEND=x11` unconditionally, as a workaround for a WebKitGTK
# crash (tauri-apps/tauri#8541). For a video player that is not a small thing:
# it drags the whole app through XWayland on a Wayland session, and the render
# path then hands mpv an X11 `Display*` instead of a `wl_display`.
#
# Murk's surface code already handles both, and the Wayland path was verified to
# start and create the render context inside the very AppImage this script
# produces, so the hook is rewritten to honour the session instead of overriding
# it: an explicit GDK_BACKEND from the environment always wins, a Wayland
# session gets `wayland`, and everything else keeps the old `x11`.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bundle="$root/src-tauri/target/release/bundle/appimage"
appdir="$bundle/Murk.AppDir"
hook="$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh"
repack="${HOME}/.cache/tauri/linuxdeploy-plugin-appimage.AppImage"

cd "$root"
pnpm tauri build --bundles appimage "$@"

[ -f "$hook" ] || { echo "no GTK AppRun hook at $hook" >&2; exit 1; }
grep -q '^export GDK_BACKEND=x11' "$hook" || {
  echo "the GTK hook no longer forces GDK_BACKEND=x11; check whether this patch is still needed" >&2
  exit 1
}
sed -i 's|^export GDK_BACKEND=x11.*|: "${GDK_BACKEND:=${WAYLAND_DISPLAY:+wayland}}"; export GDK_BACKEND="${GDK_BACKEND:-x11}"|' "$hook"

# Unlike the deb and the rpm, which only depend on the system libraries, the
# AppImage ships them: mpv, the ffmpeg codec libraries and WebKitGTK all travel
# inside the image, and all three are copyleft. The licence has to travel with
# them.
mkdir -p "$appdir/usr/share/doc/murk"
cp "$root/LICENSE" "$appdir/usr/share/doc/murk/LICENSE"

[ -x "$repack" ] || { echo "missing $repack (run a plain tauri build first to fetch it)" >&2; exit 1; }

# Repack with appimagetool directly rather than through the linuxdeploy plugin.
# The plugin runs the same appimagetool it bundles, but only forwards $APPIMAGE_COMP,
# so it packs the ~450M AppDir at zstd's default level in 128K blocks. WebKit, ICU
# and the ffmpeg codec libraries compress a lot better with a bigger window:
# `-Xcompression-level 22 -b 1M` takes the image from 146M to 135M for a few extra
# seconds of packing, and zstd keeps the fast random-access reads that squashfs
# needs at startup (xz would reach ~124M, but every page fault costs more).
tool="$root/src-tauri/target/appimagetool/appimagetool-prefix/usr/bin/appimagetool"
[ -x "$tool" ] || {
  rm -rf "$root/src-tauri/target/appimagetool"
  mkdir -p "$root/src-tauri/target/appimagetool"
  (cd "$root/src-tauri/target/appimagetool" && "$repack" --appimage-extract >/dev/null)
  mv "$root/src-tauri/target/appimagetool/squashfs-root"/* "$root/src-tauri/target/appimagetool/"
  [ -x "$tool" ] || { echo "no appimagetool inside $repack" >&2; exit 1; }
}

version=$(node -p "require('$root/package.json').version")
name="Murk_${version}_amd64.AppImage"
rm -f "$bundle"/*.AppImage
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$tool" \
  --comp zstd \
  --mksquashfs-opt -Xcompression-level --mksquashfs-opt 22 \
  --mksquashfs-opt -b --mksquashfs-opt 1M \
  "$appdir" "$bundle/$name"
echo "AppImage at $bundle/$name"
