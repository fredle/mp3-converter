# MP4 to MP3 & Audio Converter

Convert video and audio files to MP3, M4A, WAV or FLAC, offline.

## Problem

People often have a video and only want its sound (a lecture, a song, a voice note, a meeting recording), or have audio in a format their phone, car or editor won't play. The usual answers are websites that want the file uploaded, or large tools that bundle their own codecs and ask for payment before converting anything.

## Features (0.2)

- Drag and drop files or folders, or pick them, to queue video (MP4, M4V, MOV, 3GP, 3G2, AVI, WMV, ASF, MKV, WebM, MPG/MPEG, VOB, MOD, TS, MTS/M2TS, M2T, TOD) and audio (MP3, MPA, M4A, M4B, M4R, AAC, ADT, WAV, FLAC, WMA, AMR, MKA, OGG, OGA, Opus) files. Every one is a container Windows reads itself; whether the sound inside decodes depends on its codec (camcorder MTS/M2TS usually carries Dolby Digital, which does not)
- Each file in the queue shows its length and size, and once converted the new size; finished files can be played or shown in Explorer, failed ones retried on their own
- Convert to MP3 (128, 192, 256 or 320 kbps), M4A (96, 128, 160 or 192 kbps), WAV or FLAC. The four bitrates are labelled Smaller, Standard, High and Best, and the expected total size is shown for MP3, M4A and WAV
- Mono (voice): one channel for any format, at the same four quality steps with mono bitrates (MP3 64-128 kbps, M4A 48-128 kbps)
- Real progress per file and for the whole run, and Cancel: files being converted stop and leave nothing behind, files not started go back to waiting
- Title, artist, album artist, album, year, track number and genre are copied to MP3, M4A and FLAC output where Windows can read and write them (best effort; WAV gets none)
- Cover art (a JPEG or PNG embedded in an MP3, MP4/M4A or FLAC source) is carried to MP3, M4A and FLAC output, which Explorer then shows. Best effort: a picture that cannot be read or written never fails the conversion
- Trim: an optional start and end time per file; only that part is converted, cut to the exact sample (the same samples as in a conversion of the whole file). Times past the end mean "to the end"; a start at or past the end, or an end before the start, is refused. Progress and the size estimate cover only the kept part
- Audio track: files with several audio tracks (languages, commentary, separate microphone) list them with language, name, codec, channels and sample rate, in the order the file has them, and any one can be converted. The first is the default
- Volume: unchanged (the default), Normalise, or a fixed change of up to plus or minus 30 dB. Normalise measures the loudest moment of the (trimmed) file in a first pass and brings it to -1 dBFS, adding at most 30 dB. A boost goes through a look-ahead limiter so it never clips
- After a run: how many converted, the total size before and after, how many failed; Open folder and Clear finished. The taskbar button flashes if the window is in the background
- Keyboard: Ctrl+O adds files, Enter converts, Esc cancels
- Output next to the source or into a chosen folder, never overwriting an existing file
- Decoding and encoding go through Windows Media Foundation. OGG and Opus need Microsoft's free Web Media Extensions; when it is missing, say so and link to its Store page
- More than two channels are mixed down to stereo
- Several files are converted at once
- Free: every feature, three files per run. A one-off unlock (licence key, checked offline) removes the limit

## Non-goals

- No bundled codecs, so no Dolby Digital (AC-3) sound, which current versions of Windows no longer decode
- No joining files, no other editing beyond trimming and volume, and no tag editing (tags and cover art are only carried over)
- No video output
- No downloading from websites
- No cloud upload; files never leave the PC

## Evidence of demand

- https://apps.microsoft.com/search?query=mp4+to+mp3
