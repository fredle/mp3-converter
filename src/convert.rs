//! Choosing inputs, naming outputs and converting one file. Everything here
//! is plain functions so the worker threads and the tests can share it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::mf::{MfError, Progress, Target};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Format {
    #[default]
    Mp3,
    M4a,
    Wav,
    Flac,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Mp3, Format::M4a, Format::Wav, Format::Flac];

    pub fn label(self) -> &'static str {
        match self {
            Format::Mp3 => "MP3",
            Format::M4a => "M4A",
            Format::Wav => "WAV",
            Format::Flac => "FLAC",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Mp3 => "mp3",
            Format::M4a => "m4a",
            Format::Wav => "wav",
            Format::Flac => "flac",
        }
    }

    /// Bitrates on offer in kbps; empty for lossless formats. These are the
    /// rates Windows' own encoders support.
    pub fn bitrates(self) -> &'static [u32] {
        match self {
            Format::Mp3 => &[128, 192, 256, 320],
            Format::M4a => &[96, 128, 160, 192],
            Format::Wav | Format::Flac => &[],
        }
    }

    /// The same four quality steps for one channel. Windows' MP3 encoder
    /// tops out at 128 kbps for mono, and a voice needs about half the
    /// bitrate of stereo music anyway. Every rate here is one the encoders
    /// list for mono at 44.1 and 48 kHz.
    pub fn mono_bitrates(self) -> &'static [u32] {
        match self {
            Format::Mp3 => &[64, 96, 112, 128],
            Format::M4a => &[48, 64, 96, 128],
            Format::Wav | Format::Flac => &[],
        }
    }

    /// The offered bitrate nearest to `kbps`, so a setting chosen for one
    /// format still makes sense after switching to another.
    pub fn nearest_bitrate(self, kbps: u32) -> u32 {
        self.bitrates()
            .iter()
            .copied()
            .min_by_key(|rate| rate.abs_diff(kbps))
            .unwrap_or(0)
    }

    /// Which quality step (0 = smallest) `kbps` falls on.
    pub fn tier(self, kbps: u32) -> usize {
        let rate = self.nearest_bitrate(kbps);
        self.bitrates().iter().position(|r| *r == rate).unwrap_or(0)
    }

    /// The bitrate actually written for the stored `kbps` setting: the same
    /// quality step in the mono list when `mono` is on. 0 for lossless.
    pub fn effective_kbps(self, kbps: u32, mono: bool) -> u32 {
        let rates = if mono { self.mono_bitrates() } else { self.bitrates() };
        rates.get(self.tier(kbps)).copied().unwrap_or(0)
    }

    fn target(self, kbps: u32, mono: bool) -> Target {
        let kbps = self.effective_kbps(kbps, mono);
        match self {
            Format::Mp3 => Target::Mp3 { kbps },
            Format::M4a => Target::M4a { kbps },
            Format::Wav => Target::Wav,
            Format::Flac => Target::Flac,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub format: Format,
    /// Bitrate for MP3 and M4A, in kbps, as one of `Format::bitrates`.
    pub kbps: u32,
    /// One channel, for speech.
    pub mono: bool,
    /// Where to write; `None` means next to each source file.
    pub out_dir: Option<PathBuf>,
}

/// A Store codec extension Windows needs before it can read a format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    /// OGG, Opus and WebM.
    WebMedia,
}

