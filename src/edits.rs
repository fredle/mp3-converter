//! The choices a queue row can hold for its own file: start and end times as
//! typed, the audio track, and "Make louder". Plain data and functions, so
//! the window only draws them and the tests can check every rule; `mf`
//! turns the result (`Edits`) into sound.

use crate::i18n::{Strings, fill};
use crate::mf::{AudioTrack, Edits, Trim, TrimError, Volume};

/// Reads a time as people type it: plain seconds ("90", "90.5"), m:ss
/// ("1:30") or h:mm:ss ("1:02:03"), with a point or a comma before
/// fractions of a second. Empty means "not set". `Err` for anything else,
/// including minutes or seconds of 60 or more after a colon.
pub fn parse_time(text: &str) -> Result<Option<f64>, ()> {
    let text = text.trim().replace(',', ".");
    if text.is_empty() {
        return Ok(None);
    }
    let parts: Vec<&str> = text.split(':').map(str::trim).collect();
    if parts.len() > 3 {
        return Err(());
    }
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let last = parts.len() - 1;
    let mut total = 0.0;
    for (i, part) in parts.iter().enumerate() {
        let value: f64 = if i == last {
            // Seconds, maybe with a fraction: "5", "05.25", ".5".
            let mut halves = part.splitn(2, '.');
            let whole = halves.next().unwrap_or("");
            let fraction = halves.next();
            let whole_ok = digits(whole) || (whole.is_empty() && fraction.is_some());
            if !whole_ok || fraction.is_some_and(|f| !digits(f)) {
                return Err(());
            }
            part.parse().map_err(|_| ())?
        } else if digits(part) {
            part.parse().map_err(|_| ())?
        } else {
            return Err(());
        };
        // After a colon, minutes and seconds stay below 60.
        if i > 0 && value >= 60.0 {
            return Err(());
        }
        total = total * 60.0 + value;
    }
    total.is_finite().then_some(Some(total)).ok_or(())
}

/// A time as the row shows it: "0:30", "1:02:03", with tenths when the time
/// is not a whole second ("1:30.5").
pub fn format_time(seconds: f64) -> String {
    let tenths = if seconds.is_finite() && seconds > 0.0 {
        (seconds * 10.0).round() as u64
    } else {
        0
    };
    let (whole, fraction) = (tenths / 10, tenths % 10);
    let (h, m, s) = (whole / 3600, whole / 60 % 60, whole % 60);
    let base = if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    };
    if fraction > 0 {
        format!("{base}.{fraction}")
    } else {
        base
    }
}

/// Why a row's trim cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// A time field holds something that is not a time.
    BadTime,
    /// The times do not fit together or the file.
    Trim(TrimError),
}

/// The row's own words for a problem (never the engine's English).
pub fn problem_text(problem: Problem, t: &Strings) -> &'static str {
    match problem {
        Problem::BadTime | Problem::Trim(TrimError::Invalid) => t.trim_bad_time,
        Problem::Trim(TrimError::EndBeforeStart) => t.trim_end_before_start,
        Problem::Trim(TrimError::StartPastEnd) => t.trim_start_past_end,
    }
}

/// One file's choices, as the row edits them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileEdit {
    /// The row shows its controls.
    pub expanded: bool,
    /// The time fields as typed.
    pub start: String,
    pub end: String,
    /// Which audio track, counting from 0 (see `mf::audio_tracks`).
    pub track: usize,
    /// This file's own volume; `None` follows "Make louder" in the options.
    pub volume: Option<Volume>,
}

impl FileEdit {
    /// The trim as typed, before checking it against the file.
    pub fn trim(&self) -> Result<Trim, Problem> {
        let mut trim = Trim::NONE;
        trim.start = parse_time(&self.start).map_err(|_| Problem::BadTime)?;
        trim.end = parse_time(&self.end).map_err(|_| Problem::BadTime)?;
        Ok(trim)
    }

    /// The trim, checked against the file's length when it is known.
    pub fn check(&self, seconds: Option<f64>) -> Result<Trim, Problem> {
        let trim = self.trim()?;
        trim.resolve(seconds).map_err(Problem::Trim)?;
        Ok(trim)
    }

