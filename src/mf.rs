//! Reading and writing audio through Windows Media Foundation. Windows owns
//! every decoder and encoder used here (MP4/AAC, MP3, FLAC, WMA and so on),
//! so the app ships no codecs of its own.
//!
//! The decoded sound passes through this module as plain PCM, which is
//! where a trim is cut to the sample and a volume change applied (see
//! `edit`), before it goes to Windows' encoder or straight into a WAV file.

pub mod edit;

use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::Win32::Media::MediaFoundation::{
    IMFMediaBuffer, IMFMediaType, IMFSample, IMFSinkWriter, IMFSourceReader, MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
    MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_PD_DURATION,
    MF_SD_LANGUAGE, MF_SD_STREAM_NAME, MF_SOURCE_READER_ALL_STREAMS,
    MF_SOURCE_READER_MEDIASOURCE, MF_SOURCE_READERF_ENDOFSTREAM, MF_VERSION, MFAudioFormat_AAC,
    MFAudioFormat_ADTS, MFAudioFormat_ALAC, MFAudioFormat_AMR_NB, MFAudioFormat_AMR_WB,
    MFAudioFormat_Dolby_AC3, MFAudioFormat_Dolby_DDPlus, MFAudioFormat_FLAC, MFAudioFormat_Float,
    MFAudioFormat_MP3, MFAudioFormat_MPEG, MFAudioFormat_Opus, MFAudioFormat_PCM,
    MFAudioFormat_Vorbis, MFAudioFormat_WMAudio_Lossless, MFAudioFormat_WMAudioV8,
    MFAudioFormat_WMAudioV9, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
    MFCreateSinkWriterFromURL, MFCreateSourceReaderFromURL, MFMediaType_Audio, MFSTARTUP_LITE,
    MFStartup, MFT_ENUM_FLAG_ALL, MFTranscodeGetAudioOutputAvailableTypes,
};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Variant::{VT_I8, VT_LPWSTR, VT_UI8};
use windows::core::{GUID, HSTRING, Interface};

pub use edit::{Trim, TrimError, Volume};

/// How far one conversion has got, and whether to stop. Shared between the
/// worker converting a file and the window showing it.
#[derive(Default, Debug)]
pub struct Progress {
    /// Thousandths done.
    permille: AtomicU32,
    /// Whether the source's length is known, so `permille` means something.
    known: AtomicBool,
    /// Whether a worker has picked the file up.
    started: AtomicBool,
    cancel: AtomicBool,
}

impl Progress {
    fn begin(&self, known: bool) {
        self.known.store(known, Ordering::Relaxed);
        self.started.store(true, Ordering::Relaxed);
    }

    fn set(&self, permille: u32) {
        self.permille.store(permille.min(1000), Ordering::Relaxed);
    }

    /// 0.0 to 1.0, or `None` when the length is unknown.
    pub fn fraction(&self) -> Option<f32> {
        self.known
            .load(Ordering::Relaxed)
            .then(|| self.permille.load(Ordering::Relaxed) as f32 / 1000.0)
    }

    pub fn started(&self) -> bool {
        self.started.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Thousandths of `duration` reached at `position` (any unit, both the
/// same), held below 1000 until the file is really finished.
pub fn permille(position: i64, duration: u64) -> u32 {
    if duration == 0 || position <= 0 {
        return 0;
    }
    ((position as u128 * 1000 / duration as u128) as u32).min(999)
}

/// What to write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Mp3 { kbps: u32 },
    M4a { kbps: u32 },
    Flac,
    Wav,
}

/// Changes made to the sound of one file on the way through. The default
/// converts the first audio track, whole, at its own volume.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edits {
    /// Only this part of the file.
    pub trim: Trim,
    /// Which audio track, counting audio tracks only from 0 (the order of
    /// `audio_tracks`).
    pub track: usize,
    pub volume: Volume,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MfError {
    /// The file opened but has no audio track (a silent screen recording).
    NoAudio,
    /// Windows has no reader for this file: unknown, damaged, or a format
    /// that needs a codec extension.
    Unreadable(String),
    /// The chosen audio track does not exist.
    NoSuchTrack,
    /// The trim does not fit the file.
    Trim(TrimError),
    /// Stopped on request; nothing was written.
    Cancelled,
    Other(String),
}

impl From<windows::core::Error> for MfError {
    fn from(e: windows::core::Error) -> Self {
        MfError::Other(error_text(&e))
    }
}

/// Windows' explanation of an error, or its code when it has none (as for
/// a codec that gives up on a stream it cannot decode).
fn error_text(e: &windows::core::Error) -> String {
    let message = e.message();
    if message.trim().is_empty() {
        format!("Windows error 0x{:08X}", e.code().0 as u32)
    } else {
        message
    }
}

const ALL_STREAMS: u32 = MF_SOURCE_READER_ALL_STREAMS.0 as u32;
/// MF_E_INVALIDSTREAMNUMBER: past the last stream of a file.
const INVALID_STREAM: u32 = 0xC00D_36B3;
/// 100 ns units per second, Media Foundation's clock.
const TICKS: u64 = 10_000_000;

/// The sample rate and channel count a target can hold, nearest to the
/// source's. MP3 and AAC take mono or stereo at 44.1 or 48 kHz; FLAC and WAV
/// keep the rate and up to stereo is kept as is (more channels are mixed
/// down to stereo so the file plays everywhere). `mono` mixes everything
/// down to one channel, for speech.
pub fn output_shape(target: Target, rate: u32, channels: u32, mono: bool) -> (u32, u32) {
    let channels = if mono { 1 } else { channels.clamp(1, 2) };
    let rate = match target {
        Target::Mp3 { .. } | Target::M4a { .. } => {
            if rate == 48_000 || rate == 96_000 || rate == 24_000 || rate == 12_000 || rate == 8_000
            {
                48_000
            } else {
                44_100
            }
        }
        Target::Flac | Target::Wav => rate.clamp(8_000, 96_000),
    };
    (rate, channels)
}

/// Uncompressed audio: 16-bit integer, or 32-bit float when the volume is
/// being changed (so nothing is lost or clipped before the limiter).
fn pcm_type(rate: u32, channels: u32, float: bool) -> windows::core::Result<IMFMediaType> {
    let bytes = if float { 4 } else { 2 };
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        t.SetGUID(&MF_MT_SUBTYPE, if float { &MFAudioFormat_Float } else { &MFAudioFormat_PCM })?;
        t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, bytes * 8)?;
        t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels)?;
        t.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, channels * bytes)?;
        t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * channels * bytes)?;
        Ok(t)
    }
}

