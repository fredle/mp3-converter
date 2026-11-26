//! MP4 to MP3 & Audio Converter - Convert video and audio files to MP3, M4A, WAV or FLAC, offline.
//!
//!   mp3_converter.exe                 open the app
//!   mp3_converter.exe --auto-update   internal: background update check/download
//!   mp3_converter.exe <files...>      open with the files queued (Explorer verb)
//!   mp3_converter.exe --register-shell | --unregister-shell   Explorer integration

#![windows_subsystem = "windows"]

mod app;
mod feedback;
mod convert;
mod edits;
mod i18n;
mod license;
mod mf;
mod settings;
mod shell;
mod tags;
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

    // Manual (re)registration of the Explorer integration; the Velopack hooks
    // normally do this.
    match args.first().map(String::as_str) {
        Some("--register-shell") => {
            if let Err(e) = shell::register() {
                eprintln!("{e}");
            }
            return;
        }
        Some("--unregister-shell") => {
            shell::unregister();
            return;
        }
        _ => {}
    }

    // One window per user session: files from a later launch (Explorer starts
    // one process per selected file) go to the first window and this process
    // exits. A plain launch without files always opens its own window.
    let primary = shell::claim_primary();
    let files = shell::file_args(&args);
    if !primary && !files.is_empty() && shell::forward(&files) {
        return;
    }
    if primary {
        shell::start_listener();
    }

    update::maybe_spawn_background_update();

    if let Err(e) = app::run(args) {
        eprintln!("{e}");
    }
}
