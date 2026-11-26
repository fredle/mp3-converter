# MP4 to MP3 & Audio Converter

Convert video and audio files to MP3, M4A, WAV or FLAC, offline.

Sometimes you have a video and only want the sound, or audio in a format a
phone, car or editor won't play. Drop the files in, pick MP3, M4A, WAV or
FLAC, and press Convert. Nothing is uploaded: files never leave the PC.

## What it does

- Reads video (MP4, M4V, MOV, 3GP, 3G2, AVI, WMV, ASF, MKV, WebM, MPG/MPEG,
  VOB, MOD, TS, M2T, TOD) and audio (MP3, MPA, M4A, M4B, M4R, AAC, ADT,
  WAV, FLAC, WMA, AMR, MKA, OGG, OGA, Opus). Drop files or whole folders on
  the window, or use **Add files** / **Add folder**. The types come from one
  list in `convert.rs`, which the file picker, drag and drop, folder
  scanning, the empty window and the Explorer registration all use.
- In Explorer, right-click any of those files (or many at once) and choose
  **Convert with MP4 to MP3 & Audio Converter**, or use **Open with**. It is
  registered for the current user only and never changes the default app.
  Many files end up in one window: each later launch hands its files to the
  first window and exits (`shell.rs`); files that arrive during a run join
  the queue when it ends.
- Each row has a chevron that opens the file's own choices under its name:
  **Start** and **End** (as `90`, `1:30` or `1:02:03`), the **audio track**
  when the file has more than one, and **Make louder**. A closed row notes
  them in its details (`0:30 to 2:15 · Track 2 · Louder`), and a time that
  does not fit the file is explained in place; that file waits until it is
  fixed. Track lists are read in the background the first time a row opens.
- Writes MP3 (128, 192, 256 or 320 kbps), M4A (AAC at 96, 128, 160 or
  192 kbps), WAV or FLAC. The bitrates are shown as Smaller, Standard, High
  and Best with the kbps next to each, and the options show roughly how big
  the files about to be converted will be (bitrate times length for MP3 and
  M4A, the exact size for WAV; FLAC depends on the sound, so it shows none).
- **Make louder** next to Mono applies to every file without a choice of
  its own, and the **Voice** preset turns it on (with mono MP3 at the
  Standard step). **Music** and **Smallest** are the other presets.
- **Mono (voice)** writes one channel in any format, for lectures and voice
  notes. MP3 and M4A then use mono bitrates (MP3 64, 96, 112 or 128 kbps;
  M4A 48, 64, 96 or 128 kbps), because Windows' MP3 encoder stops at
  128 kbps for mono.
- Each row shows the file's length and size, and after converting the new
  size (`3:42 · 48.1 MB → 5.1 MB`). Lengths are read in the background, so
  adding a folder of hundreds of files does not hold up the window.
- Shows progress per file and for the whole run. **Cancel** (or Esc) stops
  it: files being converted stop and their partial output is deleted, files
  not yet started go back to waiting.
- Copies title, artist, album artist, album, year, track number and genre
  to MP3, M4A and FLAC files, through the same Windows property handlers
  Explorer uses. This is best effort: if Windows cannot read or write a tag
  for a format, the file is converted without it. WAV output gets no tags.
- Finished rows have Play and Show in folder; a failed row can be retried
  on its own. After a run, a line sums it up (`3 converted · 212 MB → 18 MB`)
  next to Open folder and Clear finished, and the taskbar button flashes if
  the window is in the background.
- Keyboard: Ctrl+O adds files, Enter converts, Esc cancels a run or closes
  the unlock dialog.
