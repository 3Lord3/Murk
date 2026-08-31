fn main() {
    // Fail here, with a sentence a human can act on, rather than 400 lines of
    // undefined-symbol output from the linker.
    #[cfg(target_os = "linux")]
    {
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

    tauri_build::build()
}