/// The encoder's own output type for this rate and channel count whose
/// bitrate is nearest to `kbps` (encoders only accept the exact types they
/// list). `kbps` 0 means "any", for lossless formats.
fn encoder_type(
    subtype: &GUID,
    rate: u32,
    channels: u32,
    kbps: u32,
) -> Result<IMFMediaType, MfError> {
    unsafe {
        let types = MFTranscodeGetAudioOutputAvailableTypes(subtype, MFT_ENUM_FLAG_ALL.0 as u32, None)?;
        let wanted = kbps * 1000 / 8;
        let mut best: Option<(u32, IMFMediaType)> = None;
        for i in 0..types.GetElementCount()? {
            let t: IMFMediaType = types.GetElement(i)?.cast()?;
            if t.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(0) != rate
                || t.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(0) != channels
            {
                continue;
            }
            if let Ok(bits) = t.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE)
                && bits != 16
            {
                continue;
            }
            let bytes = t.GetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND).unwrap_or(0);
            let distance = bytes.abs_diff(wanted);
            if best.as_ref().is_none_or(|(d, _)| distance < *d) {
                best = Some((distance, t));
            }
        }
        best.map(|(_, t)| t)
            .ok_or_else(|| MfError::Other("Windows has no encoder for this format".to_owned()))
    }
}

fn startup() -> windows::core::Result<()> {
    unsafe {
        // Both are per-thread/process reference counted and safe to repeat.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_LITE)
    }
}

fn create_reader(source: &Path) -> Result<IMFSourceReader, MfError> {
    unsafe {
        MFCreateSourceReaderFromURL(&HSTRING::from(source.as_os_str()), None)
            .map_err(|e| MfError::Unreadable(error_text(&e)))
    }
}

/// One audio stream as the reader numbers it.
struct Stream {
    /// The reader's stream number.
    index: u32,
    native: IMFMediaType,
}

/// The container, from the file's first bytes, where it matters: how
/// Windows numbers tracks, and whether its timestamps after a seek can be
/// trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Container {
    /// MP4, M4A, M4B, MOV, 3GP and the like.
    Mp4,
    /// A bare MP3/MP2 or ADTS AAC stream: no index and no timestamps of
    /// its own, so Windows estimates where a seek lands.
    Elementary,
    /// ASF: WMA and WMV.
    Asf,
    Other,
}

const ASF_HEADER: [u8; 8] = [0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11];

fn container_of(head: &[u8]) -> Container {
    if head.len() >= 8 && [&b"ftyp"[..], b"moov", b"mdat", b"wide", b"free", b"skip"].contains(&&head[4..8]) {
        Container::Mp4
    } else if head.starts_with(b"ID3") || (head.len() >= 2 && head[0] == 0xFF && head[1] & 0xE0 == 0xE0) {
        Container::Elementary
    } else if head.starts_with(&ASF_HEADER) {
        Container::Asf
    } else {
        Container::Other
    }
}

fn container(path: &Path) -> Container {
    use std::io::Read;
    let mut head = [0u8; 12];
    let read = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    container_of(&head[..read])
}

/// The file's audio streams, in the order the file has them, which is the
/// order other players list them in. Windows' MP4 reader numbers the
/// tracks of an MP4 or QuickTime file last to first (checked with files
/// of two and three audio tracks, with and without video), so for those
/// the order is turned round.
fn audio_streams(reader: &IMFSourceReader, mp4: bool) -> Vec<Stream> {
    let mut found = Vec::new();
    // No real file has anywhere near this many streams; the limit only
    // guards against a reader that never says "no more".
    for index in 0..256u32 {
        match unsafe { reader.GetNativeMediaType(index, 0) } {
            Ok(native) => {
                let audio = unsafe { native.GetGUID(&MF_MT_MAJOR_TYPE) }
                    .is_ok_and(|major| major == MFMediaType_Audio);
                if audio {
                    found.push(Stream { index, native });
                }
            }
            Err(e) if e.code().0 as u32 == INVALID_STREAM => break,
            // A stream Windows cannot describe (some subtitle formats): skip.
            Err(_) => {}
        }
    }
    if mp4 {
        found.reverse();
    }
    found
}