impl Codec {
    /// Store pages to open, as (Microsoft's product name, Store product id,
    /// whether it is free). The names are product names, so not translated.
    pub fn store_products(self) -> &'static [(&'static str, &'static str, bool)] {
        match self {
            Codec::WebMedia => &[("Web Media Extensions", "9N5TDP8VCMHS", true)],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConvertError {
    MissingCodec(Codec),
    /// A video with no sound, or an empty audio file.
    NoAudio,
    /// Windows could not open the file or decode its audio; the text is
    /// Windows' own explanation.
    Unreadable(String),
    /// Stopped by the user; nothing was written.
    Cancelled,
    Other(String),
}

const VIDEO: [&str; 10] = [
    "mp4", "m4v", "mov", "3gp", "3g2", "avi", "wmv", "asf", "mkv", "webm",
];
const AUDIO: [&str; 11] = [
    "mp3", "m4a", "m4b", "aac", "wav", "flac", "wma", "ogg", "oga", "opus", "adts",
];

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Whether the file is a video (as opposed to plain audio), by extension.
pub fn is_video(path: &Path) -> bool {
    VIDEO.contains(&extension_of(path).as_str())
}

/// Lengths as a media player shows them: "0:07", "3:42", "1:02:05".
pub fn format_duration(seconds: f64) -> String {
    let total = if seconds.is_finite() && seconds > 0.0 {
        seconds.round() as u64
    } else {
        0
    };
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// File sizes the way Explorer counts them (1 KB = 1024 bytes), with one
/// decimal below 100 so small differences still show.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 100.0 {
        // Rounding 99.96 up would print "100.0"; keep the format steady.
        let rounded = (value * 10.0).round() / 10.0;
        if rounded < 100.0 {
            return format!("{rounded:.1} {}", UNITS[unit]);
        }
    }
    format!("{value:.0} {}", UNITS[unit])
}

/// The output size of one file, when it can be known before converting:
/// the bitrate times the length for MP3 and M4A, the exact size for WAV.
/// FLAC depends on the sound itself, so it has no estimate.
pub fn estimate_bytes(format: Format, kbps: u32, mono: bool, info: &crate::mf::Info) -> Option<u64> {
    let seconds = info.seconds?;
    match format {
        Format::Mp3 | Format::M4a => {
            let kbps = format.effective_kbps(kbps, mono) as f64;
            Some((kbps * 1000.0 / 8.0 * seconds).round() as u64)
        }
        Format::Wav => {
            let (rate, channels) =
                crate::mf::output_shape(Target::Wav, info.rate, info.channels, mono);
            let frames = (rate as f64 * seconds).round() as u64;
            Some(44 + frames * channels as u64 * 2)
        }
        Format::Flac => None,
    }
}

pub fn is_supported(path: &Path) -> bool {
    let ext = extension_of(path);
    VIDEO.contains(&ext.as_str()) || AUDIO.contains(&ext.as_str())
}

/// Every extension the file picker should offer.
pub fn extensions() -> Vec<&'static str> {
    VIDEO.iter().chain(AUDIO.iter()).copied().collect()
}

fn codec_for(ext: &str) -> Option<Codec> {
    match ext {
        "ogg" | "oga" | "opus" | "webm" => Some(Codec::WebMedia),
        _ => None,
    }
}

/// Expands folders (recursively), keeps supported files, and drops
/// duplicates and anything already in `existing`. Order is preserved.
pub fn collect_inputs(paths: &[PathBuf], existing: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen: HashSet<PathBuf> = existing.iter().cloned().collect();
    let mut out = Vec::new();
    for path in paths {
        collect_into(path, &mut seen, &mut out, 0);
    }
    out
}

fn collect_into(path: &Path, seen: &mut HashSet<PathBuf>, out: &mut Vec<PathBuf>, depth: u32) {
    if path.is_dir() {
        if depth > 8 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        children.sort();
        for child in children {
            collect_into(&child, seen, out, depth + 1);
        }
    } else if is_supported(path) && seen.insert(path.to_path_buf()) {
        out.push(path.to_path_buf());
    }
}

/// Where the converted file goes. Never an existing file (so never the
/// source either): a taken name gets " (2)", " (3)" and so on.
pub fn output_path(source: &Path, options: &Options, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let dir = options
        .out_dir
        .clone()
        .or_else(|| source.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio");
    let ext = options.format.extension();
    let first = dir.join(format!("{stem}.{ext}"));
    if !exists(&first) {
        return first;
    }
    (2u32..)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|candidate| !exists(candidate))
        .expect("an unused file name exists")
}

/// Whether the file starts like the format its extension claims.
fn has_container_signature(path: &Path, codec: Codec) -> bool {
    use std::io::Read;
    let mut head = [0u8; 4];
    let read = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .unwrap_or(0);
    is_container_signature(&head[..read], codec)
}

fn is_container_signature(head: &[u8], codec: Codec) -> bool {
    match codec {
        // OGG pages start "OggS"; WebM is Matroska, which starts with the
        // EBML magic number.
        Codec::WebMedia => head.starts_with(b"OggS") || head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]),
    }
}

