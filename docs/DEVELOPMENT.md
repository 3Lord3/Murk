# Development

Everything a developer needs to build, run, and understand Murk. For the
contribution rules (especially the privacy rule), see
[CONTRIBUTING.md](../CONTRIBUTING.md).

## The one design decision everything else follows from

**Hiding happens at the IPC boundary, not in CSS.**

The Rust backend knows the position and the running time. When the active
profile hides them, it does not send them: the fields are *absent from the
JSON*, not blanked out in the DOM:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub position_sec: Option<f64>,
```

So a layout bug cannot reveal the position. An open devtools window cannot.
A stray `console.log` cannot. The data was never there. Under the default
profile the entire playback payload is:

```json
{"paused":false,"idle":false,"volume":100.0,"audioTracks":[…],"subtitleTracks":[…]}
```

The same reasoning runs through the rest of the backend:

* **There is no `seek_absolute` command.** Only `seek_relative(delta)`. "Jump
  to 60%" is not hidden in the UI, it cannot be expressed.
* **The file path never crosses the boundary** under any profile. A filename
  is usually the loudest spoiler available.
* **The folder picker only ever opens at folder level**, because a file
  chooser in file mode prints filenames.
* **Track titles are sanitised**, and under a title-hiding profile free text
  in a track name is dropped entirely, because muxers routinely name the subtitle
  track after the episode.
* **The window title is the constant `"Murk"`.** It is never derived from what
  is playing.

`src-tauri/src/privacy/mod.rs` is the whole boundary, and its tests are the
regression barrier:

```rust
#[test]
fn full_darkness_profile_leaks_nothing() { … }
```

### The command surface is the contract

`src-tauri/src/commands.rs` is worth reading as a list of everything the
frontend is able to ask for. There is no `get_position`, no `list_episodes`,
no `get_current_path`, not because the UI avoids calling them, but because
they do not exist to be called.

### Peeking

Peeking is a set of separate commands rather than a flag on the shared state:

* `can_finish_within(minutes) -> bool` answers the question people actually
  have ("will this be over before I have to leave?") with a yes or a no, so
  it reveals neither the position nor the running time. The threshold is
  quantised to five minutes, which caps what repeated probing can recover.
* `peek_remaining()` and `peek_episode_identity()` return real figures, only
  in the `Confirmed` mode, and the result is held in the frontend for about
  ten seconds before it disappears.

## Profiles

| Profile | What is shown | Peeking |
|---|---|---|
| **Полный мрак** | nothing | impossible |
| **Стандарт** (default) | nothing | exact figures, after confirmation |
| **Мягкий** | the bar, no numbers | after confirmation |

`HidingProfile` (`src-tauri/src/privacy/profile.rs`) is a plain data struct
with a `peek: PeekMode` field; the presets are the three rows above.

## Architecture

```
src-tauri/src/
  main.rs              entry point
  lib.rs               wiring, command registration
  player/
    mod.rs             PlayerHandle, a safe API over libmpv
    surface.rs         PlayerSurface, the one trait a port has to implement
    surface_gtk.rs     Linux: GtkGLArea under the webview (Wayland and X11)
    render.rs          GL entry points, framebuffer, scaling
    events.rs          mpv's event stream → sanitised Tauri events
    inhibit.rs         keeping the screen awake
  library/
    scan.rs            walking a series folder
    parse.rs           filename → (season, episode), for ordering only
    db.rs              SQLite: schema and queries
  privacy/
    mod.rs             PlaybackView, the only shape that crosses IPC
    profile.rs         HidingProfile and the presets
    leaks.rs           sanitisers and the window title
  commands.rs          the #[tauri::command] surface
