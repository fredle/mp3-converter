# MP4 to MP3 & Audio Converter

Convert video and audio files to MP3, M4A, WAV or FLAC, offline.

iPhone photos arrive as HEIC and web images as WebP or AVIF, which many
Windows apps, websites and printers reject. Drop them in, pick JPG, PNG or
WebP, and press Convert. Nothing is uploaded: images never leave the PC.

## What it does

- Reads HEIC/HEIF, AVIF, JXL, WebP, JPG, PNG, BMP, TIFF and GIF. Drop files or
  whole folders on the window, or use **Add photos** / **Add folder**.
- Writes JPG (quality 40-100), PNG or lossless WebP.
- Optionally shrinks each image so its longest side fits a pixel limit.
- Saves next to the original or into a folder you choose. It never overwrites
  a file: a taken name becomes `name (2).jpg`.
- Keeps the photo's orientation and its modified date.

## HEIC, AVIF and JXL need Windows' own codecs

The app ships no decoders for these formats; it reads them through Windows.
If a codec is missing, the row says "Needs a Windows extension" and the app
links to the right Microsoft Store page:

| Format | Extension | Price |
|---|---|---|
| HEIC/HEIF | HEIF Image Extension and HEVC Video Extensions | free and about £0.79 |
| AVIF | AV1 Video Extension | free |
| JXL | JPEG XL Image Extension | free |

Many PCs already have them. Install, then press **Try again**.

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

Every feature is free. A free run converts three photos at a time, and a
one-off unlock (£4.99, at https://mp3.leatham.net/buy) removes the limit. A licence key is the buyer's email signed by the
licence server and is checked offline. If `BUY_URL` in `src/license.rs` is emptied, there is no
limit at all.

## Install

- Microsoft Store: search for "MP4 to MP3 & Audio Converter".
- Or download the installer from
  [GitHub Releases](https://github.com/fredle/mp3-converter/releases).

It installs per user, needs no admin rights and updates itself.

## Data and privacy

- Settings (output format, folder, licence key) are in
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

Two tests run only when given a real input:
`HWC_HEIC_SAMPLE=<path to a .heic>` converts it through Windows' codec, and
`HWC_LICENSE_KEY=<key for buyer@example.com>` checks a server-issued key.
