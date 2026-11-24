# MP4 to MP3 & Audio Converter

Convert video and audio files to MP3, M4A, WAV or FLAC, offline.

Sometimes you have a video and only want the sound, or audio in a format a
phone, car or editor won't play. Drop the files in, pick MP3, M4A, WAV or
FLAC, and press Convert. Nothing is uploaded: files never leave the PC.

## What it does

- Reads video (MP4, M4V, MOV, 3GP, 3G2, AVI, WMV, ASF, MKV, WebM) and audio
  (MP3, M4A, M4B, AAC, WAV, FLAC, WMA, OGG, OGA, Opus). Drop files or whole
  folders on the window, or use **Add files** / **Add folder**.
- Writes MP3 (128, 192, 256 or 320 kbps), M4A (AAC at 96, 128, 160 or
  192 kbps), WAV or FLAC. The bitrates are shown as Smaller, Standard, High
  and Best with the kbps next to each, and the options show roughly how big
  the files about to be converted will be (bitrate times length for MP3 and
  M4A, the exact size for WAV; FLAC depends on the sound, so it shows none).
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
- Takes the first audio track of a video. A video with no sound is reported
  as "No sound in this file".
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
| MP4, MOV, MKV, AVI, WMV, 3GP, MP3, M4A, AAC, WAV, FLAC, WMA | nothing, built in |
| WebM | usually nothing; if Windows cannot read it, Web Media Extensions |
| OGG, OGA, Opus | Web Media Extensions, free in the Microsoft Store |
| Dolby Digital (AC-3) sound | not supported: current versions of Windows no longer include the decoder |

If the extension is missing, the row says "Needs a Windows extension" and the
app links to its Store page. Install it, then press **Try again**. A file
Windows cannot decode at all is listed with Windows' own explanation.

## Languages

The window follows Windows' display language, and the picker in the footer
overrides it: English, German, French, Spanish, Italian, Portuguese, Dutch,
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

- Settings (output format, bitrate, mono, folder, licence key) are in
  `%LOCALAPPDATA%\Mp3Converter\settings.json`.
- No telemetry and no accounts. The only network use is the update check
  against GitHub Releases, about once a day, which can be turned off in the
  footer. Buying an unlock opens your browser; the app itself sends nothing.

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
`MAC_SHOTS_VERSION` the version shown.