```

### Why the render API instead of `--wid`

mpv can either create a child window inside the host's window (`wid`) or render
into a framebuffer the host provides (`vo=libmpv`). `wid` needs a window id to
hand over, and Wayland has no such object, so it only works on X11, win32 and
macOS. Since Wayland is the priority, Murk uses the render API, and gets three
other things for free:

* compositing stops being a trick with overlapping X windows: `GtkGLArea` is
  an ordinary widget and `GtkOverlay` stacks the webview above it;
* mpv creates no window of its own, so there is no second task-bar entry with
  a filename in it, one metadata leak fewer;
* Windows, macOS and Android take the same GL path, changing only where the
  context comes from.

The Wayland/X11 branch is chosen **at runtime** by asking GDK what kind of
display it opened, so one binary works in both kinds of session.

### Screensaver

`vo=libmpv` has a side effect worth knowing about: mpv owns no window, so it
cannot talk to the screensaver and its `stop-screensaver` option does nothing.
Murk calls `gtk_application_inhibit` itself (`player/inhibit.rs`), which GTK
routes through xdg-desktop-portal and therefore works on Wayland, on X11 and
inside Flatpak. Without it the screen goes dark in the middle of an episode.

## Building

Requirements: pnpm, Node 22, Rust (stable), and the system libraries below.

```sh
./scripts/deps.sh --check     # what is missing, and the command to fix it
./scripts/deps.sh --install   # run that command
pnpm install
pnpm tauri dev
```

`deps.sh` recognises ALT Linux, Debian/Ubuntu, Fedora and Arch. It checks
**pkg-config module names**, which are the same everywhere, and only uses the
package-name table to print an install command. ALT is matched on `ID` before
anything looks for a package manager: it has `apt-get` but rpm-flavoured,
ALT-specific names and no `ID_LIKE`, so every "has apt ⇒ Debian names"
heuristic gets it wrong.

libmpv must be **client API 2.x** (`libmpv.so.2`, mpv ≥ 0.36). `build.rs` fails
with that sentence rather than with a wall of linker errors. Distributions
still on `libmpv.so.1` (Ubuntu 22.04, Debian 12) are served by the Flatpak
build (see `packaging/flatpak/`).

## Packaging

libmpv is called `mpv-libs` on Fedora and `libmpv2` on ALT, so a hardcoded
package name in the rpm could only ever be right on one of them. The rpm
therefore requires the **soname**:

```jsonc
"rpm": { "depends": ["libmpv.so.2()(64bit)", "libepoxy.so.0()(64bit)"] },
"deb": { "depends": ["libmpv2", "libepoxy0"] }
```

rpm generates `Provides:` from sonames automatically, so both distributions'
packages satisfy that one requirement and a single rpm installs on both.

Note that this has to be **declared**, not derived: Tauri's bundler writes a
fixed dependency list of its own and never runs `dpkg-shlibdeps` or rpm's ELF
scanner, so an empty `depends` produces a package with no libmpv requirement at
all: one that installs cleanly and then fails to start. See
`src-tauri/PACKAGING.md`. `scripts/check-package-deps.sh` runs against the
built bundles in CI, because "we left it empty and hoped" and "the tool filled
it in" look identical until somebody installs the package.

## Testing

```sh
./scripts/check-i18n.sh                       # catalogue parity
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --check
pnpm build    # runs vue-tsc, so type errors fail here
```

The suite covers the leak barrier (every preset, every optional field, the
file path, track titles), the filename parser against a table of real-world
names, and the resume/auto-advance logic.

The manual pass that matters, on a real MKV with two audio tracks and ASS
subtitles: open devtools and confirm the `murk://playback` payload has no
position and no duration in it.

## CI

`.github/workflows/ci.yml` runs three kinds of job:

* **tests**: the leak barrier and parser tests, plus clippy and fmt, on
  `debian:trixie`, no system libmpv needed;
* **frontend**: i18n catalogue parity and `pnpm build` (type check);
* **distros**: a plain `cargo build` inside containers for Debian, Ubuntu,
  Fedora, Arch and ALT, so a renamed package is caught the day it is renamed;
* **packages**: `tauri build --bundles deb,rpm` plus the dependency check.