//! The changes a conversion can make to the sound on the way through:
//! keeping only part of it (trim) and making it louder or quieter (volume).
//! Plain arithmetic on decoded samples, so all of it is unit tested; `mf`
//! does the reading and writing around it.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Keep only part of a file. Times are seconds from the start of the
/// source; `None` means "from the start" or "to the end". The default keeps
/// everything.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Trim {
    pub start: Option<f64>,
    pub end: Option<f64>,
}

/// Why a trim cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrimError {
    /// A time is negative or not a number.
    Invalid,
    /// The end is at or before the start.
    EndBeforeStart,
    /// The start is at or past the end of the file.
    StartPastEnd,
}

/// A checked trim: where to start, and where to stop (`None` = the end of
/// the file). Seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: Option<f64>,
}

#[allow(dead_code)] // NONE and is_none are for the window.
impl Trim {
    /// Everything: the default.
    pub const NONE: Trim = Trim {
        start: None,
        end: None,
    };

    pub fn is_none(&self) -> bool {
        self.start.is_none_or(|s| s <= 0.0) && self.end.is_none()
    }

    /// Checks the trim against the file's length in seconds (when known).
    /// A start of 0 is the same as none, and an end at or past the end of
    /// the file is the same as none; anything else must lie inside the file
    /// with the start before the end.
    pub fn resolve(&self, duration: Option<f64>) -> Result<Span, TrimError> {
        let valid = |t: Option<f64>| t.is_none_or(|t| t.is_finite() && t >= 0.0);
        if !valid(self.start) || !valid(self.end) {
            return Err(TrimError::Invalid);
        }
        let start = self.start.unwrap_or(0.0);
        let mut end = self.end;
        if let (Some(e), Some(d)) = (end, duration)
            && e >= d
        {
            end = None;
        }
        if let Some(d) = duration
            && start >= d
            && start > 0.0
        {
            return Err(TrimError::StartPastEnd);
        }
        if let Some(e) = end
            && e <= start
        {
            return Err(TrimError::EndBeforeStart);
        }
        Ok(Span { start, end })
    }
}

impl Span {
    /// How long the kept part is, when it can be known.
    pub fn length(&self, duration: Option<f64>) -> Option<f64> {
        let end = self.end.or(duration)?;
        Some((end - self.start).max(0.0))
    }

    /// The span in sample frames at `rate`: (first frame kept, first frame
    /// not kept).
    pub fn frames(&self, rate: u32) -> (u64, Option<u64>) {
        let at = |seconds: f64| (seconds * rate as f64).round() as u64;
        (at(self.start), self.end.map(at))
    }
}

/// Which part of a decoded block of `frames` frames, starting at frame
/// `at` of the source, falls inside `[start, end)`: (frames to skip, frames
/// to keep). `None` when none of it does.
pub fn cut(at: u64, frames: u64, start: u64, end: Option<u64>) -> Option<(u64, u64)> {
    let block_end = at + frames;
    let from = start.max(at);
    let to = end.map_or(block_end, |e| e.min(block_end));
    (to > from).then(|| (from - at, to - from))
}

/// Louder or quieter. The default leaves the sound exactly as decoded.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum Volume {
    #[default]
    Unchanged,
    /// Brings the loudest moment up (or down) to just below full scale:
    /// a quiet recording becomes as loud as it can be without clipping.
    Normalise,
    /// A fixed change in decibels (clamped to plus or minus `MAX_DB`), with
    /// a limiter so a boost never clips.
    Boost { db: f32 },
}

/// The loudest a sample may be after a volume change: -1 dBFS, which also
/// leaves MP3 and AAC encoders room for the small overshoot they add.
pub const CEILING: f32 = 0.891_250_9;
/// The largest boost or cut on offer, and the most normalising will add,
/// so a near-silent file does not become a wall of hiss.
pub const MAX_DB: f32 = 30.0;

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