/// The sample rate and channel count of a stream. Some containers (AMR in
/// 3GP, for one) leave them out of the stream's description; the decoder's
/// own output says. With `check`, also whether a decoder takes it at all
/// (otherwise that is only tried when the shape is missing).
fn stream_shape(reader: &IMFSourceReader, stream: &Stream, check: bool) -> (Option<(u32, u32)>, bool) {
    unsafe {
        let rate = stream.native.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).ok().filter(|r| *r > 0);
        let channels = stream.native.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).ok().filter(|c| *c > 0);
        if let (Some(r), Some(c), false) = (rate, channels, check) {
            return (Some((r, c)), true);
        }
        // Asking for "PCM, any shape" loads the decoder, if there is one.
        let decoded = MFCreateMediaType().ok().and_then(|t| {
            t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).ok()?;
            t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).ok()?;
            reader.SetCurrentMediaType(stream.index, None, &t).ok()?;
            let current = reader.GetCurrentMediaType(stream.index).ok()?;
            Some((
                current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).ok()?,
                current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).ok()?,
            ))
        });
        let shape = match (rate, channels) {
            (Some(r), Some(c)) => Some((r, c)),
            _ => decoded,
        };
        (shape, decoded.is_some())
    }
}

/// Whether a trim may seek, or must decode from the start and drop what
/// comes before it. Seeking was measured against a decode from the start
/// (MP4, M4A, MKV, FLAC, WAV, AMR, MPEG program and transport streams, AVI,
/// MP3 inside MKV and AVI): all land on the exact sample, give or take one.
/// Bare MP3 and ADTS streams came back up to 1200 samples out and WMA 16
/// (WMV was exact, but it is the same ASF reader), so those are read from
/// the start: slower on a long file, but exact. A one-hour MP3 trimmed at
/// 55 minutes takes about 7 seconds.
fn seeks_exactly(container: Container) -> bool {
    matches!(container, Container::Mp4 | Container::Other)
}
/// An open file with one audio track chosen.
struct Source {
    reader: IMFSourceReader,
    stream: u32,
    rate: u32,
    channels: u32,
    exact_seek: bool,
    path: std::path::PathBuf,
    track: usize,
    /// What `decode_to` asked for: (rate, channels, float).
    decoded: std::cell::Cell<(u32, u32, bool)>,
}

fn open_track(source: &Path, track: usize) -> Result<Source, MfError> {
    let reader = create_reader(source)?;
    let kind = container(source);
    let streams = audio_streams(&reader, kind == Container::Mp4);
    if streams.is_empty() {
        return Err(MfError::NoAudio);
    }
    let stream = streams.into_iter().nth(track).ok_or(MfError::NoSuchTrack)?;
    let (rate, channels) = stream_shape(&reader, &stream, false).0.unwrap_or((44_100, 2));
    unsafe {
        reader.SetStreamSelection(ALL_STREAMS, false)?;
        reader.SetStreamSelection(stream.index, true)?;
    }
    Ok(Source {
        reader,
        stream: stream.index,
        rate,
        channels,
        exact_seek: seeks_exactly(kind),
        path: source.to_path_buf(),
        track,
        decoded: std::cell::Cell::new((rate, channels, false)),
    })
}

impl Source {
    /// Asks the reader to decode, resample and mix down to exactly this.
    /// Fails when Windows has no decoder for the track (Dolby Digital on
    /// current Windows 11, for one).
    fn decode_to(&self, rate: u32, channels: u32, float: bool) -> Result<(), MfError> {
        unsafe {
            let pcm = pcm_type(rate, channels, float)?;
            self.reader
                .SetCurrentMediaType(self.stream, None, &pcm)
                .map_err(|e| MfError::Unreadable(error_text(&e)))?;
            self.reader.SetStreamSelection(self.stream, true)?;
        }
        self.decoded.set((rate, channels, float));
        Ok(())
    }

    /// The time of the track's first decoded sample, read with a reader of
    /// its own: reading on, then seeking back, upsets some decoders (FLAC
    /// comes back a few thousand samples out).
    fn first_time(&self) -> Result<Option<i64>, MfError> {
        let other = open_track(&self.path, self.track)?;
        let (rate, channels, float) = self.decoded.get();
        other.decode_to(rate, channels, float)?;
        Ok(other.next_sample()?.map(|s| unsafe { s.GetSampleTime() }.unwrap_or(0)))
    }

    /// Moves the read position to `ticks` (100 ns units). Windows lands on
    /// or before it, as near as the format allows.
    fn seek(&self, ticks: u64) -> windows::core::Result<()> {
        let mut at = PROPVARIANT::default();
        unsafe {
            let inner = &mut *at.Anonymous.Anonymous;
            inner.vt = VT_I8;
            inner.Anonymous.hVal = ticks.min(i64::MAX as u64) as i64;
            self.reader.SetCurrentPosition(&GUID::zeroed(), &at)
        }
    }

    /// Reads the next block of audio, or `None` at the end of the track.
    fn next_sample(&self) -> Result<Option<IMFSample>, MfError> {
        unsafe {
            loop {
                let mut flags = 0u32;
                let mut sample: Option<IMFSample> = None;
                self.reader
                    .ReadSample(self.stream, 0, None, Some(&mut flags), None, Some(&mut sample))?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    return Ok(None);
                }
                // A stream tick or format change carries no data: keep reading.
                if sample.is_some() {
                    return Ok(sample);
                }
            }
        }
    }
}

