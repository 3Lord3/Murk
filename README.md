<div align="center">

<img src="assets/banner.png" alt="Murk, the player with no progress bar" width="720">

**Watch the series, not the timeline.**

<a href="https://github.com/3Lord3/Murk/actions"><img alt="build" src="https://img.shields.io/github/actions/workflow/status/3Lord3/Murk/ci.yml?style=for-the-badge&labelColor=0f131b&label=build"></a>
<a href="#installation"><img alt="version" src="https://img.shields.io/badge/version-0.1.0-c084fc?style=for-the-badge&labelColor=0f131b"></a>
<a href="#license"><img alt="license" src="https://img.shields.io/badge/license-GPL--2.0--or--later-e2e8f0?style=for-the-badge&labelColor=0f131b"></a>
<a href="https://tauri.app"><img alt="Tauri 2" src="https://img.shields.io/badge/Tauri_2-6d76ff?style=for-the-badge&labelColor=0f131b&logo=tauri&logoColor=white"></a>
<a href="https://www.rust-lang.org"><img alt="Rust backend" src="https://img.shields.io/badge/Rust_backend-5eead4?style=for-the-badge&labelColor=0f131b&logo=rust&logoColor=white"></a>

[English](README.md) · [Русский](README.ru.md)

</div>

### Your series, without the spoilers

Murk is a desktop player for series and films that refuses to spoil them.

Ordinary players give the story away before the scene does. The progress bar
shows that the resolution is three minutes off, the window title names the
episode, a "12/24" counter tells you the season has barely started. Murk shows
none of that by default. It is not blurred, and it is not behind a toggle: the
numbers never reach the screen at all, because the backend does not send them.

<div align="center">

| | |
|---|---|
| <img src="assets/screenshot-library.png" alt="Murk library, the quiet series shelf" width="520"> | <img src="assets/screenshot-settings.png" alt="Murk settings" width="520"> |
| <img src="assets/screenshot-player.png" alt="Murk, the player with no progress bar" width="520"> | <img src="assets/screenshot-peek.png" alt="Murk, the peek panel" width="520"> |

</div>

# Features

- 🙈 **Nothing to spoil you.** No episode title, number or season, no episode
  count, no seek bar, no position, duration or time left. Hidden values stay in
  the backend, so a layout bug, an open devtools window or a stray log line
  cannot leak them.
- 👀 **Will you make it?** Find out whether you can finish a film or an episode
  in 10–60 minutes, without spoiling the exact time left or where you are in it.
- 📚 **A quiet library.** You add a series as a folder, never as a file,
  because a filename can be a spoiler in itself. Each card has one button,
  *Start* or *Continue*. No episode lists, no hints, no indicators.
- ▶️ **Endings that stay closed.** The next episode starts right away or an
  end-of-episode card appears — depending on the chosen profile.
- 🎬 **Covers that give nothing away.** No downloaded artwork. A cover is your
  own image file, or a plain colour field derived from the series name.
- 🗣️ **Multilingual.** Follows the system language, overridable in settings.

## Profiles

A profile decides, field by field, what the backend is allowed to send to the
window. Switch it in settings at any time.

| On screen | Total Murk | Standard | Soft |
|---|:--:|:--:|:--:|
| Title, season and episode number | hidden | hidden | shown |
| Number of episodes in the season | hidden | hidden | hidden |
| Seek bar | hidden | hidden | shown |
| Position, duration, time left | hidden | hidden | hidden |
| Chapter markers | hidden | hidden | hidden |
| Cover art from your folder | hidden | hidden | shown |
| What comes next | hidden | hidden | hidden |
| End of an episode | nothing: the next one just starts | a card with a countdown | a card with a countdown |
| "Will it finish in N minutes?" | refused | yes or no, in 5-minute steps | yes or no, in 5-minute steps |
| Exact time left, episode and season | refused | after a confirmation | after a confirmation |

# Translation

The interface ships in **English** and **Russian**, following your system
language and overridable in settings. Translations live in the repository as
plain JSON catalogues, and `src/locales/en.json` is the source. To add a language
or fix a translation, open a pull request against the catalogues; see
[CONTRIBUTING.md](CONTRIBUTING.md#translations).

# Installation

Murk targets Linux and is pre-release (v0.1.0). Ready-made packages for every
channel below — `.deb`, `.rpm`, AppImage and a Flatpak bundle, with
`SHA256SUMS` — are attached to each [release](https://github.com/3Lord3/Murk/releases/latest).
Nothing is published to a distribution repository or to Flathub yet, so the
other way to get it is to build it, which takes three commands:

```sh
./scripts/deps.sh --install   # system libraries; --check just lists them
pnpm install
pnpm tauri build              # or `pnpm tauri dev` to run it straight away
```

The build itself needs pnpm, Node 22 and stable Rust. Everything else is a
system library, and `deps.sh` knows the package names for Debian, Ubuntu,
Fedora and Arch.

libmpv must be **client API 2.x** (`libmpv.so.2`, mpv ≥ 0.36); the build stops
with that sentence rather than a wall of linker errors if it is older.
Distributions still on `libmpv.so.1`, notably Ubuntu 22.04 and Debian 12,
are served by the Flatpak build.

## Packages

Bundles land in `src-tauri/target/release/bundle/`.

| Channel | Distributions | Build with | Status |
|---|---|---|---|
| `.deb` | Debian, Ubuntu | `pnpm tauri build --bundles deb` | works; built and checked in CI |
| `.rpm` | Fedora | `pnpm tauri build --bundles rpm` | works; dependencies declared by soname |
| AppImage | all | `./scripts/appimage.sh` | works; bundles mpv and WebKitGTK |
| Flatpak | all | [`packaging/flatpak/`](packaging/flatpak/README.md) | works; verified in the sandbox |

Build the AppImage with the script rather than with `tauri build --bundles
appimage`. Tauri's AppRun hook forces `GDK_BACKEND=x11`, which would drag a
video player through XWayland in every Wayland session; the script undoes that
and repacks the image.

Dependencies are declared by hand, because Tauri's bundler runs neither
`dpkg-shlibdeps` nor rpm's ELF scanner and would otherwise ship a package that
installs cleanly and then fails to start. `scripts/check-package-deps.sh`
guards against that in CI. The reasoning is in
[src-tauri/PACKAGING.md](src-tauri/PACKAGING.md).

# Development

- [CONTRIBUTING.md](CONTRIBUTING.md): the rules, and above all the one rule,
  *nothing that could spoil the story crosses the boundary.*
- [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md): architecture, building, testing.

# License

Murk is free software, licensed under the **GPL-2.0-or-later**: GNU General
Public License, either version 2, or (at your option) any later version. The
full text is in [LICENSE](LICENSE).