impl Volume {
    pub fn is_unchanged(&self) -> bool {
        match self {
            Volume::Unchanged => true,
            Volume::Normalise => false,
            Volume::Boost { db } => *db == 0.0 || !db.is_finite(),
        }
    }

    /// Whether the whole file has to be measured before it is written.
    pub fn needs_measuring(&self) -> bool {
        matches!(self, Volume::Normalise)
    }

    /// The gain to apply, given the loudest sample in the file (full scale
    /// = 1.0) when `needs_measuring`.
    pub fn gain(&self, peak: f32) -> f32 {
        match self {
            Volume::Unchanged => 1.0,
            Volume::Boost { db } if db.is_finite() => db_to_gain(db.clamp(-MAX_DB, MAX_DB)),
            Volume::Boost { .. } => 1.0,
            Volume::Normalise => normalise_gain(peak),
        }
    }
}

/// Peak normalisation: the gain that puts `peak` at `CEILING`, at most
/// `MAX_DB` up. Silence is left alone.
pub fn normalise_gain(peak: f32) -> f32 {
    if !(peak.is_finite() && peak > 1e-6) {
        return 1.0;
    }
    (CEILING / peak).min(db_to_gain(MAX_DB))
}

/// The loudest sample in a block of float samples.
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// Applies a gain and keeps every sample at or below `CEILING` with a
/// look-ahead limiter, turning float samples into 16-bit ones.
///
/// The limiter works out, for each frame, the gain that would keep it under
/// the ceiling, takes the smallest such gain over the next `look` frames,
/// lets it recover slowly (`release`), and smooths it with a moving average
/// as long as the look-ahead. Because the audio is delayed by the same
/// `look` frames, the gain has always come down before a peak arrives: the
/// output cannot exceed the ceiling, and loud moments are turned down
/// smoothly instead of being clipped flat. With nothing to limit, the gain
/// stays exactly `gain` and the samples are simply scaled.
pub struct Limiter {
    gain: f32,
    channels: usize,
    look: usize,
    /// Delayed input frames, interleaved, already scaled by `gain`.
    delay: VecDeque<f32>,
    /// Increasing minimum of the wanted gains over the look-ahead, with the
    /// frame each came from.
    minima: VecDeque<(u64, f32)>,
    frame: u64,
    envelope: f32,
    release: f32,
    window: VecDeque<f32>,
    sum: f64,
}

impl Limiter {
    pub fn new(gain: f32, rate: u32, channels: u32) -> Limiter {
        let channels = channels.max(1) as usize;
        // 5 ms look-ahead, 200 ms release.
        let look = (rate as usize / 200).max(1);
        let window: VecDeque<f32> = std::iter::repeat_n(1.0, look + 1).collect();
        Limiter {
            gain,
            channels,
            look,
            delay: VecDeque::with_capacity((look + 1) * channels),
            minima: VecDeque::new(),
            frame: 0,
            envelope: 1.0,
            release: 1.0 - (-1.0 / (0.2 * rate.max(1) as f32)).exp(),
            sum: (look + 1) as f64,
            window,
        }
    }