/// The source's length in 100 ns units, when the file says.
fn duration(reader: &IMFSourceReader) -> Option<u64> {
    unsafe {
        let mut value = reader
            .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
            .ok()?;
        let inner = &value.Anonymous.Anonymous;
        let length = (inner.vt == VT_UI8).then(|| inner.Anonymous.uhVal);
        let _ = PropVariantClear(&mut value);
        length.filter(|d| *d > 0)
    }
}

/// A text attribute of one stream (its language or name), when the file
/// has one.
fn stream_text(reader: &IMFSourceReader, stream: u32, key: &GUID) -> Option<String> {
    unsafe {
        let mut value = reader.GetPresentationAttribute(stream, key).ok()?;
        let inner = &value.Anonymous.Anonymous;
        let text = (inner.vt == VT_LPWSTR && !inner.Anonymous.pwszVal.is_null())
            .then(|| inner.Anonymous.pwszVal.to_string().ok())
            .flatten();
        let _ = PropVariantClear(&mut value);
        text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
    }
}

/// What the window shows about a file before converting it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    /// Length in seconds, when known.
    pub seconds: Option<f64>,
    pub rate: u32,
    pub channels: u32,
}

impl Info {
    /// The same file as it will be after `trim`: only the length changes,
    /// so `estimate_bytes` on the result sizes the trimmed output. A trim
    /// that does not fit the file leaves it as it is.
    pub fn trimmed(&self, trim: &Trim) -> Info {
        let seconds = match trim.resolve(self.seconds) {
            Ok(span) => span.length(self.seconds),
            Err(_) => self.seconds,
        };
        Info { seconds, ..*self }
    }
}

/// Opens `source` just far enough to learn its length and the shape of its
/// first audio track. `None` when Windows cannot read its audio. (The
/// window asks `probe_track` for the track a row has chosen.)
#[cfg(test)]
pub fn probe(source: &Path) -> Option<Info> {
    probe_track(source, 0)
}

/// `probe` for the audio track `track` (counting from 0, as in
/// `audio_tracks`).
pub fn probe_track(source: &Path, track: usize) -> Option<Info> {
    startup().ok()?;
    let src = open_track(source, track).ok()?;
    Some(Info {
        seconds: duration(&src.reader).map(|d| d as f64 / TICKS as f64),
        rate: src.rate,
        channels: src.channels,
    })
}

/// One audio track of a file, as `audio_tracks` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioTrack {
    /// Counting audio tracks only, from 0: the value for `Edits::track`.
    pub index: usize,
    /// The language the file gives (an RFC 1766 tag such as "en" or
    /// "en-US", as Windows reports it), if any.
    pub language: Option<String>,
    /// The track's own name ("Commentary"), if the file has one.
    pub name: Option<String>,
    /// The codec, for display ("AAC", "MP3", "Dolby Digital"...), when known.
    pub codec: Option<&'static str>,
    /// 0 when neither the file nor a decoder says.
    pub channels: u32,
    pub rate: u32,
    /// Whether Windows can decode it. Dolby Digital tracks are listed but
    /// cannot be converted on current Windows.
    pub decodable: bool,
}

fn codec_name(subtype: &GUID) -> Option<&'static str> {
    let names: [(&GUID, &str); 16] = [
        (&MFAudioFormat_AAC, "AAC"),
        (&MFAudioFormat_ADTS, "AAC"),
        (&MFAudioFormat_MP3, "MP3"),
        (&MFAudioFormat_MPEG, "MPEG audio"),
        (&MFAudioFormat_FLAC, "FLAC"),
        (&MFAudioFormat_ALAC, "ALAC"),
        (&MFAudioFormat_PCM, "PCM"),
        (&MFAudioFormat_Float, "PCM"),
        (&MFAudioFormat_Opus, "Opus"),
        (&MFAudioFormat_Vorbis, "Vorbis"),
        (&MFAudioFormat_WMAudioV8, "WMA"),
        (&MFAudioFormat_WMAudioV9, "WMA Pro"),
        (&MFAudioFormat_WMAudio_Lossless, "WMA Lossless"),
        (&MFAudioFormat_AMR_NB, "AMR"),
        (&MFAudioFormat_Dolby_AC3, "Dolby Digital"),
        (&MFAudioFormat_Dolby_DDPlus, "Dolby Digital Plus"),
    ];
    // MP4 and 3GP readers describe AMR by its four-character code.
    let fourcc = |code: &[u8; 4]| subtype.data1 == u32::from_le_bytes(*code) && subtype.data4 == MFAudioFormat_PCM.data4;
    names.iter().find(|(g, _)| *g == subtype).map(|(_, n)| *n).or_else(|| {
        if *subtype == MFAudioFormat_AMR_WB || fourcc(b"sawb") {
            Some("AMR-WB")
        } else if fourcc(b"samr") {
            Some("AMR")
        } else {
            None
        }
    })
}