    /// The volume this file gets, given "Make louder" for every file.
    pub fn volume(&self, louder: bool) -> Volume {
        self.volume.unwrap_or(if louder {
            Volume::Normalise
        } else {
            Volume::Unchanged
        })
    }

    pub fn louder(&self, louder: bool) -> bool {
        self.volume(louder) == Volume::Normalise
    }

    /// Ticks or clears this file's "Make louder". Choosing what every file
    /// gets anyway drops the file's own choice, so it follows the options
    /// again.
    pub fn set_louder(&mut self, on: bool, louder: bool) {
        self.volume = (on != louder).then_some(if on {
            Volume::Normalise
        } else {
            Volume::Unchanged
        });
    }

    /// What a conversion of this file uses, or why it cannot start.
    pub fn edits(&self, seconds: Option<f64>, louder: bool) -> Result<Edits, Problem> {
        Ok(Edits {
            trim: self.check(seconds)?,
            track: self.track,
            volume: self.volume(louder),
        })
    }
}

/// The short note a collapsed row shows for its own choices, such as
/// "0:30 to 2:15 · Track 2 · Louder". `None` when the file is converted as
/// every other file is. A trim that cannot be read is left to the row's
/// problem line.
pub fn indicator(edit: &FileEdit, louder: bool, t: &Strings) -> Option<String> {
    let mut parts = Vec::new();
    if let Ok(trim) = edit.trim()
        && !trim.is_none()
    {
        let start = trim.start.filter(|s| *s > 0.0).map(format_time);
        let end = trim.end.map(format_time);
        parts.push(match (start, end) {
            (Some(start), Some(end)) => fill(t.trim_range, &[("start", &start), ("end", &end)]),
            (Some(start), None) => fill(t.trim_from, &[("start", &start)]),
            (None, Some(end)) => fill(t.trim_until, &[("end", &end)]),
            (None, None) => String::new(),
        });
    }
    if edit.track > 0 {
        parts.push(fill(t.track_n, &[("n", &(edit.track + 1).to_string())]));
    }
    if let Some(volume) = edit.volume
        && volume != edit_default(louder)
    {
        parts.push(
            if volume == Volume::Normalise {
                t.louder_short
            } else {
                t.volume_original
            }
            .to_owned(),
        );
    }
    parts.retain(|p| !p.is_empty());
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn edit_default(louder: bool) -> Volume {
    FileEdit::default().volume(louder)
}

/// A language's own name for an RFC 1766 or ISO 639 tag ("en", "en-US",
/// "eng", "ger"), for the languages people most often find in films. `None`
/// for unknown or undetermined tags, and for right-to-left scripts, which
/// the window cannot shape.
pub fn language_name(tag: &str) -> Option<&'static str> {
    let primary = tag
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let name = match primary.as_str() {
        "en" | "eng" => "English",
        "de" | "deu" | "ger" => "Deutsch",
        "fr" | "fra" | "fre" => "Français",
        "es" | "spa" => "Español",
        "it" | "ita" => "Italiano",
        "pt" | "por" => "Português",
        "nl" | "nld" | "dut" => "Nederlands",
        "pl" | "pol" => "Polski",
        "tr" | "tur" => "Türkçe",
        "ru" | "rus" => "Русский",
        "uk" | "ukr" => "Українська",
        "sv" | "swe" => "Svenska",
        "da" | "dan" => "Dansk",
        "no" | "nor" | "nb" | "nob" | "nn" | "nno" => "Norsk",
        "fi" | "fin" => "Suomi",
        "cs" | "ces" | "cze" => "Čeština",
        "hu" | "hun" => "Magyar",
        "el" | "ell" | "gre" => "Ελληνικά",
        "ja" | "jpn" => "日本語",
        "ko" | "kor" => "한국어",
        "zh" | "zho" | "chi" => "中文",
        _ => return None,
    };
    Some(name)
}

