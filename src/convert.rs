//! Reading, resizing and writing images. Everything here is plain functions
//! so the worker threads and the tests can share it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageDecoder, ImageReader};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Format {
    #[default]
    Jpg,
    Png,
    Webp,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Jpg, Format::Png, Format::Webp];

    pub fn label(self) -> &'static str {
        match self {
            Format::Jpg => "JPG",
            Format::Png => "PNG",
            Format::Webp => "WebP",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpg => "jpg",
            Format::Png => "png",
            Format::Webp => "webp",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub format: Format,
    /// JPG quality, 1-100.
    pub quality: u8,
    /// Longest edge in pixels; larger images are scaled down to fit.
    pub max_edge: Option<u32>,
    /// Where to write; `None` means next to each source file.
    pub out_dir: Option<PathBuf>,
}

/// A Store codec extension Windows needs before it can read a format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    Heic,
    Avif,
    Jxl,
}

impl Codec {
    /// Store pages to open, as (Microsoft's product name, Store product id,
    /// whether it is free). The names are product names, so not translated.
    pub fn store_products(self) -> &'static [(&'static str, &'static str, bool)] {
        match self {
            Codec::Heic => &[
                ("HEIF Image Extension", "9PMMSR1CGPWG", true),
                ("HEVC Video Extensions", "9NMZLZ57R3T7", false),
            ],
            Codec::Avif => &[("AV1 Video Extension", "9MVZQVXJBQ9V", true)],
            Codec::Jxl => &[("JPEG XL Image Extension", "9MZPRTH5C0TB", true)],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConvertError {
    MissingCodec(Codec),
    Other(String),
}

/// Formats the `image` crate reads itself, without Windows codecs.
const NATIVE: [&str; 8] = ["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff", "gif"];
/// Formats read through Windows' codecs.
const VIA_WINDOWS: [&str; 6] = ["heic", "heif", "hif", "avif", "jxl", "jfif"];

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

pub fn is_supported(path: &Path) -> bool {
    let ext = extension_of(path);
    NATIVE.contains(&ext.as_str()) || VIA_WINDOWS.contains(&ext.as_str())
}

fn codec_for(ext: &str) -> Option<Codec> {
    match ext {
        "heic" | "heif" | "hif" => Some(Codec::Heic),
        "avif" => Some(Codec::Avif),
        "jxl" => Some(Codec::Jxl),
        _ => None,
    }
}

/// Expands folders (recursively), keeps supported images, and drops
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

/// Size after fitting the longest edge into `max_edge`. Never enlarges.
pub fn target_size(width: u32, height: u32, max_edge: Option<u32>) -> (u32, u32) {
    let Some(max) = max_edge.filter(|m| *m > 0) else {
        return (width, height);
    };
    let longest = width.max(height);
    if longest <= max {
        return (width, height);
    }
    let scale = max as f64 / longest as f64;
    let fit = |v: u32| ((v as f64 * scale).round() as u32).max(1);
    (fit(width), fit(height))
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
        .unwrap_or("image");
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

fn decode_native(path: &Path) -> Result<DynamicImage, String> {
    let reader = ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    Ok(image)
}

pub fn decode(path: &Path) -> Result<DynamicImage, ConvertError> {
    let ext = extension_of(path);
    if NATIVE.contains(&ext.as_str()) {
        // A file with the wrong extension (a HEIC renamed .jpg is common)
        // still gets a chance through Windows.
        return decode_native(path).or_else(|native_error| {
            crate::wic::decode(path)
                .map(DynamicImage::ImageRgba8)
                .map_err(|_| ConvertError::Other(native_error))
        });
    }
    match crate::wic::decode(path) {
        Ok(image) => Ok(DynamicImage::ImageRgba8(image)),
        Err(e) if e.missing_codec => match codec_for(&ext) {
            // Windows gives the same answer for a damaged file as for a
            // missing codec; only send people to the Store for a real one.
            Some(codec) if has_container_signature(path, codec) => {
                Err(ConvertError::MissingCodec(codec))
            }
            Some(_) => Err(ConvertError::Other(format!(
                "Not a valid {} file",
                ext.to_ascii_uppercase()
            ))),
            None => Err(ConvertError::Other(e.message)),
        },
        Err(e) => Err(ConvertError::Other(e.message)),
    }
}

/// Whether the file starts like the format its extension claims.
fn has_container_signature(path: &Path, codec: Codec) -> bool {
    use std::io::Read;
    let mut head = [0u8; 12];
    let read = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .unwrap_or(0);
    is_container_signature(&head[..read], codec)
}

fn is_container_signature(head: &[u8], codec: Codec) -> bool {
    match codec {
        // HEIC and AVIF are ISO media files: a box size, then "ftyp".
        Codec::Heic | Codec::Avif => head.len() >= 8 && &head[4..8] == b"ftyp",
        // JPEG XL is either a bare codestream or its own container.
        Codec::Jxl => {
            head.starts_with(&[0xFF, 0x0A])
                || head.starts_with(&[0, 0, 0, 0x0C, b'J', b'X', b'L', b' '])
        }
    }
}

/// JPG has no transparency, so transparent pixels are put on white rather
/// than turning black.
fn flatten_on_white(image: &DynamicImage) -> image::RgbImage {
    if !image.color().has_alpha() {
        return image.to_rgb8();
    }
    let rgba = image.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (src, dst) in rgba.pixels().zip(rgb.pixels_mut()) {
        let alpha = src[3] as u32;
        for c in 0..3 {
            dst[c] = ((src[c] as u32 * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        }
    }
    rgb
}

pub fn encode(image: &DynamicImage, path: &Path, options: &Options) -> Result<(), String> {
    use image::ImageEncoder;
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut writer = std::io::BufWriter::new(file);
    let result = match options.format {
        Format::Jpg => {
            let rgb = flatten_on_white(image);
            image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut writer,
                options.quality.clamp(1, 100),
            )
            .write_image(
                &rgb,
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
        }
        Format::Png => {
            let rgba = image.to_rgba8();
            image::codecs::png::PngEncoder::new(&mut writer).write_image(
                &rgba,
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
        }
        Format::Webp => {
            let rgba = image.to_rgba8();
            image::codecs::webp::WebPEncoder::new_lossless(&mut writer).write_image(
                &rgba,
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
        }
    };
    use std::io::Write;
    let result = result
        .map_err(|e| e.to_string())
        .and_then(|()| writer.flush().map_err(|e| e.to_string()));
    if result.is_err() {
        drop(writer);
        let _ = std::fs::remove_file(path);
    }
    result
}

/// Converts one file and returns where it was written.
pub fn convert_file(source: &Path, options: &Options) -> Result<PathBuf, ConvertError> {
    let mut image = decode(source)?;
    let (width, height) = target_size(image.width(), image.height(), options.max_edge);
    if (width, height) != (image.width(), image.height()) {
        image = image.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
    }
    if let Some(dir) = &options.out_dir {
        std::fs::create_dir_all(dir).map_err(|e| ConvertError::Other(e.to_string()))?;
    }
    let target = output_path(source, options, |p| p.exists());
    encode(&image, &target, options).map_err(ConvertError::Other)?;
    // Keep the photo's date so sorting by date still works.
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
            quality: 90,
            max_edge: None,
            out_dir: None,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hwc-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn supported_extensions_ignore_case() {
        assert!(is_supported(Path::new("IMG_0001.HEIC")));
        assert!(is_supported(Path::new("a.webp")));
        assert!(is_supported(Path::new("a.Avif")));
        assert!(!is_supported(Path::new("notes.txt")));
        assert!(!is_supported(Path::new("noextension")));
    }

    #[test]
    fn target_size_fits_longest_edge_and_never_enlarges() {
        assert_eq!(target_size(4000, 3000, Some(2000)), (2000, 1500));
        assert_eq!(target_size(3000, 4000, Some(2000)), (1500, 2000));
        assert_eq!(target_size(800, 600, Some(2000)), (800, 600));
        assert_eq!(target_size(800, 600, None), (800, 600));
        assert_eq!(target_size(800, 600, Some(0)), (800, 600));
        assert_eq!(target_size(10000, 1, Some(100)), (100, 1));
    }

    #[test]
    fn output_path_avoids_existing_files() {
        let src = Path::new("C:/photos/IMG_1.heic");
        let opts = options(Format::Jpg);
        assert_eq!(
            output_path(src, &opts, |_| false),
            PathBuf::from("C:/photos/IMG_1.jpg")
        );
        let taken = [
            PathBuf::from("C:/photos/IMG_1.jpg"),
            PathBuf::from("C:/photos/IMG_1 (2).jpg"),
        ];
        assert_eq!(
            output_path(src, &opts, |p| taken.contains(&p.to_path_buf())),
            PathBuf::from("C:/photos/IMG_1 (3).jpg")
        );
    }

    #[test]
    fn output_path_uses_chosen_folder() {
        let mut opts = options(Format::Png);
        opts.out_dir = Some(PathBuf::from("D:/out"));
        assert_eq!(
            output_path(Path::new("C:/photos/a.webp"), &opts, |_| false),
            PathBuf::from("D:/out/a.png")
        );
    }

    #[test]
    fn converts_png_to_jpg_with_resize_and_keeps_source() {
        let dir = temp_dir("png-jpg");
        let src = dir.join("in.png");
        image::RgbaImage::from_pixel(400, 200, image::Rgba([10, 200, 30, 255]))
            .save(&src)
            .unwrap();
        let mut opts = options(Format::Jpg);
        opts.max_edge = Some(100);
        let out = convert_file(&src, &opts).unwrap();
        assert_eq!(out, dir.join("in.jpg"));
        let converted = image::open(&out).unwrap();
        assert_eq!((converted.width(), converted.height()), (100, 50));
        assert!(src.exists());
        // A second run must not overwrite the first result.
        assert_eq!(convert_file(&src, &opts).unwrap(), dir.join("in (2).jpg"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_format_conversion_does_not_overwrite_the_source() {
        let dir = temp_dir("same");
        let src = dir.join("in.png");
        image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]))
            .save(&src)
            .unwrap();
        let out = convert_file(&src, &options(Format::Png)).unwrap();
        assert_eq!(out, dir.join("in (2).png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transparent_pixels_become_white_in_jpg() {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([0, 0, 0, 0]),
        ));
        let rgb = flatten_on_white(&image);
        assert_eq!(rgb.get_pixel(0, 0).0, [255, 255, 255]);
        let opaque = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([10, 20, 30, 255]),
        ));
        assert_eq!(flatten_on_white(&opaque).get_pixel(0, 0).0, [10, 20, 30]);
    }

    #[test]
    fn writes_webp_and_png() {
        let dir = temp_dir("formats");
        let src = dir.join("in.bmp");
        image::RgbImage::from_pixel(16, 9, image::Rgb([200, 100, 50]))
            .save(&src)
            .unwrap();
        for format in [Format::Webp, Format::Png] {
            let out = convert_file(&src, &options(format)).unwrap();
            let converted = image::open(&out).unwrap();
            assert_eq!((converted.width(), converted.height()), (16, 9));
            assert_eq!(converted.to_rgb8().get_pixel(3, 3).0, [200, 100, 50]);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collects_folders_and_skips_duplicates_and_other_files() {
        let dir = temp_dir("collect");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        for name in ["a.jpg", "b.txt", "sub/c.heic"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let a = dir.join("a.jpg");
        let found = collect_inputs(&[dir.clone(), a.clone()], &[]);
        assert_eq!(found, vec![a.clone(), sub.join("c.heic")]);
        assert_eq!(collect_inputs(&[dir.clone()], &[a]), vec![sub.join("c.heic")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_through_windows_codecs() {
        // .jfif is routed to Windows' own JPEG decoder, which every Windows
        // install has, so this exercises the same path HEIC takes.
        let dir = temp_dir("wic");
        let jpg = dir.join("photo.jpg");
        image::RgbImage::from_pixel(64, 48, image::Rgb([250, 250, 250]))
            .save(&jpg)
            .unwrap();
        let src = dir.join("photo.jfif");
        std::fs::rename(&jpg, &src).unwrap();
        let out = convert_file(&src, &options(Format::Png)).unwrap();
        let converted = image::open(&out).unwrap().to_rgb8();
        assert_eq!((converted.width(), converted.height()), (64, 48));
        assert!(converted.get_pixel(10, 10).0.iter().all(|c| *c >= 245));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_heic_is_an_error_not_a_panic() {
        let dir = temp_dir("badheic");
        let src = dir.join("broken.heic");
        std::fs::write(&src, b"not an image").unwrap();
        assert_eq!(
            convert_file(&src, &options(Format::Jpg)),
            Err(ConvertError::Other("Not a valid HEIC file".to_owned()))
        );
        assert!(!dir.join("broken.jpg").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Runs only when HWC_HEIC_SAMPLE points at a real HEIC file, since the
    /// build machine may not have the HEVC extension installed.
    #[test]
    fn converts_a_real_heic_when_a_sample_is_provided() {
        let Ok(sample) = std::env::var("HWC_HEIC_SAMPLE") else {
            return;
        };
        let dir = temp_dir("realheic");
        let src = dir.join("IMG_3001.heic");
        std::fs::copy(&sample, &src).unwrap();
        let out = convert_file(&src, &options(Format::Jpg)).unwrap();
        assert_eq!(out, dir.join("IMG_3001.jpg"));
        let converted = image::open(&out).unwrap();
        assert!(converted.width() > 0 && converted.height() > 0);
        println!("heic -> jpg {}x{}", converted.width(), converted.height());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn container_signatures() {
        let heic = [0, 0, 0, 0x18, b'f', b't', b'y', b'p', b'h', b'e', b'i', b'c'];
        assert!(is_container_signature(&heic, Codec::Heic));
        assert!(is_container_signature(&heic, Codec::Avif));
        assert!(!is_container_signature(b"not an image", Codec::Heic));
        assert!(!is_container_signature(&[], Codec::Heic));
        assert!(is_container_signature(&[0xFF, 0x0A, 1], Codec::Jxl));
        assert!(!is_container_signature(&heic, Codec::Jxl));
    }

    #[test]
    fn unreadable_file_reports_an_error() {
        let dir = temp_dir("bad");
        let src = dir.join("broken.png");
        std::fs::write(&src, b"not an image").unwrap();
        assert!(matches!(
            convert_file(&src, &options(Format::Jpg)),
            Err(ConvertError::Other(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
