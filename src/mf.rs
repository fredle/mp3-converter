//! Reading and writing audio through Windows Media Foundation. Windows owns
//! every decoder and encoder used here (MP4/AAC, MP3, FLAC, WMA and so on),
//! so the app ships no codecs of its own.

use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::Win32::Media::MediaFoundation::{
    IMFMediaType, IMFSample, IMFSinkWriter, IMFSourceReader, MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
    MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_PD_DURATION,
    MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_FIRST_AUDIO_STREAM,
    MF_SOURCE_READER_MEDIASOURCE, MF_SOURCE_READERF_ENDOFSTREAM, MF_VERSION, MFAudioFormat_AAC,
    MFAudioFormat_FLAC, MFAudioFormat_MP3, MFAudioFormat_PCM, MFCreateMediaType,
    MFCreateSinkWriterFromURL, MFCreateSourceReaderFromURL, MFMediaType_Audio, MFSTARTUP_LITE,
    MFStartup, MFT_ENUM_FLAG_ALL, MFTranscodeGetAudioOutputAvailableTypes,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Variant::VT_UI8;
use windows::core::{GUID, HSTRING, Interface};

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

/// Thousandths of `duration` reached at `position` (both in 100 ns units),
/// held below 1000 until the file is really finished.
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

#[derive(Debug, PartialEq, Eq)]
pub enum MfError {
    /// The file opened but has no audio track (a silent screen recording).
    NoAudio,
    /// Windows has no reader for this file: unknown, damaged, or a format
    /// that needs a codec extension.
    Unreadable(String),
    /// Stopped on request; nothing was written.
    Cancelled,
    Other(String),
}

impl From<windows::core::Error> for MfError {
    fn from(e: windows::core::Error) -> Self {
        MfError::Other(e.message())
    }
}

const FIRST_AUDIO: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
const ALL_STREAMS: u32 = MF_SOURCE_READER_ALL_STREAMS.0 as u32;
/// MF_E_INVALIDSTREAMNUMBER: asking for the first audio stream of a file
/// that has none.
const INVALID_STREAM: u32 = 0xC00D_36B3;

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

fn pcm_type(rate: u32, channels: u32) -> windows::core::Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels)?;
        t.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, channels * 2)?;
        t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * channels * 2)?;
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

fn open_reader(source: &Path) -> Result<(IMFSourceReader, u32, u32), MfError> {
    unsafe {
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(source.as_os_str()), None)
            .map_err(|e| MfError::Unreadable(e.message()))?;
        reader.SetStreamSelection(ALL_STREAMS, false)?;
        if let Err(e) = reader.SetStreamSelection(FIRST_AUDIO, true) {
            return Err(if e.code().0 as u32 == INVALID_STREAM {
                MfError::NoAudio
            } else {
                e.into()
            });
        }
        let native = reader
            .GetNativeMediaType(FIRST_AUDIO, 0)
            .map_err(|_| MfError::NoAudio)?;
        let rate = native.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(44_100);
        let channels = native.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(2);
        Ok((reader, rate, channels))
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

/// What the window shows about a file before converting it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    /// Length in seconds, when known.
    pub seconds: Option<f64>,
    pub rate: u32,
    pub channels: u32,
}

/// Opens `source` just far enough to learn its length and audio shape.
/// `None` when Windows cannot read its audio.
pub fn probe(source: &Path) -> Option<Info> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_LITE).ok()?;
    }
    let (reader, rate, channels) = open_reader(source).ok()?;
    Some(Info {
        seconds: duration(&reader).map(|d| d as f64 / 10_000_000.0),
        rate,
        channels,
    })
}

/// Records how far `sample` reaches into a file `length` long.
fn report(progress: &Progress, sample: &IMFSample, length: Option<u64>) {
    let Some(length) = length else { return };
    unsafe {
        let start = sample.GetSampleTime().unwrap_or(0);
        let span = sample.GetSampleDuration().unwrap_or(0);
        progress.set(permille(start.saturating_add(span), length));
    }
}