/// Converts one file and returns where it was written. `progress` follows
/// the conversion and can cancel it.
pub fn convert_file(
    source: &Path,
    options: &Options,
    progress: &Progress,
) -> Result<PathBuf, ConvertError> {
    if progress.cancelled() {
        return Err(ConvertError::Cancelled);
    }
    if let Some(dir) = &options.out_dir {
        std::fs::create_dir_all(dir).map_err(|e| ConvertError::Other(e.to_string()))?;
    }
    let target = output_path(source, options, |p| p.exists());
    let shape = options.format.target(options.kbps, options.mono);
    match crate::mf::transcode(source, &target, shape, options.mono, progress) {
        Ok(()) => {}
        Err(MfError::Cancelled) => return Err(ConvertError::Cancelled),
        Err(MfError::NoAudio) => return Err(ConvertError::NoAudio),
        Err(MfError::Unreadable(message)) => {
            // Windows gives much the same answer for a damaged file as for a
            // missing codec; only send people to the Store for a real one.
            return Err(match codec_for(&extension_of(source)) {
                Some(codec) if has_container_signature(source, codec) => {
                    ConvertError::MissingCodec(codec)
                }
                _ => ConvertError::Unreadable(message),
            });
        }
        Err(MfError::Other(message)) => return Err(ConvertError::Other(message)),
    }
    // Title, artist and so on, where both formats can hold them. Never
    // fails the conversion; WAV files get none.
    if options.format != Format::Wav {
        crate::tags::copy(source, &target);
    }
    // Keep the file's date so sorting by date still works. After the tags,
    // because writing them touches the file.
    if let Ok(modified) = std::fs::metadata(source).and_then(|m| m.modified())
        && let Ok(file) = std::fs::File::options().write(true).open(&target)
    {
        let _ = file.set_modified(modified);
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(format: Format) -> Options {
        Options {
            format,
            kbps: 192,
            mono: false,
            out_dir: None,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mac-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Two seconds of a 440 Hz tone as a 16-bit WAV file.
    fn write_tone(path: &Path, rate: u32, channels: u32) {
        write_tone_for(path, rate, channels, 2);
    }

    /// `seconds` of a 440 Hz tone as a 16-bit WAV file.
    pub(crate) fn write_tone_for(path: &Path, rate: u32, channels: u32, seconds: u32) {
        let frames = rate * seconds;
        let mut bytes = crate::mf::wav_header(rate, channels, frames * channels * 2).to_vec();
        for i in 0..frames {
            let t = i as f32 / rate as f32;
            let v = ((t * 440.0 * std::f32::consts::TAU).sin() * 12_000.0) as i16;
            for _ in 0..channels {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        std::fs::write(path, bytes).unwrap();
    }

    /// (sample rate, channels, seconds) of a 16-bit WAV file.
    fn wav_info(path: &Path) -> (u32, u32, f32) {
        let b = std::fs::read(path).unwrap();
        let rate = u32::from_le_bytes(b[24..28].try_into().unwrap());
        let channels = u16::from_le_bytes(b[22..24].try_into().unwrap()) as u32;
        let data = u32::from_le_bytes(b[40..44].try_into().unwrap());
        (rate, channels, data as f32 / (rate * channels * 2) as f32)
    }

    #[test]
    fn supported_extensions_ignore_case() {
        assert!(is_supported(Path::new("Holiday.MP4")));
        assert!(is_supported(Path::new("a.mkv")));
        assert!(is_supported(Path::new("a.Flac")));
        assert!(!is_supported(Path::new("notes.txt")));
        assert!(!is_supported(Path::new("photo.jpg")));
        assert!(!is_supported(Path::new("noextension")));
    }

    #[test]
    fn bitrates_snap_to_what_the_format_offers() {
        assert_eq!(Format::Mp3.nearest_bitrate(192), 192);
        assert_eq!(Format::M4a.nearest_bitrate(320), 192);
        assert_eq!(Format::Mp3.nearest_bitrate(96), 128);
        assert_eq!(Format::Wav.nearest_bitrate(192), 0);
        assert_eq!(Format::M4a.target(256, false), Target::M4a { kbps: 192 });
        assert_eq!(Format::Flac.target(256, false), Target::Flac);
    }

    #[test]
    fn output_path_avoids_existing_files() {
        let src = Path::new("C:/videos/clip.mp4");
        let opts = options(Format::Mp3);
        assert_eq!(
            output_path(src, &opts, |_| false),
            PathBuf::from("C:/videos/clip.mp3")
        );
        let taken = [
            PathBuf::from("C:/videos/clip.mp3"),
            PathBuf::from("C:/videos/clip (2).mp3"),
        ];
        assert_eq!(
            output_path(src, &opts, |p| taken.contains(&p.to_path_buf())),
            PathBuf::from("C:/videos/clip (3).mp3")
        );
    }

    #[test]
    fn output_path_uses_chosen_folder() {
        let mut opts = options(Format::Flac);
        opts.out_dir = Some(PathBuf::from("D:/out"));
        assert_eq!(
            output_path(Path::new("C:/music/a.wav"), &opts, |_| false),
            PathBuf::from("D:/out/a.flac")
        );
    }

    #[test]
    fn collects_folders_and_skips_duplicates_and_other_files() {
        let dir = temp_dir("collect");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        for name in ["a.mp4", "b.txt", "sub/c.flac"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let a = dir.join("a.mp4");
        let found = collect_inputs(&[dir.clone(), a.clone()], &[]);
        assert_eq!(found, vec![a.clone(), sub.join("c.flac")]);
        assert_eq!(collect_inputs(&[dir.clone()], &[a]), vec![sub.join("c.flac")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn container_signatures() {
        assert!(is_container_signature(b"OggS", Codec::WebMedia));
        assert!(is_container_signature(&[0x1A, 0x45, 0xDF, 0xA3], Codec::WebMedia));
        assert!(!is_container_signature(b"nope", Codec::WebMedia));
        assert!(!is_container_signature(&[], Codec::WebMedia));
    }

    #[test]
    fn unreadable_file_reports_an_error_and_leaves_nothing_behind() {
        let dir = temp_dir("bad");
        let src = dir.join("broken.mp4");
        std::fs::write(&src, b"not a video").unwrap();
        assert!(matches!(
            convert_file(&src, &options(Format::Mp3), &Progress::default()),
            Err(ConvertError::Unreadable(_))
        ));
        assert!(!dir.join("broken.mp3").exists());
        // A broken .ogg is not a missing codec either.
        let ogg = dir.join("broken.ogg");
        std::fs::write(&ogg, b"not audio").unwrap();
        assert!(matches!(
            convert_file(&ogg, &options(Format::Mp3), &Progress::default()),
            Err(ConvertError::Unreadable(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wav_round_trip_keeps_length_and_never_overwrites() {
        let dir = temp_dir("wav");
        let src = dir.join("tone.wav");
        write_tone(&src, 22_050, 1);
        let out = convert_file(&src, &options(Format::Wav), &Progress::default()).unwrap();
        assert_eq!(out, dir.join("tone (2).wav"));
        let (rate, channels, seconds) = wav_info(&out);
        assert_eq!((rate, channels), (22_050, 1));
        assert!((seconds - 2.0).abs() < 0.05, "{seconds}");
        assert_eq!(convert_file(&src, &options(Format::Wav), &Progress::default()).unwrap(), dir.join("tone (3).wav"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Encodes with Windows' own encoders, then decodes the result again to
    /// check it is real audio of the right length. A Windows edition without
    /// the Media Foundation encoders (some servers) skips the format.
    #[test]
    fn encodes_every_format_and_resamples_odd_inputs() {
        let dir = temp_dir("formats");
        for (name, rate, channels) in [("cd.wav", 44_100, 2), ("voice.wav", 22_050, 1), ("dat.wav", 48_000, 2)] {
            let src = dir.join(name);
            write_tone(&src, rate, channels);
            for format in [Format::Mp3, Format::M4a, Format::Flac] {
                let out = match convert_file(&src, &options(format), &Progress::default()) {
                    Ok(out) => out,
                    Err(ConvertError::Other(m)) if m.contains("no encoder") => {
                        println!("skipped {format:?}: {m}");
                        continue;
                    }
                    Err(e) => panic!("{name} -> {format:?}: {e:?}"),
                };
                assert!(std::fs::metadata(&out).unwrap().len() > 2_000, "{out:?}");
                let back = convert_file(&out, &options(Format::Wav), &Progress::default()).unwrap();
                let (_, back_channels, seconds) = wav_info(&back);
                assert_eq!(back_channels, channels, "{name} -> {format:?}");
                assert!((seconds - 2.0).abs() < 0.2, "{name} -> {format:?}: {seconds}s");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn durations_read_like_a_player() {
        assert_eq!(format_duration(0.0), "0:00");
        assert_eq!(format_duration(7.4), "0:07");
        assert_eq!(format_duration(59.6), "1:00");
        assert_eq!(format_duration(222.0), "3:42");
        assert_eq!(format_duration(3725.0), "1:02:05");
        assert_eq!(format_duration(-3.0), "0:00");
        assert_eq!(format_duration(f64::NAN), "0:00");
    }

    #[test]
    fn sizes_read_like_explorer() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1000), "1000 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(5_347_737), "5.1 MB");
        assert_eq!(format_size(50_436_505), "48.1 MB");
        assert_eq!(format_size(222_298_112), "212 MB");
        assert_eq!(format_size(104_857_000), "100 MB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn mono_keeps_the_quality_step_at_rates_the_encoders_offer() {
        assert_eq!(Format::Mp3.tier(320), 3);
        assert_eq!(Format::Mp3.effective_kbps(320, false), 320);
        assert_eq!(Format::Mp3.effective_kbps(320, true), 128);
        assert_eq!(Format::Mp3.effective_kbps(128, true), 64);
        assert_eq!(Format::M4a.effective_kbps(160, true), 96);
        assert_eq!(Format::Flac.effective_kbps(192, true), 0);
        assert_eq!(Format::Mp3.target(192, true), Target::Mp3 { kbps: 96 });
        for format in Format::ALL {
            assert_eq!(format.bitrates().len(), format.mono_bitrates().len());
        }
    }

    #[test]
    fn estimates_only_what_can_be_known() {
        let info = crate::mf::Info {
            seconds: Some(60.0),
            rate: 44_100,
            channels: 2,
        };
        // 192 kbps for a minute: 1.44 MB.
        assert_eq!(estimate_bytes(Format::Mp3, 192, false, &info), Some(1_440_000));
        assert_eq!(estimate_bytes(Format::Mp3, 192, true, &info), Some(720_000));
        // WAV is exact: 44-byte header plus 16-bit samples.
        assert_eq!(estimate_bytes(Format::Wav, 0, false, &info), Some(44 + 44_100 * 60 * 4));
        assert_eq!(estimate_bytes(Format::Wav, 0, true, &info), Some(44 + 44_100 * 60 * 2));
        assert_eq!(estimate_bytes(Format::Flac, 0, false, &info), None);
        let unknown = crate::mf::Info { seconds: None, ..info };
        assert_eq!(estimate_bytes(Format::Mp3, 192, false, &unknown), None);
    }

    #[test]
    fn wav_estimate_matches_the_written_file() {
        let dir = temp_dir("estimate");
        let src = dir.join("tone.wav");
        write_tone(&src, 48_000, 2);
        let info = crate::mf::probe(&src).unwrap();
        assert!((info.seconds.unwrap() - 2.0).abs() < 0.01, "{info:?}");
        let out = convert_file(&src, &options(Format::Wav), &Progress::default()).unwrap();
        let written = std::fs::metadata(&out).unwrap().len();
        assert_eq!(estimate_bytes(Format::Wav, 0, false, &info), Some(written));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mono_writes_one_channel_in_every_format() {
        let dir = temp_dir("mono");
        let src = dir.join("stereo.wav");
        write_tone(&src, 44_100, 2);
        let mut opts = options(Format::Wav);
        opts.mono = true;
        for format in Format::ALL {
            opts.format = format;
            let out = match convert_file(&src, &opts, &Progress::default()) {
                Ok(out) => out,
                Err(ConvertError::Other(m)) if m.contains("no encoder") => continue,
                Err(e) => panic!("{format:?}: {e:?}"),
            };
            let info = crate::mf::probe(&out).unwrap();
            assert_eq!(info.channels, 1, "{format:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn progress_reaches_the_end_for_every_format() {
        let dir = temp_dir("progress");
        let src = dir.join("tone.wav");
        write_tone(&src, 44_100, 2);
        for format in Format::ALL {
            let progress = Progress::default();
            match convert_file(&src, &options(format), &progress) {
                Ok(_) => assert_eq!(progress.fraction(), Some(1.0), "{format:?}"),
                Err(ConvertError::Other(m)) if m.contains("no encoder") => {}
                Err(e) => panic!("{format:?}: {e:?}"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Cancels part-way through a long file: the conversion stops with
    /// `Cancelled` and leaves no output behind, for the encoders and WAV.
    #[test]
    fn cancel_stops_part_way_and_leaves_nothing_behind() {
        let dir = temp_dir("cancel");
        let src = dir.join("long.wav");
        write_tone_for(&src, 44_100, 2, 120);
        // Already cancelled before it starts.
        let stopped = Progress::default();
        stopped.cancel();
        assert_eq!(
            convert_file(&src, &options(Format::Mp3), &stopped),
            Err(ConvertError::Cancelled)
        );
        for format in [Format::Mp3, Format::Wav] {
            let progress = std::sync::Arc::new(Progress::default());
            let watcher = {
                let progress = progress.clone();
                std::thread::spawn(move || {
                    let started = std::time::Instant::now();
                    while progress.fraction().unwrap_or(0.0) < 0.05
                        && started.elapsed().as_secs() < 30
                    {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    progress.cancel();
                })
            };
            let result = convert_file(&src, &options(format), &progress);
            watcher.join().unwrap();
            match result {
                Err(ConvertError::Cancelled) => {}
                Err(ConvertError::Other(m)) if m.contains("no encoder") => continue,
                other => panic!("{format:?}: {other:?}"),
            }
            let fraction = progress.fraction().unwrap();
            assert!(fraction > 0.0 && fraction < 1.0, "{format:?}: {fraction}");
            let left: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .map(|e| e.file_name())
                .collect();
            assert_eq!(left, vec![std::ffi::OsString::from("long.wav")], "{format:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Tags an MP3, converts it to M4A and FLAC and reads the tags back.
    /// Skips a format when Windows has no encoder or no property handler
    /// for it.
    #[test]
    fn tags_are_carried_over() {
        use windows::Win32::Storage::EnhancedStorage::{
            PKEY_Music_AlbumTitle, PKEY_Music_Artist, PKEY_Title,
        };
        let dir = temp_dir("tags");
        let src = dir.join("tone.wav");
        write_tone(&src, 44_100, 2);
        let mp3 = match convert_file(&src, &options(Format::Mp3), &Progress::default()) {
            Ok(out) => out,
            Err(ConvertError::Other(m)) if m.contains("no encoder") => {
                println!("skipped: {m}");
                return;
            }
            Err(e) => panic!("{e:?}"),
        };
        let wrote = crate::tags::set_text(&mp3, &PKEY_Title, "Morning Tone", false)
            && crate::tags::set_text(&mp3, &PKEY_Music_Artist, "Test Artist", true)
            && crate::tags::set_text(&mp3, &PKEY_Music_AlbumTitle, "Test Album", false);
        if !wrote {
            println!("skipped: no MP3 property handler");
            return;
        }
        assert_eq!(crate::tags::get_text(&mp3, &PKEY_Title).as_deref(), Some("Morning Tone"));
        for format in [Format::M4a, Format::Flac, Format::Wav] {
            let out = match convert_file(&mp3, &options(format), &Progress::default()) {
                Ok(out) => out,
                Err(ConvertError::Other(m)) if m.contains("no encoder") => continue,
                Err(e) => panic!("{format:?}: {e:?}"),
            };
            let title = crate::tags::get_text(&out, &PKEY_Title);
            if format == Format::Wav {
                // WAV gets none, and the conversion still succeeds.
                assert_ne!(title.as_deref(), Some("Morning Tone"));
                continue;
            }
            if title.is_none() && crate::tags::copy(&mp3, &out) == 0 {
                println!("skipped {format:?}: no property handler");
                continue;
            }
            assert_eq!(title.as_deref(), Some("Morning Tone"), "{format:?}");
            assert_eq!(
                crate::tags::get_text(&out, &PKEY_Music_Artist).as_deref(),
                Some("Test Artist"),
                "{format:?}"
            );
            assert_eq!(
                crate::tags::get_text(&out, &PKEY_Music_AlbumTitle).as_deref(),
                Some("Test Album"),
                "{format:?}"
            );
            // Writing the tags did not undo the date carry-over.
            let modified = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
            assert_eq!(modified(&out), modified(&mp3), "{format:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Runs only when MAC_SAMPLES points at a folder of real media files.
    #[test]
    fn converts_real_samples_when_provided() {
        let Ok(folder) = std::env::var("MAC_SAMPLES") else {
            return;
        };
        let out_dir = temp_dir("samples");
        let mut opts = options(Format::Mp3);
        opts.out_dir = Some(out_dir.clone());
        for source in collect_inputs(&[PathBuf::from(folder)], &[]) {
            let result = convert_file(&source, &opts, &Progress::default());
            println!("{:?} -> {:?}", source.file_name().unwrap(), result.as_ref().map(|p| std::fs::metadata(p).unwrap().len()));
        }
        if std::env::var("MAC_KEEP").is_err() { let _ = std::fs::remove_dir_all(&out_dir); } else { println!("kept {out_dir:?}"); }
    }
}
