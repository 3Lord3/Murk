fn main() {
    // `#[cfg(target_os = ...)]` in a build script describes the machine running
    // the script, not the machine the binary is for. Cross-compiling would then
    // check the wrong platform's libraries: a Windows-hosted build for Linux
    // would demand MPV_LIB_DIR, and a Linux-hosted build for Windows would skip
    // the link search and fail with undefined symbols. Cargo passes the real
    // answer in the environment.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    // Fail here, with a sentence a human can act on, rather than 400 lines of
    // undefined-symbol output from the linker.
    match target_os.as_str() {
        "linux" => check_linux_libraries(),
        "windows" => configure_windows_link(),
        _ => {}
    }

    tauri_build::build()
}

fn check_linux_libraries() {
    // `mpv.pc` reports the *client API* version (2.x), not the mpv release
    // version. libmpv2 speaks client API 2, i.e. libmpv.so.2 / mpv >= 0.36.
    if let Err(e) = pkg_config::Config::new()
        .atleast_version("2.0")
        .probe("mpv")
    {
        panic!(
            "\n\nlibmpv >= client API 2.0 (libmpv.so.2, mpv >= 0.36) is required.\n\
             pkg-config said: {e}\n\
             Run `scripts/deps.sh --check` for the install command for your distribution.\n\
             Distributions still shipping libmpv.so.1 (Ubuntu 22.04, Debian 12)\n\
             are served by the Flatpak build instead.\n"
        );
    }

    // libepoxy is dlopen'd at runtime (see player/render.rs), so it is not
    // linked here and needs no headers. It is still checked, because a
    // missing libepoxy is a startup failure rather than a link error, and
    // failing now is far easier to diagnose. GTK 3 links it too, so this
    // check has never yet failed in practice.
    if pkg_config::Config::new().probe("epoxy").is_err() {
        println!("cargo:warning=libepoxy not found by pkg-config; Murk loads it at runtime and will fail to start without it");
    }
}

/// Windows has no pkg-config: libmpv2-sys emits `cargo:rustc-link-lib=mpv` and
/// the linker needs an import library called `mpv.lib` on its search path. It
/// comes out of the libmpv development package (`scripts/deps.ps1` fetches one
/// and derives the import library), and MPV_LIB_DIR says where it landed.
fn configure_windows_link() {
    println!("cargo:rerun-if-env-changed=MPV_LIB_DIR");
    match std::env::var("MPV_LIB_DIR") {
        Ok(dir) if std::path::Path::new(&dir).join("mpv.lib").is_file() => {
            println!("cargo:rustc-link-search=native={dir}");
        }
        Ok(dir) => panic!(
            "\n\nMPV_LIB_DIR is set to {dir}, but there is no mpv.lib in it.\n\
             Run `scripts/deps.ps1` to fetch the libmpv development files.\n"
        ),
        Err(_) => panic!(
            "\n\nlibmpv is required, and on Windows it is not discoverable:\n\
             set MPV_LIB_DIR to the directory holding mpv.lib.\n\
             `scripts/deps.ps1` downloads the libmpv build, writes the import\n\
             library and prints the value to use.\n"
        ),
    }
}