    /// Feeds interleaved float samples and appends whatever has come out
    /// of the look-ahead as 16-bit samples.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<i16>) {
        for frame in input.chunks_exact(self.channels) {
            self.push(frame, out);
        }
    }

    /// Flushes the frames still held in the look-ahead.
    pub fn finish(&mut self, out: &mut Vec<i16>) {
        let silence = vec![0.0; self.channels];
        // Only `look` real frames remain; the extra zeros push them out.
        let real = self.delay.len() / self.channels;
        for _ in 0..real {
            self.push_scaled(&silence, out);
        }
    }

    fn push(&mut self, frame: &[f32], out: &mut Vec<i16>) {
        let scaled: Vec<f32> = frame.iter().map(|s| s * self.gain).collect();
        self.push_scaled(&scaled, out);
    }

    fn push_scaled(&mut self, frame: &[f32], out: &mut Vec<i16>) {
        let loudest = peak(frame);
        let wanted = if loudest > CEILING {
            CEILING / loudest
        } else {
            1.0
        };
        let t = self.frame;
        self.frame += 1;
        while self.minima.back().is_some_and(|(_, g)| *g >= wanted) {
            self.minima.pop_back();
        }
        self.minima.push_back((t, wanted));
        while self
            .minima
            .front()
            .is_some_and(|(at, _)| at + (self.look as u64) < t)
        {
            self.minima.pop_front();
        }
        let lowest = self.minima.front().map_or(1.0, |(_, g)| *g);
        let recovered = self.envelope + (1.0 - self.envelope) * self.release;
        self.envelope = lowest.min(recovered);
        self.window.push_back(self.envelope);
        self.sum += self.envelope as f64;
        if let Some(old) = self.window.pop_front() {
            self.sum -= old as f64;
        }
        self.delay.extend(frame.iter().copied());
        if self.delay.len() > self.look * self.channels {
            let smoothed = (self.sum / (self.look + 1) as f64) as f32;
            let g = smoothed.min(1.0);
            for _ in 0..self.channels {
                let s = self.delay.pop_front().unwrap_or(0.0) * g;
                out.push(to_i16(s.clamp(-CEILING, CEILING)));
            }
        }
    }
}