- Takes the first audio track of a video (in the file's own order), or a
  chosen one: `mf::audio_tracks` lists each with its language, name, codec,
  channels and sample rate. A video with no sound is reported as "No sound
  in this file".
- Trims (an optional start and end per file), cut to the exact sample: the
  same samples as in a conversion of the whole file. Formats whose
  timestamps are exact after a seek (MP4, MKV, FLAC, WAV, AMR, MPEG
  program and transport streams, AVI) seek to just before the start; bare
  MP3 and ADTS streams and WMA/WMV are decoded from the beginning instead,
  because Windows' timestamps after a seek there were measured to be up to
  1200 samples out.
- Volume: Normalise (peak normalisation to -1 dBFS in two passes, at most
  +30 dB) or a fixed change of up to ±30 dB through a 5 ms look-ahead
  limiter. Peak normalisation was chosen over loudness (LUFS) matching
  because it can never clip or need limiting and changes nothing but the
  level; a recording with one loud click is lifted less, which is the safe
  way to be wrong. The sound is processed as 32-bit float until the final
  16-bit samples.
- Copies cover art (JPEG or PNG) from MP3, MP4/M4A and FLAC sources to MP3,
  M4A and FLAC output. Windows' property handlers can show art but not write
  it, so `tags::art` writes the ID3 APIC frame, the MP4 `covr` item and the
  FLAC PICTURE block itself, using existing padding where it can.
- Mixes anything with more than two channels down to stereo. MP3 and M4A are
  written at 44.1 or 48 kHz; WAV and FLAC keep the source's sample rate.
- Saves next to the original or into a folder you choose. It never overwrites
  a file: a taken name becomes `name (2).mp3`. The new file keeps the
  original's modified date.
- Converts several files at once.

## Formats come from Windows

The app ships no codecs. It reads and writes everything through Windows Media
Foundation, so what it can open is what Windows can decode:

| Format | What Windows needs |
|---|---|
| MP4, MOV, MKV, AVI, WMV, 3GP, MPG, VOB, TS, MP3, M4A, AAC, WAV, FLAC, WMA, AMR (narrow-band) | nothing, built in |
| MTS, M2TS (camcorders, Blu-ray) | opens, but their usual Dolby Digital or Blu-ray LPCM sound cannot be decoded |
| WebM | usually nothing; if Windows cannot read it, Web Media Extensions |
| OGG, OGA, Opus | Web Media Extensions, free in the Microsoft Store |
| Dolby Digital (AC-3) sound | not supported: current versions of Windows no longer include the decoder |
| AMR-WB (wide-band) | not supported: Windows reads the file but has no decoder |

If the extension is missing, the row says "Needs a Windows extension" and the
app links to its Store page. Install it, then press **Try again**. A file
Windows cannot decode at all is listed with Windows' own explanation.

## Languages

The window follows Windows' display language, and the picker in Settings
(the gear in the header) overrides it: English, German, French, Spanish, Italian, Portuguese, Dutch,
Polish, Turkish, Russian, Japanese, Korean, and Simplified and Traditional
Chinese. All text is in `src/i18n.rs`; a test checks that every language has
every text with the same placeholders. Japanese, Korean and Chinese are drawn
with Windows' own fonts, loaded only when needed. Right-to-left and
complex-script languages are not offered because egui cannot shape them.
The licence server's pages (`server/src/i18n.ts`) use the same languages,
chosen from the browser's `Accept-Language`.

## Free and unlocked

Every feature is free. A free run converts three files at a time, and a
one-off unlock (£4.99, at https://mp3.leatham.net/buy) removes the limit. A
licence key is the buyer's email signed by the licence server
(`MAC1.<email>.<signature>`) and is checked offline. If `BUY_URL` in
`src/license.rs` is emptied, there is no limit at all.

## Install

- Microsoft Store: search for "MP4 to MP3 & Audio Converter".
- Or download the installer from
  [GitHub Releases](https://github.com/fredle/mp3-converter/releases).

It installs per user, needs no admin rights and updates itself.

## Data and privacy

- Settings (output format, bitrate, mono, Make louder, folder, licence key)
  are in
  `%LOCALAPPDATA%\Mp3Converter\settings.json`.
- No telemetry and no accounts. The only network use is the update check
  against GitHub Releases, about once a day, which can be turned off in
  Settings. Buying an unlock opens your browser; the app itself sends nothing.

## The licence server

`server/` is a small Node service with no database (see `server/README.md`):

- `GET /buy` redirects to the Stripe Payment Link.
- `GET /thanks?session_id=…` checks with Stripe that the checkout was paid and
  shows the licence key for the buyer's email.

The signing key is `server/deploy/secrets/license-signing-key.pem`. It is
git-ignored and never deployed by the factory: copy it by hand to
`/srv/apps/mp3-converter/deploy/secrets/` on the host and keep a backup,
because the app only accepts keys signed by it.

## Development

```text
cargo build --release
cargo test
```

`HWC_THEME=light` or `HWC_THEME=dark` forces a look without changing
Windows' setting. The colours are in `src/theme.rs`, where a test checks
that every text colour reaches a 4.5:1 contrast on what it is drawn on, in
both looks.

Two tests run only when given a real input:
`MAC_SAMPLES=<folder of media files>` converts every file in it through
Windows' codecs, and `HWC_LICENSE_KEY=<key for buyer@example.com>` checks a
server-issued key.

Store screenshots: `MAC_SHOTS=<folder>` makes the app draw three made-up
scenes (ready, converted, empty), write each as a raw `<n>_<w>x<h>.rgba`
frame into the folder and close, without converting anything or saving
settings. `MAC_SHOTS_LANG=<locale>` picks the language and
`MAC_SHOTS_VERSION` the version shown. For design review,
`MAC_SHOTS_ALL=1` adds seven more scenes (files dragged over, more files
than a free run takes, converting, Settings open, failures, keyboard focus,
a row opened with its own trim, two audio tracks and Make louder),
`MAC_SHOTS_MIN=1` uses the smallest window and `MAC_SHOTS_SCALE=1.5` draws at
150% (on a display at 100%).