/// Reads the next block of audio, or `None` at the end of the file.
fn next_sample(reader: &IMFSourceReader) -> Result<Option<IMFSample>, MfError> {
    unsafe {
        loop {
            let mut flags = 0u32;
            let mut sample: Option<IMFSample> = None;
            reader.ReadSample(FIRST_AUDIO, 0, None, Some(&mut flags), None, Some(&mut sample))?;
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

/// Converts the first audio track of `source` into `dest`. `dest` must not
/// exist; on failure or cancel nothing is left behind. `progress` is
/// updated as the audio is read and checked for a cancel between blocks.
pub fn transcode(
    source: &Path,
    dest: &Path,
    target: Target,
    mono: bool,
    progress: &Progress,
) -> Result<(), MfError> {
    let result = transcode_inner(source, dest, target, mono, progress);
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    } else {
        progress.set(1000);
    }
    result
}

fn transcode_inner(
    source: &Path,
    dest: &Path,
    target: Target,
    mono: bool,
    progress: &Progress,
) -> Result<(), MfError> {
    unsafe {
        // Both are per-thread/process reference counted and safe to repeat.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_LITE)?;

        let (reader, source_rate, source_channels) = open_reader(source)?;
        let length = duration(&reader);
        progress.begin(length.is_some());
        if progress.cancelled() {
            return Err(MfError::Cancelled);
        }
        let (rate, channels) = output_shape(target, source_rate, source_channels, mono);
        // The reader decodes, resamples and mixes down to exactly this.
        let pcm = pcm_type(rate, channels)?;
        // Fails when Windows has no decoder for the track (Dolby Digital on
        // current Windows 11, for one).
        reader
            .SetCurrentMediaType(FIRST_AUDIO, None, &pcm)
            .map_err(|e| MfError::Unreadable(e.message()))?;
        reader.SetStreamSelection(FIRST_AUDIO, true)?;

        if target == Target::Wav {
            return write_wav(&reader, dest, rate, channels, progress, length);
        }
        let out_type = match target {
            Target::Mp3 { kbps } => encoder_type(&MFAudioFormat_MP3, rate, channels, kbps)?,
            Target::M4a { kbps } => encoder_type(&MFAudioFormat_AAC, rate, channels, kbps)?,
            Target::Flac => {
                // The FLAC encoder lists no types to choose from; it takes
                // the PCM shape as given.
                let t = pcm_type(rate, channels)?;
                t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_FLAC)?;
                t
            }
            Target::Wav => unreachable!(),
        };
        let writer: IMFSinkWriter =
            MFCreateSinkWriterFromURL(&HSTRING::from(dest.as_os_str()), None, None)?;
        let stream = writer.AddStream(&out_type)?;
        writer.SetInputMediaType(stream, &pcm, None)?;
        writer.BeginWriting()?;
        let mut wrote = false;
        while let Some(sample) = next_sample(&reader)? {
            if progress.cancelled() {
                return Err(MfError::Cancelled);
            }
            writer.WriteSample(stream, &sample)?;
            report(progress, &sample, length);
            wrote = true;
        }
        if !wrote {
            return Err(MfError::NoAudio);
        }
        writer.Finalize()?;
        Ok(())
    }
}

/// WAV is simple enough to write directly: a 44-byte header, then the PCM.
fn write_wav(
    reader: &IMFSourceReader,
    dest: &Path,
    rate: u32,
    channels: u32,
    progress: &Progress,
    length: Option<u64>,
) -> Result<(), MfError> {
    let io = |e: std::io::Error| MfError::Other(e.to_string());
    let file = std::fs::File::create(dest).map_err(io)?;
    let mut out = std::io::BufWriter::new(file);
    out.write_all(&wav_header(rate, channels, 0)).map_err(io)?;
    let mut bytes: u64 = 0;
    while let Some(sample) = next_sample(reader)? {
        if progress.cancelled() {
            return Err(MfError::Cancelled);
        }
        report(progress, &sample, length);
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer()?;
            let mut data = std::ptr::null_mut();
            let mut len = 0u32;
            buffer.Lock(&mut data, None, Some(&mut len))?;
            let written = out.write_all(std::slice::from_raw_parts(data, len as usize));
            buffer.Unlock()?;
            written.map_err(io)?;
            bytes += len as u64;
        }
    }
    if bytes == 0 {
        return Err(MfError::NoAudio);
    }
    // A WAV file holds at most 4 GB (over six hours of CD-quality stereo).
    let size = u32::try_from(bytes)
        .ok()
        .filter(|b| *b <= u32::MAX - 36)
        .ok_or_else(|| MfError::Other("Too long for a WAV file".to_owned()))?;
    out.seek(SeekFrom::Start(0)).map_err(io)?;
    out.write_all(&wav_header(rate, channels, size)).map_err(io)?;
    out.flush().map_err(io)
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
mod tests {
    use super::*;

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