/// Lists the audio tracks of `source`, in the order `Edits::track` counts
/// them. Most files have one; films and screen recordings can have several
/// (languages, commentary, microphone and system sound).
pub fn audio_tracks(source: &Path) -> Result<Vec<AudioTrack>, MfError> {
    startup()?;
    let reader = create_reader(source)?;
    let streams = audio_streams(&reader, container(source) == Container::Mp4);
    if streams.is_empty() {
        return Err(MfError::NoAudio);
    }
    let mut tracks = Vec::with_capacity(streams.len());
    for (index, stream) in streams.into_iter().enumerate() {
        let (shape, decodable) = stream_shape(&reader, &stream, true);
        let (rate, channels) = shape.unwrap_or((0, 0));
        let codec = unsafe { stream.native.GetGUID(&MF_MT_SUBTYPE) }
            .ok()
            .and_then(|g| codec_name(&g));
        tracks.push(AudioTrack {
            index,
            language: stream_text(&reader, stream.index, &MF_SD_LANGUAGE),
            name: stream_text(&reader, stream.index, &MF_SD_STREAM_NAME),
            codec,
            channels,
            rate,
            decodable,
        });
    }
    Ok(tracks)
}

/// Where converted audio goes: Windows' encoder, or a WAV file written
/// here. Takes 16-bit PCM and timestamps it from zero.
enum Output {
    Encoder {
        writer: IMFSinkWriter,
        stream: u32,
        rate: u32,
        frame_bytes: u32,
        /// Frames handed to the writer so far.
        frames: u64,
        /// The buffer being filled for the next `WriteSample`, and how many
        /// bytes are in it (see `ENCODER_BATCH`).
        batch: Option<IMFMediaBuffer>,
        filled: usize,
    },
    Wav {
        out: std::io::BufWriter<std::fs::File>,
        rate: u32,
        channels: u32,
        bytes: u64,
    },
}

/// Bytes of PCM gathered into one sample for Windows' encoder: about 5.5 s
/// of 48 kHz stereo, a whole number of frames for any 16-bit shape. Handing
/// over each decoded block (4 KB or so) on its own left the worker waiting
/// on the sink writer most of the time. Measured on a 4-minute stereo file:
/// to MP3 8.3 s with 4 KB samples, 1.6 s with 256 KB, 1.4 s from 512 KB up;
/// to M4A 8.7 s, 3.7 s, 1.5 s from 1 MB up; to FLAC 7.9 s, 3.5 s, 1.7 s at
/// 1 MB and 0.85 s from 2 MB up. 2 MB cost four workers 30-50 MB more at
/// the peak than 1 MB for only FLAC's gain, so 1 MB.
const ENCODER_BATCH: usize = 1024 * 1024;

fn io(e: std::io::Error) -> MfError {
    MfError::Other(e.to_string())
}

impl Output {
    fn open(dest: &Path, target: Target, rate: u32, channels: u32) -> Result<Output, MfError> {
        if target == Target::Wav {
            let file = std::fs::File::create(dest).map_err(io)?;
            let mut out = std::io::BufWriter::new(file);
            out.write_all(&wav_header(rate, channels, 0)).map_err(io)?;
            return Ok(Output::Wav {
                out,
                rate,
                channels,
                bytes: 0,
            });
        }
        let out_type = match target {
            Target::Mp3 { kbps } => encoder_type(&MFAudioFormat_MP3, rate, channels, kbps)?,
            Target::M4a { kbps } => encoder_type(&MFAudioFormat_AAC, rate, channels, kbps)?,
            Target::Flac => unsafe {
                // The FLAC encoder lists no types to choose from; it takes
                // the PCM shape as given.
                let t = pcm_type(rate, channels, false)?;
                t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_FLAC)?;
                t
            },
            Target::Wav => unreachable!(),
        };
        unsafe {
            let writer: IMFSinkWriter =
                MFCreateSinkWriterFromURL(&HSTRING::from(dest.as_os_str()), None, None)?;
            let stream = writer.AddStream(&out_type)?;
            writer.SetInputMediaType(stream, &pcm_type(rate, channels, false)?, None)?;
            writer.BeginWriting()?;
            Ok(Output::Encoder {
                writer,
                stream,
                rate,
                frame_bytes: channels * 2,
                frames: 0,
                batch: None,
                filled: 0,
            })
        }
    }

    fn write(&mut self, mut pcm: &[u8]) -> Result<(), MfError> {
        while !pcm.is_empty() {
            let full = match self {
                Output::Wav { out, bytes, .. } => {
                    out.write_all(pcm).map_err(io)?;
                    *bytes += pcm.len() as u64;
                    return Ok(());
                }
                Output::Encoder { batch, filled, .. } => {
                    let buffer = match batch {
                        Some(buffer) => buffer,
                        None => batch.insert(unsafe { MFCreateMemoryBuffer(ENCODER_BATCH as u32)? }),
                    };
                    // Whole frames only: `pcm` holds whole frames, and the
                    // batch is a multiple of every frame size.
                    let n = pcm.len().min(ENCODER_BATCH - *filled);
                    unsafe {
                        let mut data = std::ptr::null_mut();
                        buffer.Lock(&mut data, None, None)?;
                        std::ptr::copy_nonoverlapping(pcm.as_ptr(), data.add(*filled), n);
                        buffer.Unlock()?;
                    }
                    *filled += n;
                    pcm = &pcm[n..];
                    *filled == ENCODER_BATCH
                }
            };
            if full {
                self.flush()?;
            }
        }
        Ok(())
    }

    /// Hands the gathered PCM to the encoder as one sample.
    fn flush(&mut self) -> Result<(), MfError> {
        let Output::Encoder {
            writer,
            stream,
            rate,
            frame_bytes,
            frames,
            batch,
            filled,
        } = self
        else {
            return Ok(());
        };
        let Some(buffer) = batch.take() else { return Ok(()) };
        let len = std::mem::take(filled) as u32;
        if len == 0 {
            return Ok(());
        }
        unsafe {
            buffer.SetCurrentLength(len)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            let count = (len / *frame_bytes) as u64;
            let at = |f: u64| (f as u128 * TICKS as u128 / *rate as u128) as i64;
            sample.SetSampleTime(at(*frames))?;
            sample.SetSampleDuration(at(*frames + count) - at(*frames))?;
            writer.WriteSample(*stream, &sample)?;
            *frames += count;
        }
        Ok(())
    }

    fn written(&self) -> bool {
        match self {
            Output::Wav { bytes, .. } => *bytes > 0,
            Output::Encoder { frames, filled, .. } => *frames > 0 || *filled > 0,
        }
    }

    fn finish(mut self) -> Result<(), MfError> {
        self.flush()?;
        match self {
            Output::Encoder { writer, .. } => unsafe { Ok(writer.Finalize()?) },
            Output::Wav {
                mut out,
                rate,
                channels,
                bytes,
            } => {
                // A WAV file holds at most 4 GB (over six hours of
                // CD-quality stereo).
                let size = u32::try_from(bytes)
                    .ok()
                    .filter(|b| *b <= u32::MAX - 36)
                    .ok_or_else(|| MfError::Other("Too long for a WAV file".to_owned()))?;
                out.seek(SeekFrom::Start(0)).map_err(io)?;
                out.write_all(&wav_header(rate, channels, size)).map_err(io)?;
                out.flush().map_err(io)
            }
        }
    }
}

