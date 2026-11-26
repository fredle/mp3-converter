//! Windows Explorer integration, all per-user (HKCU, no admin rights):
//!
//! * a right-click verb on every supported media type, written under
//!   `SystemFileAssociations`, which never changes the default app;
//! * an entry in Explorer's "Open with" list (`Applications\<exe>`);
//! * single-instance forwarding, so selecting many files and choosing the
//!   verb ends up as one window holding all of them.
//!
//! Registration runs from the Velopack install/update hooks and is removed by
//! the uninstall hook (see `update::run_app_hooks`).
//!
//! Why forwarding instead of a COM `DropTarget`/`IExecuteCommand` handler: a
//! plain verb makes Explorer start one process per selected file. The first
//! one becomes the primary window; the others drop their paths into a small
//! inbox folder and exit, and the primary picks them up. That needs no COM
//! server, no registered CLSID and works for any number of files, with no
//! timing race (the inbox simply waits until the primary looks).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE_FILE_NAME, FindCloseChangeNotification, FindFirstChangeNotificationW,
    FindNextChangeNotification,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteKeyW, RegDeleteTreeW, RegSetValueExW,
};
#[cfg(test)]
use windows::Win32::System::Registry::{KEY_READ, RegOpenKeyExW};
use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
use windows::core::{HSTRING, PCWSTR, w};

/// Text of the Explorer context-menu entry. English only: a registry verb has
/// no per-language form without shipping a resource DLL. `&&` is a literal `&`.
pub const VERB_TEXT: &str = "Convert with MP4 to MP3 && Audio Converter";
/// Registry key name of the verb.
const VERB_KEY: &str = "Mp3Converter.Convert";
/// Where the registry writes go, relative to HKCU.
const CLASSES: &str = r"Software\Classes";
// ---------------------------------------------------------------- pure logic

/// Sorted, unique, lower-case extensions without dots.
fn normalised(exts: Vec<&str>) -> Vec<String> {
    let mut all: Vec<String> = exts.into_iter().map(|e| e.to_ascii_lowercase()).collect();
    all.sort();
    all.dedup();
    all
}

/// Every extension that gets integration: the types the app names
/// (`convert::listed_extensions`), so Explorer never offers the verb on a
/// type the app accepts only quietly.
pub fn shell_extensions() -> Vec<String> {
    normalised(crate::convert::listed_extensions())
}

/// Every extension registration may have written, in this version or an
/// earlier one that also listed the quiet types: what unregistering cleans.
fn removal_extensions() -> Vec<String> {
    normalised(crate::convert::extensions())
}

/// `"C:\path\app.exe" "%1"`
pub fn open_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

/// One registry value to write; `name == None` is the key's default value.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Entry {
    pub key: String,
    pub name: Option<String>,
    pub value: String,
}

fn exe_file_name(exe: &Path) -> String {
    exe.file_name().and_then(|n| n.to_str()).unwrap_or("mp3_converter.exe").to_owned()
}

fn verb_key(base: &str, ext: &str) -> String {
    format!(r"{base}\SystemFileAssociations\.{ext}\shell\{VERB_KEY}")
}

fn app_key(base: &str, exe: &Path) -> String {
    format!(r"{base}\Applications\{}", exe_file_name(exe))
}

/// Everything registration writes, under `base` (normally `Software\Classes`).
pub fn plan(base: &str, exe: &Path, exts: &[String]) -> Vec<Entry> {
    let e = |key: String, name: Option<&str>, value: String| Entry {
        key,
        name: name.map(str::to_owned),
        value,
    };
    let command = open_command(exe);
    let icon = format!("\"{}\",0", exe.display());
    let mut out = Vec::new();
    for ext in exts {
        let verb = verb_key(base, ext);
        out.push(e(verb.clone(), None, VERB_TEXT.to_owned()));
        out.push(e(verb.clone(), Some("Icon"), icon.clone()));
        out.push(e(format!(r"{verb}\command"), None, command.clone()));
    }
    let app = app_key(base, exe);
    out.push(e(app.clone(), Some("FriendlyAppName"), "MP4 to MP3 & Audio Converter".to_owned()));
    out.push(e(format!(r"{app}\shell\open\command"), None, command));
    for ext in exts {
        out.push(e(format!(r"{app}\SupportedTypes"), Some(&format!(".{ext}")), String::new()));
    }
    out
}

