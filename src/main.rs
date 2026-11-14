//! MP4 to MP3 & Audio Converter - Convert video and audio files to MP3, M4A, WAV or FLAC, offline.
//!
//!   mp3_converter.exe                 open the app
//!   mp3_converter.exe --auto-update   internal: background update check/download

#![windows_subsystem = "windows"]

mod app;
mod convert;
mod i18n;
mod license;
mod mf;
mod settings;
mod theme;
mod update;

fn main() {
    // First thing, always: Velopack may need to run an install/uninstall/
    // update hook and exit before any of our own argument handling runs.
    update::run_app_hooks();

    let args: Vec<String> = std::env::args().skip(1).collect();

    // The detached child spawned below; must not spawn another.
    if args.first().map(String::as_str) == Some("--auto-update") {
        update::run_background_update();
        return;
    }
    update::maybe_spawn_background_update();

    if let Err(e) = app::run(args) {
        eprintln!("{e}");
    }
}
