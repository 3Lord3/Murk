# Working on Murk

## The rule

Anything that could tell a viewer where they are in a story must not cross the
IPC boundary unless the active profile says it may.

In practice that means: **if you add a field to `PlaybackView`, add it as
`Option<T>` with `#[serde(skip_serializing_if = "Option::is_none")]`, route it
through `HidingProfile`, and add it to the leak test.** A field that is always
sent, or sent as `0` when hidden, has quietly broken the guarantee: the value
`0` is itself information, and a field present in the JSON is a field a bug can
render.

`PlaybackView::project` is the only path from backend state to the frontend.
Keep it that way. If you find yourself wanting a second emitter or a command
that returns a number directly, that is the design telling you to stop.

## Things that are easy to reintroduce by accident

* **A window title.** `gtk_window.set_title` is called exactly once, with the
  constant `"Murk"`. Do not derive it from anything.
* **An error message containing a path.** Command errors are stable codes
  chosen at the call site (`fail("could_not_read_image")`), looked up in the
  message catalogues by the frontend; `anyhow`'s chain is logged, not returned.
* **A file dialog in file mode.** `open({ directory: true })`. The list view of
  a file chooser prints filenames.
* **An absolute seek.** There is no `seek_absolute`, and adding one would make
  "jump to 60%" expressible again.
* **Logging.** Paths belong at `trace` level, which is off by default. Reach for
  `MURK_LOG=trace` only when you mean it.
* **mpv options.** `config=no` and `load-scripts=no` are what keep a user's
  `~/.config/mpv` from loading an OSC or an MPRIS script into Murk's process.

## Translations

The interface strings live in `src/locales/*.json`, in the same pull request as
the code that uses them. The AppStream metadata a software centre shows
(`packaging/flatpak/io.murk.player.metainfo.xml`) is translated the same way.

**English is the source language.** `src/locales/en.json` is the catalogue every
other one is checked against, and it is edited together with the code that uses
the new key. Every other catalogue, `ru.json` included, is edited directly in
the repository.

Adding a string to the interface:

1. Add the key to `src/locales/en.json` and use it through `t("…")`. Never write
   the words into a component, and never assemble a sentence out of fragments:
   `t("settings.profiles.meta", { bar, peek })` is translatable, `"Progress bar: " + bar`
   is not.
2. Run `./scripts/check-i18n.sh`. It fails on keys that exist only in a
   translation, on empty values, and on Cyrillic left behind in a component.
3. Translate the new key in the other catalogues you can read. A key left
   missing renders in English, which is exactly the intended state.

A missing translation is never a build failure: vue-i18n falls back to English,
and so does AppStream.

Adding a language means:
1. a new catalogue `src/locales/<code>.json`, translated from `en.json`;
2. one line in `LOCALE_NAMES` (`src/i18n/index.ts`), which is where the
   language's own name for itself lives and the only place a non-English word is
   allowed outside the catalogues.

## Tests

```sh
./scripts/check-i18n.sh
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --check
pnpm build     # runs vue-tsc, so type errors fail here
```

The privacy and parser modules have no system dependencies, so they can be
tested without libmpv installed.

## Frontend types are the second line of defence

Hideable fields are optional in `PlaybackView` on the TypeScript side too:

```ts
positionSec?: number;
```

which means `state.positionSec.toFixed(0)` does not compile. A component has to
say out loud what it does when the value is absent. Do not add `!` to make that
go away.

## Verifying by hand

The step that actually tests the architecture is the last one: open devtools,
look at a `murk://playback` event, and confirm there is no position and no
duration in the payload.