/// Keys removed by unregistration (whole trees).
pub fn removal_keys(base: &str, exe: &Path, exts: &[String]) -> Vec<String> {
    let mut keys: Vec<String> = exts.iter().map(|x| verb_key(base, x)).collect();
    keys.push(app_key(base, exe));
    keys
}

/// The paths among command-line arguments: everything that is not a `--flag`.
pub fn file_args(args: &[String]) -> Vec<String> {
    args.iter().filter(|a| !a.starts_with("--") && !a.is_empty()).cloned().collect()
}

fn encode_paths(paths: &[String]) -> String {
    paths.join("\n")
}

fn decode_paths(text: &str) -> Vec<PathBuf> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect()
}

// ------------------------------------------------------------------ registry

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn set_value(key: &str, name: Option<&str>, value: &str) -> Result<(), String> {
    let mut hkey = HKEY::default();
    let subkey = HSTRING::from(key);
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &subkey,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        )
        .ok()
        .map_err(|e| format!("{key}: {e}"))?;
        let data = wide(value);
        let bytes = std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2);
        let name_w = name.map(HSTRING::from);
        let name_p = name_w.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
        let r = RegSetValueExW(hkey, name_p, None, REG_SZ, Some(bytes)).ok();
        let _ = RegCloseKey(hkey);
        r.map_err(|e| format!("{key}: {e}"))
    }
}

/// Whether `HKCU\<key>` exists.
#[cfg(test)]
pub fn key_exists(key: &str) -> bool {
    let mut hkey = HKEY::default();
    let subkey = HSTRING::from(key);
    unsafe {
        let ok = RegOpenKeyExW(HKEY_CURRENT_USER, &subkey, None, KEY_READ, &mut hkey).is_ok();
        if ok {
            let _ = RegCloseKey(hkey);
        }
        ok
    }
}

fn delete_tree(key: &str) {
    let subkey = HSTRING::from(key);
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &subkey);
        // RegDeleteTreeW removes the children; remove the key itself too.
        let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &subkey);
    }
}

/// Writes the registration under `HKCU\<base>`. Removes the previous one first
/// so an extension dropped from the list does not linger.
pub fn register_under(base: &str, exe: &Path) -> Result<(), String> {
    unregister_under(base, exe);
    let exts = shell_extensions();
    for entry in plan(base, exe, &exts) {
        set_value(&entry.key, entry.name.as_deref(), &entry.value)?;
    }
    Ok(())
}

pub fn unregister_under(base: &str, exe: &Path) {
    let exts = removal_extensions();
    for key in removal_keys(base, exe, &exts) {
        delete_tree(&key);
    }
    for ext in &exts {
        // Only succeed when empty, so other programs' verbs are untouched.
        let shell = HSTRING::from(format!(r"{base}\SystemFileAssociations\.{ext}\shell"));
        let assoc = HSTRING::from(format!(r"{base}\SystemFileAssociations\.{ext}"));
        unsafe {
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &shell);
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &assoc);
        }
    }
}

/// Registers the running exe for the current user.
pub fn register() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    register_under(CLASSES, &exe)
}

pub fn unregister() {
    if let Ok(exe) = std::env::current_exe() {
        unregister_under(CLASSES, &exe);
    }
}

// ------------------------------------------------------- single-instance part

static INBOX_QUEUE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static CONTEXT: OnceLock<eframe::egui::Context> = OnceLock::new();

fn inbox_dir() -> PathBuf {
    crate::settings::data_dir().join("inbox")
}