/// Converts `source` into `dest`. `dest` must not exist; on failure or
/// cancel nothing is left behind. `progress` is updated as the audio is
/// read and checked for a cancel between blocks.
pub fn transcode(
    source: &Path,
    dest: &Path,
    target: Target,
    mono: bool,
    edits: &Edits,
    progress: &Progress,
) -> Result<(), MfError> {
    let result = transcode_inner(source, dest, target, mono, edits, progress);
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    } else {
        progress.set(1000);
    }
    result
}

/// One pass over the kept part of a track: what it reads, and the share of
/// the progress bar it fills.
struct Pass<'a> {
    progress: &'a Progress,
    /// Frames the kept part is long, when known.
    total: Option<u64>,
    /// Thousandths of the bar already full before this pass, and how many
    /// this pass fills.
    base: u32,
    share: u32,
}

impl Pass<'_> {
    fn report(&self, done: u64) {
        if let Some(total) = self.total {
            let p = permille(done.min(i64::MAX as u64) as i64, total);
            self.progress.set(self.base + p * self.share / 1000);
        }
    }
}

/// Reads the kept part of the track, frame-exact, and hands each piece of
/// PCM (whole frames of `frame_bytes`) to `sink`. Returns the frames read.
///
/// Times count from the track's first decoded sample, so a trim cuts
/// exactly the same samples out of the result as the user would find in a
/// conversion of the whole file. Without a seek that is simply counting
/// frames. Where seeking is exact (see `seeks_exactly`), a trim notes the
/// first sample's time, seeks to a second before its start and drops the
/// decoded audio up to the exact start frame; if Windows lands after the
/// start anyway, it goes back to the beginning. Other formats are read
/// from the beginning, dropping everything before the start.
fn read_span(
    src: &Source,
    rate: u32,
    frame_bytes: u32,
    (start, end): (u64, Option<u64>),
    pass: &Pass,
    mut sink: impl FnMut(&[u8]) -> Result<(), MfError>,
) -> Result<u64, MfError> {
    let to_frames = |ticks: i64| (ticks.max(0) as u128 * rate as u128 / TICKS as u128) as u64;
    let time = |sample: &IMFSample| unsafe { sample.GetSampleTime() }.unwrap_or(0);
    let mut sought = false;
    let mut origin: Option<i64> = None;

    // Within the first second there is nothing worth skipping.
    let preroll = start.saturating_sub(rate as u64);
    if preroll > 0 && src.exact_seek
        && let Some(first) = src.first_time()?
    {
        origin = Some(first);
        sought = src
            .seek((preroll as u128 * TICKS as u128 / rate as u128) as u64)
            .is_ok();
    }
    let mut position: Option<u64> = None;
    let mut done = 0u64;
    while let Some(sample) = src.next_sample()? {
        if pass.progress.cancelled() {
            return Err(MfError::Cancelled);
        }
        let at = match position {
            Some(p) => p,
            None => {
                let t = time(&sample);
                let p = to_frames(t - *origin.get_or_insert(t));
                if sought && p > start {
                    // Landed past the start: read it all from the top.
                    sought = false;
                    src.seek(0)?;
                    continue;
                }
                p
            }
        };
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer()?;
            let mut data = std::ptr::null_mut();
            let mut len = 0u32;
            buffer.Lock(&mut data, None, Some(&mut len))?;
            let bytes = std::slice::from_raw_parts(data, len as usize);
            let frames = (len / frame_bytes) as u64;
            let kept = edit::cut(at, frames, start, end).map(|(skip, take)| {
                let from = (skip * frame_bytes as u64) as usize;
                (take, &bytes[from..from + (take * frame_bytes as u64) as usize])
            });
            let result = match kept {
                Some((take, piece)) => {
                    done += take;
                    sink(piece)
                }
                None => Ok(()),
            };
            buffer.Unlock()?;
            result?;
            position = Some(at + frames);
        }
        pass.report(done);
        if end.is_some_and(|e| position.is_some_and(|p| p >= e)) {
            break;
        }
    }
    Ok(done)
}

