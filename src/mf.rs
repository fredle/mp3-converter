//! Reading and writing audio through Windows Media Foundation. Windows owns
//! every decoder and encoder used here (MP4/AAC, MP3, FLAC, WMA and so on),
//! so the app ships no codecs of its own.

use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use windows::Win32::Media::MediaFoundation::{
    IMFMediaType, IMFSample, IMFSinkWriter, IMFSourceReader, MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
    MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_FIRST_AUDIO_STREAM,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_VERSION, MFAudioFormat_AAC, MFAudioFormat_FLAC,
    MFAudioFormat_MP3, MFAudioFormat_PCM, MFCreateMediaType, MFCreateSinkWriterFromURL,
    MFCreateSourceReaderFromURL, MFMediaType_Audio, MFSTARTUP_LITE, MFStartup,
    MFT_ENUM_FLAG_ALL, MFTranscodeGetAudioOutputAvailableTypes,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::{GUID, HSTRING, Interface};

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
/// down to stereo so the file plays everywhere).
pub fn output_shape(target: Target, rate: u32, channels: u32) -> (u32, u32) {
    let channels = channels.clamp(1, 2);
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
/// exist; on failure nothing is left behind.
pub fn transcode(source: &Path, dest: &Path, target: Target) -> Result<(), MfError> {
    let result = transcode_inner(source, dest, target);
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

fn transcode_inner(source: &Path, dest: &Path, target: Target) -> Result<(), MfError> {
    unsafe {
        // Both are per-thread/process reference counted and safe to repeat.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_LITE)?;

        let (reader, source_rate, source_channels) = open_reader(source)?;
        let (rate, channels) = output_shape(target, source_rate, source_channels);
        // The reader decodes, resamples and mixes down to exactly this.
        let pcm = pcm_type(rate, channels)?;
        // Fails when Windows has no decoder for the track (Dolby Digital on
        // current Windows 11, for one).
        reader
            .SetCurrentMediaType(FIRST_AUDIO, None, &pcm)
            .map_err(|e| MfError::Unreadable(e.message()))?;
        reader.SetStreamSelection(FIRST_AUDIO, true)?;

        if target == Target::Wav {
            return write_wav(&reader, dest, rate, channels);
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
            writer.WriteSample(stream, &sample)?;
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
fn write_wav(reader: &IMFSourceReader, dest: &Path, rate: u32, channels: u32) -> Result<(), MfError> {
    let io = |e: std::io::Error| MfError::Other(e.to_string());
    let file = std::fs::File::create(dest).map_err(io)?;
    let mut out = std::io::BufWriter::new(file);
    out.write_all(&wav_header(rate, channels, 0)).map_err(io)?;
    let mut bytes: u64 = 0;
    while let Some(sample) = next_sample(reader)? {
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
        assert_eq!(output_shape(mp3, 44_100, 2), (44_100, 2));
        assert_eq!(output_shape(mp3, 48_000, 6), (48_000, 2));
        assert_eq!(output_shape(mp3, 22_050, 1), (44_100, 1));
        assert_eq!(output_shape(mp3, 96_000, 2), (48_000, 2));
        assert_eq!(output_shape(Target::M4a { kbps: 128 }, 11_025, 0), (44_100, 1));
    }

    #[test]
    fn lossless_targets_keep_the_sample_rate() {
        assert_eq!(output_shape(Target::Flac, 22_050, 1), (22_050, 1));
        assert_eq!(output_shape(Target::Wav, 48_000, 8), (48_000, 2));
        assert_eq!(output_shape(Target::Wav, 192_000, 2), (96_000, 2));
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