/// True for the first instance of this user session. The mutex lives as long
/// as the process, so it is released automatically even after a crash.
pub fn claim_primary() -> bool {
    unsafe {
        let handle = CreateMutexW(None, false, w!(r"Local\Mp3Converter.PrimaryInstance"));
        let already = GetLastError() == ERROR_ALREADY_EXISTS;
        // Deliberately never closed: closing would give up the claim.
        std::mem::forget(handle);
        !already
    }
}

/// Hands paths to the primary instance. False if they could not be written.
pub fn forward(paths: &[String]) -> bool {
    forward_to(&inbox_dir(), paths)
}

fn forward_to(dir: &Path, paths: &[String]) -> bool {
    if paths.is_empty() || std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    // Let the primary window take focus when it receives them.
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = format!("{}-{nanos}", std::process::id());
    let tmp = dir.join(format!("{name}.tmp"));
    let done = dir.join(format!("{name}.txt"));
    std::fs::write(&tmp, encode_paths(paths)).is_ok() && std::fs::rename(&tmp, &done).is_ok()
}

/// Reads and removes every pending inbox file in `dir`; files older than
/// `max_age` (left by a dead session) are deleted unread.
fn drain_dir(dir: &Path, max_age: Duration) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > max_age);
        if !stale && let Ok(text) = std::fs::read_to_string(&path) {
            out.extend(decode_paths(&text));
        }
        let _ = std::fs::remove_file(&path);
    }
    out
}

/// Longest the listener sleeps without a word from Windows: a safety net
/// should a change notification ever be missed. The window never redraws
/// for it.
const INBOX_SAFETY_WAKE_MS: u32 = 5_000;

/// Watches `dir` for files appearing, being renamed or deleted in it. `None`
/// when it cannot (the folder could not be made, say).
fn watch(dir: &Path) -> Option<HANDLE> {
    let _ = std::fs::create_dir_all(dir);
    unsafe { FindFirstChangeNotificationW(&HSTRING::from(dir.as_os_str()), false, FILE_NOTIFY_CHANGE_FILE_NAME).ok() }
}