fn floats(bytes: &[u8]) -> impl Iterator<Item = f32> + '_ {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn transcode_inner(
    source: &Path,
    dest: &Path,
    target: Target,
    mono: bool,
    edits: &Edits,
    progress: &Progress,
) -> Result<(), MfError> {
    startup()?;
    let mut src = open_track(source, edits.track)?;
    let length = duration(&src.reader).map(|d| d as f64 / TICKS as f64);
    let span = edits.trim.resolve(length).map_err(MfError::Trim)?;
    let kept = span.length(length);
    progress.begin(kept.is_some());
    if progress.cancelled() {
        return Err(MfError::Cancelled);
    }
    let (rate, channels) = output_shape(target, src.rate, src.channels, mono);
    let frames = span.frames(rate);
    let total = kept.map(|s| (s * rate as f64).round() as u64);
    let volume = edits.volume;
    let float = !volume.is_unchanged();
    let frame_bytes = channels * if float { 4 } else { 2 };
    src.decode_to(rate, channels, float)?;

    let mut gain = volume.gain(0.0);
    let mut base = 0;
    if volume.needs_measuring() {
        // Pass one: find the loudest moment of the kept part.
        let mut loudest = 0f32;
        let pass = Pass { progress, total, base: 0, share: 500 };
        read_span(&src, rate, frame_bytes, frames, &pass, |pcm| {
            loudest = loudest.max(floats(pcm).fold(0.0, |m, s| m.max(s.abs())));
            Ok(())
        })?;
        gain = volume.gain(loudest);
        base = 500;
        // Read it again from a fresh start: not every format seeks back.
        src = open_track(source, edits.track)?;
        src.decode_to(rate, channels, float)?;
    }

    let mut out = Output::open(dest, target, rate, channels)?;
    let pass = Pass { progress, total, base, share: 1000 - base };
    if float {
        let mut limiter = edit::Limiter::new(gain, rate, channels);
        let mut block: Vec<i16> = Vec::new();
        let mut bytes: Vec<u8> = Vec::new();
        let mut samples: Vec<f32> = Vec::new();
        let mut emit = |block: &mut Vec<i16>, out: &mut Output| {
            bytes.clear();
            bytes.extend(block.iter().flat_map(|s| s.to_le_bytes()));
            block.clear();
            out.write(&bytes)
        };
        read_span(&src, rate, frame_bytes, frames, &pass, |pcm| {
            samples.clear();
            samples.extend(floats(pcm));
            limiter.process(&samples, &mut block);
            emit(&mut block, &mut out)
        })?;
        limiter.finish(&mut block);
        emit(&mut block, &mut out)?;
    } else {
        read_span(&src, rate, frame_bytes, frames, &pass, |pcm| out.write(pcm))?;
    }
    if !out.written() {
        return Err(MfError::NoAudio);
    }
    out.finish()
}