/// How the track list names a track: "Track 2 · English · Commentary · AAC",
/// ending in "can't be converted" when Windows cannot decode it.
/// `drawable` says whether the window's fonts have the glyphs of a
/// language's own name; when they do not, the tag itself is shown.
pub fn track_label(track: &AudioTrack, t: &Strings, drawable: impl Fn(&str) -> bool) -> String {
    let mut parts = vec![fill(t.track_n, &[("n", &(track.index + 1).to_string())])];
    if let Some(tag) = track.language.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
        match language_name(tag) {
            Some(name) if drawable(name) => parts.push(name.to_owned()),
            // "und" is Matroska's "not given".
            _ if tag.eq_ignore_ascii_case("und") => {}
            _ => parts.push(tag.to_owned()),
        }
    }
    if let Some(name) = track.name.as_deref().map(str::trim).filter(|n| !n.is_empty())
        && !parts.iter().any(|p| p.eq_ignore_ascii_case(name))
        && drawable(name)
    {
        parts.push(name.to_owned());
    }
    if let Some(codec) = track.codec {
        parts.push(codec.to_owned());
    }
    if !track.decodable {
        parts.push(t.track_unsupported.to_owned());
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

    #[test]
    fn times_read_the_ways_people_type_them() {
        assert_eq!(parse_time(""), Ok(None));
        assert_eq!(parse_time("   "), Ok(None));
        assert_eq!(parse_time("90"), Ok(Some(90.0)));
        assert_eq!(parse_time("90.5"), Ok(Some(90.5)));
        assert_eq!(parse_time("1:30"), Ok(Some(90.0)));
        assert_eq!(parse_time(" 1:30 "), Ok(Some(90.0)));
        assert_eq!(parse_time("0:05"), Ok(Some(5.0)));
        assert_eq!(parse_time("1:5"), Ok(Some(65.0)));
        assert_eq!(parse_time("1:30,5"), Ok(Some(90.5)));
        assert_eq!(parse_time("1:02:03"), Ok(Some(3723.0)));
        assert_eq!(parse_time("125:00"), Ok(Some(7500.0)));
        assert_eq!(parse_time(".5"), Ok(Some(0.5)));
        for bad in ["abc", "1:60", "1:75:00", "-5", "1::2", ":30", "1:2:3:4", "1.5:00", "1:3x", "1.", "1e3", "inf"] {
            assert_eq!(parse_time(bad), Err(()), "{bad}");
        }
    }

    #[test]
    fn times_show_as_a_player_shows_them() {
        assert_eq!(format_time(0.0), "0:00");
        assert_eq!(format_time(30.0), "0:30");
        assert_eq!(format_time(135.0), "2:15");
        assert_eq!(format_time(90.5), "1:30.5");
        assert_eq!(format_time(59.96), "1:00");
        assert_eq!(format_time(3723.0), "1:02:03");
        assert_eq!(format_time(f64::NAN), "0:00");
        for text in ["0:30", "2:15", "1:30.5", "1:02:03"] {
            let seconds = parse_time(text).unwrap().unwrap();
            assert_eq!(format_time(seconds), text);
        }
    }

    fn edit(start: &str, end: &str) -> FileEdit {
        FileEdit {
            start: start.to_owned(),
            end: end.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn trims_are_checked_against_the_file() {
        let length = Some(300.0);
        assert_eq!(edit("", "").check(length), Ok(Trim::NONE));
        assert_eq!(
            edit("0:30", "2:15").check(length),
            Ok(Trim { start: Some(30.0), end: Some(135.0) })
        );
        assert_eq!(edit("x", "").check(length), Err(Problem::BadTime));
        assert_eq!(
            edit("2:15", "0:30").check(length),
            Err(Problem::Trim(TrimError::EndBeforeStart))
        );
        assert_eq!(
            edit("6:00", "").check(length),
            Err(Problem::Trim(TrimError::StartPastEnd))
        );
        // Past the end means "to the end"; an unknown length checks order only.
        assert!(edit("0:30", "9:00").check(length).is_ok());
        assert!(edit("6:00", "").check(None).is_ok());
        let t = Lang::En.strings();
        assert_eq!(problem_text(Problem::BadTime, t), "Use a time such as 1:30");
        assert_eq!(
            problem_text(Problem::Trim(TrimError::EndBeforeStart), t),
            "The end must be after the start"
        );
        // Every language has its own words for every problem.
        for lang in Lang::ALL {
            let t = lang.strings();
            for problem in [
                Problem::BadTime,
                Problem::Trim(TrimError::EndBeforeStart),
                Problem::Trim(TrimError::StartPastEnd),
            ] {
                assert!(!problem_text(problem, t).is_empty());
            }
        }
    }

    #[test]
    fn louder_follows_the_options_until_the_file_chooses() {
        let mut e = FileEdit::default();
        assert_eq!(e.volume(false), Volume::Unchanged);
        assert_eq!(e.volume(true), Volume::Normalise);
        e.set_louder(true, false);
        assert_eq!(e.volume, Some(Volume::Normalise));
        assert!(e.louder(false) && e.louder(true));
        // Back to what every file gets: no choice of its own.
        e.set_louder(false, false);
        assert_eq!(e.volume, None);
        e.set_louder(false, true);
        assert_eq!(e.volume, Some(Volume::Unchanged));
        assert!(!e.louder(true));
        let edits = FileEdit { track: 1, ..edit("0:30", "") }.edits(Some(60.0), true).unwrap();
        assert_eq!(
            edits,
            Edits {
                trim: Trim { start: Some(30.0), end: None },
                track: 1,
                volume: Volume::Normalise
            }
        );
        assert_eq!(edit("2:00", "1:00").edits(None, false).map(|_| ()), Err(Problem::Trim(TrimError::EndBeforeStart)));
    }

    #[test]
    fn the_indicator_names_only_this_files_own_choices() {
        let t = Lang::En.strings();
        assert_eq!(indicator(&FileEdit::default(), false, t), None);
        assert_eq!(indicator(&edit("0:00", ""), false, t), None);
        assert_eq!(indicator(&edit("0:30", "2:15"), false, t).as_deref(), Some("0:30 to 2:15"));
        assert_eq!(indicator(&edit("30", ""), false, t).as_deref(), Some("From 0:30"));
        assert_eq!(indicator(&edit("", "135"), false, t).as_deref(), Some("Until 2:15"));
        assert_eq!(indicator(&edit("nonsense", ""), false, t), None);
        let mut e = FileEdit { track: 1, ..edit("0:30", "2:15") };
        e.set_louder(true, false);
        assert_eq!(indicator(&e, false, t).as_deref(), Some("0:30 to 2:15 · Track 2 · Louder"));
        // Louder for every file: only a file that opts out says so.
        assert_eq!(indicator(&e, true, t).as_deref(), Some("0:30 to 2:15 · Track 2"));
        let mut quiet = FileEdit::default();
        quiet.set_louder(false, true);
        assert_eq!(indicator(&quiet, true, t).as_deref(), Some("Original volume"));
        let de = Lang::De.strings();
        assert_eq!(indicator(&edit("0:30", "2:15"), false, de).as_deref(), Some("0:30 bis 2:15"));
    }

    #[test]
    fn tracks_are_named_with_what_is_known() {
        let t = Lang::En.strings();
        let track = |index, language: Option<&str>, name: Option<&str>, codec, decodable| AudioTrack {
            index,
            language: language.map(str::to_owned),
            name: name.map(str::to_owned),
            codec,
            channels: 2,
            rate: 48_000,
            decodable,
        };
        let all = |_: &str| true;
        assert_eq!(track_label(&track(0, None, None, None, true), t, all), "Track 1");
        assert_eq!(
            track_label(&track(0, Some("en"), None, Some("AAC"), true), t, all),
            "Track 1 · English · AAC"
        );
        assert_eq!(
            track_label(&track(1, Some("ger"), Some("Kommentar"), Some("AAC"), true), t, all),
            "Track 2 · Deutsch · Kommentar · AAC"
        );
        assert_eq!(
            track_label(&track(2, Some("und"), None, Some("Dolby Digital"), false), t, all),
            "Track 3 · Dolby Digital · can't be converted"
        );
        assert_eq!(track_label(&track(0, Some("xx-YY"), None, None, true), t, all), "Track 1 · xx-YY");
        // Without a font for its script, a language shows as its tag.
        let latin = |s: &str| s.is_ascii() || s.chars().all(|c| (c as u32) < 0x0250);
        assert_eq!(track_label(&track(0, Some("ja"), None, None, true), t, latin), "Track 1 · ja");
        assert_eq!(language_name("en-US"), Some("English"));
        assert_eq!(language_name("pt_BR"), Some("Português"));
        assert_eq!(language_name("ar"), None);
        assert_eq!(language_name(""), None);
    }
}