/// Primary instance only: watches the inbox for files forwarded by later
/// launches. The thread sleeps until Windows reports a change in the inbox
/// folder (a forwarded list is renamed into place), so an idle window costs
/// nothing; it used to list the folder every 200 ms.
pub fn start_listener() {
    let dir = inbox_dir();
    // Anything older than a minute is from a dead session.
    drain_dir(&dir, Duration::from_secs(60));
    std::thread::spawn(move || {
        let mut handle = watch(&dir);
        loop {
            match handle {
                Some(h) => unsafe {
                    let _ = WaitForSingleObject(h, INBOX_SAFETY_WAKE_MS);
                    // Re-arm before reading, so nothing that arrives while
                    // the folder is read goes unnoticed.
                    if FindNextChangeNotification(h).is_err() {
                        let _ = FindCloseChangeNotification(h);
                        handle = None;
                    }
                },
                None => {
                    std::thread::sleep(Duration::from_millis(INBOX_SAFETY_WAKE_MS as u64));
                    handle = watch(&dir);
                }
            }
            let got = drain_dir(&dir, Duration::from_secs(3600));
            if got.is_empty() {
                continue;
            }
            if let Ok(mut q) = INBOX_QUEUE.lock() {
                q.extend(got);
            }
            if let Some(ctx) = CONTEXT.get() {
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
        }
    });
}

/// UI side: call once with the egui context so forwarded files wake the window.
pub fn attach(ctx: &eframe::egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}

/// UI side: call every frame; returns files forwarded by other launches since
/// the last call, to be added to the queue.
pub fn take_forwarded() -> Vec<PathBuf> {
    INBOX_QUEUE.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe() -> PathBuf {
        PathBuf::from(r"C:\Users\x y\AppData\Local\Mp3Converter\current\mp3_converter.exe")
    }

    #[test]
    fn extensions_cover_new_types_once() {
        let e = shell_extensions();
        for x in ["mp4", "mp3", "amr", "mpg", "mpeg", "ts"] {
            assert_eq!(e.iter().filter(|v| *v == x).count(), 1, "{x}");
        }
        assert!(e.iter().all(|x| !x.starts_with('.') && *x == x.to_ascii_lowercase()));
        // Accepted quietly: never offered in Explorer, but cleaned up from
        // an earlier registration that did offer them.
        let removed = removal_extensions();
        for x in ["mts", "m2ts"] {
            assert!(!e.iter().any(|v| v == x), "{x}");
            assert!(removed.iter().any(|v| v == x), "{x}");
        }
        assert!(e.iter().all(|x| removed.contains(x)));
    }

    #[test]
    fn command_quotes_path_and_argument() {
        assert_eq!(
            open_command(&exe()),
            r#""C:\Users\x y\AppData\Local\Mp3Converter\current\mp3_converter.exe" "%1""#
        );
    }

    #[test]
    fn plan_never_touches_default_app() {
        let exts = vec!["mp3".to_owned(), "ts".to_owned()];
        let p = plan(CLASSES, &exe(), &exts);
        // Nothing may be written to HKCU\Software\Classes\.<ext>.
        assert!(p.iter().all(|e| !e.key.starts_with(r"Software\Classes\.")));
        assert!(p.iter().any(|e| e.key
            == r"Software\Classes\SystemFileAssociations\.ts\shell\Mp3Converter.Convert\command"));
        assert!(p.iter().any(|e| e.key == r"Software\Classes\Applications\mp3_converter.exe\SupportedTypes"
            && e.name.as_deref() == Some(".mp3")));
    }

    #[test]
    fn removal_matches_what_is_written() {
        let exts = vec!["mp3".to_owned()];
        let keys = removal_keys(CLASSES, &exe(), &exts);
        for e in plan(CLASSES, &exe(), &exts) {
            assert!(keys.iter().any(|k| e.key == *k || e.key.starts_with(&format!("{k}\\"))), "{}", e.key);
        }
    }

    #[test]
    fn file_args_skip_flags() {
        let a: Vec<String> = ["--auto-update", r"C:\a b.mp3", "", "x.wav"].map(String::from).to_vec();
        assert_eq!(file_args(&a), vec![r"C:\a b.mp3".to_owned(), "x.wav".to_owned()]);
    }

    #[test]
    fn inbox_round_trip_and_stale_cleanup() {
        let dir = std::env::temp_dir().join(format!("mp3c-inbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(forward_to(&dir, &["C:\\a.mp3".into(), "C:\\b c.wav".into()]));
        assert!(forward_to(&dir, &["C:\\d.flac".into()]));
        let mut got = drain_dir(&dir, Duration::from_secs(60));
        got.sort();
        assert_eq!(got, vec![PathBuf::from("C:\\a.mp3"), "C:\\b c.wav".into(), "C:\\d.flac".into()]);
        assert!(drain_dir(&dir, Duration::from_secs(60)).is_empty());
        // Files older than the limit are dropped unread.
        assert!(forward_to(&dir, &["C:\\e.mp3".into()]));
        std::thread::sleep(Duration::from_millis(20));
        assert!(drain_dir(&dir, Duration::from_millis(1)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_round_trip_in_scratch_key() {
        let root = format!(r"Software\Mp3ConverterTest{}", std::process::id());
        let base = format!(r"{root}\Classes");
        register_under(&base, &exe()).unwrap();
        assert!(key_exists(&format!(
            r"{base}\SystemFileAssociations\.mp3\shell\Mp3Converter.Convert\command"
        )));
        assert!(key_exists(&format!(r"{base}\Applications\mp3_converter.exe\SupportedTypes")));
        unregister_under(&base, &exe());
        assert!(!key_exists(&format!(r"{base}\SystemFileAssociations\.mp3\shell")));
        assert!(!key_exists(&format!(r"{base}\Applications\mp3_converter.exe")));
        delete_tree(&root);
        assert!(!key_exists(&root));
    }
}