/// The header of a 16-bit PCM WAV file with `data_bytes` of audio.
pub fn wav_header(rate: u32, channels: u32, data_bytes: u32) -> [u8; 44] {
    let block = (channels * 2) as u16;
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes());
    h[22..24].copy_from_slice(&(channels as u16).to_le_bytes());
    h[24..28].copy_from_slice(&rate.to_le_bytes());
    h[28..32].copy_from_slice(&(rate * block as u32).to_le_bytes());
    h[32..34].copy_from_slice(&block.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    h
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use windows::Win32::Media::MediaFoundation::{
        IMFAttributes, MF_TRANSCODE_CONTAINERTYPE, MFCreateAttributes,
    };

    /// Writes `seconds` of a 440 Hz tone at each of `amplitudes` (one audio
    /// track per amplitude), AAC at 44.1 kHz stereo, in the container
    /// `container` (an `MFTranscodeContainerType_*`; `None` picks it from
    /// the file name). `Err` when this Windows cannot write it.
    pub(crate) fn write_aac_tracks(
        dest: &Path,
        container: Option<&GUID>,
        amplitudes: &[f32],
        seconds: u32,
    ) -> Result<(), MfError> {
        let tone = |track: usize, frame: u32| {
            (frame as f32 / 44_100.0 * 440.0 * std::f32::consts::TAU).sin() * amplitudes[track]
        };
        write_aac(dest, container, amplitudes.len(), seconds, tone)
    }

    /// `write_aac_tracks` with any signal: `signal(track, frame)`.
    pub(crate) fn write_aac(
        dest: &Path,
        container: Option<&GUID>,
        tracks: usize,
        seconds: u32,
        signal: impl Fn(usize, u32) -> f32,
    ) -> Result<(), MfError> {
        startup()?;
        let rate = 44_100;
        unsafe {
            let attributes = match container {
                Some(kind) => {
                    let mut a: Option<IMFAttributes> = None;
                    MFCreateAttributes(&mut a, 1)?;
                    let a = a.ok_or_else(|| MfError::Other("no attributes".into()))?;
                    a.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, kind)?;
                    Some(a)
                }
                None => None,
            };
            let writer =
                MFCreateSinkWriterFromURL(&HSTRING::from(dest.as_os_str()), None, attributes.as_ref())?;
            let aac = encoder_type(&MFAudioFormat_AAC, rate, 2, 128)?;
            let pcm = pcm_type(rate, 2, false)?;
            let mut streams = Vec::new();
            for _ in 0..tracks {
                let s = writer.AddStream(&aac)?;
                writer.SetInputMediaType(s, &pcm, None)?;
                streams.push(s);
            }
            writer.BeginWriting()?;
            let block = rate / 10;
            for n in 0..seconds * 10 {
                for (track, stream) in streams.iter().enumerate() {
                    let mut bytes = Vec::with_capacity(block as usize * 4);
                    for i in 0..block {
                        let v = edit::to_i16(signal(track, n * block + i));
                        bytes.extend_from_slice(&v.to_le_bytes());
                        bytes.extend_from_slice(&v.to_le_bytes());
                    }
                    let buffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
                    let mut data = std::ptr::null_mut();
                    buffer.Lock(&mut data, None, None)?;
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
                    buffer.Unlock()?;
                    buffer.SetCurrentLength(bytes.len() as u32)?;
                    let sample = MFCreateSample()?;
                    sample.AddBuffer(&buffer)?;
                    let tick = TICKS as i64 / 10;
                    sample.SetSampleTime(n as i64 * tick)?;
                    sample.SetSampleDuration(tick)?;
                    writer.WriteSample(*stream, &sample)?;
                }
            }
            writer.Finalize()?;
        }
        Ok(())
    }

    #[test]
    fn lossy_targets_use_44_or_48_khz_mono_or_stereo() {
        let mp3 = Target::Mp3 { kbps: 192 };
        assert_eq!(output_shape(mp3, 44_100, 2, false), (44_100, 2));
        assert_eq!(output_shape(mp3, 48_000, 6, false), (48_000, 2));
        assert_eq!(output_shape(mp3, 22_050, 1, false), (44_100, 1));
        assert_eq!(output_shape(mp3, 96_000, 2, false), (48_000, 2));
        assert_eq!(output_shape(Target::M4a { kbps: 128 }, 11_025, 0, false), (44_100, 1));
    }

    #[test]
    fn lossless_targets_keep_the_sample_rate() {
        assert_eq!(output_shape(Target::Flac, 22_050, 1, false), (22_050, 1));
        assert_eq!(output_shape(Target::Wav, 48_000, 8, false), (48_000, 2));
        assert_eq!(output_shape(Target::Wav, 192_000, 2, false), (96_000, 2));
    }

    #[test]
    fn mono_forces_one_channel_for_every_target() {
        for target in [Target::Mp3 { kbps: 96 }, Target::M4a { kbps: 64 }, Target::Flac, Target::Wav] {
            assert_eq!(output_shape(target, 48_000, 6, true).1, 1, "{target:?}");
            assert_eq!(output_shape(target, 44_100, 1, true).1, 1, "{target:?}");
        }
        assert_eq!(output_shape(Target::Wav, 22_050, 2, true), (22_050, 1));
    }

    #[test]
    fn permille_follows_the_position_and_stops_short_of_done() {
        assert_eq!(permille(0, 1000), 0);
        assert_eq!(permille(-5, 1000), 0);
        assert_eq!(permille(250, 1000), 250);
        assert_eq!(permille(1000, 1000), 999);
        assert_eq!(permille(5000, 1000), 999);
        assert_eq!(permille(10, 0), 0);
        // Hours of audio in 100 ns units do not overflow.
        assert_eq!(permille(18_000_000_000_000, 36_000_000_000_000), 500);
    }

    #[test]
    fn progress_reports_unknown_until_a_length_is_known() {
        let p = Progress::default();
        assert_eq!(p.fraction(), None);
        assert!(!p.started());
        p.begin(true);
        p.set(420);
        assert_eq!(p.fraction(), Some(0.42));
        assert!(p.started());
        assert!(!p.cancelled());
        p.cancel();
        assert!(p.cancelled());
    }

    #[test]
    fn info_trimmed_sizes_the_kept_part() {
        let info = Info { seconds: Some(60.0), rate: 44_100, channels: 2 };
        let trim = Trim { start: Some(10.0), end: Some(40.0) };
        assert_eq!(info.trimmed(&trim).seconds, Some(30.0));
        assert_eq!(info.trimmed(&Trim { start: Some(50.0), end: None }).seconds, Some(10.0));
        assert_eq!(info.trimmed(&Trim::NONE), info);
        // A trim that does not fit changes nothing.
        assert_eq!(info.trimmed(&Trim { start: Some(70.0), end: None }), info);
        let unknown = Info { seconds: None, ..info };
        assert_eq!(unknown.trimmed(&Trim { start: Some(5.0), end: Some(8.0) }).seconds, Some(3.0));
        assert_eq!(unknown.trimmed(&Trim { start: Some(5.0), end: None }).seconds, None);
    }

    #[test]
    fn codec_names() {
        assert_eq!(codec_name(&MFAudioFormat_AAC), Some("AAC"));
        assert_eq!(codec_name(&MFAudioFormat_Dolby_AC3), Some("Dolby Digital"));
        assert_eq!(codec_name(&MFAudioFormat_AMR_WB), Some("AMR-WB"));
        assert_eq!(codec_name(&GUID::zeroed()), None);
    }

    #[test]
    fn wav_header_layout() {
        let h = wav_header(44_100, 2, 1000);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()), 1036);
        assert_eq!(u16::from_le_bytes(h[22..24].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(h[24..28].try_into().unwrap()), 44_100);
        assert_eq!(u32::from_le_bytes(h[28..32].try_into().unwrap()), 176_400);
        assert_eq!(u32::from_le_bytes(h[40..44].try_into().unwrap()), 1000);
    }
}
