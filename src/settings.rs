//! Small persisted preferences in %LOCALAPPDATA%\Mp3Converter\settings.json.
//! Add app-specific fields here, always with `#[serde(default)]` so older
//! settings files keep loading.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    /// Check for updates in the background and install them silently.
    #[serde(default = "default_true")]
    pub auto_update: bool,
    /// Unix seconds of the last background update check, to throttle to once
    /// a day.
    #[serde(default)]
    pub last_update_check: u64,
    #[serde(default)]
    pub format: crate::convert::Format,
    /// JPG quality, 1-100.
    #[serde(default = "default_quality")]
    pub quality: u8,
    /// Resize so the longest edge is at most `max_edge` pixels.
    #[serde(default)]
    pub resize: bool,
    #[serde(default = "default_max_edge")]
    pub max_edge: u32,
    /// Folder for converted files; `None` writes next to each original.
    #[serde(default)]
    pub out_dir: Option<std::path::PathBuf>,
    /// The licence key as pasted; verified on every start.
    #[serde(default)]
    pub license_key: String,
    /// Display language; `None` follows Windows.
    #[serde(default)]
    pub language: Option<crate::i18n::Lang>,
}

fn default_quality() -> u8 {
    90
}

fn default_max_edge() -> u32 {
    2048
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_update: true,
            last_update_check: 0,
            format: Default::default(),
            quality: default_quality(),
            resize: false,
            max_edge: default_max_edge(),
            out_dir: None,
            license_key: String::new(),
            language: None,
        }
    }
}

pub fn data_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into()))
        .join("Mp3Converter")
}

fn settings_path() -> std::path::PathBuf {
    data_dir().join("settings.json")
}

pub fn load() -> Settings {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) {
    let _ = std::fs::create_dir_all(data_dir());
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(settings_path(), json);
    }
}
