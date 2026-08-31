# Flatpak

Built and run: `flatpak-builder` completes against `org.gnome.Platform//48`,
and the resulting application starts, shows its interface and creates the mpv
render context. What is *not* yet verified is playback of a real file inside
the sandbox, and folder selection through the portal.

## Building

The build needs the network (see below), so pass `--allow-network` if your
flatpak-builder defaults to offline builds.

```sh
flatpak install -y flathub org.gnome.Platform//48 org.gnome.Sdk//48 \
  org.freedesktop.Sdk.Extension.rust-stable//24.08 \
  org.freedesktop.Sdk.Extension.node22//24.08
flatpak-builder --force-clean --user --install --install-deps-from=flathub \
  build packaging/flatpak/io.murk.player.yml
flatpak run io.murk.player
```

## Two things that are easy to get wrong

**Build through the Tauri CLI.** `build-commands` runs `pnpm tauri build
--no-bundle`, not `pnpm build` followed by `cargo build --release`. Only the
CLI makes the binary embed `dist/`; a bare cargo build leaves it pointing at
the dev server, and the window is black with `Could not connect to localhost`
in dark grey on it. The native packages never hit this because they go through
the CLI already.

**libass is a module.** The GNOME runtime does not ship it, and mpv configured
without it plays films with no subtitles at all.

## What to check first

1. **Runtime version.** `org.gnome.Platform` 48 is assumed. If Flathub has moved
   on, bump `runtime-version` and the two SDK extension versions together.
2. **mpv's dependencies.** mpv 0.40 wants libplacebo ≥ 7.349 and a full ffmpeg.
   libplacebo is built as a module and ffmpeg comes from the
   `ffmpeg-full` extension. If meson complains about a missing dependency, it
   most likely belongs in that list rather than being genuinely absent.
3. **Network during build.** `--share=network` is on the murk module so cargo
   and pnpm can fetch. For a Flathub submission that is not allowed: generate
   `cargo-sources.json` and `node-sources.json` with the
   [flatpak-builder-tools](https://github.com/flatpak/flatpak-builder-tools)
   generators and drop the flag.

## Why the sandbox helps

`--filesystem=host` is deliberately absent. Folders reach Murk through the
file-chooser portal, so the application can only see what the user picked by
hand. That is the usual sandboxing argument, but here it also serves the
product: there is no way for Murk to stumble across a folder full of episode
names it was not shown.