pub fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * 32_767.0).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_trim_keeps_everything() {
        assert!(Trim::NONE.is_none());
        assert!(Trim::default().is_none());
        assert!(
            Trim {
                start: Some(0.0),
                end: None
            }
            .is_none()
        );
        assert_eq!(
            Trim::NONE.resolve(Some(10.0)),
            Ok(Span {
                start: 0.0,
                end: None
            })
        );
        assert_eq!(
            Trim::NONE.resolve(None),
            Ok(Span {
                start: 0.0,
                end: None
            })
        );
        assert_eq!(
            Trim::NONE.resolve(Some(10.0)).unwrap().length(Some(10.0)),
            Some(10.0)
        );
        assert_eq!(Trim::NONE.resolve(None).unwrap().length(None), None);
    }

    #[test]
    fn trims_are_checked_and_clamped_to_the_file() {
        let t = |s, e| Trim { start: s, end: e };
        assert_eq!(
            t(Some(2.0), Some(5.0)).resolve(Some(10.0)),
            Ok(Span {
                start: 2.0,
                end: Some(5.0)
            })
        );
        // An end past the file means "to the end".
        let span = t(Some(2.0), Some(50.0)).resolve(Some(10.0)).unwrap();
        assert_eq!(
            span,
            Span {
                start: 2.0,
                end: None
            }
        );
        assert_eq!(span.length(Some(10.0)), Some(8.0));
        // Unknown length: the end is taken as given.
        assert_eq!(
            t(None, Some(50.0)).resolve(None).unwrap().length(None),
            Some(50.0)
        );
        assert_eq!(
            t(Some(5.0), Some(5.0)).resolve(Some(10.0)),
            Err(TrimError::EndBeforeStart)
        );
        assert_eq!(
            t(Some(6.0), Some(5.0)).resolve(None),
            Err(TrimError::EndBeforeStart)
        );
        assert_eq!(
            t(Some(10.0), None).resolve(Some(10.0)),
            Err(TrimError::StartPastEnd)
        );
        assert_eq!(
            t(Some(12.0), Some(15.0)).resolve(Some(10.0)),
            Err(TrimError::StartPastEnd)
        );
        assert_eq!(t(Some(-1.0), None).resolve(None), Err(TrimError::Invalid));
        assert_eq!(
            t(None, Some(f64::NAN)).resolve(None),
            Err(TrimError::Invalid)
        );
        assert_eq!(
            t(Some(1.5), Some(2.0))
                .resolve(None)
                .unwrap()
                .frames(48_000),
            (72_000, Some(96_000))
        );
    }

    #[test]
    fn cut_keeps_only_frames_inside_the_span() {
        // Block entirely before, inside, after.
        assert_eq!(cut(0, 100, 200, None), None);
        assert_eq!(cut(300, 100, 200, Some(1000)), Some((0, 100)));
        assert_eq!(cut(1000, 100, 200, Some(1000)), None);
        // Straddling the start, the end, and both.
        assert_eq!(cut(150, 100, 200, None), Some((50, 50)));
        assert_eq!(cut(950, 100, 200, Some(1000)), Some((0, 50)));
        assert_eq!(cut(100, 1000, 200, Some(300)), Some((100, 100)));
    }

    #[test]
    fn volume_gains() {
        assert_eq!(Volume::default(), Volume::Unchanged);
        assert!(Volume::Unchanged.is_unchanged());
        assert!(Volume::Boost { db: 0.0 }.is_unchanged());
        assert!(!Volume::Normalise.is_unchanged());
        assert!((Volume::Boost { db: 6.0 }.gain(0.0) - 1.995).abs() < 0.01);
        assert!((Volume::Boost { db: -6.0 }.gain(0.0) - 0.501).abs() < 0.01);
        // Clamped to +30 dB.
        assert!((Volume::Boost { db: 90.0 }.gain(0.0) - 31.62).abs() < 0.1);
        // Normalising a quiet file brings its peak to the ceiling...
        assert!((Volume::Normalise.gain(0.1) * 0.1 - CEILING).abs() < 1e-6);
        // ...turns a clipping one down a little...
        assert!(Volume::Normalise.gain(1.0) < 1.0);
        // ...adds no more than 30 dB, and leaves silence alone.
        assert!((Volume::Normalise.gain(0.0001) - 31.62).abs() < 0.1);
        assert_eq!(Volume::Normalise.gain(0.0), 1.0);
    }

    fn sine(frames: usize, channels: usize, amplitude: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let v = (i as f32 * 440.0 / 48_000.0 * std::f32::consts::TAU).sin() * amplitude;
                std::iter::repeat_n(v, channels)
            })
            .collect()
    }

    fn run(limiter: &mut Limiter, input: &[f32], block: usize) -> Vec<i16> {
        let mut out = Vec::new();
        for chunk in input.chunks(block) {
            limiter.process(chunk, &mut out);
        }
        limiter.finish(&mut out);
        out
    }

    #[test]
    fn limiter_passes_quiet_audio_through_scaled_and_in_order() {
        let input = sine(4_800, 2, 0.1);
        let mut limiter = Limiter::new(2.0, 48_000, 2);
        // Blocks of whole frames, as the reader delivers them.
        let out = run(&mut limiter, &input, 666);
        assert_eq!(out.len(), input.len());
        for (a, b) in input.iter().zip(&out) {
            assert!((to_i16(a * 2.0) - b).abs() <= 1, "{a} {b}");
        }
    }

    #[test]
    fn limiter_never_exceeds_the_ceiling() {
        // A loud sine with a sudden spike, boosted by 20 dB.
        let mut input = sine(48_000, 1, 0.3);
        input[24_000] = 1.0;
        let gain = db_to_gain(20.0);
        let mut limiter = Limiter::new(gain, 48_000, 1);
        let out = run(&mut limiter, &input, 1024);
        assert_eq!(out.len(), input.len());
        let limit = to_i16(CEILING);
        let loudest = out.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(loudest <= limit as u16, "{loudest} > {limit}");
        // And it is still loud: close to the ceiling, not squashed.
        assert!(loudest as f32 > limit as f32 * 0.95, "{loudest}");
    }

    #[test]
    fn peaks_and_conversions() {
        assert_eq!(peak(&[0.1, -0.5, 0.3]), 0.5);
        assert_eq!(peak(&[]), 0.0);
        assert_eq!(to_i16(1.5), 32_767);
        assert_eq!(to_i16(-1.5), -32_767);
    }
}
